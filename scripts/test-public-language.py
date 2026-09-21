#!/usr/bin/env python3
"""Tests for the public Markdown language boundary."""

import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("check-public-language.py")
SPEC = importlib.util.spec_from_file_location("check_public_language", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class PublicLanguageTest(unittest.TestCase):
    def test_detects_chinese_prose_but_ignores_code_and_translation_link(self):
        with tempfile.TemporaryDirectory() as value:
            path = Path(value) / "README.md"
            path.write_text(
                "# English\n\n[Simplified Chinese / 简体中文](README.zh-CN.md)\n\n"
                "Inline `中文 fixture` is code.\n\n```json\n{\"zh\": \"中文\"}\n```\n\n中文泄漏。\n",
                encoding="utf-8",
            )
            self.assertEqual(MODULE.prose_violations(path), [(11, "中文泄漏。")])

    def test_chinese_translation_is_not_an_english_default(self):
        self.assertFalse(MODULE.is_english_default(Path("guide.zh-CN.md")))
        self.assertFalse(MODULE.is_english_default(Path("guide.zh.md")))
        self.assertTrue(MODULE.is_english_default(Path("guide.en.md")))
        self.assertTrue(MODULE.is_english_default(Path("README.md")))

    def test_discovery_includes_untracked_markdown(self):
        with tempfile.TemporaryDirectory() as value:
            root = Path(value)
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            (root / "README.md").write_text("# Tracked\n", encoding="utf-8")
            subprocess.run(["git", "add", "README.md"], cwd=root, check=True)
            untracked = root / "NEW.md"
            untracked.write_text("# New\n", encoding="utf-8")
            self.assertIn(untracked, MODULE.tracked_markdown(root))

    def test_repository_public_surface_is_clean(self):
        root = Path(__file__).resolve().parent.parent
        self.assertEqual(MODULE.check(root), [])


if __name__ == "__main__":
    unittest.main()
