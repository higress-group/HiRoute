// Cordis llm/stream middleware: request-local native metadata, independent of pi-ai caching.
import { AsyncLocalStorage } from "node:async_hooks";
import { createHash } from "node:crypto";
export const inject = ["sessions"];
const symbol = Symbol.for("hiroute.dsh.routing-context.v1");
const id = (value) => typeof value === "string" && value.length > 0 &&
  Buffer.byteLength(value) <= 128 && !/[\x00-\x1f\x7f-\x9f]/.test(value);
const nativeTurnId = (value) => Number.isSafeInteger(value) && value > 0 ? String(value) : undefined;
const MAX_MESSAGES = 128;
const MAX_USER_MESSAGES = 32;
const MAX_TURNS = 16;
const MAX_BODY_BYTES = 2 * 1024 * 1024;
const USER_SOURCES = new Set(["user", "runtime-context", "compact-checkpoint", "agent-message"]);

function textDigest(content, nativeProjection = false) {
  const parts = typeof content === "string" ? [content] : Array.isArray(content) && content.length > 0
    && content.every(p => p && ["text", "input_text"].includes(p.type) && typeof p.text === "string")
    ? content.map(p => p.text) : undefined;
  if (!parts || !parts.some(p => p.length > 0)) return;
  let bytes = 0;
  for (const part of parts) {
    if (part.length > MAX_BODY_BYTES - bytes) return;
    bytes += Buffer.byteLength(part);
    if (bytes > MAX_BODY_BYTES) return;
  }
  // dsh-llm-pi-ai flattens supported pure-text blocks with join("") inside
  // each envelope. The final wire layout is still fingerprinted independently.
  const encoded = JSON.stringify(nativeProjection ? [parts.join("")] : parts);
  if (Buffer.byteLength(encoded) > MAX_BODY_BYTES) return;
  return { sha256: createHash("sha256").update(encoded).digest("hex"), bytes: Buffer.byteLength(encoded) };
}

function nativeUsers(options, session, currentTurn) {
  if (!session || !Array.isArray(options.messages) || options.messages.length > MAX_MESSAGES) return;
  const turns = new Map();
  let turn;
  for (let i = Math.max(0, session.seq - 512); i < session.seq; ++i) {
    const event = session.eventAt(i);
    if (event?.type === "turn/start") {
      turn = nativeTurnId(event.data?.turn);
      if (!turn) return;
    }
    if (event?.type === "user/message" && event.data?.source?.kind === "user" && id(turn) && id(event.data.id)) {
      if (turns.has(event.data.id)) return;
      turns.set(event.data.id, turn);
    }
  }
  const users = [];
  let bytes = 0;
  for (let i = 0; i < options.messages.length; ++i) {
    const message = options.messages[i];
    // The selected native pi-ai adapter moves only the leading system message
    // into its system slot. Later system messages are projected as user text.
    if (message.role !== "user" && !(message.role === "system" && (i > 0 || options.system !== undefined))) continue;
    if (message.role === "user" && !USER_SOURCES.has(message.source?.kind)) return;
    const fingerprint = textDigest(message.content, true);
    if (!fingerprint || (bytes += fingerprint.bytes) > MAX_BODY_BYTES) return;
    const real = message.role === "user" && message.source?.kind === "user";
    const nativeTurn = real ? turns.get(message.id) : undefined;
    if (real && !id(nativeTurn)) return;
    users.push({ sha256: fingerprint.sha256, ...(real ? { turn: nativeTurn } : {}) });
  }
  if (users.length > MAX_MESSAGES || !users.some(u => u.turn === currentTurn)) return;
  return users;
}

