#!/usr/bin/env python3
"""Classify a fresh Yapin contract snapshot for the scheduled watch workflow.

Compares the new snapshot with the previous one (restored from the Actions
cache) and with Rappin's bundled docs/openapi.json, then writes result.json,
summary.md and issue.md to --out. Sets `changed=true` in GITHUB_OUTPUT when
the live contract changed. A new build hash on its own is not a change.

`promote --candidate DIR --baseline DIR` replaces the baseline with the
candidate only when the candidate was fetched later, so re-running an old run
can never move the cached baseline backwards.
"""
import argparse
from datetime import datetime
import json
import os
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'skills' / 'yapin-api-audit' / 'scripts'))
import audit  # noqa: E402

WATCHED_CONTENT = ('api.html', 'developers.html', 'changelog.json')
LIST_LIMIT = 40


def read_json(path):
    return json.loads(Path(path).read_text())


def fetched_at(snapshot):
    sources = read_json(Path(snapshot) / 'manifest.json').get('sources', {})
    return sources.get('version.json', {}).get('fetched_at')


def live_changes(report, old_version, new_version):
    reasons = []
    if report['changes']:
        reasons.append(f"openapi.json: {len(report['changes'])} changed entries")
    for name in WATCHED_CONTENT:
        evidence = report.get('evidence', {}).get(name)
        if evidence and evidence.get('comparable') and not evidence.get('content_equal'):
            reasons.append(f'{name}: readable content changed')
    for key in ('api', 'sdk', 'packages'):
        if old_version.get(key) != new_version.get(key):
            reasons.append(f'version.json {key}: {json.dumps(old_version.get(key))} -> {json.dumps(new_version.get(key))}')
    return reasons


def entries(report):
    return [f"{change['kind']} {change['pointer']}" for change in report['changes']]


def bullet_list(items, code=True):
    shown = [f'- `{item}`' if code else f'- {item}' for item in items[:LIST_LIMIT]]
    if len(items) > LIST_LIMIT:
        shown.append(f'- ... and {len(items) - LIST_LIMIT} more (see the run artifact)')
    return shown


def check(current, previous, bundled):
    current = Path(current)
    version = read_json(current / 'version.json')
    result = {'checked_at': fetched_at(current), 'api': version.get('api'),
              'build': version.get('build'), 'sdk': version.get('sdk'), 'reports': {}}
    if previous and (Path(previous) / 'version.json').is_file():
        report = audit.compare(previous, current)
        old_version = read_json(Path(previous) / 'version.json')
        result.update(baseline=False, previous_checked_at=fetched_at(previous),
                      previous_build=old_version.get('build'),
                      live_changes=live_changes(report, old_version, version),
                      live_changed_entries=entries(report))
        result['reports']['live'] = report
    else:
        result.update(baseline=True, live_changes=[], live_changed_entries=[])
    bundled_report = audit.compare(bundled, current)
    result['bundled_changed_entries'] = entries(bundled_report)
    result['reports']['bundled'] = bundled_report
    return result


def summary(result):
    lines = ['## Yapin API watch', '',
             f"API {result['api']}, build `{result['build']}`, SDKs `{json.dumps(result['sdk'])}`, "
             f"checked {result['checked_at']}.", '']
    if result['baseline']:
        lines.append('No previous snapshot in the cache, so this run is the new baseline.')
    elif result['live_changes']:
        lines.append(f"**Live contract changed** since {result['previous_checked_at']} "
                     f"(build `{result['previous_build']}`):")
        lines += bullet_list(result['live_changes'], code=False)
        if result['live_changed_entries']:
            lines += ['', 'Changed openapi.json entries:'] + bullet_list(result['live_changed_entries'])
    else:
        lines.append(f"No live contract change since {result['previous_checked_at']} "
                     f"(build `{result['previous_build']}`).")
    lines += ['', f"Rappin's bundled `docs/openapi.json` differs from live in "
                  f"{len(result['bundled_changed_entries'])} entries:"]
    lines += bullet_list(result['bundled_changed_entries'])
    return '\n'.join(lines) + '\n'


def issue_body(result, run_url):
    return (summary(result) + '\nFull reports are in the run artifact'
            + (f': {run_url}' if run_url else '.') + '\n\n'
            + "Next: run the audit skill, update the vault's `shared/yapin-api/contract-checks.md`, "
              'and decide whether Rappin needs a change.\n')


def snapshot_time(snapshot):
    """When the snapshot's version.json was fetched, or None if unknown."""
    try:
        stamp = fetched_at(snapshot)
        return datetime.fromisoformat(stamp) if stamp else None
    except (OSError, ValueError, TypeError):
        return None


def promote(candidate, baseline):
    """Make `candidate` the baseline if it is newer. Returns whether it did."""
    candidate, baseline = Path(candidate), Path(baseline)
    new_time = snapshot_time(candidate)
    if new_time is None:
        raise ValueError(f'{candidate}: snapshot has no readable fetched_at; not promoting it')
    old_time = snapshot_time(baseline) if baseline.exists() else None
    if old_time is not None and new_time <= old_time:
        return False
    shutil.rmtree(baseline, ignore_errors=True)
    shutil.move(str(candidate), str(baseline))
    return True


def write_github(variable, text):
    if os.environ.get(variable):
        with open(os.environ[variable], 'a', encoding='utf-8') as stream:
            stream.write(text)


def promote_main(argv):
    parser = argparse.ArgumentParser(prog='yapin_api_watch.py promote')
    parser.add_argument('--candidate', required=True)
    parser.add_argument('--baseline', required=True)
    args = parser.parse_args(argv)
    previous = snapshot_time(args.baseline)
    candidate = snapshot_time(args.candidate)
    promoted = promote(args.candidate, args.baseline)
    write_github('GITHUB_OUTPUT', f'promoted={str(promoted).lower()}\n')
    message = (f'Baseline advanced to the snapshot fetched {candidate.isoformat()}.' if promoted else
               f'Kept the newer cached baseline (fetched {previous.isoformat()}); this run\'s '
               f'snapshot (fetched {candidate.isoformat()}) is older and was not saved.')
    write_github('GITHUB_STEP_SUMMARY', f'\n{message}\n')
    print(json.dumps({'promoted': promoted, 'candidate': candidate.isoformat(),
                      'baseline': previous.isoformat() if previous else None}))
    return 0


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if argv[:1] == ['promote']:
        return promote_main(argv[1:])
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--current', required=True)
    parser.add_argument('--previous')
    parser.add_argument('--bundled', required=True)
    parser.add_argument('--out', required=True)
    parser.add_argument('--run-url', default='')
    args = parser.parse_args(argv)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    result = check(args.current, args.previous, args.bundled)
    reports = result.pop('reports')
    for name, report in reports.items():
        (out / f'{name}-diff.json').write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')
    (out / 'result.json').write_text(json.dumps(result, indent=2, ensure_ascii=False) + '\n')
    (out / 'summary.md').write_text(summary(result))
    (out / 'issue.md').write_text(issue_body(result, args.run_url))
    changed = bool(result['live_changes'])
    write_github('GITHUB_OUTPUT', f'changed={str(changed).lower()}\n')
    write_github('GITHUB_STEP_SUMMARY', summary(result))
    print(json.dumps({key: result[key] for key in ('api', 'build', 'baseline', 'live_changes')}
                     | {'bundled_changes': len(result['bundled_changed_entries'])}))
    return 0


if __name__ == '__main__':
    sys.exit(main())
