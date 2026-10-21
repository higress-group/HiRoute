"""Ordinary native autoload and failure degradation through the real Gateway.

Call after automatic model configuration, with one selected Responses Plan temporarily
using the existing DecisionEndpoint as a saved custom decision service. The selected
ordinary model source must use PersistedRouteOracle. The caller retains ownership of
Product, settings, source, Plan restoration and decision endpoint teardown.
"""
import json
from pathlib import Path
import secrets
import subprocess

from additional_model_fixture import successful_native_result
from contextlib import contextmanager
from agent_product_support import run_native_command


HISTORY_CASES = ("agent.models.native-adapter-autoload-history",
                    "agent.models.native-adapter-missing-request-scope",
                    "agent.models.native-adapter-restart-conservative")


def exercise_native_history(product, binary, node, selector, oracle, decisions, harness):
    """decisions() returns actual custom-service request bodies in arrival order."""
    provider, model = selector.split("/", 1)
    session = "native-autoload-" + secrets.token_hex(10) if harness == "pi" else None
    required = list(HISTORY_CASES)
    report = {"required_cases": required, "selected_cases": required, "cases": []}

    def ask(label, disabled=False):
        nonlocal session
        prompt = oracle.arm(label)
        before = len(oracle.calls)
        if harness == "pi":
            command = [str(node), str(binary), "--provider", provider, "--model", model,
                       "--session-id", session, "--mode", "json", "--print", "--offline", "--tools", "read"]
            if disabled:
                command.append("--no-extensions")
        else:
            command = [str(binary), "run", "--format", "json", "--model", selector]
            if session is not None:
                command += ["--session", session]
        command.append(prompt)
        output = run_native_command(product, command, timeout=60,
                                    label="ordinary native adapter/history")
        observed = successful_native_result(output, oracle.receipt, harness)
        if session is not None:
            assert observed == session, "native client started a replacement session"
        session = observed
        assert oracle.calls[before:] == [label], "native client did not reach the selected production route once"
        return prompt

    @contextmanager
    def missing_adapter():
        if harness == "pi":
            yield
            return
        config = Path(product.env["XDG_CONFIG_HOME"]) / "opencode"
        adapter = config / "plugins/hiroute-routing.js"
        disabled = adapter.with_suffix(".disabled")
        assert adapter.is_file() and not disabled.exists()
        adapter.rename(disabled)
        try:
            yield
        finally:
            disabled.rename(adapter)

    first = ask("adapter-first")
    before = len(decisions())
    ask("adapter-second")
    actual = decisions()[before:]
    assert actual and any(first in json.dumps(body["visible_conversation"]) for body in actual), \
        "ordinary native client did not autoload the installed routing adapter or lost accepted Main history"
    report["cases"].append({"id": required[0], "state": "green", "native_session_id": session})

    before = len(decisions())
    with missing_adapter():
        ask("adapter-disabled-first", True)
        ask("adapter-disabled-second", True)
    actual = decisions()[before:]
    assert len(actual) >= 2 and all(body["history_partial"] and body["visible_conversation"] == []
                                  and body["assessment_target"] is None for body in actual), \
        "a real client without its extension inferred shared history"
    report["cases"].append({"id": required[1], "state": "green", "native_session_id": session})

    # Refresh proven history after the explicit unavailable-adapter control.
    before = len(decisions())
    ask("adapter-restored")
    actual = decisions()[before:]
    assert actual and all(body["history_partial"] and body["assessment_target"] is None for body in actual), \
        "restoring the adapter concealed intervening uncaptured native user turns"
    product.stop()
    product.start()
    before = len(decisions())
    ask("adapter-gateway-restart")
    actual = decisions()[before:]
    assert actual and all(body["history_partial"] and body["visible_conversation"] == []
                         and body["assessment_target"] is None for body in actual), \
        "Gateway restart revived an unproven accepted execution"
    report["cases"].append({"id": required[2], "state": "green", "native_session_id": session})
    return report


def assert_default_startup_after_restore(product, binary, node, settings, harness):
    """After existing production restore_models + full config checks, use real RPC startup."""
    assert settings.read() == settings.baseline
    if harness == "opencode":
        config = Path(product.env["XDG_CONFIG_HOME"]) / "opencode"
        assert not (config / "plugins/hiroute-routing.js").exists()
        assert not (config / "plugins/hiroute-routing.mjs").exists()
        from native_acp_client import prompt as acp_prompt
        _, native_session = acp_prompt(product, [str(binary), "acp"], None)
        return {"state": "green", "session_created": bool(native_session), "model_requests": 0}
    config = Path(product.env["PI_CODING_AGENT_DIR"])
    assert not (config / "extensions/hiroute-routing.js").exists()
    assert not (config / "extensions/hiroute-routing.mjs").exists()
    expected_provider = settings.default_baseline["defaultProvider"]
    expected_model = settings.default_baseline["defaultModel"]
    command = [str(node), str(binary), "--mode", "rpc", "--offline", "--no-session"]
    result = subprocess.run(command,
        input=b'{"id":"restored-native-default","type":"get_state"}\n',
        env=product.env, cwd=product.project, capture_output=True, timeout=30)
    product.outputs.extend([result.stdout, result.stderr])
    assert result.returncode == 0, "restored Pi failed its actual default-model startup"
    replies = [json.loads(line) for line in result.stdout.splitlines() if line.startswith(b"{")]
    state = next(row for row in replies if row.get("id") == "restored-native-default")
    assert state.get("success") is True
    model = state["data"]["model"]
    assert (model["provider"], model["id"]) == (expected_provider, expected_model)
    return {"state": "green", "default_provider": expected_provider, "default_model": expected_model,
            "native_rpc": "get_state", "inference_requests": 0}
