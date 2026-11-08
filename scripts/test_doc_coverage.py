import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('coverage_check', Path(__file__).with_name('check-doc-coverage.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class CoverageTests(unittest.TestCase):
    def test_complete(self):
        self.assertEqual(module.check({'src/lib.rs': dict(total=10, with_docs=10, total_examples=4, with_examples=4)})['total'], 10)

    def test_incomplete_and_malformed(self):
        good = dict(total=10, with_docs=10, total_examples=4, with_examples=4)
        for data in ({}, [], {'file': {}}, {'file': dict(good, with_docs=9)},
                     {'file': dict(good, with_examples=3)}, {'file': dict(good, with_docs=11)},
                     {'file': dict(good, total=True)}, {'file': dict(good, total_examples=0, with_examples=0)}):
            with self.subTest(data=data), self.assertRaises(ValueError):
                module.check(data)
