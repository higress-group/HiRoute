"""The packaged callback must preserve native SDK kwargs and affect only its owned route."""
import json
import importlib.util
import pathlib
import unittest


PATH = pathlib.Path(__file__).resolve().parents[2] / "assets/agents/hermes/__init__.py"
spec = importlib.util.spec_from_file_location("hiroute_hermes", PATH)
plugin = importlib.util.module_from_spec(spec)
spec.loader.exec_module(plugin)


def configuration(mode="codex_responses"):
    name = "hiroute-main-example-plan"
    return {
        "model": {"provider": name, "default": "plan-alias"},
        "providers": {name: {
            "api": "http://127.0.0.1:5837/v1", "transport": mode,
            "default_model": "plan-alias",
            "extra_headers": {"X-HiRoute-Token": "synthetic-local-token"},
        }},
        "plugins": {"entries": {"hiroute": {
            "provider_id": name, "model": "plan-alias",
            "base_url": "http://127.0.0.1:5837/v1", "api_mode": mode,
        }}},
    }


class HermesAdapterTests(unittest.TestCase):
    def test_native_envelope_preserves_stream_tools_and_existing_headers_for_all_modes(self):
        for mode in ("chat_completions", "codex_responses", "anthropic_messages"):
            with self.subTest(mode=mode):
                adapter = plugin._adapter(configuration(mode))
                request = {"stream": True, "model": "plan-alias", "tools": [{"type": "function"}],
                           "input": [{"type": "function_call_output", "output": "OK"}],
                           "extra_headers": {"X-Preserve": "yes", "x-hermes-session-id": "old",
                                             "x-hiroute-token": "stale"}}
                context = dict(provider="custom", model="plan-alias", base_url="http://127.0.0.1:5837/v1",
                               api_mode=mode, session_id="native-session")
                result = adapter(request, **context)
                self.assertEqual(set(result), {"request", "source"})
                self.assertIs(result["request"]["tools"], request["tools"])
                self.assertIs(result["request"]["input"], request["input"])
                self.assertIs(result["request"]["stream"], True)
                self.assertEqual(result["request"]["extra_headers"], {
                    "X-Preserve": "yes", "X-Hermes-Session-Id": "native-session",
                    "X-HiRoute-Token": "synthetic-local-token", "X-HiRoute-Agent": "hermes",
                    "X-HiRoute-Routing-Context": json.dumps({"version":2,"agent":"hermes","session_id":"native-session","purpose":"main","input":{"kind":"observed_batch"}}, separators=(",", ":"))})
                self.assertEqual(request["extra_headers"]["x-hermes-session-id"], "old")
                self.assertEqual(result["source"], "hiroute")

    def test_missing_invalid_or_over_bound_identity_never_removes_route_authorization(self):
        adapter = plugin._adapter(configuration())
        for identity in (None, "", "x\nheader", "x\x00", "x\x7f", "a" * 129, "正常", "\ud800"):
            with self.subTest(identity=repr(identity)):
                result = adapter({"extra_headers": {"X-HERMES-SESSION-ID": "must-remove"}},
                                 provider="custom", model="plan-alias", base_url="http://127.0.0.1:5837/v1",
                                 api_mode="codex_responses", session_id=identity)
                self.assertEqual(result["request"]["extra_headers"], {"X-HiRoute-Token": "synthetic-local-token", "X-HiRoute-Agent": "hermes"})

    def test_each_foreign_route_dimension_is_unchanged(self):
        adapter = plugin._adapter(configuration())
        context = dict(provider="custom", model="plan-alias", base_url="http://127.0.0.1:5837/v1",
                       api_mode="codex_responses", session_id="native-session")
        request = {"model": "plan-alias", "extra_headers": {"X-Preserve": "yes"}}
        for key in ("provider", "model", "base_url", "api_mode"):
            with self.subTest(key=key):
                self.assertIsNone(adapter(request, **{**context, key: "foreign"}))
                self.assertEqual(request, {"model": "plan-alias", "extra_headers": {"X-Preserve": "yes"}})

    def test_inconsistent_or_foreign_configuration_cannot_register_authorization(self):
        for key in ("provider", "default"):
            config = configuration()
            config["model"][key] = "foreign"
            self.assertIsNone(plugin._adapter(config))
        config = configuration()
        config["providers"]["hiroute-main-example-plan"]["extra_headers"]["X-HiRoute-Token"] = "short"
        self.assertIsNone(plugin._adapter(config))


if __name__ == "__main__":
    unittest.main()
