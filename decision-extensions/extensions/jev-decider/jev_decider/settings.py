"""Deployment configuration and bounded policy/key-file loading."""
from __future__ import annotations

import math
import os
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import urlsplit

from .decision import COMPLEX_BRANCH, SIMPLE_BRANCH
from .protocol import ProtocolError, encode_json, strict_json


DEFAULT_MODEL = "typesafe/jev-1.13"


DEFAULT_UPSTREAM = "https://openrouter.ai/api/alpha/decisions"


MAX_REQUEST_TIMEOUT_SECONDS = 3600.0


FORBIDDEN_INBOUND_HEADERS = {
    "host",
    "content-length",
    "content-type",
    "transfer-encoding",
    "connection",
    "te",
    "trailer",
    "upgrade",
}


@dataclass(frozen=True)
class Settings:
    mode: str
    api_key: str
    upstream_url: str
    model: str
    request_timeout_seconds: float
    max_state_tokens: int
    max_concurrency: int
    simple_threshold: float
    competence_floor: float
    inbound_header_name: str | None
    inbound_header_value: str | None
    branch_criteria: dict[str, str] = field(default_factory=lambda: load_policy(None))
    policy_configured: bool = False

    @classmethod
    def from_env(cls) -> "Settings":
        mode = os.getenv("JEV_MODE", "auto")
        key_file = os.getenv("OPENROUTER_API_KEY_FILE", "")
        if not key_file:
            raise RuntimeError("OPENROUTER_API_KEY_FILE is required")
        try:
            key_path = Path(key_file)
            if not key_path.is_absolute():
                raise RuntimeError("OPENROUTER_API_KEY_FILE must be an absolute path")
            key_bytes = key_path.read_bytes()
        except OSError as error:
            raise RuntimeError("OPENROUTER_API_KEY_FILE cannot be read") from error
        if len(key_bytes) > 16 * 1024:
            raise RuntimeError("OPENROUTER_API_KEY_FILE is too large")
        try:
            api_key = key_bytes.decode("utf-8").strip()
        except UnicodeDecodeError as error:
            raise RuntimeError("OPENROUTER_API_KEY_FILE must contain UTF-8 text") from error
        upstream_url = os.getenv("OPENROUTER_DECISIONS_URL", DEFAULT_UPSTREAM)
        timeout = _finite_env("JEV_REQUEST_TIMEOUT_SECONDS", 2.8)
        max_state_tokens = _positive_int_env("JEV_MAX_STATE_TOKENS", 24_000)
        max_concurrency = _positive_int_env("JEV_MAX_CONCURRENCY", 32)
        if mode == "auto" and (
            "JEV_SIMPLE_THRESHOLD" in os.environ
            or "JEV_COMPETENCE_FLOOR" in os.environ
        ):
            raise RuntimeError("rules thresholds are not accepted in auto mode")
        simple_threshold = _probability_env("JEV_SIMPLE_THRESHOLD", 0.80)
        competence_floor = _probability_env("JEV_COMPETENCE_FLOOR", 0.50)
        header_name = os.getenv("DECIDER_AUTH_HEADER_NAME")
        header_value = os.getenv("DECIDER_AUTH_HEADER_VALUE")
        if mode not in {"auto", "rules"}:
            raise RuntimeError("JEV_MODE must be auto or rules")
        if not api_key or any(character in api_key for character in "\r\n"):
            raise RuntimeError("OPENROUTER_API_KEY_FILE contains an invalid key")
        parsed = urlsplit(upstream_url)
        if parsed.scheme not in {"http", "https"} or not parsed.netloc or parsed.fragment:
            raise RuntimeError("OPENROUTER_DECISIONS_URL must be an HTTP(S) URL without a fragment")
        if timeout <= 0 or timeout > MAX_REQUEST_TIMEOUT_SECONDS:
            raise RuntimeError("JEV_REQUEST_TIMEOUT_SECONDS must be in (0, 3600]")
        if bool(header_name) != bool(header_value):
            raise RuntimeError("both DECIDER_AUTH_HEADER_NAME and DECIDER_AUTH_HEADER_VALUE are required")
        if header_name is not None and not _valid_header_name(header_name):
            raise RuntimeError("DECIDER_AUTH_HEADER_NAME is invalid")
        if header_value is not None and (
            not header_value or any(character in header_value for character in "\r\n")
        ):
            raise RuntimeError("DECIDER_AUTH_HEADER_VALUE is invalid")
        return cls(
            mode=mode,
            api_key=api_key,
            upstream_url=upstream_url,
            model=os.getenv("JEV_MODEL", DEFAULT_MODEL),
            request_timeout_seconds=timeout,
            max_state_tokens=max_state_tokens,
            max_concurrency=max_concurrency,
            simple_threshold=simple_threshold,
            competence_floor=competence_floor,
            inbound_header_name=header_name,
            inbound_header_value=header_value,
            branch_criteria=load_policy(os.getenv("JEV_POLICY_FILE")),
            policy_configured="JEV_POLICY_FILE" in os.environ,
        )


def load_policy(filename: str | None) -> dict[str, str]:
    path = Path(filename) if filename is not None else Path(__file__).with_name("policy.default.json")
    if not path.is_absolute():
        raise RuntimeError("JEV_POLICY_FILE must be an absolute path")
    try:
        with path.open("rb") as stream:
            data = stream.read(4097)
    except OSError as error:
        raise RuntimeError("JEV_POLICY_FILE cannot be read") from error
    if len(data) > 4096:
        raise RuntimeError("JEV_POLICY_FILE must be at most 4096 bytes")
    try:
        data.decode("utf-8")
        value = strict_json(data)
        if not isinstance(value, dict) or set(value) != {"simple", "complex"}:
            raise ProtocolError("invalid policy fields")
        if any(not isinstance(text, str) or not text.strip() for text in value.values()):
            raise ProtocolError("invalid policy descriptions")
        # Validate escaped Unicode too, before a request attempts UTF-8 encoding.
        encode_json(value)
    except (ProtocolError, UnicodeError) as error:
        raise RuntimeError("JEV_POLICY_FILE requires UTF-8 JSON with nonempty simple and complex strings") from error
    return {SIMPLE_BRANCH: value["simple"], COMPLEX_BRANCH: value["complex"]}


def _finite_env(name: str, default: float) -> float:
    try:
        value = float(os.getenv(name, str(default)))
    except ValueError as error:
        raise RuntimeError(f"{name} must be a finite number") from error
    if not math.isfinite(value):
        raise RuntimeError(f"{name} must be a finite number")
    return value


def _positive_int_env(name: str, default: int) -> int:
    try:
        value = int(os.getenv(name, str(default)))
    except ValueError as error:
        raise RuntimeError(f"{name} must be a positive integer") from error
    if value <= 0:
        raise RuntimeError(f"{name} must be a positive integer")
    return value


def _probability_env(name: str, default: float) -> float:
    value = _finite_env(name, default)
    if not 0 <= value <= 1:
        raise RuntimeError(f"{name} must be in [0, 1]")
    return value


def _valid_header_name(value: str) -> bool:
    token = "!#$%&'*+-.^_`|~"
    return (
        0 < len(value) <= 128
        and all(character.isascii() and (character.isalnum() or character in token) for character in value)
        and value.lower() not in FORBIDDEN_INBOUND_HEADERS
    )
