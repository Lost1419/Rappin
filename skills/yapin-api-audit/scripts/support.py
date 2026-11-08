#!/usr/bin/env python3
"""Pinned published-crate snapshots and evidence-bound feature coverage reports."""
import argparse
import copy
from datetime import datetime, timezone
import io
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import sys
import tarfile
import tempfile
import tomllib
from urllib.request import Request, urlopen

import audit

STATUSES = {'supported', 'partial', 'missing', 'unknown', 'not_applicable'}
SURFACES = ('typed', 'low_level', 'validated_bindings', 'worker', 'cli_mcp')
MAX_ARCHIVE = 16 * 1024 * 1024
MAX_SOURCE = 32 * 1024 * 1024
VERSION = re.compile(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-[0-9A-Za-z.-]+)?')


def get(url, limit):
    with urlopen(Request(url, headers={'User-Agent': 'Rappin-contract-review'}), timeout=30) as response:
        if response.status != 200:
            raise ValueError(f'{url}: unexpected HTTP {response.status}')
        data = response.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f'{url}: exceeds {limit} bytes')
    return data


def safe_path(value):
    path = PurePosixPath(value)
    if (not value or '\\' in value or path.is_absolute() or any(':' in part for part in path.parts)
            or any(part != part.rstrip(' .') for part in path.parts)
            or any(p in ('..', '.') for p in value.split('/'))):
        raise ValueError('unsafe evidence/archive path')
    return path


def publish(staging, output):
    output = Path(output)
    output.mkdir()  # Exclusive reservation; no replacement of existing snapshots.
    try:
        (output / '.incomplete').touch()
        for path in staging.iterdir():
            path.rename(output / path.name)
        (output / '.incomplete').unlink()
    except BaseException:
        shutil.rmtree(output)
        raise


def unpack(archive, staging, version):
    files = {}
    total = 0
    prefix = f'rappin-rs-{version}'
    with tarfile.open(fileobj=io.BytesIO(archive), mode='r:gz') as bundle:
        for index, member in enumerate(bundle):
            if index >= 2000:
                raise ValueError('archive has too many members')
            path = safe_path(member.name)
            if path.parts[0] != prefix:
                raise ValueError('archive has an unexpected package root')
            if member.isdir():
                continue
            if not member.isfile() or len(path.parts) < 2:
                raise ValueError('archive contains a link or special file')
            relative = PurePosixPath(*path.parts[1:]).as_posix()
            if relative in files:
                raise ValueError('archive contains duplicate paths')
            total += member.size
            if member.size > audit.LIMIT or total > MAX_SOURCE:
                raise ValueError('unpacked archive exceeds source bounds')
            with bundle.extractfile(member) as stream:
                data = stream.read(audit.LIMIT + 1)
            if len(data) != member.size:
                raise ValueError('archive member size mismatch')
            destination = staging / 'source' / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)  # Never execute or preserve archive permissions.
            files[relative] = audit.digest(data)
    if 'Cargo.toml' not in files or 'docs/openapi.json' not in files:
        raise ValueError('crate lacks the expected manifest or bundled contract')
    return files


def release(version, output):
    if not VERSION.fullmatch(version):
        raise ValueError('select an explicit three-part crate version')
    output = Path(output)
    if output.exists():
        raise ValueError('release snapshot output already exists')
    registry = f'https://crates.io/api/v1/crates/rappin-rs/{version}'
    archive_url = f'https://static.crates.io/crates/rappin-rs/rappin-rs-{version}.crate'
    metadata = json.loads(get(registry, audit.LIMIT))['version']
    if metadata['num'] != version or not re.fullmatch('[0-9a-f]{64}', metadata['checksum']):
        raise ValueError('invalid registry version/checksum response')
    archive = get(archive_url, MAX_ARCHIVE)
    if audit.digest(archive) != metadata['checksum']:
        raise ValueError('downloaded crate checksum differs from registry')
    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix='.rappin-release-', dir=output.parent))
    try:
        files = unpack(archive, staging, version)
        cargo = tomllib.loads((staging / 'source/Cargo.toml').read_text())['package']
        if cargo['name'] != 'rappin-rs' or cargo['version'] != version:
            raise ValueError('crate Cargo manifest differs from selected package/version')
        vcs_path = staging / 'source/.cargo_vcs_info.json'
        vcs = json.loads(vcs_path.read_text()).get('git', {}) if vcs_path.exists() else {}
        commit = vcs.get('sha1')
        if vcs.get('dirty') or not isinstance(commit, str) or not re.fullmatch('[0-9a-f]{40}', commit):
            commit = None
        manifest = {'format': 1, 'package': 'rappin-rs', 'version': version,
                    'crate_sha256': metadata['checksum'], 'yanked_at_fetch': metadata['yanked'],
                    'registry_url': registry, 'archive_url': archive_url,
                    'fetched_at': datetime.now(timezone.utc).isoformat(),
                    'commit': commit, 'files': files}
        (staging / 'release.json').write_text(json.dumps(manifest, indent=2) + '\n')
        publish(staging, output)
    finally:
        shutil.rmtree(staging, ignore_errors=True)
    return manifest


