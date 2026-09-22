#!/usr/bin/env python3
"""Private product fixture must never overwrite a selected Codex installation."""

from pathlib import Path
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO / 'crates/daemon/tests/support'))

from delegation_product import select_real_main_codex
from publication_product import Product


class CodexFixtureSelectionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.selected = root / 'installed-codex'
        self.selected.write_bytes(b'installed native Codex remains intact\n')
        self.product = Product(REPO, root=root / 'product')

    def test_fixture_refuses_to_write_through_selected_engine_symlink(self):
        (self.product.root / 'bin/codex').symlink_to(self.selected)
        with self.assertRaisesRegex(RuntimeError, 'linked Codex binary'):
            self.product.install_codex_fixture()
        self.assertEqual(self.selected.read_bytes(), b'installed native Codex remains intact\n')

    def test_cpa_fixture_is_installed_before_selecting_real_engine(self):
        select_real_main_codex(self.product, self.selected)
        private_codex = self.product.root / 'bin/codex'
        self.assertTrue(private_codex.is_symlink())
        self.assertEqual(private_codex.resolve(), self.selected.resolve())
        self.assertEqual(self.selected.read_bytes(), b'installed native Codex remains intact\n')
        self.assertTrue((self.product.cpa_fixture / 'auth.json').is_file())
        self.assertTrue((self.product.cpa_fixture / 'models_cache.json').is_file())


if __name__ == '__main__':
    unittest.main()
