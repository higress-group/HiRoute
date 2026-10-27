"""HiRoute's native Hermes request adapter. No tools or outbound clients."""
from collections.abc import Mapping
from collections import OrderedDict
from pathlib import Path
import json
import threading
import time
import uuid


_MANAGERS = OrderedDict()
_MANAGERS_LOCK = threading.Lock()
_PROOF_TTL = 3600


def _state(row):
    if row.get("ended_at") is None:
        return "open"
    return "compression_closed" if row.get("end_reason") == "compression" else "other_closed"


def _native_row(row, session):
    return (isinstance(row, Mapping) and row.get("id") == session
            and all(k in row for k in ("parent_session_id", "ended_at", "end_reason"))
            and (row["parent_session_id"] is None or _identity(row["parent_session_id"])))


class _LineageReader:
    """One bounded/coalescing worker per selected profile, using public SDK reads.

    Contracts: Hermes 0.20 bounded ancestry and d923a5eb: SessionDB(read_only=True), get_session,
    get_compression_chain (native forward cap 1000), find_live_compression_child.
    Missing APIs disable this optional metadata, never the model adapter.
    """

    def __init__(self, path, identity, factory):
        self.path, self.identity, self.factory = path, identity, factory
        self.observer, self.sequence = uuid.uuid4().hex, 0
        self.lock = threading.Lock()
        self.wake = threading.Event()
        self.pending, self.cache = OrderedDict(), OrderedDict()
        self.active = False
        self.closed = False
        self.worker = threading.Thread(target=self._run, name="hiroute-hermes-lineage", daemon=True)
        self.worker.start()

    def schedule(self, session, ticket=None):
        with self.lock:
            if self.closed:
                return
            if session in self.pending or len(self.pending) < 64:
                previous = self.pending.get(session)
                self.pending[session] = ticket or previous
                self.pending.move_to_end(session)
                self.wake.set()

    def retire_idle(self):
        with self.lock:
            if self.active or self.pending:
                return False
            self.closed = True
            self.wake.set()
        self.worker.join(0.01)
        return not self.worker.is_alive()

    def stamp(self):
        self.sequence += 1
        return {"observer_id": self.observer, "sequence": self.sequence,
                "observed_at_ms": int(time.time() * 1000), "_monotonic": time.monotonic()}

    @staticmethod
    def wire_stamp(stamp):
        return {**{k: v for k, v in stamp.items() if k != "_monotonic"},
                "age_ms": max(0, int((time.monotonic() - stamp["_monotonic"]) * 1000))}

    def header(self, session):
        ticket = {"ready": threading.Event(), "sample": None}
        # Every call owns a fresh current-row sample, even with a warm ancestor
        # proof. A blocked SDK cannot extend the hook's 50 ms wait or queue.
        self.schedule(session, ticket)
        ticket["ready"].wait(0.05)
        with self.lock:
            cached = self.cache.get(session)
            if cached and time.monotonic() - cached["sample"]["_monotonic"] < cached["ttl"]:
                ids, complete, proof = cached["ids"][:], cached["complete"], cached["sample"]
                negative = cached.get("invalid")
                self.cache.move_to_end(session)
            else:
                ids, complete, proof, negative = [session], False, None, None
        state = ticket["sample"]
        if proof is None and state is None:
            self.schedule(session)
            return None
        proof = proof or state[1]
        payload = {"version": 1, "ids": ids, "complete": complete,
                   "proof_observation": self.wire_stamp(proof)}
        if state and time.monotonic() - state[1]["_monotonic"] <= 1:
            payload["current_state"] = {"value": state[0], "observation": self.wire_stamp(state[1])}
            if state[2] is not None:
                payload["current_state"]["started_at_ms"] = state[2]
        if negative:
            payload["invalid_edges"] = {"edges": negative, "observation": self.wire_stamp(proof)}
        # Fit only complete suffixes; never trim JSON bytes or invent a root.
        while len(json.dumps(payload, separators=(",", ":")).encode()) > 8192:
            if "invalid_edges" in payload:
                payload.pop("invalid_edges")
            elif len(payload["ids"]) > 1:
                payload["ids"].pop(0)
                payload["complete"] = False
            else:
                return None
        self.schedule(session)  # refresh during the ensuing HTTP/model work
        return json.dumps(payload, separators=(",", ":"))

    def _run(self):
        while True:
            self.wake.wait()
            with self.lock:
                if self.closed:
                    return
                if not self.pending:
                    self.wake.clear()
                    continue
                session, ticket = self.pending.popitem(last=False)
                self.active = True
            try:
                self._read(session, ticket)
            except Exception:
                # Missing/locked data is not proof that a formerly confirmed
                # native edge was withdrawn. No exception text or native rows
                # leave the adapter.
                pass
            finally:
                if ticket:
                    ticket["ready"].set()
                with self.lock:
                    self.active = False

    def _read(self, current, ticket):
        stat = self.path.stat()
        if (stat.st_dev, stat.st_ino) != self.identity:
            with self.lock:
                self.cache.clear()
            return
        reader = self.factory(db_path=self.path, read_only=True)
        try:
            if not getattr(reader, "read_only", False) or not all(callable(getattr(reader, k, None)) for k in (
                    "get_session", "find_live_compression_child")):
                return
            row = reader.get_session(current)
            if not _native_row(row, current):
                return
            observed = self.stamp()
            if ticket:
                started = row.get("started_at")
                # SessionDB creates this timestamp once; reopening does not
                # replace it. Older/malformed rows simply lack origin proof.
                started_ms = (int(started * 1000) if isinstance(started, (int, float))
                              and not isinstance(started, bool)
                              and 0 <= started <= observed["observed_at_ms"] / 1000 else None)
                ticket["sample"] = (_state(row), observed, started_ms)
            candidates, rows = [current], {current: row}
            complete = False
            for _ in range(31):  # the own-row probe is the first of <=32 reads
                parent = row.get("parent_session_id")
                if not parent:
                    complete = True
                    break
                if not _identity(parent) or parent in rows:
                    return
                row = reader.get_session(parent)
                if not _native_row(row, parent):
                    return
                rows[parent] = row
                candidates.append(parent)
            if not row.get("parent_session_id"):
                complete = True
            candidates.reverse()
            chain_reader = getattr(reader, "get_compression_chain", None)
            if callable(chain_reader):
                canonical = chain_reader(candidates[0])  # native cap: 1000
                confirmed = (current in canonical and canonical[:canonical.index(current) + 1] == candidates)
            elif callable(getattr(reader, "get_compression_lineage", None)):
                # Hermes 0.20's public lineage method performs unbounded child
                # enumeration. Validate only this <=32-row ancestry instead.
                canonical = candidates
                confirmed = all(_state(rows[parent]) == "compression_closed"
                                and _compression_child(rows[child])
                                for parent, child in zip(candidates, candidates[1:]))
            else:
                return
            ambiguous = False
            if confirmed and len(candidates) > 1 and rows[current].get("ended_at") is None:
                child = reader.find_live_compression_child(candidates[-2])
                confirmed = bool(child and child.get("id") == current)
                ambiguous = not confirmed
            sample = self.stamp()
            cached = {"ids": candidates if confirmed else [current], "complete": complete and confirmed,
                      "sample": sample, "ttl": _PROOF_TTL if confirmed else 5}
            if not confirmed:
                canonical_pairs = set(zip(canonical, canonical[1:]))
                cached["invalid"] = [list(pair) for pair in zip(candidates, candidates[1:])
                                     if _state(rows[pair[0]]) != "compression_closed"
                                     or pair[0] in canonical and pair not in canonical_pairs]
                if ambiguous:
                    cached["invalid"].append(candidates[-2:])
                # A verified withdrawal is dated evidence, not an unknown lookup.
                # Keep it across model generation just like positive proof; its
                # original stamp cannot override a newer successful verification.
                if cached["invalid"]:
                    cached["ttl"] = _PROOF_TTL
            with self.lock:
                self.cache[current] = cached
                self.cache.move_to_end(current)
                while len(self.cache) > 128:
                    self.cache.popitem(last=False)
        finally:
            reader.close()


