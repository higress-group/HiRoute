"""Hermes public turn hooks cannot prove original-user provenance."""
import copy
import json
import sys
import types
import unittest
from concurrent.futures import ThreadPoolExecutor
from unittest.mock import patch

from test_hermes_plugin import configuration, plugin


def context(**changes):
    return {"provider": "custom", "model": "plan-alias", "base_url": "http://127.0.0.1:5837/v1",
            "api_mode": "chat_completions", "session_id": "parent", "task_id": "task", "turn_id": "turn",
            **changes}


class NativeContext:
    def __init__(self):
        self.middleware = {}
        self.hooks = {}

    def register_hook(self, name, callback):
        self.hooks[name] = callback

    def register_middleware(self, name, callback):
        self.middleware[name] = callback


def registered(native=None):
    native = native or NativeContext()
    package, config = types.ModuleType("hermes_cli"), types.ModuleType("hermes_cli.config")
    package.__path__ = []
    config.load_config_readonly = lambda: configuration("chat_completions")
    with patch.dict(sys.modules, {"hermes_cli": package, "hermes_cli.config": config}):
        plugin.register(native)
    return native


class TurnInputTests(unittest.TestCase):
    def test_public_hook_availability_never_bootstraps_a_real_or_synthetic_user(self):
        native = registered()
        self.assertEqual(native.hooks, {})
        for content in ("actual real user", "[ASYNC DELEGATION BATCH COMPLETE] native child results",
                        "an unmarked generated message", [{"type": "text", "text": "actual user"}]):
            with self.subTest(content=content):
                payload = {"messages": [{"role": "user", "content": content}]}
                result = native.middleware["llm_request"](payload, **context())
                metadata = json.loads(result["request"]["extra_headers"]["X-HiRoute-Routing-Context"])
                self.assertEqual(metadata["input"], {"kind": "observed_batch", "event_id": "turn"})
                self.assertEqual(metadata["version"], 2)
                self.assertEqual(metadata["turn_id"], "turn")
                self.assertIs(result["request"]["messages"], payload["messages"])

    def test_older_sdk_without_whole_turn_hooks_remains_connectable(self):
        native = types.SimpleNamespace(middleware={})
        native.register_middleware = lambda name, callback: native.middleware.update({name: callback})
        self.assertIs(registered(native), native)
        self.assertIn("llm_request", native.middleware)

    def test_all_transports_keep_auth_body_tools_and_stream_without_origin_reads(self):
        for mode in ("chat_completions", "codex_responses", "anthropic_messages"):
            with self.subTest(mode=mode):
                adapter = plugin._adapter(configuration(mode))
                key = "input" if mode == "codex_responses" else "messages"
                body = [{"role": "user", "content": "private original input"}]
                payload = {"model": "plan-alias", key: body, "tools": [{"type": "function"}],
                           "stream": True, "stream_options": {"include_usage": True},
                           "extra_headers": {"X-Preserve": "yes", "x-hiroute-token": "stale"}}
                before = copy.deepcopy(payload)
                with patch.object(plugin, "_lineage_header", return_value="existing-native-lineage"):
                    result = adapter(payload, **context(api_mode=mode))
                self.assertEqual(payload, before)
                for name in (key, "tools", "stream_options"):
                    self.assertIs(result["request"][name], payload[name])
                self.assertIs(result["request"]["stream"], True)
                headers = result["request"]["extra_headers"]
                self.assertEqual(headers["X-HiRoute-Token"], "synthetic-local-token")
                self.assertEqual(headers["X-Preserve"], "yes")
                self.assertEqual(headers["X-Hermes-Compression-Lineage"], "existing-native-lineage")
                self.assertNotIn("private original input", json.dumps(headers))
                metadata = json.loads(headers["X-HiRoute-Routing-Context"])
                self.assertEqual(metadata["input"]["kind"], "observed_batch")

    def test_concurrent_native_branches_preserve_identity_without_new_user_claims(self):
        adapter = registered().middleware["llm_request"]

        def invoke(index):
            result = adapter({"messages": [{"role": "user", "content": "equal content"}]},
                             **context(session_id="session-" + str(index), turn_id="turn-" + str(index)))
            headers = result["request"]["extra_headers"]
            metadata = json.loads(headers["X-HiRoute-Routing-Context"])
            return headers["X-Hermes-Session-Id"], metadata

        with patch.object(plugin, "_lineage_header", return_value=None), ThreadPoolExecutor(max_workers=4) as pool:
            results = list(pool.map(invoke, range(8)))
        self.assertEqual(len({session for session, _ in results}), 8)
        self.assertTrue(all(metadata["input"]["kind"] == "observed_batch" for _, metadata in results))


if __name__ == "__main__":
    unittest.main()