def load_release(root):
    root = Path(root).resolve()
    if (root / '.incomplete').exists():
        raise ValueError('release snapshot is incomplete')
    manifest = json.loads(audit.read(root / 'release.json'))
    if manifest['package'] != 'rappin-rs' or not VERSION.fullmatch(manifest['version']):
        raise ValueError('invalid release snapshot identity')
    if (not re.fullmatch('[0-9a-f]{64}', manifest['crate_sha256'])
            or not {'Cargo.toml', 'docs/openapi.json'} <= manifest['files'].keys()):
        raise ValueError('release snapshot lacks checksum or required source files')
    for name, checksum in manifest['files'].items():
        safe_path(name)
        path = root / 'source' / name
        if path.is_symlink() or any(p.is_symlink() for p in path.parents if p != root.parent):
            raise ValueError('release snapshot contains symbolic links')
        if audit.digest(audit.read(path)) != checksum:
            raise ValueError(f'{name}: release evidence checksum mismatch')
    cargo = tomllib.loads((root / 'source/Cargo.toml').read_text())['package']
    if cargo['name'] != manifest['package'] or cargo['version'] != manifest['version']:
        raise ValueError('release snapshot Cargo identity mismatch')
    return manifest


def api_identity(snapshot):
    snapshot = Path(snapshot)
    spec, evidence, provenance = audit.load(snapshot)
    if not provenance.get('sources') or set(provenance['sources']) != set(audit.SOURCES):
        raise ValueError('a complete API snapshot with source provenance is required')
    if any(provenance['sources'][name]['url'] != url for name, url in audit.SOURCES.items()):
        raise ValueError('API snapshot source URLs differ from the fixed official sources')
    return spec, {
        'version': spec['info']['version'],
        'openapi_sha256': audit.digest(audit.read(snapshot / 'openapi.json')),
        'developer_text_sha256': audit.digest(audit.visible(evidence['developers.html']).encode()),
        'sources': provenance['sources'],
    }


