"""Deterministic Jev request preparation and answer interpretation.

This module performs no I/O and owns no sessions, credentials or deadlines.
The optional trace dictionary receives only the existing diagnostic facts,
including preparation facts when a later validation step fails.
"""
from __future__ import annotations

import hashlib
import json
import math
from dataclasses import dataclass
from typing import Any, Protocol

from .protocol import ProtocolError, encode_json


class DecisionSettings(Protocol):
    """Only the non-secret configuration consumed by a decision."""

    mode: str
    model: str
    max_state_tokens: int
    simple_threshold: float
    competence_floor: float
    branch_criteria: dict[str, str]
    policy_configured: bool


@dataclass(frozen=True)
class PreparedDecision:
    body: dict[str, Any]
    target_trimmed: bool


SIMPLE_BRANCH = "smart_saving_simple"


COMPLEX_BRANCH = "smart_saving_complex"


COMPETENCE_CRITERIA = [
    "The branch was not competent for the task: it failed to make useful progress, repeatedly made avoidable errors, or required substantial correction.",
    "The branch made useful but incomplete or uneven progress; the available evidence does not establish consistently competent execution.",
    "The branch was competent for the task: it advanced or completed the work reliably with an appropriate process and no material correction.",
]


def criteria_hash(branches: dict[str, str]) -> str:
    return hashlib.sha256(json.dumps(branches, sort_keys=True, ensure_ascii=False).encode()).hexdigest()


def prepare_state(value: dict[str, Any], max_tokens: int) -> tuple[dict[str, Any], bool]:
    state = {
        "branches": value["branches"],
        "latest_user": value["latest_user"],
        "visible_conversation": list(value["visible_conversation"]),
        "history_partial": value["history_partial"],
        "assessment_from": value["assessment_from"],
    }
    original_from = value["assessment_from"]
    removed = 0
    while len(encode_json(state)) > max_tokens and state["visible_conversation"]:
        state["visible_conversation"].pop(0)
        removed += 1
        if original_from is not None:
            if removed >= len(value["visible_conversation"]):
                state["assessment_from"] = None
            else:
                state["assessment_from"] = max(0, original_from - removed)
    if removed:
        state["history_partial"] = True
    if len(encode_json(state)) > max_tokens:
        raise ProtocolError("branches and latest_user exceed the configured Jev context budget")
    target_trimmed = original_from is not None and removed > original_from
    return state, target_trimmed


def questions(mode: str, branches: dict[str, str], has_target: bool) -> dict[str, Any]:
    if mode == "rules" and set(branches) != {SIMPLE_BRANCH, COMPLEX_BRANCH}:
        raise ProtocolError("rules mode requires the two smart-saving branches")
    if mode == "auto":
        decision = {
            "type": "choice",
            "instructions": (
                "Choose the best branch for the current routing round's latest_user. It may continue the "
                "same user task, repeat prior text, or be a summary after a HiRoute rerouting boundary. "
                "Consider the task's complexity and the visible history of how prior branches performed. "
                "Preserve quality; use an economy branch only when it is appropriate for this work."
            ),
            "criteria": branches,
        }
    else:
        decision = {
            "type": "choice",
            "instructions": (
                "Classify the current routing round's latest_user by task complexity. It may continue the "
                "same user task, repeat prior text, or be a summary. Use history only to resolve references "
                "and understand the actual work; do not call a simple task complex merely because a prior "
                "tool failed."
            ),
            "criteria": {
                SIMPLE_BRANCH: branches[SIMPLE_BRANCH],
                COMPLEX_BRANCH: branches[COMPLEX_BRANCH],
            },
        }
    result: dict[str, Any] = {"branch" if mode == "auto" else "complexity": decision}
    if has_target:
        result["competence"] = {
            "type": "score",
            "instructions": (
                "Rate whether the branch used by the assessable tail of visible_conversation was competent "
                "for those routing rounds, including rounds with unknown terminal status. Consider progress, "
                "tool activity, failed and recovered attempts, round trips, and explicit feedback in latest_user. "
                "Repeated text, a summary, a continuation message, or silence is not by itself praise or a "
                "complaint. Do not rate the current not-yet-executed routing round."
            ),
            "criteria": COMPETENCE_CRITERIA,
        }
    return result


def _choice(answer: Any, branches: dict[str, str], *, probabilities: bool) -> tuple[str, dict[str, float]]:
    if not isinstance(answer, dict) or answer.get("type") != "choice":
        raise ProtocolError("missing or invalid Choice answer")
    selected = answer.get("choice")
    if selected not in branches:
        raise ProtocolError("Choice answer selected an unknown branch")
    if not probabilities:
        return selected, {}
    distribution = answer.get("probabilities")
    if not isinstance(distribution, dict) or set(distribution) != set(branches):
        raise ProtocolError("Choice probabilities do not match branches")
    parsed: dict[str, float] = {}
    for branch, probability in distribution.items():
        if isinstance(probability, bool) or not isinstance(probability, (int, float)):
            raise ProtocolError("Choice probability is not numeric")
        probability = float(probability)
        if not math.isfinite(probability) or not 0 <= probability <= 1:
            raise ProtocolError("Choice probability is outside [0,1]")
        parsed[branch] = probability
    if not math.isclose(sum(parsed.values()), 1.0, abs_tol=1e-6):
        raise ProtocolError("Choice probabilities do not sum to one")
    if parsed[selected] + 1e-12 < max(parsed.values()):
        raise ProtocolError("Choice is inconsistent with its probabilities")
    return selected, parsed


