"""Offline proof of release pinning, stale coverage, and archive safety."""
import copy
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import audit
import support


class SupportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.spec = {'openapi': '3.1.0', 'info': {'version': '1.3.0'},
                     'paths': {'/messages': {'post': {'operationId': 'send'}}}}
        self.files = {'Cargo.toml': '[package]\nname="rappin-rs"\nversion="0.1.28"\n[features]\ncli=[]\n',
                      'docs/openapi.json': json.dumps(self.spec), 'src/client.rs': 'pub fn request() {}'}
        self.release = self.root / 'release'
        self.snapshot = self.root / 'api'
        self.snapshot.mkdir()
        sources = {}
        for name, url in audit.SOURCES.items():
            data = json.dumps(self.spec).encode() if name == 'openapi.json' else (
                b'<html>public API</html>' if name.endswith('.html') else b'{}')
            (self.snapshot / name).write_bytes(data)
            sources[name] = {'url': url, 'sha256': audit.digest(data), 'bytes': len(data),
                             'fetched_at': '2026-11-08T00:00:00+00:00'}
        (self.snapshot / 'manifest.json').write_text(json.dumps({'format': 1, 'sources': sources}))

    def tearDown(self):
        self.temp.cleanup()

    def archive(self, extras=()):
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode='w:gz') as archive:
            for path, value in self.files.items():
                data = value.encode()
                info = tarfile.TarInfo('rappin-rs-0.1.28/' + path)
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data))
            for info in extras:
                archive.addfile(info)
        return stream.getvalue()

    def download(self, archive=None, checksum=None):
        archive = archive or self.archive()
        metadata = {'version': {'num': '0.1.28', 'checksum': checksum or audit.digest(archive), 'yanked': False}}
        with patch.object(support, 'get', side_effect=[json.dumps(metadata).encode(), archive]):
            return support.release('0.1.28', self.release)

    def review(self):
        sdk = self.download()
        _, api = support.api_identity(self.snapshot)
        item = {'path': 'src/client.rs', 'sha256': sdk['files']['src/client.rs'], 'symbol': 'request'}
        row = {'id': 'sending', 'title': 'Sending', 'rollout': 'flag', 'api_sources': [],
               'required_features': {key: [] for key in support.SURFACES},
               'operations': [{'method': 'POST', 'path': '/messages'}],
               'surfaces': {key: {'status': 'supported', 'reason': 'Fixture source review',
                                  'evidence': [item]} for key in support.SURFACES}}
        data = {'format': 1, 'sdk': {'package': 'rappin-rs', 'version': sdk['version'],
                                    'crate_sha256': sdk['crate_sha256']},
                'api': api, 'reviewed_at': '2026-11-08', 'features': [row]}
        path = self.root / 'review.json'
        path.write_text(json.dumps(data))
        return path, data

    def test_published_checksum_required_before_source_publication(self):
        with self.assertRaisesRegex(ValueError, 'checksum differs'):
            self.download(checksum='0' * 64)
        self.assertFalse(self.release.exists())

    def test_archive_traversal_links_and_duplicate_paths_are_rejected(self):
        for path, kind in [('rappin-rs-0.1.28/../../escape', tarfile.REGTYPE),
                           ('rappin-rs-0.1.28/C:/escape', tarfile.REGTYPE),
                           ('rappin-rs-0.1.28/.. /escape', tarfile.REGTYPE),
                           ('rappin-rs-0.1.28/link', tarfile.SYMTYPE),
                           ('rappin-rs-0.1.28/src/client.rs', tarfile.REGTYPE)]:
            info = tarfile.TarInfo(path)
            info.type = kind
            with self.assertRaises(ValueError):
                support.unpack(self.archive([info]), self.root / 'stage', '0.1.28')
        self.assertFalse((self.root.parent / 'escape').exists())

    def test_pinned_review_is_reproduced_without_certifying_whole_endpoint(self):
        path, _ = self.review()
        result = support.report(self.release, self.snapshot, path)
        self.assertTrue(result['review_matches'])
        self.assertEqual(result['features'][0]['surfaces']['typed']['status'], 'supported')
        self.assertEqual(result['operations'][0]['status'], 'unknown')
        self.assertIn('Typed SDK', support.markdown(result))
        self.assertIn('1.3.0', support.markdown(result))

    def test_same_version_api_or_guide_change_makes_every_claim_unknown(self):
        for member, data in [('openapi.json', dict(self.spec, description='new semantics')),
                              ('developers.html', '<html>changed access precedence</html>')]:
            with self.subTest(member=member):
                # Separate fixture snapshot for each case.
                path, _ = self.review() if not self.release.exists() else (self.root / 'review.json', None)
                manifest = json.loads((self.snapshot / 'manifest.json').read_text())
                original = (self.snapshot / member).read_bytes()
                original_hash = manifest['sources'][member]['sha256']
                value = json.dumps(data).encode() if member.endswith('.json') else data.encode()
                (self.snapshot / member).write_bytes(value)
                manifest['sources'][member]['sha256'] = audit.digest(value)
                (self.snapshot / 'manifest.json').write_text(json.dumps(manifest))
                result = support.report(self.release, self.snapshot, path)
                self.assertFalse(result['review_matches'])
                self.assertTrue(all(v['status'] == 'unknown' for v in result['features'][0]['surfaces'].values()))
                self.assertIsNone(result['features'][0]['required_features'])
                (self.snapshot / member).write_bytes(original)
                manifest['sources'][member]['sha256'] = original_hash
                (self.snapshot / 'manifest.json').write_text(json.dumps(manifest))

    def test_wrong_release_and_forged_source_evidence_cannot_claim_support(self):
        path, data = self.review()
        wrong = copy.deepcopy(data)
        wrong['sdk']['version'] = '0.1.27'
        path.write_text(json.dumps(wrong))
        self.assertFalse(support.report(self.release, self.snapshot, path)['review_matches'])
        wrong = copy.deepcopy(data)
        wrong['sdk']['crate_sha256'] = '0' * 64
        path.write_text(json.dumps(wrong))
        self.assertFalse(support.report(self.release, self.snapshot, path)['review_matches'])
        data['features'][0]['surfaces']['typed']['evidence'][0]['sha256'] = '0' * 64
        path.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError, 'does not match'):
            support.report(self.release, self.snapshot, path)

    def test_modified_release_files_and_missing_review_evidence_are_rejected(self):
        path, data = self.review()
        data['features'][0]['surfaces']['typed']['evidence'] = []
        path.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError, 'require source evidence'):
            support.report(self.release, self.snapshot, path)
        (self.release / 'source/src/client.rs').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
            support.load_release(self.release)

    def test_cargo_identity_and_official_source_urls_are_required(self):
        self.files['Cargo.toml'] = '[package]\nname="other"\nversion="0.1.28"'
        with self.assertRaisesRegex(ValueError, 'Cargo manifest differs'):
            self.download()
        self.assertFalse(self.release.exists())
        manifest = json.loads((self.snapshot / 'manifest.json').read_text())
        manifest['sources']['developers.html']['url'] = 'https://example.com/unofficial'
        (self.snapshot / 'manifest.json').write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, 'official sources'):
            support.api_identity(self.snapshot)


if __name__ == '__main__':
    unittest.main()