def report(release_root, snapshot, reviews):
    sdk = load_release(release_root)
    spec, api = api_identity(snapshot)
    review = json.loads(audit.read(reviews))
    if review.get('format') != 1 or review['sdk'].get('package') != 'rappin-rs':
        raise ValueError('unsupported review format or package')
    matches = (review['sdk']['version'] == sdk['version']
               and review['sdk']['crate_sha256'] == sdk['crate_sha256']
               and review['api']['openapi_sha256'] == api['openapi_sha256']
               and review['api']['developer_text_sha256'] == api['developer_text_sha256'])
    features = copy.deepcopy(review['features'])
    operation_reviews = copy.deepcopy(review.get('operations', []))
    keys = [(row['method'].upper(), row['path']) for row in operation_reviews]
    if len(set(keys)) != len(keys):
        raise ValueError('duplicate reviewed operations')
    if matches and any(method.lower() not in spec['paths'].get(path, {}) for method, path in keys):
        raise ValueError('review names an operation absent from the API snapshot')
    cargo_features = tomllib.loads((Path(release_root) / 'source/Cargo.toml').read_text()).get('features', {})
    if len({row['id'] for row in features}) != len(features):
        raise ValueError('duplicate reviewed feature IDs')
    for row in features + operation_reviews:
        if set(row['surfaces']) != set(SURFACES):
            raise ValueError('feature must assess each coverage surface')
        if set(row['required_features']) != set(SURFACES):
            raise ValueError('feature requirements must be recorded for each surface')
        if matches and any(feature not in cargo_features for values in row['required_features'].values()
                           for feature in values):
            raise ValueError('review names an absent Cargo feature')
        for surface in row['surfaces'].values():
            if surface['status'] not in STATUSES or not surface.get('reason'):
                raise ValueError('invalid coverage status or missing reason')
            if matches and surface['status'] in ('supported', 'partial', 'missing') and not surface.get('evidence'):
                raise ValueError('coverage claims require source evidence')
            if matches:
                for item in surface.get('evidence', []):
                    safe_path(item['path'])
                    if sdk['files'].get(item['path']) != item['sha256']:
                        raise ValueError('review evidence does not match the selected crate')
                    if item.get('symbol') and item['symbol'] not in (
                        Path(release_root) / 'source' / item['path']).read_text():
                        raise ValueError('review symbol absent from selected crate')
            else:
                surface.update({'status': 'unknown', 'reason':
                                'Saved review does not match this exact crate and API/guide snapshot.',
                                'evidence': []})
        for operation in row.get('operations', []):
            if matches and operation['method'].lower() not in spec['paths'].get(operation['path'], {}):
                raise ValueError('review names an operation absent from the API snapshot')
        if not matches:
            # Old rollout and feature requirements are not current assertions either.
            row['required_features'] = None
            row['rollout'] = 'Unknown for this evidence selection; reassess before use.'
            if 'notes' in row:
                row['notes'] = 'Unknown for this evidence selection; reassess before use.'
    operations = []
    for path, definition in sorted(spec['paths'].items()):
        for method in sorted(audit.METHODS & definition.keys()):
            associated = [r['id'] for r in features if any(
                op['path'] == path and op['method'].lower() == method for op in r.get('operations', []))]
            reviewed = next((r for r in operation_reviews if r['path'] == path
                             and r['method'].lower() == method), None)
            operation = {'path': path, 'method': method.upper(),
                         'operation_id': definition[method].get('operationId'),
                         'status': 'unknown', 'reviewed_feature_ids': associated}
            if reviewed:
                operation.update(reviewed)
                operation['method'] = method.upper()
                operation['status'] = 'reviewed' if matches else 'unknown'
            operations.append(operation)
    return {'format': 1, 'generated_at': datetime.now(timezone.utc).isoformat(),
            'sdk': {k: v for k, v in sdk.items() if k != 'files'}, 'api': api,
            'review_matches': matches, 'reviewed_at': review['reviewed_at'] if matches else None,
            'review_method': 'Human/agent source review; live authenticated behavior was not tested.',
            'features': features, 'operations': operations,
            'scope': 'Feature and explicit operation source review. Only explicit operation rows assess endpoint '
                     'coverage; generic raw requests never establish typed or validated support.'}


