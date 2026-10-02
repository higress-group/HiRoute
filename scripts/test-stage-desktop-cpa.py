#!/usr/bin/env python3
"""Pin checks run without invoking native macOS tooling or copying an artifact."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('stage', Path(__file__).with_name('stage-desktop-cpa.py'))
stage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stage)


class Pins(unittest.TestCase):
    def setUp(self):
        self.manifest = stage.read_manifest()
        self.source = json.loads((stage.MANIFEST.parents[3] / 'vendor/cpa/source.json').read_text())

    def test_historical_pin_rejected_before_file_access(self):
        with patch.object(stage, 'host_identity', return_value=('macos', 'aarch64')):
            with self.assertRaisesRegex(stage.StageError, 'DEVELOPMENT_CPA_PIN_STALE'):
                stage.stage(Path('/missing/source'), Path('/missing/daemon'))

    def test_current_generated_manifest_can_be_selected(self):
        current = copy.deepcopy(self.manifest)
        current['development_only'] = False
        current['artifacts'][0].update(version=self.source['version'], commit=self.source['commit'])
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'cpa-artifacts.json'
            path.write_text(json.dumps(current))
            with patch.object(stage, 'host_identity', return_value=('macos', 'aarch64')):
                self.assertEqual(stage.selected_artifact(stage.read_manifest(path))['version'], self.source['version'])

    def test_wrong_source_commit_rejected_even_with_current_version(self):
        self.manifest['artifacts'][0]['version'] = self.source['version']
        self.manifest['artifacts'][0]['commit'] = '0' * 40
        with patch.object(stage, 'host_identity', return_value=('macos', 'aarch64')):
            with self.assertRaisesRegex(stage.StageError, 'DEVELOPMENT_CPA_PIN_STALE'):
                stage.selected_artifact(self.manifest)

    def test_product_cpa_fixtures_match_runtime_pin(self):
        root = stage.MANIFEST.parents[3]
        for path in ['crates/daemon/tests/support/cpa_upstream.py',
                     'crates/daemon/tests/support/standalone_headless_product.py',
                     'crates/daemon/tests/subscription_management_product.rs',
                     'crates/cpa-bridge/src/lib.rs']:
            with self.subTest(path=path):
                self.assertIn(self.source['version'], (root / path).read_text())


if __name__ == '__main__':
    unittest.main()
