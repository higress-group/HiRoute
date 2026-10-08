"""Strict HiRoute Decision API values and JSON encoding; no transport or I/O."""
from __future__ import annotations

import json
from typing import Any


class ProtocolError(ValueError):
    pass


class DuplicateField(ProtocolError):
    pass


def _strict_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise DuplicateField(f"duplicate field: {key}")
        value[key] = item
    return value


def strict_json(data: bytes) -> Any:
    try:
        return json.loads(data, object_pairs_hook=_strict_object, parse_constant=lambda _: (_ for _ in ()).throw(ProtocolError("non-finite number")))
    except (UnicodeDecodeError, json.JSONDecodeError, DuplicateField) as error:
        raise ProtocolError("invalid JSON") from error


def _exact(value: dict[str, Any], fields: set[str], optional: set[str] | None = None) -> None:
    optional = optional or set()
    if set(value) - fields - optional or fields - set(value):
        raise ProtocolError("unexpected or missing field")


def _text(value: Any, *, empty: bool = False) -> str:
    if not isinstance(value, str) or (not empty and not value):
        raise ProtocolError("expected text")
    return value


def _content_part(value: Any, *, tool: bool) -> None:
    if not isinstance(value, dict):
        raise ProtocolError("content part must be an object")
    kind = value.get("kind")
    if kind == "text":
        _exact(value, {"kind", "text"})
        _text(value["text"], empty=True)
    elif kind == "unavailable":
        _exact(value, {"kind", "source_kind"})
        _text(value["source_kind"])
    elif tool and kind == "tool_activity":
        _exact(value, {"kind", "tool", "status"})
        _text(value["tool"])
        if value["status"] not in {"completed", "failed", "unknown"}:
            raise ProtocolError("invalid tool status")
    else:
        raise ProtocolError("unsupported content part")


def validate_hiroute_request(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ProtocolError("request must be an object")
    _exact(
        value,
        {
            "decision",
            "latest_user",
            "visible_conversation",
            "history_partial",
            "assessment_target",
        },
    )
    _definition(value["decision"])
    latest = value["latest_user"]
    if not isinstance(latest, list) or not latest:
        raise ProtocolError("latest_user must be a non-empty array")
    for part in latest:
        _content_part(part, tool=False)
    conversation = value["visible_conversation"]
    if not isinstance(conversation, list):
        raise ProtocolError("visible_conversation must be an array")
    for turn in conversation:
        if not isinstance(turn, dict):
            raise ProtocolError("turn must be an object")
        _exact(turn, {"user", "status", "steps"})
        if turn["status"] not in {"completed", "failed", "interrupted", "unknown"}:
            raise ProtocolError("invalid turn status")
        if not isinstance(turn["user"], list):
            raise ProtocolError("turn user must be an array")
        for part in turn["user"]:
            _content_part(part, tool=False)
        if not isinstance(turn["steps"], list):
            raise ProtocolError("turn steps must be an array")
        for step in turn["steps"]:
            if not isinstance(step, list):
                raise ProtocolError("step must be an array")
            for part in step:
                _content_part(part, tool=True)
    if not isinstance(value["history_partial"], bool):
        raise ProtocolError("history_partial must be boolean")
    target = value["assessment_target"]
    if target is not None:
        if not isinstance(target, dict):
            raise ProtocolError("assessment_target must be an object or null")
        _exact(target, {"from", "instructions", "criteria"})
        start = target["from"]
        if type(start) is not int or not 0 <= start < len(conversation):
            raise ProtocolError("assessment_target.from is outside visible_conversation")
        _text(target["instructions"])
        criteria = target["criteria"]
        if not isinstance(criteria, list) or len(criteria) != 3:
            raise ProtocolError("assessment requires three anchors")
        for index, item in enumerate(criteria):
            if not isinstance(item, dict):
                raise ProtocolError("assessment anchor must be an object")
            _exact(item, {"score", "criterion"})
            if type(item["score"]) not in (float, int) or item["score"] != index / 2:
                raise ProtocolError("assessment anchors must be 0, 0.5, 1 in order")
            _text(item["criterion"])
    return value


def encode_json(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


def _definition(value: Any, *, refinement: bool = False) -> None:
    if not isinstance(value, dict):
        raise ProtocolError("decision definition must be an object")
    kind = value.get("kind")
    if kind not in ({"ordinal"} if refinement else {"ordinal", "categorical"}):
        raise ProtocolError("unsupported decision kind")
    key = "levels" if kind == "ordinal" else "options"
    _exact(value, {"kind", "instructions", key})
    _text(value["instructions"])
    entries = value[key]
    if not isinstance(entries, list) or not 2 <= len(entries) <= 16:
        raise ProtocolError("decision needs 2–16 distinct entries")
    ids: set[str] = set()
    for entry in entries:
        if not isinstance(entry, dict):
            raise ProtocolError("decision entry must be an object")
        _exact(entry, {"id", "criterion"}, {"refinement"} if kind == "categorical" else set())
        identifier = _text(entry["id"])
        _text(entry["criterion"])
        if identifier in ids:
            raise ProtocolError("duplicate allowed ID")
        ids.add(identifier)
        if "refinement" in entry:
            _definition(entry["refinement"], refinement=True)
