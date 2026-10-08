"""Deployment configuration and bounded policy/key-file loading."""
from __future__ import annotations

import math
import os
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlsplit



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
    api_key: str
    upstream_url: str
    model: str
    request_timeout_seconds: float
    max_request_bytes: int
    max_concurrency: int
    inbound_header_name: str | None
    inbound_header_value: str | None
    @classmethod
    def from_env(cls) -> "Settings":
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
        max_request_bytes = _positive_int_env("JEV_MAX_REQUEST_BYTES", 256 * 1024)
        max_concurrency = _positive_int_env("JEV_MAX_CONCURRENCY", 32)
        header_name = os.getenv("DECIDER_AUTH_HEADER_NAME")
        header_value = os.getenv("DECIDER_AUTH_HEADER_VALUE")
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
            api_key=api_key,
            upstream_url=upstream_url,
            model=os.getenv("JEV_MODEL", DEFAULT_MODEL),
            request_timeout_seconds=timeout,
            max_request_bytes=max_request_bytes,
            max_concurrency=max_concurrency,
            inbound_header_name=header_name,
            inbound_header_value=header_value,
        )


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


def _valid_header_name(value: str) -> bool:
    token = "!#$%&'*+-.^_`|~"
    return (
        0 < len(value) <= 128
        and all(character.isascii() and (character.isalnum() or character in token) for character in value)
        and value.lower() not in FORBIDDEN_INBOUND_HEADERS
    )
