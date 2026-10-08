"""One provider call; HiRoute owns grouping, thresholds and frozen assessment standards."""
from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Any, Protocol
from .protocol import ProtocolError, encode_json

class DecisionSettings(Protocol):
    model: str
    max_request_bytes: int

@dataclass(frozen=True)
class PreparedDecision:
    body: dict[str, Any]
    definition: dict[str, Any]
    choice: str | None
    degrees: dict[str, str]
    assessment: str | None
    target_trimmed: bool

def prepare_decision(settings: DecisionSettings, incoming: dict[str, Any], trace: dict[str, Any] | None = None) -> PreparedDecision:
    questions: dict[str, Any] = {}
    degrees: dict[str, str] = {}
    definition = incoming["decision"]
    def add(question: dict[str, Any]) -> str:
        identifier = f"q{len(questions)}"
        questions[identifier] = question
        return identifier
    def degree(value: dict[str, Any]) -> str:
        return add({"type": "score", "instructions": "Assess state.latest_user only, not historical competence. " + value["instructions"], "criteria": [level["criterion"] for level in value["levels"]]})
    choice = None
    if definition["kind"] == "ordinal":
        degrees["root"] = degree(definition)
    else:
        choice = add({"type": "choice", "instructions": definition["instructions"], "criteria": {option["id"]: option["criterion"] for option in definition["options"]}})
        for option in definition["options"]:
            if "refinement" in option:
                degrees[option["id"]] = degree(option["refinement"])
    target = incoming["assessment_target"]
    assessment = None
    if target is not None:
        assessment = add({"type": "score", "instructions": "Evaluate only the completed stage from state.visible_conversation[state.assessment_from] through the end. state.latest_user is not yet executed. " + target["instructions"], "criteria": [item["criterion"] for item in target["criteria"]]})
    state = {"latest_user": incoming["latest_user"], "visible_conversation": list(incoming["visible_conversation"]), "history_partial": incoming["history_partial"], "assessment_from": target["from"] if target else None}
    body = {"model": settings.model, "state": state, "questions": questions}
    removed = 0
    while len(encode_json(body)) > settings.max_request_bytes and state["visible_conversation"]:
        state["visible_conversation"].pop(0)
        removed += 1
        state["history_partial"] = True
        if target:
            state["assessment_from"] = max(0, target["from"] - removed)
    if not state["visible_conversation"] and assessment:
        questions.pop(assessment)
        assessment = None
        state["assessment_from"] = None
    if len(encode_json(body)) > settings.max_request_bytes:
        raise ProtocolError("current input and questions exceed the configured context budget")
    if trace is not None:
        trace.update(kind=definition["kind"], question_count=len(questions), input_turns=len(incoming["visible_conversation"]), retained_turns=len(state["visible_conversation"]), request_bytes=len(encode_json(body)))
    return PreparedDecision(body, definition, choice, degrees, assessment, bool(target and removed > target["from"]))

def _degree(answer: Any, definition: dict[str, Any]) -> dict[str, Any]:
    levels = definition["levels"]
    empty = {"kind": "ordinal", "probabilities": {}}
    if not isinstance(answer, dict) or answer.get("type") != "score":
        return empty
    probabilities = answer.get("probabilities")
    if not isinstance(probabilities, dict) or set(probabilities) != {str(i) for i in range(len(levels))}:
        return empty
    if any(type(p) not in (int, float) or not math.isfinite(p) or not 0 <= p <= 1 for p in probabilities.values()):
        return empty
    total = sum(probabilities.values())
    if total == 0 or abs(total - 1) > 1e-6:
        return empty
    return {"kind": "ordinal", "probabilities": {level["id"]: probabilities[str(i)] / total for i, level in enumerate(levels)}}

def decision_response(prepared: PreparedDecision, upstream: Any, trace: dict[str, Any] | None = None) -> dict[str, Any]:
    if not isinstance(upstream, dict) or not isinstance(upstream.get("answers"), dict):
        raise ProtocolError("upstream response has no answers")
    answers = upstream["answers"]
    definition = prepared.definition
    if definition["kind"] == "ordinal":
        decision = _degree(answers.get(prepared.degrees["root"]), definition)
    else:
        answer = answers.get(prepared.choice)
        if not isinstance(answer, dict) or answer.get("type") != "choice":
            raise ProtocolError("invalid category answer")
        selected = next((option for option in definition["options"] if option["id"] == answer.get("choice")), None)
        if selected is None:
            raise ProtocolError("category answer is outside allowed IDs")
        decision = {"kind": "categorical", "choice": selected["id"]}
        if "refinement" in selected:
            decision["refinement"] = _degree(answers.get(prepared.degrees[selected["id"]]), selected["refinement"])
    response = {"decision": decision}
    answer = answers.get(prepared.assessment) if prepared.assessment else None
    if isinstance(answer, dict) and answer.get("type") == "score":
        score = answer.get("score")
        if type(score) in (int, float) and math.isfinite(score) and 0 <= score <= 2:
            response["assessment"] = {"score": score / 2, "partial": prepared.target_trimmed}
    if trace is not None:
        trace.update(assessment_present="assessment" in response, target_trimmed=prepared.target_trimmed)
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
