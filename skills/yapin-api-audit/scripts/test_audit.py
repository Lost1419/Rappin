"""Offline regression checks for evidence and contract comparison."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import audit


class AuditTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.spec = {'openapi': '3.1.0', 'info': {'version': '1.3.0'},
                     'paths': {'/messages': {'post': {'operationId': 'send',
                         'description': 'text required'}}},
                     'components': {'schemas': {'Override': {'enum': ['speak']}}}}

    def tearDown(self):
        self.temp.cleanup()

    def file(self, name, value):
        path = self.root / name
        path.write_text(json.dumps(value))
        return path

    def test_same_version_description_enum_and_new_field_are_detected(self):
        newer = copy.deepcopy(self.spec)
        newer['paths']['/messages']['post']['description'] = 'text or sticker required'
        newer['paths']['/messages']['post']['nullable'] = True
        newer['components']['schemas']['Override']['enum'].append('use_activities')
        report = audit.compare(self.file('before.json', self.spec), self.file('after.json', newer))
        self.assertFalse(report['openapi_equal'])
        self.assertEqual(len(report['changes']), 3)
        self.assertEqual(report['versions']['baseline'], report['versions']['current'])
        self.assertEqual(report['changed_schemas'], ['Override'])
        self.assertEqual(report['changed_operations'][0]['coverage'], 'unassessed')

    def test_nested_types_order_additions_and_removals(self):
        self.assertTrue(audit.differences({'x': [True]}, {'x': [1]}))
        self.assertTrue(audit.differences({'x': [1]}, {'x': [1.0]}))
        self.assertFalse(audit.differences({'a': 1, 'b': 2}, {'b': 2, 'a': 1}))
        self.assertEqual(audit.differences({'a/b~c': None}, {})[0]['pointer'], '/a~1b~0c')
        newer = copy.deepcopy(self.spec)
        newer['paths'] = {'/new': {'get': {'operationId': 'read'}}}
        report = audit.compare(self.file('a.json', self.spec), self.file('b.json', newer))
        self.assertEqual({row['operation_id'] for row in report['changed_operations']}, {'send', 'read'})

    def test_guide_text_ignores_scripts_and_whitespace_but_keeps_contract_text(self):
        self.assertEqual(audit.visible(b'<p>hello world</p><script>token()</script>'),
                         audit.visible(b'<p> hello  world </p><style>other</style>'))
        self.assertNotEqual(audit.visible(b'<p>old rule</p>'), audit.visible(b'<p>new rule</p>'))

    def test_snapshot_success_checksum_and_interruption(self):
        class Response:
            status = 200
            def __init__(self, data): self.data = data
            def __enter__(self): return self
            def __exit__(self, *args): pass
            def read(self, count): return self.data[:count]
        def fetch(request, timeout):
            if request.full_url.endswith('openapi.json'):
                return Response(json.dumps(self.spec).encode())
            if request.full_url.endswith('/developers') or request.full_url.endswith('/developers/api'):
                return Response(b'<html><p>public guide</p></html>')
            return Response(b'{}')
        output = self.root / 'snapshot'
        with patch.object(audit, 'urlopen', fetch):
            audit.snapshot(output)
            self.assertEqual(audit.load(output)[0], self.spec)
            with self.assertRaises(ValueError): audit.snapshot(output)
        (output / 'developers.html').write_text('<html>changed</html>')
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'): audit.load(output)
        (output / '.incomplete').touch()
        with self.assertRaisesRegex(ValueError, 'interrupted'): audit.load(output)

    def test_failed_or_oversized_fetch_does_not_publish_snapshot(self):
        class Response:
            status = 200
            def __enter__(self): return self
            def __exit__(self, *args): pass
            def read(self, count): return b'x' * count
        output = self.root / 'failed'
        with patch.object(audit, 'urlopen', side_effect=OSError('offline')):
            with self.assertRaises(OSError): audit.snapshot(output)
        self.assertFalse(output.exists())
        with patch.object(audit, 'urlopen', return_value=Response()):
            with self.assertRaisesRegex(ValueError, 'exceeds'): audit.snapshot(output)
        self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