def markdown(data):
    def cell(value):
        return str(value).replace('|', '\\|').replace('\n', ' ')
    sdk = data['sdk']
    rows = [f'# Yapin API support: Rappin {sdk["version"]}', '',
            f'Generated: {data["generated_at"]}. Source review: {data["reviewed_at"] or "unmatched"}.', '',
            f'API {data["api"]["version"]}; exact published crate checksum `{sdk["crate_sha256"]}`.', '',
            f'[Published release](https://crates.io/crates/rappin-rs/{sdk["version"]}) · '
            '[Official API](https://www.yapin.gg/openapi.json) · '
            '[Developer guide](https://www.yapin.gg/developers)', '',
            data['review_method'], '', data['scope'], '',
            f'Inventory: {len(data["operations"])} documented operations. '
            f'{sum(row["status"] == "reviewed" for row in data["operations"])} have explicit operation reviews; '
            f'{len(data["features"])} selected features also have detailed reviews.', '',
            '| Feature | Typed SDK | Low-level requests | Validated bindings | Worker | CLI/MCP |',
            '|---|---|---|---|---|---|']
    for row in data['features']:
        rows.append('| ' + ' | '.join([cell(row['title'])] +
                    [row['surfaces'][key]['status'] for key in SURFACES]) + ' |')
    for row in data['features']:
        rows += ['', '## ' + row['title'], '', 'Availability: ' + row['rollout'], '']
        for name in SURFACES:
            surface = row['surfaces'][name]
            links = []
            for evidence in surface.get('evidence', []):
                if sdk.get('commit'):
                    links.append(f'[{evidence["path"]}](https://github.com/Lost1419/Rappin/blob/{sdk["commit"]}/{evidence["path"]})')
            required = row['required_features']
            feature_text = ', '.join('`'+v+'`' for v in required[name]) if required is not None else 'unknown'
            feature_text = feature_text or 'no optional feature'
            rows.append(f'- **{name} — {surface["status"]}:** {surface["reason"]} Cargo: {feature_text}.' +
                        (' Sources: ' + ', '.join(dict.fromkeys(links)) + '.' if links else ''))
        rows += ['', 'Platform references: ' + ', '.join(
                 f'[source {i+1}]({url})' for i, url in enumerate(row['api_sources'])) + '.']
    rows += ['', '## Operation coverage', '',
             'Supported means the reviewed surface represents this endpoint; it does not certify live '
             'access, platform rollout or undocumented fields. Partial rows name their limitations.', '',
             '| Method / path | Typed SDK | Low-level requests | Validated bindings | Worker | CLI/MCP |',
             '|---|---|---|---|---|---|']
    for row in data['operations']:
        surfaces = row.get('surfaces', {})
        rows.append('| ' + ' | '.join([cell(row['method'] + ' ' + row['path'])] +
                    [surfaces.get(key, {}).get('status', 'unknown') for key in SURFACES]) + ' |')
    for row in data['operations']:
        if 'surfaces' not in row:
            continue
        rows += ['', '### ' + row['method'] + ' ' + row['path'], '',
                 'Availability: ' + row['rollout'], '', row.get('notes', ''), '']
        for name in SURFACES:
            surface = row['surfaces'][name]
            required = row['required_features']
            flags = ', '.join(required[name]) if required is not None else 'unknown'
            links = [f'[{e["path"]}](https://github.com/Lost1419/Rappin/blob/{sdk["commit"]}/{e["path"]})'
                     for e in surface.get('evidence', []) if sdk.get('commit')]
            rows.append(f'- **{name} — {surface["status"]}:** {surface["reason"]} '
                        f'Cargo: {flags or "no optional feature"}.' +
                        (' Sources: ' + ', '.join(dict.fromkeys(links)) + '.' if links else ''))
    rows += ['', '## Evidence and freshness', '',
             f'OpenAPI SHA-256: `{data["api"]["openapi_sha256"]}`.', '',
             f'Developer text SHA-256: `{data["api"]["developer_text_sha256"]}`.', '',
             'Source capture times (UTC):', '']
    for name, source in sorted(data['api']['sources'].items()):
        rows.append(f'- [{name}]({source["url"]}): {source["fetched_at"]}; SHA-256 `{source["sha256"]}`.')
    rows += ['',
             'A matching version number alone cannot refresh this review. Changed crate bytes, '
             'OpenAPI bytes or readable guide text make saved statuses unknown until reassessed. '
             'A low-level request can still be refused by platform permissions or rollout flags.', '',
             'Reproduce with the public [audit skill](https://github.com/Lost1419/Rappin/tree/main/skills/yapin-api-audit). '
             'Its release downloader does not build or execute crate code.', '']
    return '\n'.join(rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    download = sub.add_parser('release', help='Download one exact published Rappin crate without building it')
    download.add_argument('--version', required=True)
    download.add_argument('--out', required=True)
    render = sub.add_parser('report', help='Bind reviewed coverage to an exact release and API snapshot')
    render.add_argument('release')
    render.add_argument('snapshot')
    render.add_argument('--reviews', required=True)
    render.add_argument('--json', required=True)
    render.add_argument('--markdown', required=True)
    args = parser.parse_args()
    created = []
    try:
        if args.command == 'release':
            result = release(args.version, args.out)
            print(json.dumps({k: result[k] for k in ('version', 'crate_sha256', 'fetched_at')}))
        else:
            result = report(args.release, args.snapshot, args.reviews)
            for path, content in [(Path(args.json), json.dumps(result, indent=2) + '\n'),
                                  (Path(args.markdown), markdown(result))]:
                with path.open('x', encoding='utf-8') as stream:
                    created.append(path)
                    stream.write(content)
            print(json.dumps({'review_matches': result['review_matches'],
                              'features': len(result['features']), 'operations': len(result['operations'])}))
        return 0
    except (OSError, ValueError, KeyError, TypeError, tarfile.TarError, RecursionError) as error:
        for path in created:
            path.unlink(missing_ok=True)
        print(json.dumps({'error': str(error)}), file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
