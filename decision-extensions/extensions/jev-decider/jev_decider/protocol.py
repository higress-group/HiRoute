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
        return json.loads(data, object_pairs_hook=_strict_object)
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
            "branches",
            "latest_user",
            "visible_conversation",
            "history_partial",
            "assessment_from",
        },
    )
    branches = value["branches"]
    if not isinstance(branches, dict) or not branches:
        raise ProtocolError("branches must be a non-empty object")
    for branch, description in branches.items():
        _text(branch)
        _text(description)
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
        _exact(turn, {"branch_id", "user", "status", "steps"}, {"executed_branch_id"})
        if turn["branch_id"] is not None:
            _text(turn["branch_id"])
        if "executed_branch_id" in turn:
            _text(turn["executed_branch_id"])
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
    assessment_from = value["assessment_from"]
    if assessment_from is not None and (
        isinstance(assessment_from, bool)
        or not isinstance(assessment_from, int)
        or assessment_from < 0
        or assessment_from >= len(conversation)
    ):
        raise ProtocolError("assessment_from is outside visible_conversation")
    return value


def encode_json(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()
