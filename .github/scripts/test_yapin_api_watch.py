"""Offline tests for the scheduled Yapin API watch. No network access."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import yapin_api_watch as watch

SPEC = {'openapi': '3.1.0', 'info': {'version': '1.5.0'},
        'paths': {'/messages': {'post': {'operationId': 'send', 'description': 'old'}}}}


class WatchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bundled = self.root / 'bundled.json'
        self.bundled.write_text(json.dumps(SPEC))

    def snapshot(self, name, spec=SPEC, build='aaa', api='1.5.0', changelog='2026-11-14',
                 guide='<html><p>guide</p></html>'):
        path = self.root / name
        path.mkdir()
        (path / 'openapi.json').write_text(json.dumps(spec))
        (path / 'version.json').write_text(json.dumps({'api': api, 'build': build, 'sdk': {'js': '1.8.0'}}))
        (path / 'changelog.json').write_text(json.dumps({'entries': [{'date': changelog}]}))
        for page in ('api.html', 'developers.html'):
            (path / page).write_text(guide)
        # A real manifest, checksummed like audit.snapshot writes it; compare verifies these.
        sources = {source: {'sha256': watch.audit.digest((path / source).read_bytes()),
                            'fetched_at': f'{name}T19:00:00+00:00'}
                   for source in watch.audit.SOURCES}
        (path / 'manifest.json').write_text(json.dumps({'format': 1, 'sources': sources}))
        return path

    def run_main(self, current, previous=None):
        out = self.root / f'out-{current.name}'
        outputs = self.root / 'github-output'
        outputs.write_text('')
        argv = ['--current', str(current), '--bundled', str(self.bundled), '--out', str(out)]
        if previous:
            argv += ['--previous', str(previous)]
        with patch.dict(os.environ, {'GITHUB_OUTPUT': str(outputs), 'GITHUB_STEP_SUMMARY': ''}):
            self.assertEqual(watch.main(argv), 0)
        return json.loads((out / 'result.json').read_text()), outputs.read_text(), out

    def test_missing_previous_is_a_baseline(self):
        result, outputs, out = self.run_main(self.snapshot('2026-11-26'),
                                             previous=self.root / 'missing')
        self.assertTrue(result['baseline'])
        self.assertEqual(outputs, 'changed=false\n')
        self.assertIn('new baseline', (out / 'summary.md').read_text())
        self.assertTrue((out / 'bundled-diff.json').exists())

    def test_new_build_and_guide_markup_alone_are_not_changes(self):
        previous = self.snapshot('2026-11-26')
        current = self.snapshot('2026-11-27', build='bbb',
                                guide='<html><script>track()</script><p>guide</p></html>')
        result, outputs, _ = self.run_main(current, previous)
        self.assertEqual(result['live_changes'], [])
        self.assertEqual(result['previous_build'], 'aaa')
        self.assertEqual(outputs, 'changed=false\n')

    def test_spec_version_and_changelog_changes_are_reported(self):
        previous = self.snapshot('2026-11-26')
        newer = json.loads(json.dumps(SPEC))
        newer['paths']['/messages']['post']['description'] = 'new'
        current = self.snapshot('2026-11-27', spec=newer, api='1.6.0', changelog='2026-11-27')
        result, outputs, out = self.run_main(current, previous)
        reasons = ' | '.join(result['live_changes'])
        self.assertIn('openapi.json: 1 changed entries', reasons)
        self.assertIn('changelog.json: readable content changed', reasons)
        self.assertIn('version.json api: "1.5.0" -> "1.6.0"', reasons)
        self.assertEqual(result['bundled_changed_entries'], ['changed /paths/~1messages/post/description'])
        self.assertEqual(outputs, 'changed=true\n')
        issue = (out / 'issue.md').read_text()
        self.assertIn('Live contract changed', issue)
        self.assertIn('/paths/~1messages/post/description', issue)

    def test_long_entry_lists_are_truncated(self):
        items = [f'changed /p/{n}' for n in range(watch.LIST_LIMIT + 5)]
        lines = watch.bullet_list(items)
        self.assertEqual(len(lines), watch.LIST_LIMIT + 1)
        self.assertIn('5 more', lines[-1])


if __name__ == '__main__':
    unittest.main()
