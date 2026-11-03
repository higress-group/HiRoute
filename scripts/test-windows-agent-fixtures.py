#!/usr/bin/env python3
"""Native npm preparation preserves foreign files and its measured source."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("fixtures", Path(__file__).with_name("windows-agent-fixtures.py"))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


class NativeEntryPreparation(unittest.TestCase):
    def test_reviewed_placeholder_becomes_the_same_native_image_and_is_idempotent(self):
        for copy_fallback in (False, True):
            with self.subTest(copy_fallback=copy_fallback), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                source, destination = root / "source.exe", root / "declared.exe"
                source.write_bytes(b"MZ native fixture")
                destination.write_bytes(b"reviewed placeholder")
                placeholder = hashlib.sha256(destination.read_bytes()).hexdigest()
                if copy_fallback:
                    with patch.object(fixtures.os, "link", side_effect=OSError("copy required")):
                        fixtures.materialize_native_entry(source, destination, placeholder)
                else:
                    fixtures.materialize_native_entry(source, destination, placeholder)
                self.assertEqual(destination.read_bytes(), source.read_bytes())
                self.assertEqual(source.read_bytes(), b"MZ native fixture")
                original = destination.stat().st_ino
                fixtures.materialize_native_entry(source, destination, placeholder)
                self.assertEqual(destination.stat().st_ino, original)
                self.assertEqual(sorted(path.name for path in root.iterdir()), ["declared.exe", "source.exe"])

    def test_unknown_entry_is_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, destination = root / "source.exe", root / "declared.exe"
            source.write_bytes(b"MZ native fixture")
            destination.write_bytes(b"unrecognized existing material")
            with self.assertRaises(ValueError):
                fixtures.materialize_native_entry(source, destination, "0" * 64)
            self.assertEqual(destination.read_bytes(), b"unrecognized existing material")


if __name__ == "__main__":
    unittest.main()
