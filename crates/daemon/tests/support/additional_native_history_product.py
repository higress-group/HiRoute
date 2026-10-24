"""Ordinary native autoload and failure degradation through the real Gateway.

Call after automatic model configuration, with one selected Responses Plan temporarily
using the existing DecisionEndpoint as a saved custom decision service. The selected
ordinary model source must use PersistedRouteOracle. The caller retains ownership of
Product, settings, source, Plan restoration and decision endpoint teardown.
"""
import json
from copy import deepcopy
from pathlib import Path
import secrets
import subprocess
import time

from additional_model_fixture import successful_native_result
from contextlib import contextmanager
from agent_product_support import run_native_command


HISTORY_CASES = ("agent.models.native-adapter-autoload-history",
                    "agent.models.native-adapter-missing-observed-recovery",
                    "agent.models.native-adapter-restart-recovery")


def receipt_facts(receipt, kind):
    return [row["fact"] for row in receipt["ordered_facts"] if row["fact"]["kind"] == kind]


def stage_identity(finished):
    identity = {key: finished[key] for key in (
        "segment_id", "plan_id", "plan_revision", "selected_branch_id", "executed_branch_id",
        "model_configuration_id", "profile_digest")}
    identity["branch_execution"] = finished.get("branch_execution")
    return identity


def freeze_completed_stage(receipts, anchor_request_id):
    """Reconstruct the stage from completed execution facts before sending the next input.

    One same-model stage can span several independently opened agent turns. Neither
    a later assessment nor its proposed target supplies the expected membership.
    """
    anchors = receipt_facts(receipts[anchor_request_id], "agent_turn_finished")
    anchors = [row for row in anchors if anchor_request_id in row["history_membership"]["request_ids"]]
    assert anchors and len({row["agent_turn_id"] for row in anchors}) == 1, "no unique completed anchor"
    anchor = max(anchors, key=lambda row: row["history_membership"]["revision"])
    scope, identity = anchor["history_membership"], stage_identity(anchor)
    assert anchor["attribution"] == "single", "anchor has no single actual execution identity"
    closed = {}
    for receipt in receipts.values():
        for finished in receipt_facts(receipt, "agent_turn_finished"):
            membership = finished["history_membership"]
            if (membership["branch_id"] != scope["branch_id"] or
                    finished["segment_id"] != anchor["segment_id"] or
                    membership["revision"] > scope["revision"]):
                continue
            assert stage_identity(finished) == identity, "stage contains foreign execution identity"
            assert finished["status"] == "completed" and not finished["history_partial"], "incomplete turn"
            assert not membership["capture_partial"] and not membership["clipped"], "incomplete capture"
            assert membership["request_ids"] and len(set(membership["request_ids"])) == len(
                membership["request_ids"]), "empty or duplicate capture membership"
            turn = finished["agent_turn_id"]
            if turn not in closed or membership["revision"] > closed[turn]["history_membership"]["revision"]:
                closed[turn] = finished
    request_ids = set().union(*(set(row["history_membership"]["request_ids"]) for row in closed.values()))
    assert anchor_request_id in request_ids, "stage lost the preceding execution"
    for request_id in request_ids:
        receipt = receipts.get(request_id)
        assert receipt and receipt["outcome"] == "accepted", "stage member lacks an accepted receipt"
        attempts = receipt_facts(receipt, "attempt_started")
        assert len(attempts) == 1, "ordinary fixture expected one actual attempt"
        assert all(attempts[0][key] == identity[key] for key in (
            "model_configuration_id", "profile_digest")), "member used a foreign model/profile"
    turns = sorted(closed.values(), key=lambda row: row["ordinal"])
    return deepcopy({
        "identity": identity, "branch_id": scope["branch_id"], "revision": scope["revision"],
        "request_ids": sorted(request_ids), "anchor_request_id": anchor_request_id,
        "source_unverified": any(row["history_membership"]["source_unverified"] for row in turns),
        "from_turn_id": turns[0]["agent_turn_id"], "through_turn_id": turns[-1]["agent_turn_id"],
        "from_ordinal": turns[0]["ordinal"], "through_ordinal": turns[-1]["ordinal"],
    })


def assert_assessed_stage(expected, score, current_request_id):
    membership = score["history_membership"]
    actual = membership["request_ids"]
    assert len(actual) == len(set(actual)) and set(actual) == set(expected["request_ids"]), \
        "assessment has wrong, missing or extra stage members"
    assert expected["anchor_request_id"] in actual and current_request_id not in actual, \
        "assessment lost preceding execution or includes current input"
    assert score["trigger_request_id"] == current_request_id, "wrong assessment trigger"
    assert membership["branch_id"] == expected["branch_id"] and membership["revision"] == expected["revision"], \
        "wrong immutable stage revision"
    assert score["score"] == 0.75 and not score["partial"], "missing complete quality result"
    assert not membership["capture_partial"] and not membership["clipped"], "incomplete assessed capture"
    assert membership["source_unverified"] == expected["source_unverified"], "altered source uncertainty"
    for key in ("segment_id", "plan_id", "plan_revision", "model_configuration_id", "profile_digest"):
        assert score[key] == expected["identity"][key], "foreign assessed execution: " + key
    for key in ("from_turn_id", "through_turn_id", "from_ordinal", "through_ordinal"):
        assert score["target_" + key] == expected[key], "wrong assessed turn boundary: " + key


