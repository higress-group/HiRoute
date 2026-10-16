"""HiRoute's native Hermes request adapter. No tools or outbound clients."""
from collections.abc import Mapping


def _identity(value):
    try:
        encoded = value.encode("utf-8") if isinstance(value, str) else b""
    except UnicodeEncodeError:
        return False
    return (
        isinstance(value, str)
        and value.isascii()
        and 0 < len(encoded) <= 128
        and not any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in value)
    )


def _token(value):
    return (
        isinstance(value, str)
        and 16 <= len(value) <= 128
        and all(c.isascii() and (c.isalnum() or c in "._~-") for c in value)
    )


def _adapter(config):
    """Read only the host's effective, already selected native configuration."""
    try:
        entry = config["plugins"]["entries"]["hiroute"]
        provider_id = entry["provider_id"]
        provider = config["providers"][provider_id]
        route = ("custom", entry["model"], entry["base_url"], entry["api_mode"])
        bearer = provider["extra_headers"]["X-HiRoute-Token"]
        if not (
            isinstance(provider_id, str)
            and provider_id.startswith("hiroute-main-")
            and config["model"]["provider"] == provider_id
            and config["model"]["default"] == route[1]
            and provider["default_model"] == route[1]
            and provider["api"] == route[2]
            and provider["transport"] == route[3]
            and route[3] in ("chat_completions", "codex_responses", "anthropic_messages")
            and _token(bearer)
        ):
            return None
    except (KeyError, TypeError):
        return None

    def llm_request(request, **context):
        selected = tuple(context.get(k) for k in ("provider", "model", "base_url", "api_mode"))
        if selected != route or not isinstance(request, dict):
            return None
        existing = request.get("extra_headers")
        if existing is not None and not isinstance(existing, Mapping):
            return None
        # Case-insensitive replacement avoids duplicate identity or authorization headers.
        headers = {
            k: v for k, v in (existing or {}).items()
            if not isinstance(k, str) or k.lower() not in ("x-hiroute-token", "x-hermes-session-id")
        }
        headers["X-HiRoute-Token"] = bearer
        session = context.get("session_id")
        if _identity(session):
            headers["X-Hermes-Session-Id"] = session
        return {"request": {**request, "extra_headers": headers}, "source": "hiroute"}

    return llm_request


def register(ctx):
    from hermes_cli.config import load_config_readonly

    adapter = _adapter(load_config_readonly() or {})
    if adapter is not None:
        ctx.register_middleware("llm_request", adapter)
