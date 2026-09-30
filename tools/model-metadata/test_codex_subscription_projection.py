"""Regression checks against the generated production subscription projection."""
import json
import hashlib
from pathlib import Path
import unittest

import validate_inference_rules

ROOT = Path(__file__).resolve().parents[2]
MODEL_ID = "model.openai.gpt-6.1-sol"


class CodexSubscriptionProjectionTests(unittest.TestCase):
    def setUp(self):
        self.bundle = json.loads(
            (ROOT / "assets/release-facts/current/bundle/model-data.json").read_text()
        )

    def test_gpt61_has_native_max_in_the_existing_responses_offer(self):
        data = self.bundle["data"]
        definition = next(model for model in data["models"]
                          if model["model_configuration_id"] == MODEL_ID)
        self.assertEqual(definition["capabilities"]["context_tokens"], 272000)
        native = next(model for model in self.bundle["rating_snapshot"]["models"]
                      if model["model_configuration_id"] == MODEL_ID)
        self.assertEqual(native["capability"], {
            "kind": "discrete", "parameter": "reasoning_effort",
            "profiles": ["low", "medium", "high", "xhigh", "max"],
        })
        capabilities = [item for item in data["model_endpoint_capabilities"]
                        if item["model_configuration_id"] == MODEL_ID]
        self.assertEqual(len(capabilities), 1)
        self.assertEqual(capabilities[0]["connector_id"], "connector.cpa.codex")
        self.assertEqual(capabilities[0]["upstream_protocol"], "responses")
        self.assertEqual(capabilities[0]["upstream_model_id"], "gpt-6.1-sol")
        offers = [item for item in data["offers"]
                  if item["offer_id"] == "offer.codex.subscription"]
        self.assertEqual(len(offers), 1)
        self.assertIn(MODEL_ID, offers[0]["model_configuration_ids"])

    def test_subscription_metadata_does_not_inherit_api_prices_or_global_context(self):
        records = self.bundle["metadata_catalog"]["model_records"]
        native = next(record for record in records
                      if record["provider_id"] == "openai-codex"
                      and record["upstream_model_id"] == "gpt-6.1-sol")
        api = next(record for record in records
                   if record["provider_id"] == "openai-platform"
                   and record["upstream_model_id"] == "gpt-6.1-sol")
        self.assertEqual(native["context_tokens"]["value"], 272000)
        self.assertEqual(api["context_tokens"]["value"], 1050000)
        self.assertEqual(native["cost_hint_state"], "not_recorded")
        self.assertEqual(native["cost_hints"], [])
        self.assertEqual(native["field_provenance"]["max_output_tokens"], {
            "basis": "inferred", "rule_key": "rule.codex.gpt-6.1-sol-output/v1",
        })
        rule = next(rule for rule in self.bundle["metadata_catalog"]["inference_rules"]
                    if rule["rule_key"] == "rule.codex.gpt-6.1-sol-output/v1")
        self.assertEqual(rule["collected_on"], "2026-09-30")
        original = next(rule for rule in self.bundle["metadata_catalog"]["inference_rules"]
                        if rule["rule_key"] == "rule.codex.auto-review/v1")
        self.assertEqual(original["collected_on"], "2026-09-13")

    def test_rule_specific_date_rejects_invalid_dates(self):
        catalog = json.loads(
            (ROOT / "assets/model-data/current/inputs/metadata-catalog.json").read_text()
        )
        rules = json.loads((ROOT / "tools/model-metadata/inference-rules.json").read_text())
        rule = next(rule for rule in rules["model_rules"]
                    if rule["rule_key"] == "rule.codex.gpt-6.1-sol-output/v1")
        rule["collected_on"] = "not-a-date"
        errors = []
        digest = lambda value: "sha256:" + hashlib.sha256(json.dumps(
            value, sort_keys=True, ensure_ascii=False, separators=(",", ":")
        ).encode()).hexdigest()
        validate_inference_rules.validate_rule_catalog(
            catalog, rules, {source["source_key"]: source for source in catalog["evidence_sources"]},
            digest(catalog), digest, lambda values: values == sorted(set(values)), errors,
        )
        self.assertIn("inference rule rule.codex.gpt-6.1-sol-output/v1 has invalid collected_on", errors)


if __name__ == "__main__":
    unittest.main()