def _compression_child(row):
    if row.get("source") == "tool":
        return False
    config = row.get("model_config")
    try:
        if isinstance(config, str) and len(config) > 16384:
            return False
        config = json.loads(config) if isinstance(config, str) else (config or {})
    except (ValueError, TypeError):
        return False
    return (isinstance(config, Mapping) and config.get("_branched_from") is None
            and config.get("_delegate_from") is None)


def _lineage_header(session):
    try:
        from hermes_constants import get_hermes_home
        from hermes_state import SessionDB
        path = (Path(get_hermes_home()) / "state.db").resolve(strict=True)
        stat = path.stat()
        identity = (stat.st_dev, stat.st_ino)
        key = (str(path), identity)
        with _MANAGERS_LOCK:
            manager = _MANAGERS.get(key)
            if manager is None:
                # Replaced databases/profiles may retire an idle worker. Blocked
                # or busy readers remain counted; never spawn around the bound.
                if len(_MANAGERS) >= 4:
                    retired = next((k for k, v in _MANAGERS.items() if v.retire_idle()), None)
                    if retired is None:
                        return None
                    _MANAGERS.pop(retired)
                manager = _LineageReader(path, identity, SessionDB)
                _MANAGERS[key] = manager
            _MANAGERS.move_to_end(key)
        return manager.header(session)
    except Exception:
        return None


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
            if not isinstance(k, str) or k.lower() not in ("x-hiroute-token", "x-hermes-session-id", "x-hermes-compression-lineage", "x-hiroute-agent", "x-hiroute-routing-context", "x-hermes-observed-input")
        }
        headers["X-HiRoute-Token"] = bearer
        headers["X-HiRoute-Agent"] = "hermes"
        session = context.get("session_id")
        if _identity(session):
            headers["X-Hermes-Session-Id"] = session
            turn = context.get("turn_id")
            request_id = context.get("api_request_id")
            # Whole-turn callbacks precede native memory/reminder composition
            # and also receive async notifications. Only the final observed
            # batch is available; equal clean text cannot identify its instance.
            headers["X-HiRoute-Routing-Context"] = json.dumps({
                "version": 2, "agent": "hermes", "session_id": session, "purpose": "main",
                **({"turn_id": turn} if _identity(turn) else {}),
                **({"request_id": request_id} if _identity(request_id) else {}),
                "input": {"kind": "observed_batch",
                          **({"event_id": turn} if _identity(turn) else {})},
            }, separators=(",", ":"))
            try:
                lineage = _lineage_header(session)
                if lineage:
                    headers["X-Hermes-Compression-Lineage"] = lineage
            except Exception:
                pass  # Optional lineage never removes minimal context or auth.
        return {"request": {**request, "extra_headers": headers}, "source": "hiroute"}

    return llm_request


def register(ctx):
    from hermes_cli.config import load_config_readonly

    adapter = _adapter(load_config_readonly() or {})
    if adapter is not None:
        ctx.register_middleware("llm_request", adapter)
