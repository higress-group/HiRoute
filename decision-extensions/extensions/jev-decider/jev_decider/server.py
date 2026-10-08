"""HTTP lifecycle for the official Jev decision service.

Pure decisions live in decision.py; strict wire values in protocol.py; deployment
configuration in settings.py. The service has no persistent routing or upgrade state.
"""
from __future__ import annotations

import asyncio
import hmac
import json
import logging
import os
import time
import uuid
from typing import Any

from aiohttp import ClientError, ClientSession, ClientTimeout, web

from .decision import decision_response, prepare_decision, reported_usage
from .protocol import ProtocolError, strict_json, validate_hiroute_request
from .settings import Settings


MAX_UPSTREAM_RESPONSE_BYTES = 64 * 1024


LOGGER = logging.getLogger("jev_decider")


def log_event(event: str, **fields: Any) -> None:
    LOGGER.info(json.dumps({"timestamp_ms": time.time_ns() // 1_000_000,
                            "event": event, **fields}, ensure_ascii=False))


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


async def decide(request: web.Request) -> web.Response:
    decision_id = uuid.uuid4().hex
    started = time.monotonic()
    trace: dict[str, Any] = {"decision_id": decision_id, "phase": "authentication",
                             "upstream_usage": None}
    settings: Settings = request.app[SETTINGS]
    log_event("decision_started", decision_id=decision_id, model=settings.model)

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
            prepared = prepare_decision(settings, incoming, trace)
            session: ClientSession = request.app[CLIENT]
            upstream_started = True
            trace["phase"] = "queue"
            queued = time.monotonic()
            async with request.app[CONCURRENCY]:
                trace.update(queue_ms=round((time.monotonic() - queued) * 1000, 2), phase="upstream")
                async with session.post(
                    settings.upstream_url,
                    json=prepared.body,
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
            result = decision_response(prepared, upstream, trace)
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
    log_event("service_started", model=settings.model,
              request_timeout_seconds=settings.request_timeout_seconds,
              max_request_bytes=settings.max_request_bytes, max_concurrency=settings.max_concurrency)
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
