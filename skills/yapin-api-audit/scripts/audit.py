#!/usr/bin/env python3
"""Public Yapin evidence snapshots and exhaustive structural JSON comparison."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys
import tempfile
from datetime import datetime, timezone
from html.parser import HTMLParser
from urllib.request import Request, urlopen

LIMIT = 8 * 1024 * 1024
SOURCES = {
    'openapi.json': 'https://www.yapin.gg/openapi.json',
    'developers.html': 'https://www.yapin.gg/developers',
    'api.html': 'https://www.yapin.gg/developers/api',
    'version.json': 'https://www.yapin.gg/api/bot/v1/version',
    'changelog.json': 'https://www.yapin.gg/api/bot/v1/changelog?limit=10',
}
METHODS = {'get', 'put', 'post', 'delete', 'options', 'head', 'patch', 'trace'}
MISSING = object()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def read(path):
    with Path(path).open('rb') as stream:
        data = stream.read(LIMIT + 1)
    if len(data) > LIMIT:
        raise ValueError(f'{path}: exceeds {LIMIT} bytes')
    return data


def snapshot(destination):
    destination = Path(destination)
    if destination.exists():
        raise ValueError('snapshot output already exists; select a new directory')
    destination.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix='.yapin-audit-', dir=destination.parent))
    manifest = {'format': 1, 'sources': {}}
    try:
        for name, url in SOURCES.items():
            request = Request(url, headers={'User-Agent': 'Rappin-contract-review'})
            with urlopen(request, timeout=30) as response:
                if response.status != 200:
                    raise ValueError(f'{url}: unexpected HTTP {response.status}')
                data = response.read(LIMIT + 1)
            if len(data) > LIMIT:
                raise ValueError(f'{url}: exceeds {LIMIT} bytes')
            if name.endswith('.json'):
                json.loads(data)
            else:
                if '<html' not in data.decode('utf-8').lower():
                    raise ValueError(f'{url}: response is not an HTML document')
            (staging / name).write_bytes(data)
            manifest['sources'][name] = {
                'url': url, 'sha256': digest(data), 'bytes': len(data),
                'fetched_at': datetime.now(timezone.utc).isoformat(),
            }
        spec = json.loads((staging / 'openapi.json').read_bytes())
        if not isinstance(spec.get('paths'), dict) or 'openapi' not in spec:
            raise ValueError('public response is not an OpenAPI document')
        (staging / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        # mkdir is the exclusive destination reservation; never replace an existing snapshot.
        destination.mkdir()
        try:
            (destination / '.incomplete').touch()
            for path in staging.iterdir():
                path.rename(destination / path.name)
            (destination / '.incomplete').unlink()
        except BaseException:
            shutil.rmtree(destination)
            raise
    finally:
        shutil.rmtree(staging, ignore_errors=True)
    return {'snapshot': str(destination.resolve()), 'sources': manifest['sources']}


class VisibleText(HTMLParser):
    def __init__(self):
        super().__init__()
        self.hidden = 0
        self.parts = []

    def handle_starttag(self, tag, attrs):
        if tag in ('script', 'style'):
            self.hidden += 1

    def handle_endtag(self, tag):
        if tag in ('script', 'style'):
            self.hidden = max(0, self.hidden - 1)

    def handle_data(self, data):
        if not self.hidden:
            self.parts.extend(data.split())


def visible(data):
    parser = VisibleText()
    parser.feed(data.decode('utf-8'))
    return ' '.join(parser.parts)


def load(source):
    source = Path(source)
    if source.is_file():
        data = read(source)
        return json.loads(data), {}, {'file': str(source), 'sha256': digest(data),
                                      'fetched_at': None}
    if (source / '.incomplete').exists():
        raise ValueError('snapshot publication was interrupted; capture a new snapshot')
    manifest_path = source / 'manifest.json'
    provenance = json.loads(read(manifest_path)) if manifest_path.exists() else {
        'directory': str(source), 'fetched_at': None,
        'warning': 'No snapshot manifest; source/date provenance unavailable',
    }
    for name, metadata in provenance.get('sources', {}).items():
        # Only the fixed snapshot filenames are allowed, never manifest-supplied paths.
        if name not in SOURCES:
            raise ValueError(f'unknown snapshot member: {name}')
        if digest(read(source / name)) != metadata['sha256']:
            raise ValueError(f'{name}: snapshot checksum mismatch')
    evidence = {name: read(source / name) for name in SOURCES
                if name != 'openapi.json' and (source / name).exists()}
    return json.loads(read(source / 'openapi.json')), evidence, provenance


def pointer(path, name):
    return path + '/' + str(name).replace('~', '~0').replace('/', '~1')


def summary(value):
    encoded = json.dumps(value, ensure_ascii=False, sort_keys=True)
    if len(encoded) <= 1200:
        return {'value': value}
    return {'preview': encoded[:1200], 'truncated': True,
            'sha256': digest(encoded.encode())}


def differences(old, new, path=''):
    if isinstance(old, dict) and isinstance(new, dict):
        changes = []
        for key in sorted(old.keys() | new.keys()):
            changes.extend(differences(old.get(key, MISSING), new.get(key, MISSING),
                                       pointer(path, key)))
        return changes
    if isinstance(old, list) and isinstance(new, list) and len(old) == len(new):
        return [change for i, (a, b) in enumerate(zip(old, new))
                for change in differences(a, b, pointer(path, i))]
    # Recurse before equality: even nested True/1 and 1/1.0 are distinct JSON types.
    if type(old) is type(new) and old == new:
        return []
    change = {'pointer': path, 'kind': 'added' if old is MISSING else
              'removed' if new is MISSING else 'changed'}
    if old is not MISSING:
        change['old'] = summary(old)
    if new is not MISSING:
        change['new'] = summary(new)
    return [change]


def compare(baseline, current):
    old, old_evidence, old_provenance = load(baseline)
    new, new_evidence, new_provenance = load(current)
    if not all(isinstance(x, dict) and isinstance(x.get('paths'), dict)
               and 'openapi' in x for x in (old, new)):
        raise ValueError('both inputs must be OpenAPI documents')
    changes = differences(old, new)
    operations = []
    for path in sorted(old['paths'].keys() | new['paths'].keys()):
        before, after = old['paths'].get(path, {}), new['paths'].get(path, {})
        for method in sorted(METHODS & (before.keys() | after.keys())):
            a, b = before.get(method, MISSING), after.get(method, MISSING)
            if differences(a, b):
                operations.append({'path': path, 'method': method.upper(),
                                   'operation_id': (b if b is not MISSING else a).get('operationId'),
                                   'coverage': 'unassessed'})
    old_schemas = old.get('components', {}).get('schemas', {})
    new_schemas = new.get('components', {}).get('schemas', {})
    schemas = [name for name in sorted(old_schemas.keys() | new_schemas.keys())
               if differences(old_schemas.get(name, MISSING), new_schemas.get(name, MISSING))]
    evidence = {}
    for name in sorted(old_evidence.keys() | new_evidence.keys()):
        a, b = old_evidence.get(name), new_evidence.get(name)
        evidence[name] = {'comparable': a is not None and b is not None}
        if a is not None and b is not None:
            equal = visible(a) == visible(b) if name.endswith('.html') else not differences(
                json.loads(a), json.loads(b))
            evidence[name].update({'raw_equal': a == b, 'content_equal': equal})
    return {'format': 1, 'baseline': old_provenance, 'current': new_provenance,
            'versions': {'baseline': old.get('info', {}).get('version'),
                         'current': new.get('info', {}).get('version')},
            'openapi_equal': not changes, 'changes': changes,
            'changed_operations': operations, 'changed_schemas': schemas,
            'evidence': evidence,
            'coverage_note': 'Unassessed. Shared schemas require tracing transitive references; '
                             'raw request support and name matches do not prove typed coverage.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    fetch = commands.add_parser('snapshot', help='Fetch public sources into a new directory')
    fetch.add_argument('--out', required=True)
    diff = commands.add_parser('compare', help='Compare snapshot directories or OpenAPI JSON files')
    diff.add_argument('baseline')
    diff.add_argument('current')
    diff.add_argument('--output', help='Write a new JSON report (never overwrite an existing file)')
    args = parser.parse_args()
    try:
        report = snapshot(args.out) if args.command == 'snapshot' else compare(args.baseline, args.current)
        text = json.dumps(report, indent=2, ensure_ascii=False) + '\n'
        if args.command == 'compare' and args.output:
            with Path(args.output).open('x', encoding='utf-8') as stream:
                stream.write(text)
            print(json.dumps({'report': args.output, 'changes': len(report['changes']),
                              'openapi_equal': report['openapi_equal']}))
        else:
            print(text, end='')
        return 0
    except (OSError, ValueError, KeyError, TypeError, RecursionError) as error:
        print(json.dumps({'error': str(error)}), file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
