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
            "route",
            "latest_user",
            "visible_conversation",
            "history_partial",
            "preceding_stage",
        },
    )
    _route(value["route"])
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
    target = value["preceding_stage"]
    if target is not None:
        if not isinstance(target, dict):
            raise ProtocolError("preceding_stage must be an object or null")
        _exact(target, {"from", "category", "group", "complete"})
        start = target["from"]
        if type(start) is not int or not 0 <= start < len(conversation):
            raise ProtocolError("preceding_stage.from is outside visible_conversation")
        categories = {"smart_saving": ["regular", "primary"]} if value["route"]["kind"] == "smart_saving" else {c["id"]: c["groups"] for c in value["route"]["categories"]}
        if target["category"] not in categories or target["group"] not in categories[target["category"]] or type(target["complete"]) is not bool:
            raise ProtocolError("preceding_stage is outside the published route")
    return value


def encode_json(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


def _route(value: Any) -> None:
    if not isinstance(value, dict):
        raise ProtocolError("route must be an object")
    if value.get("kind") == "smart_saving":
        _exact(value, {"kind", "groups"})
        if value["groups"] != ["regular", "primary"]:
            raise ProtocolError("invalid smart-saving groups")
    elif value.get("kind") == "branches":
        _exact(value, {"kind", "categories", "default_category"})
        categories = value["categories"]
        if not isinstance(categories, list) or not 2 <= len(categories) <= 16:
            raise ProtocolError("route needs 2–16 categories")
        ids = set()
        for category in categories:
            if not isinstance(category, dict):
                raise ProtocolError("category must be an object")
            _exact(category, {"id", "condition", "groups"})
            identifier = _text(category["id"])
            if len(identifier) > 128 or identifier == "smart_saving" or identifier in ids:
                raise ProtocolError("invalid category ID")
            ids.add(identifier)
            _text(category["condition"])
            if category["groups"] not in (["regular"], ["regular", "primary"]):
                raise ProtocolError("invalid category groups")
        if value["default_category"] not in ids:
            raise ProtocolError("default category is not published")
    else:
        raise ProtocolError("unknown route kind")