def native_decision_receipt(product, since_ns, expected_stage=None):
    """Prove the real selected branch, rather than accepting a classifier call plus fallback."""
    deadline = time.monotonic() + 30
    while True:
        sessions = product.public_cli("sessions list --include-unlinked --limit 100")[1]["data"]["sessions"]
        decisions, seen, receipts = [], set(), {}
        for session in sessions:
            detail = product.public_cli("sessions show " + session["session_id"])[1]["data"]
            for turn in detail["turns"]:
                for receipt_id in turn["receipt_ids"]:
                    if receipt_id in seen:
                        continue
                    seen.add(receipt_id)
                    receipt = product.public_cli("sessions receipt " + receipt_id)[1]["data"]
                    if receipt.get("outcome") != "accepted" or receipt["trust"].get("agent_plan_id") != product.plan_id:
                        continue
                    receipts[receipt["request_id"]] = receipt
                    for event in receipt["ordered_facts"]:
                        if event["fact"]["kind"] == "route_decision" and event["occurred_at_unix_nanos"] >= since_ns:
                            scores = [row["fact"] for row in receipt["ordered_facts"]
                                      if row["fact"]["kind"] == "branch_assessment_recorded"]
                            decisions.append({"receipt_id": receipt_id, "request_id": receipt["request_id"],
                                              "decision": event["fact"].get("complexity"), "assessments": scores})
        if decisions:
            assert len(decisions) == 1, "native prompt did not have exactly one accepted routing decision"
            decision = decisions[0]["decision"]
            assert decision and decision["decision_source"] == "external_classifier", \
                "native classification request did not produce an actual external decision"
            assert not decision["fallback_used"] and decision["branch_id"] == "native", \
                "native prompt used fallback or ignored the selected non-default branch"
            result = decisions[0]
            if receipt_facts(receipts[result["request_id"]], "agent_turn_finished") and (
                    expected_stage is None or result["assessments"]):
                if expected_stage is not None:
                    assert len(result["assessments"]) == 1, "native request did not assess exactly one stage"
                    assert_assessed_stage(expected_stage, result["assessments"][0], result["request_id"])
                result["completed_stage"] = freeze_completed_stage(receipts, result["request_id"])
                return result
        assert time.monotonic() < deadline, "native routing receipt was not retained"
        time.sleep(.05)


def assert_assessed_execution(bodies, prompt, output, current):
    """Bind an assessment to the previous actual input/output, excluding current input."""
    assert bodies, "no native classification was observed"
    for body in bodies:
        target = body.get("preceding_stage")
        assert isinstance(target, dict), "previous captured execution is not assessed"
        start = target.get("from")
        visible = body["visible_conversation"]
        assert type(start) is int and 0 <= start < len(visible), "invalid assessment boundary"
        assessed = json.dumps(visible[start:])
        assert prompt in assessed and output in assessed, "assessment lost actual previous execution"
        assert current not in assessed, "assessment incorrectly includes the new input"
        assert current in json.dumps(body["latest_user"]), "new native input was not classified"


def exercise_native_history(product, binary, node, selector, oracle, decisions, harness):
    """decisions() returns actual custom-service request bodies in arrival order."""
    provider, model = selector.split("/", 1)
    session = "native-autoload-" + secrets.token_hex(10) if harness == "pi" else None
    required = list(HISTORY_CASES)
    report = {"required_cases": required, "selected_cases": required, "cases": [], "routing_receipts": []}

    def ask(label, disabled=False, expected_stage=None):
        nonlocal session
        prompt = oracle.arm(label)
        before = len(oracle.calls)
        since_ns = time.time_ns()
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
        report["routing_receipts"].append({"label": label,
            **native_decision_receipt(product, since_ns, expected_stage)})
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
    first_output = oracle.receipt
    first_stage = report["routing_receipts"][-1]["completed_stage"]
    before = len(decisions())
    second = ask("adapter-second", expected_stage=first_stage)
    actual = decisions()[before:]
    assert_assessed_execution(actual, first, first_output, second)
    report["cases"].append({"id": required[0], "state": "green", "native_session_id": session})

    with missing_adapter():
        disabled_a = ask("adapter-disabled-first", True)
        disabled_output = oracle.receipt
        disabled_stage = report["routing_receipts"][-1]["completed_stage"]
        before = len(decisions())
        disabled_b = ask("adapter-disabled-second", True, disabled_stage)
    actual = decisions()[before:]
    assert_assessed_execution(actual, disabled_a, disabled_output, disabled_b)
    assert all("unverified_request_user_origin" in json.dumps(body["visible_conversation"])
               for body in actual), "missing adapter fabricated verified input origin"
    report["cases"].append({"id": required[1], "state": "green", "native_session_id": session})

    # Missing enrichment does not erase the actual execution captured on the
    # observed lane. Restoring enrichment must not create an artificial gap.
    disabled_output = oracle.receipt
    disabled_stage = report["routing_receipts"][-1]["completed_stage"]
    before = len(decisions())
    restored = ask("adapter-restored", expected_stage=disabled_stage)
    actual = decisions()[before:]
    assert_assessed_execution(actual, disabled_b, disabled_output, restored)
    product.stop()
    product.start()
    before = len(decisions())
    restart_a = ask("adapter-gateway-restart")
    actual = decisions()[before:]
    assert actual and all(body["history_partial"] and body["visible_conversation"] == []
                         and body["preceding_stage"] is None for body in actual), \
        "Gateway restart revived an unproven accepted execution"
    restart_output = oracle.receipt
    restart_stage = report["routing_receipts"][-1]["completed_stage"]
    before = len(decisions())
    restart_b = ask("adapter-gateway-restart-followup", expected_stage=restart_stage)
    assert_assessed_execution(decisions()[before:], restart_a, restart_output, restart_b)
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
