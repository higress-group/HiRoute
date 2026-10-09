"""The bundled limits must describe the provider, not a short probe budget."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "current_model_generator", ROOT / "assets/model-data/current/generate.py")
GENERATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GENERATOR)


class OpenRouterLimitsTests(unittest.TestCase):
    def test_production_bundle_uses_exact_provider_directory_limits(self):
        bundle = json.loads((ROOT / "assets/release-facts/current/bundle/model-data.json").read_text())
        models = {item["model_configuration_id"]: item for item in bundle["data"]["models"]}
        expected = {
            "model.openrouter.liquid-lfm-2.5-2.6b-free": (65536, 8192),
            "model.openrouter.nvidia-nemotron-3-super-120b-a12b-free": (262144, 235929),
        }
        for identity, (context, output) in expected.items():
            with self.subTest(identity=identity):
                capability = models[identity]["capabilities"]
                self.assertEqual(capability["context_tokens"], context)
                self.assertEqual(capability["max_output_tokens"], output)

    def test_projection_rejects_wrong_provider_identity_and_unknown_limits(self):
        root = ROOT / "assets/model-data/current"
        maintenance = json.loads((root / "maintenance.json").read_text())
        catalog = json.loads((root / "inputs/metadata-catalog.json").read_text())
        seed = json.loads((ROOT / maintenance["source_bundle"]).read_text())["data"]
        for field, invalid in [
            ("upstream_model_id", "another/model"),
            ("provider_id", "another-provider"),
            ("max_output_tokens", {"state": "runtime-required", "value": None}),
        ]:
            changed = copy.deepcopy(catalog)
            key = maintenance["current_provider_limits"][0]["model_record_key"]
            record = next(item for item in changed["model_metadata_records"]
                          if item["model_record_key"] == key)
            record[field] = invalid
            with self.subTest(field=field), self.assertRaises(ValueError):
                GENERATOR.apply_current_provider_limits(copy.deepcopy(seed), maintenance, changed)


if __name__ == "__main__":
    unittest.main()
