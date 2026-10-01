from __future__ import annotations

import asyncio
import hashlib
import hmac
import json
import logging
import math
import os
import time
import uuid
from pathlib import Path
from dataclasses import dataclass, field
from typing import Any
from urllib.parse import urlsplit

from aiohttp import ClientError, ClientSession, ClientTimeout, web


SIMPLE_BRANCH = "smart_saving_simple"
COMPLEX_BRANCH = "smart_saving_complex"
DEFAULT_MODEL = "typesafe/jev-1.13"
DEFAULT_UPSTREAM = "https://openrouter.ai/api/alpha/decisions"
MAX_UPSTREAM_RESPONSE_BYTES = 64 * 1024
MAX_REQUEST_TIMEOUT_SECONDS = 3600.0
LOGGER = logging.getLogger("jev_decider")


def log_event(event: str, **fields: Any) -> None:
    LOGGER.info(json.dumps({"timestamp_ms": time.time_ns() // 1_000_000,
                            "event": event, **fields}, ensure_ascii=False))


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
COMPETENCE_CRITERIA = [
    "The branch was not competent for the task: it failed to make useful progress, repeatedly made avoidable errors, or required substantial correction.",
    "The branch made useful but incomplete or uneven progress; the available evidence does not establish consistently competent execution.",
    "The branch was competent for the task: it advanced or completed the work reliably with an appropriate process and no material correction.",
]


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
        _encoded(value)
    except (ProtocolError, UnicodeError) as error:
        raise RuntimeError("JEV_POLICY_FILE requires UTF-8 JSON with nonempty simple and complex strings") from error
    return {SIMPLE_BRANCH: value["simple"], COMPLEX_BRANCH: value["complex"]}


def criteria_hash(branches: dict[str, str]) -> str:
    return hashlib.sha256(json.dumps(branches, sort_keys=True, ensure_ascii=False).encode()).hexdigest()


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


def _encoded(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


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
    while len(_encoded(state)) > max_tokens and state["visible_conversation"]:
        state["visible_conversation"].pop(0)
        removed += 1
        if original_from is not None:
            if removed >= len(value["visible_conversation"]):
                state["assessment_from"] = None
            else:
                state["assessment_from"] = max(0, original_from - removed)
    if removed:
        state["history_partial"] = True
    if len(_encoded(state)) > max_tokens:
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
    settings: Settings,
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


async def health(_: web.Request) -> web.Response:
    return web.json_response({"status": "ok"})


async def _bounded_response(response: Any) -> bytes:
    declared = response.content_length
    if declared is not None and declared > MAX_UPSTREAM_RESPONSE_BYTES:
        raise ProtocolError("upstream response is too large")
    body = bytearray()
    async for chunk in response.content.iter_chunked(16 * 1024):
        if len(body) + len(chunk) > MAX_UPSTREAM_RESPONSE_BYTES:
            raise ProtocolError("upstream response is too large")
        body.extend(chunk)
    return bytes(body)


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


async def decide(request: web.Request) -> web.Response:
    decision_id = uuid.uuid4().hex
    started = time.monotonic()
    trace: dict[str, Any] = {"decision_id": decision_id, "phase": "authentication",
                             "upstream_usage": None}
    settings: Settings = request.app[SETTINGS]
    log_event("decision_started", decision_id=decision_id, mode=settings.mode, model=settings.model)

    def finish(body: dict[str, Any], status: int = 200) -> web.Response:
        log_event("decision_completed", **trace, status=status,
                  error=body.get("error"), duration_ms=round((time.monotonic() - started) * 1000, 2))
        return web.json_response(body, status=status, headers={"X-Jev-Decision-Id": decision_id})

    if settings.inbound_header_name is not None:
        supplied = request.headers.get(settings.inbound_header_name, "")
        if not hmac.compare_digest(supplied, settings.inbound_header_value or ""):
            return finish({"error": "unauthorized"}, status=401)
    upstream_started = False
    try:
        async with asyncio.timeout(settings.request_timeout_seconds):
            trace["phase"] = "validation"
            incoming = validate_hiroute_request(strict_json(await request.read()))
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
                state_bytes=len(_encoded(state)), history_partial=state["history_partial"],
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
            session: ClientSession = request.app[CLIENT]
            upstream_started = True
            trace["phase"] = "queue"
            queued = time.monotonic()
            async with request.app[CONCURRENCY]:
                trace.update(queue_ms=round((time.monotonic() - queued) * 1000, 2), phase="upstream")
                async with session.post(
                    settings.upstream_url,
                    json=body,
                    headers={
                        "Authorization": f"Bearer {settings.api_key}",
                        "Content-Type": "application/json",
                        "Accept": "application/json",
                    },
                    allow_redirects=False,
                ) as upstream_response:
                    trace["upstream_status"] = upstream_response.status
                    if upstream_response.status != 200:
                        return finish(
                            {"error": "upstream_rejected", "upstream_status": upstream_response.status},
                            status=502,
                        )
                    upstream = strict_json(await _bounded_response(upstream_response))
                    trace["upstream_usage"] = reported_usage(upstream)
            trace["phase"] = "decision"
            result = decision_response(settings, state, upstream, target_trimmed, trace)
            return finish(result)
    except ProtocolError as error:
        trace["validation_error"] = str(error)
        if upstream_started:
            return finish({"error": "upstream_invalid"}, status=502)
        return finish(
            {"error": str(error)},
            status=413 if "context budget" in str(error) else 400,
        )
    except asyncio.TimeoutError:
        return finish({"error": "upstream_timeout"}, status=504)
    except ClientError:
        return finish({"error": "upstream_invalid"}, status=502)
    except asyncio.CancelledError:
        log_event("decision_cancelled", **trace,
                  duration_ms=round((time.monotonic() - started) * 1000, 2))
        raise
    except Exception as error:
        log_event("decision_failed", **trace, error_type=type(error).__name__,
                  duration_ms=round((time.monotonic() - started) * 1000, 2))
        raise


async def _client(app: web.Application) -> None:
    settings: Settings = app[SETTINGS]
    log_event("service_started", mode=settings.mode, model=settings.model,
              request_timeout_seconds=settings.request_timeout_seconds,
              max_state_tokens=settings.max_state_tokens, max_concurrency=settings.max_concurrency,
              simple_threshold=settings.simple_threshold if settings.mode == "rules" else None,
              competence_floor=settings.competence_floor if settings.mode == "rules" else None,
              criteria_source="policy" if settings.policy_configured else "default",
              criteria_sha256=criteria_hash(settings.branch_criteria))
    app[CLIENT] = ClientSession(
        timeout=ClientTimeout(total=settings.request_timeout_seconds),
        trust_env=True,
    )


async def _close_client(app: web.Application) -> None:
    await app[CLIENT].close()


SETTINGS = web.AppKey("settings", Settings)
CLIENT = web.AppKey("client", ClientSession)
CONCURRENCY = web.AppKey("concurrency", asyncio.Semaphore)


def create_app(settings: Settings | None = None) -> web.Application:
    app = web.Application(client_max_size=0)
    app[SETTINGS] = settings or Settings.from_env()
    app[CONCURRENCY] = asyncio.Semaphore(app[SETTINGS].max_concurrency)
    app.on_startup.append(_client)
    app.on_cleanup.append(_close_client)
    app.router.add_get("/health", health)
    app.router.add_post("/v1/decisions", decide)
    return app


def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    web.run_app(
        create_app(),
        host=os.getenv("HOST", "127.0.0.1"),
        port=int(os.getenv("PORT", "8080")),
        handler_cancellation=True,
    )


if __name__ == "__main__":
    main()
