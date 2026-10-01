"""Check reasoning prefill in the actual bundled catalog, not a parallel fixture."""
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]


class BailianReasoningProjectionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.catalog = json.loads((ROOT / "assets/release-facts/current/bundle/model-data.json")
                                 .read_text())["metadata_catalog"]

    def test_deepseek_efforts_reach_the_shipped_prefill_catalog(self):
        expected = {
            "deepseek-v4-1-flash": ["low", "high", "max"],
            "deepseek-v4-flash-0731": ["low", "high", "max"],
            "deepseek-v4-flash": ["high", "max"],
            # The shared canonical record also serves the unversioned Pro ID.
            "deepseek-v4-pro-0813": ["high", "max"],
        }
        for key, profiles in expected.items():
            with self.subTest(model=key):
                model = next(m for m in self.catalog["canonical_models"] if m["model_key"] == key)
                self.assertEqual(model["reasoning"]["kind"], "toggle-plus-discrete")
                self.assertEqual(model["reasoning"]["profiles"], profiles)
                self.assertEqual(model["reasoning"]["default"], "high")
                self.assertIn("source-150", model["evidence_refs"])

    def test_token_plan_keeps_all_native_faces_and_account_qualification(self):
        for product in ("bailian-token-personal-cn-beijing", "bailian-token-team-cn-beijing"):
            with self.subTest(product=product):
                binding = next(b for b in self.catalog["endpoint_bindings"]
                               if b["product_key"] == product
                               and b["upstream_model_id"] == "deepseek-v4.1-flash")
                self.assertEqual(binding["interface_candidates"], [
                    f"{product}/anthropic-messages", f"{product}/openai-chat",
                    f"{product}/openai-responses",
                ])
                self.assertEqual(binding["protocol_qualification"], "runtime-required")
                self.assertEqual(binding["availability"]["state"], "conditional")


if __name__ == "__main__":
    unittest.main()
