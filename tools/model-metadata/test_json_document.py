"""Whitespace must preserve strings, canonical semantics and complete metadata."""
import json
import unittest
from json_document import document


class JsonDocumentTests(unittest.TestCase):
    def test_large_document_preserves_escaped_strings_unicode_and_determinism(self):
        value = {'rows': [{'id': n, 'text': '引号 \\" ,},{ \\n' * 30}
                          for n in range(400)]}
        first = document(value)
        self.assertEqual(first, document(value))
        self.assertEqual(json.loads(first), value)
        self.assertGreater(first.count(b'\n'), 1)
        self.assertLess(max(map(len, first.splitlines())), 68000)
        compact = json.dumps(value, sort_keys=True, ensure_ascii=False,
                             separators=(',', ':')).encode()
        self.assertEqual(first.replace(b'\n', b''), compact)

    def test_small_artifacts_keep_their_existing_exact_bytes(self):
        self.assertEqual(document({'b': 2, 'a': 1}), b'{"a":1,"b":2}\n')
        self.assertEqual(document({'b': 2, 'a': 1}, sort_keys=False), b'{"b":2,"a":1}\n')


if __name__ == '__main__':
    unittest.main()