function userProof(context, input, init) {
  if (!context.users || typeof init?.body !== "string" || Buffer.byteLength(init.body) > MAX_BODY_BYTES) return;
  const body = JSON.parse(init.body);
  const messages = body.input ?? body.messages;
  if (!Array.isArray(messages) || messages.length > MAX_MESSAGES * 4) return;
  const wire = [];
  for (const message of messages) {
    if (message.role !== "user") continue;
    const content = message.content;
    // Native Messages tool-result-only envelopes are not user input.
    if (Array.isArray(content) && content.every(p => p?.type === "tool_result")) continue;
    const fingerprint = textDigest(content);
    if (!fingerprint) return;
    wire.push(fingerprint.sha256);
  }
  if (wire.length !== context.users.length || wire.some((value, i) => value !== context.users[i].sha256)) return;
  const groups = [];
  const seen = new Set();
  let count = 0;
  for (let index = 0; index < context.users.length; ++index) {
    const user = context.users[index];
    if (!user.turn) continue;
    let group = groups.at(-1);
    if (group?.turn_id !== user.turn) {
      if (seen.has(user.turn)) return;
      seen.add(user.turn);
      groups.push(group = { turn_id: user.turn, messages: [] });
    }
    group.messages.push({ index, sha256: wire[index] });
    if (++count > MAX_USER_MESSAGES || groups.length > MAX_TURNS) return;
  }
  if (groups.at(-1)?.turn_id !== context.turn_id) return;
  return groups;
}

function transport() {
  if (globalThis[symbol]) return globalThis[symbol];
  const storage = new AsyncLocalStorage();
  const original = globalThis.fetch;
  globalThis.fetch = function (input, init) {
    let next = init;
    try {
      const context = storage.getStore();
      const headers = new Headers(init?.headers ?? (input instanceof Request ? input.headers : undefined));
      // Touch only the exact admitted provider call, identified by its existing managed grant.
      if (context && headers.get("x-hiroute-agent") === "dsh" && (headers.has("x-hiroute-token") || headers.has("authorization"))) {
        headers.set("x-deepseek-harness-session-id", context.session_id);
        const { parent, users, ...wire } = context;
        const proof = context.purpose === "main" ? userProof(context, input, init) : undefined;
        if (proof) wire.user_turns = proof;
        const encoded = JSON.stringify(wire);
        if (Buffer.byteLength(encoded) > 4096) return original.call(this, input, init);
        headers.set("x-hiroute-routing-context", encoded);
        if (id(parent)) headers.set("x-deepseek-harness-parent-session-id", parent);
        next = { ...init, headers };
      }
    } catch {
      // Formatting/adapter failures must never replace a native model response.
    }
    return original.call(this, input, next);
  };
  globalThis[symbol] = storage;
  return storage;
}

export function apply(ctx) {
  const storage = transport();
  ctx.on("llm/stream", (options, next) => {
    let context;
    try {
      if (/^hiroute-(main-|worker(?:$|-))/.test(options.provider) && id(options.sessionId)
          && [undefined, "main", "compaction", "session-title"].includes(options.purpose)) {
        // Read the native event log by bounded index: compacted surface messages
        // may contain only a synthetic checkpoint and omit the real user's ID.
        const session = ctx.sessions?.get(options.sessionId);
        let turn;
        if (session && Number.isSafeInteger(session.seq)) {
          for (let i = session.seq - 1, floor = Math.max(0, session.seq - 512); i >= floor; --i) {
            const event = session.eventAt(i);
            if (event?.type === "turn/start") { turn = nativeTurnId(event.data?.turn); break; }
          }
        }
        const checkpoint = options.messages.findLast((message) => message.source?.kind === "compact-checkpoint")?.source.compactionId;
        context = {
          version: 1, agent: "dsh", session_id: String(options.sessionId),
          ...(id(session?.header?.parentSession) ? { parent: session.header.parentSession } : {}),
          purpose: options.purpose ?? "main", ...(id(turn) ? { turn_id: turn } : {}),
          ...(id(checkpoint) ? { checkpoint } : {}),
          ...(id(turn) && [undefined, "main"].includes(options.purpose) ? { users: nativeUsers(options, session, turn) } : {}),
        };
      }
    } catch { /* Missing metadata keeps the static route conservative. */ }
    const stream = context ? storage.run(context, next) : next();
    if (!context) return stream;
    const iterator = stream[Symbol.asyncIterator]();
    return {
      [Symbol.asyncIterator]() { return this; },
      next(value) { return storage.run(context, () => iterator.next(value)); },
      return(value) { return storage.run(context, () => iterator.return?.(value) ?? Promise.resolve({ done: true, value })); },
      throw(error) { return storage.run(context, () => iterator.throw ? iterator.throw(error) : Promise.reject(error)); },
    };
  });
}
