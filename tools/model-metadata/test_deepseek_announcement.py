"""Directory announcements must not qualify a host or reprice an existing alias."""
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]


class DeepSeekAnnouncementTests(unittest.TestCase):
    def test_official_directory_intent_keeps_host_qualification_and_prices_separate(self):
        source = json.loads((ROOT / "assets/model-data/current/inputs/metadata-catalog.json").read_text())
        bundle = json.loads((ROOT / "assets/release-facts/current/bundle/model-data.json").read_text())
        model_id = "model.deepseek.v4-1-flash"
        definition = next(m for m in bundle["data"]["models"] if m["model_configuration_id"] == model_id)
        canonical = next(m for m in source["models"] if m["model_key"] == "deepseek-v4-1-flash")
        self.assertEqual(definition["display_name"], canonical["display_name"])
        self.assertEqual(definition["capabilities"]["context_tokens"], canonical["context_tokens"])
        self.assertIn("source-144", canonical["evidence_refs"])
        capabilities = [c for c in bundle["data"]["model_endpoint_capabilities"] if c["model_configuration_id"] == model_id]
        self.assertEqual([c["upstream_model_id"] for c in capabilities], ["deepseek-flash"])
        self.assertFalse(any(r["model_configuration_id"] == model_id for r in bundle["data"]["price_rates"]))
        for binding in source["endpoint_bindings"]:
            if binding["model_key"] == canonical["model_key"]:
                self.assertEqual(binding["availability"]["state"], "conditional")
                self.assertEqual(binding["protocol_qualification"], "runtime-required")
        records = [r for r in bundle["rating_snapshot"]["records"] if r["model_configuration_id"] == model_id]
        self.assertTrue(records)
        self.assertTrue(all(r["overall"]["state"] == "unknown" for r in records))


if __name__ == "__main__":
    unittest.main()
