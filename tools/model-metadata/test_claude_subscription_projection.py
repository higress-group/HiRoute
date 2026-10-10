"""Claude subscription regressions against the production client bundle.

Static capabilities do not prove account availability. The production inventory
absence guard is exercised by the public subscription product regressions in
crates/daemon/tests/publication_process.rs.
"""
import copy
import importlib.util
import json
from pathlib import Path
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
BUNDLE = ROOT / "assets/release-facts/current/bundle/model-data.json"
CATALOG = ROOT / "assets/model-data/current/inputs/metadata-catalog.json"
PRODUCT = "claude-code-subscription"
CONNECTOR = "connector.cpa.claude"
HAIKU = "model.anthropic.claude-haiku-4-5"
EXPECTED_BINDINGS = {
    "claude-fable-5-1": "model.anthropic.claude-fable-5-1",
    "claude-haiku-4-5": HAIKU,
    "claude-haiku-4-5-20251001": HAIKU,
    "claude-haiku-5-5": "model.anthropic.claude-haiku-5-5",
    "claude-opus-5": "model.anthropic.claude-opus-5",
    "claude-opus-5-5": "model.anthropic.claude-opus-5-5",
    "claude-sonnet-5": "model.anthropic.claude-sonnet-5",
    "claude-sonnet-5-5": "model.anthropic.claude-sonnet-5-5",
}
SPEC = importlib.util.spec_from_file_location(
    "claude_subscription_test_generator", ROOT / "assets/model-data/current/generate.py"
)
GENERATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GENERATOR)


def claude_capabilities(projection):
    return [value for value in projection["model_endpoint_capabilities"]
            if value["connector_id"] == CONNECTOR]


class ClaudeSubscriptionProjectionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.bundle = json.loads(BUNDLE.read_text())
        cls.catalog = json.loads(CATALOG.read_text())

    def binding(self, catalog, upstream="claude-haiku-4-5"):
        return next(value for value in catalog["endpoint_bindings"]
                    if value["product_key"] == PRODUCT
                    and value["upstream_model_id"] == upstream)

    def model(self, catalog, key="claude-haiku-4-5"):
        return next(value for value in catalog["models"] if value["model_key"] == key)

    def test_bundled_eight_exact_routes_join_seven_model_and_reasoning_identities(self):
        data = self.bundle["data"]
        capabilities = claude_capabilities(data)
        actual = {value["upstream_model_id"]: value["model_configuration_id"]
                  for value in capabilities}
        self.assertEqual(len(capabilities), 8, "missing subscription capabilities cause fallback")
        self.assertEqual(actual, EXPECTED_BINDINGS)
        self.assertEqual(len(set(actual.values())), 7)
        for capability in capabilities:
            with self.subTest(upstream=capability["upstream_model_id"]):
                identity = capability["model_configuration_id"]
                self.assertEqual(capability["endpoint_profile_id"], "endpoint.cpa.claude")
                self.assertEqual(capability["protocol_endpoint_id"], "endpoint.cpa.claude.messages")
                self.assertEqual(capability["upstream_protocol"], "messages")
                self.assertEqual(capability["required_adapter_ref"], "adapter.anthropic-messages.v1")
                self.assertEqual(capability["required_adapter_revision"], 1)
                self.assertRegex(capability["evidence_digest"], r"^sha256:[0-9a-f]{64}$")
                self.assertEqual(sum(value["model_configuration_id"] == identity
                                     for value in data["models"]), 1)
                self.assertEqual(sum(value["model_configuration_id"] == identity
                                     for value in self.bundle["rating_snapshot"]["models"]), 1)
        offers = [value for value in data["offers"]
                  if value["offer_id"] == "offer.claude.subscription"]
        self.assertEqual(len(offers), 1)
        self.assertEqual(set(offers[0]["model_configuration_ids"]), set(EXPECTED_BINDINGS.values()))
        self.assertEqual(offers[0]["endpoint_profile_id"], "endpoint.cpa.claude")
        self.assertEqual(offers[0]["entitlement_id"], "claude-subscription")
        self.assertEqual(offers[0]["billing_class"], "subscription")

    def test_bundled_haiku_limits_vision_and_toggle_keep_the_budget_audit(self):
        definition = next(value for value in self.bundle["data"]["models"]
                          if value["model_configuration_id"] == HAIKU)
        self.assertEqual(definition["capabilities"]["context_tokens"], 200000)
        self.assertEqual(definition["capabilities"]["max_output_tokens"], 64000)
        for capability in ("vision", "tool", "streaming"):
            self.assertIs(definition["capabilities"][capability], True)
        native = next(value for value in self.bundle["rating_snapshot"]["models"]
                      if value["model_configuration_id"] == HAIKU)
        self.assertEqual(native["capability"], {"kind": "toggle", "parameter": "enable_thinking"})
        self.assertNotIn("native_render_convention", native)
        audit = next(value for value in self.bundle["metadata_catalog"]["canonical_models"]
                     if value["model_key"] == "claude-haiku-4-5")
        self.assertEqual(audit["canonical_identity"], "claude-haiku-4-5-20251001")
        self.assertEqual(audit["reasoning"]["kind"], "manual-budget")
        self.assertEqual(audit["reasoning"]["profiles"], [])
        self.assertIn("fixed 1024-token", audit["data_note"])
        self.assertIn("not the model's native default or arbitrary-budget support", audit["data_note"])
        self.assertIn("max_tokens > 1024", audit["data_note"])
        self.assertIn("minimum budget 1024", audit["reasoning"]["note"])
        self.assertIn("budget_tokens < max_tokens", audit["reasoning"]["note"])

    def test_bundled_5x_models_use_their_exact_efforts_and_adaptive_convention(self):
        native = {value["model_configuration_id"]: value
                  for value in self.bundle["rating_snapshot"]["models"]}
        audit = {value["model_key"]: value
                 for value in self.bundle["metadata_catalog"]["canonical_models"]}
        for key in ("claude-fable-5-1", "claude-haiku-5-5", "claude-opus-5",
                    "claude-opus-5-5", "claude-sonnet-5", "claude-sonnet-5-5"):
            with self.subTest(model=key):
                value = native[f"model.anthropic.{key}"]
                self.assertEqual(value["capability"], {
                    "kind": "discrete", "parameter": "output_config.effort",
                    "profiles": ["low", "medium", "high", "xhigh", "max"],
                })
                self.assertEqual(value["capability"]["profiles"], audit[key]["reasoning"]["profiles"])
                self.assertEqual(value["native_render_convention"], "claude_adaptive_effort_messages")

    def test_narrowing_one_model_effort_does_not_expand_or_change_another(self):
        catalog = copy.deepcopy(self.catalog)
        self.model(catalog, "claude-sonnet-5")["reasoning"]["profiles"] = ["low", "medium", "high"]
        projection = GENERATOR.build_runtime_projection(catalog)
        native = {value["model"]["model_configuration_id"]: value["native_reasoning"]
                  for value in projection["models"]}
        self.assertEqual(native["model.anthropic.claude-sonnet-5"]["capability"]["profiles"],
                         ["low", "medium", "high"])
        self.assertEqual(native["model.anthropic.claude-opus-5"]["capability"]["profiles"],
                         ["low", "medium", "high", "xhigh", "max"])
        for profiles in ([], ["high", "high"], ["high", "ultra"]):
            with self.subTest(profiles=profiles):
                invalid = copy.deepcopy(catalog)
                self.model(invalid, "claude-sonnet-5")["reasoning"]["profiles"] = profiles
                with self.assertRaises(ValueError):
                    GENERATOR.build_runtime_projection(invalid)

    def test_unknown_directory_models_and_wrong_products_do_not_gain_subscription_routes(self):
        catalog = copy.deepcopy(self.catalog)
        unknown = copy.deepcopy(self.model(catalog))
        unknown.update(model_key="claude-haiku-4-5-unknown", canonical_identity="claude-haiku-4-5-unknown",
                       upstream_ids=["claude-haiku-4-5-unknown"])
        catalog["models"].append(unknown)
        projection = GENERATOR.build_runtime_projection(catalog)
        self.assertEqual({value["upstream_model_id"] for value in claude_capabilities(projection)},
                         set(EXPECTED_BINDINGS))
        self.assertNotIn("model.anthropic.claude-haiku-4-5-unknown",
                         {value["model"]["model_configuration_id"] for value in projection["models"]})
        wrong = copy.deepcopy(self.catalog)
        binding = copy.deepcopy(self.binding(wrong))
        other_product = next(value for value in wrong["access_products"]
                             if value["provider"] == "Anthropic" and value["product_key"] != PRODUCT)
        binding["product_key"] = other_product["product_key"]
        binding["interface_candidates"] = [next(value["interface_key"]
                                                for value in other_product["interfaces"]
                                                if value["protocol"] == "anthropic-messages")]
        binding["binding_key"] = f"{other_product['product_key']}/claude-haiku-4-5/claude-haiku-4-5"
        wrong["endpoint_bindings"] = [value for value in wrong["endpoint_bindings"]
                                      if value["product_key"] != PRODUCT] + [binding]
        projection = GENERATOR.build_runtime_projection(wrong)
        self.assertEqual(claude_capabilities(projection), [])
        self.assertFalse(any(value["offer_id"] == "offer.claude.subscription"
                             for value in projection["offers"]))

    def test_unregistered_aliases_and_cross_publisher_evidence_fail_closed(self):
        for alias in ("haiku", "claude-haiku-4-5-latest", "claude-haiku-4-5-other"):
            with self.subTest(alias=alias):
                catalog = copy.deepcopy(self.catalog)
                self.binding(catalog)["upstream_model_id"] = alias
                with self.assertRaises(ValueError):
                    GENERATOR.build_runtime_projection(catalog)
        catalog = copy.deepcopy(self.catalog)
        self.model(catalog)["publisher"] = "OpenAI"
        with self.assertRaises(ValueError):
            GENERATOR.build_runtime_projection(catalog)
        catalog = copy.deepcopy(self.catalog)
        source_key = self.binding(catalog)["evidence_refs"][0]
        next(value for value in catalog["evidence_sources"]
             if value["source_key"] == source_key)["authority"] = "developers.openai.com"
        with self.assertRaises(ValueError):
            GENERATOR.build_runtime_projection(catalog)

    def test_product_binding_and_thinking_subset_must_keep_the_reviewed_contract(self):
        for field, value in (("lifecycle", "retired"), ("interface_candidates", []),
                             ("protocol_qualification", "verified-primary"),
                             ("availability", {"state": "verified-primary"}),
                             ("capability_overrides", {"context_tokens": 100000})):
            with self.subTest(field=field):
                catalog = copy.deepcopy(self.catalog)
                self.binding(catalog)[field] = value
                with self.assertRaises(ValueError):
                    GENERATOR.build_runtime_projection(catalog)
        for field, value in (("enabled_budget_tokens", 2048), ("scope", "arbitrary-budget"),
                             ("note", ""), ("parameter", "output_config.effort")):
            with self.subTest(thinking_field=field):
                catalog = copy.deepcopy(self.catalog)
                self.binding(catalog)["reasoning_projection"][field] = value
                with self.assertRaises(ValueError):
                    GENERATOR.build_runtime_projection(catalog)
        catalog = copy.deepcopy(self.catalog)
        self.binding(catalog, "claude-opus-5")["reasoning_projection"]["thinking_type"] = "enabled"
        with self.assertRaises(ValueError):
            GENERATOR.build_runtime_projection(catalog)

    def test_static_bindings_remain_conditional_and_do_not_prove_account_inventory(self):
        bindings = [value for value in self.bundle["metadata_catalog"]["endpoint_bindings"]
                    if value["product_key"] == PRODUCT]
        self.assertEqual(len(bindings), 8)
        for binding in bindings:
            with self.subTest(upstream=binding["upstream_model_id"]):
                self.assertEqual(binding["availability"]["state"], "conditional")
                self.assertEqual(binding["protocol_qualification"], "runtime-required")
                self.assertIn("selected account must still expose this exact ID", binding["availability"]["condition"])
        # No account inventory is injected here; the Rust production-entry guard
        # remains necessary to prove that an absent model cannot be selected.

    def test_claude_projection_keeps_codex_data_and_seed_prices_unchanged(self):
        without_claude = copy.deepcopy(self.catalog)
        without_claude["endpoint_bindings"] = [value for value in without_claude["endpoint_bindings"]
                                               if value["product_key"] != PRODUCT]
        client_runs = json.loads((ROOT / "assets/model-data/current/inputs/client-discovery-runs.json").read_text())
        client_runs["catalog_digest_compared"] = GENERATOR.digest(without_claude)
        original_pinned_json = GENERATOR.pinned_json

        def pinned_counterfactual(path, expected_digest):
            if path == "assets/model-data/current/inputs/metadata-catalog.json":
                return copy.deepcopy(without_claude)
            if path == "assets/model-data/current/inputs/client-discovery-runs.json":
                return copy.deepcopy(client_runs)
            return original_pinned_json(path, expected_digest)

        with mock.patch.object(GENERATOR, "pinned_json", side_effect=pinned_counterfactual):
            counterfactual = json.loads(GENERATOR.compile_candidate()["model-data.json"])

        def codex_slice(bundle):
            data = bundle["data"]
            capabilities = [value for value in data["model_endpoint_capabilities"]
                            if value["connector_id"] == "connector.cpa.codex"]
            offers = [value for value in data["offers"]
                      if value["offer_id"] == "offer.codex.subscription"]
            self.assertTrue(capabilities, "Codex assertion must not compare empty projections")
            self.assertTrue(offers, "the compiled Codex offer must actually be present")
            identities = {value["model_configuration_id"] for value in capabilities}
            models = [value for value in data["models"]
                      if value["model_configuration_id"] in identities]
            native = [value for value in bundle["rating_snapshot"]["models"]
                      if value["model_configuration_id"] in identities]
            self.assertEqual({value["model_configuration_id"] for value in models}, identities)
            self.assertEqual({value["model_configuration_id"] for value in native}, identities)
            return capabilities, offers, models, native

        self.assertEqual(codex_slice(self.bundle), codex_slice(counterfactual))
        self.assertEqual(self.bundle["data"]["price_rates"], counterfactual["data"]["price_rates"])
        seed = json.loads((ROOT / "assets/model-data/current/rating-seed.json").read_text())
        self.assertEqual(self.bundle["data"]["price_rates"], seed["data"]["price_rates"])


if __name__ == "__main__":
    unittest.main()