def _competence(answer: Any) -> float | None:
    if not isinstance(answer, dict) or answer.get("type") != "score":
        return None
    score = answer.get("score")
    if isinstance(score, bool) or not isinstance(score, (int, float)):
        return None
    score = float(score)
    if not math.isfinite(score) or not 0 <= score <= 2:
        return None
    return score / 2


def decision_response(
    settings: DecisionSettings,
    request: dict[str, Any],
    upstream: Any,
    target_trimmed: bool,
    trace: dict[str, Any] | None = None,
) -> dict[str, Any]:
    trace = trace if trace is not None else {}
    if not isinstance(upstream, dict) or not isinstance(upstream.get("answers"), dict):
        raise ProtocolError("upstream response has no answers")
    answers = upstream["answers"]
    if settings.mode == "auto":
        branch, _ = _choice(answers.get("branch"), request["branches"], probabilities=False)
        trace.update(upstream_choice=branch, reason="auto_choice")
    else:
        selected, distribution = _choice(
            answers.get("complexity"), request["branches"], probabilities=True
        )
        competence = _competence(answers.get("competence"))
        branch = (
            SIMPLE_BRANCH
            if distribution[SIMPLE_BRANCH] >= settings.simple_threshold
            and (competence is None or competence >= settings.competence_floor)
            else COMPLEX_BRANCH
        )
        opportunity = distribution[SIMPLE_BRANCH] >= settings.simple_threshold
        guard = competence is None or competence >= settings.competence_floor
        trace.update(
            upstream_choice=selected, probabilities=distribution,
            simple_threshold=settings.simple_threshold,
            competence_floor=settings.competence_floor,
            simplicity_passed=opportunity, competence_passed=guard,
            reason=("complexity_threshold" if not opportunity else
                    "competence_guard" if not guard else "economy_eligible"),
        )
    trace.update(
        competence=_competence(answers.get("competence")),
        competence_status=("missing" if "competence" not in answers else
                           "invalid" if _competence(answers["competence"]) is None else "valid"),
        branch_id=branch,
    )
    response: dict[str, Any] = {"branch_id": branch}
    if request["assessment_from"] is not None:
        score = _competence(answers.get("competence"))
        if score is not None:
            response["assessment"] = {"score": score, "partial": target_trimmed}
    return response


def reported_usage(upstream: Any) -> dict[str, int | float] | None:
    """Keep only reported numeric billing facts, never guess a missing charge."""
    usage = upstream.get("usage") if isinstance(upstream, dict) else None
    if not isinstance(usage, dict):
        return None
    result: dict[str, int | float] = {}
    for name in ("input_tokens", "output_tokens"):
        value = usage.get(name)
        if type(value) is int and value >= 0:
            result[name] = value
    cost = usage.get("cost")
    if type(cost) in (int, float) and cost >= 0:
        try:
            if math.isfinite(cost):
                result["cost"] = cost
        except OverflowError:
            pass
    return result or None


def prepare_decision(
    settings: DecisionSettings,
    incoming: dict[str, Any],
    trace: dict[str, Any] | None = None,
) -> PreparedDecision:
    """Prepare one upstream request from an already validated HiRoute request.

    Policy precedence, whole-turn trimming and question text are the existing
    service behavior. Neither preparation nor answer interpretation calls Jev.
    """
    trace = trace if trace is not None else {}
    criteria_source = "request"
    if set(incoming["branches"]) == {SIMPLE_BRANCH, COMPLEX_BRANCH}:
        incoming = {**incoming, "branches": settings.branch_criteria}
        criteria_source = "policy" if settings.policy_configured else "default"
    elif settings.policy_configured:
        raise ProtocolError("configured policy requires the two smart-saving branches")
    trace.update(criteria_source=criteria_source,
                 criteria_sha256=criteria_hash(incoming["branches"]))
    state, target_trimmed = prepare_state(incoming, settings.max_state_tokens)
    trace.update(
        input_turns=len(incoming["visible_conversation"]),
        retained_turns=len(state["visible_conversation"]),
        state_bytes=len(encode_json(state)), history_partial=state["history_partial"],
        assessment_from=state["assessment_from"], target_trimmed=target_trimmed,
    )
    body = {
        "model": settings.model,
        "state": state,
        "questions": questions(
            settings.mode,
            incoming["branches"],
            state["assessment_from"] is not None,
        ),
    }
    return PreparedDecision(body=body, target_trimmed=target_trimmed)
