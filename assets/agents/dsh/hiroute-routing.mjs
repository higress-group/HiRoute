// Cordis llm/stream middleware: immutable per-call metadata, independent of pi-ai caching.
import { AsyncLocalStorage } from "node:async_hooks";
import { createHash } from "node:crypto";
export const inject = ["sessions"];
const symbol = Symbol.for("hiroute.dsh.routing-context.v2");
const id = (value) => typeof value === "string" && value.length > 0 && value.length <= 128 &&
  Buffer.byteLength(value) <= 128 && !/[\x00-\x1f\x7f-\x9f]/.test(value);
const nativeTurnId = (value) => Number.isSafeInteger(value) && value > 0 ? String(value) : undefined;
const MAX_MESSAGES = 512;
const MAX_USER_MESSAGES = 128;
const MAX_CURRENT_MESSAGES = 32;
const MAX_EVENTS = 512;
const MAX_BODY_BYTES = 2 * 1024 * 1024;
// Native context producers supplement an activation; peers and unknown producers
// still invalidate its current-input proof. Desktop also installs these two.
const SUPPLEMENTAL_SOURCES = new Set(["runtime-context", "compact-checkpoint", "skill-catalog", "time-context"]);
const digest = (value) => createHash("sha256").update(JSON.stringify(value)).digest("hex");

function textPart(part, native) {
  if (!part || !(native ? part.type === "text" : ["text", "input_text"].includes(part.type)) || typeof part.text !== "string") return false;
  return Object.keys(part).every(key => ["type", "text"].includes(key) || (!native && key === "cache_control" &&
    part.cache_control?.type === "ephemeral" && [undefined, "5m", "1h"].includes(part.cache_control.ttl) &&
    Object.keys(part.cache_control).every(field => ["type", "ttl"].includes(field))));
}

function textDigest(content, nativeProjection = false) {
  const parts = typeof content === "string" ? [content] : Array.isArray(content) && content.length > 0
    && content.length <= MAX_MESSAGES && content.every(p => textPart(p, nativeProjection))
    ? content.map(p => p.text) : undefined;
  if (!parts || !parts.some(p => p.length > 0)) return;
  let bytes = 0;
  for (const part of parts) {
    if (part.length > MAX_BODY_BYTES - bytes) return;
    bytes += Buffer.byteLength(part);
    if (bytes > MAX_BODY_BYTES) return;
  }
  // The supported dsh-llm-pi-ai adapter flattens text inside each envelope.
  // Final wire envelopes are hashed separately; never join across envelopes.
  const projected = nativeProjection ? [parts.join("")] : parts;
  const encoded = JSON.stringify(projected);
  if (Buffer.byteLength(encoded) > MAX_BODY_BYTES) return;
  return { sha256: digest(projected), bytes: Buffer.byteLength(encoded) };
}

function currentTurn(session) {
  const seq = session?.seq;
  if (!Number.isSafeInteger(seq) || seq < 0) return;
  const messages = new Map();
  let bytes = 0;
  for (let i = seq - 1, floor = Math.max(0, seq - MAX_EVENTS); i >= floor; --i) {
    const event = session.eventAt(i);
    if (event?.type === "turn/start") {
      const turn = nativeTurnId(event.data?.turn);
      return turn ? { turn, messages } : undefined;
    }
    if (event?.type !== "user/message" || event.data?.source?.kind !== "user") continue;
    const message = event.data;
    if (!id(message.id) || messages.has(message.id) || messages.size >= MAX_CURRENT_MESSAGES) return;
    const fingerprint = textDigest(message.content, true);
    if (!fingerprint || (bytes += fingerprint.bytes) > MAX_BODY_BYTES) return;
    messages.set(message.id, fingerprint.sha256);
  }
}

function nativeUsers(options, current, instructionUsers) {
  const messages = options.messages;
  if (!current?.messages.size || !Array.isArray(messages) || messages.length > MAX_MESSAGES) return;
  const users = [];
  const found = new Set();
  let bytes = 0;
  let lastCurrent = -1;
  for (let i = 0; i < messages.length; ++i) {
    const message = messages[i];
    // Older pi-ai bridges projected in-history instructions as User envelopes;
    // 0.2.1-rc.3 preserves their role. Match either complete final User scope,
    // never choose a client version or discard a mismatching wire envelope.
    if (message.role !== "user" && (!instructionUsers || !["developer", "system"].includes(message.role)) ||
        (instructionUsers && message.role === "system" && i === 0 && options.system === undefined)) continue;
    const fingerprint = textDigest(message.content, true);
    if (!fingerprint || (bytes += fingerprint.bytes) > MAX_BODY_BYTES || users.length >= MAX_USER_MESSAGES) return;
    const selected = message.role === "user" && message.source?.kind === "user" && current.messages.has(message.id);
    if (selected) {
      if (found.has(message.id) || current.messages.get(message.id) !== fingerprint.sha256) return;
      found.add(message.id);
      lastCurrent = i;
    }
    users.push(Object.freeze({ sha256: fingerprint.sha256, selected }));
  }
  if (found.size !== current.messages.size) return;
  // A late peer, settlement or unknown producer is its own observed input.
  // Its physical turn/sender cannot make it the latest human's task. Keep all
  // native bytes; the Gateway observes the complete latest envelope instead.
  for (let i = lastCurrent + 1; i < messages.length; ++i) {
    if (messages[i].role === "user" && !SUPPLEMENTAL_SOURCES.has(messages[i].source?.kind)) return;
  }
  return Object.freeze(users);
}

function userProof(context, init) {
  if (!context.users || typeof init?.body !== "string" || init.body.length > MAX_BODY_BYTES || Buffer.byteLength(init.body) > MAX_BODY_BYTES) return;
  const body = JSON.parse(init.body);
  const messages = body.input ?? body.messages;
  if (!Array.isArray(messages) || messages.length > MAX_MESSAGES) return;
  const wire = [];
  for (const message of messages) {
    if (message.role !== "user") continue;
    if (!Object.keys(message).every(key => ["role", "content"].includes(key) || (key === "type" && message.type === "message"))) return;
    const content = message.content;
    // Native Messages tool-result-only envelopes have no User input ordinal.
    if (Array.isArray(content) && content.length > 0 && content.every(p => p?.type === "tool_result")) continue;
    const fingerprint = textDigest(content);
    if (!fingerprint || wire.length >= MAX_USER_MESSAGES) return;
    wire.push(fingerprint.sha256);
  }
  const users = context.users.find(candidate => wire.length === candidate.length &&
    wire.every((value, i) => value === candidate[i].sha256));
  if (!users) return;
  return { kind: "native_messages", event_id: context.wire.turn_id, coverage: "window",
    messages: users.flatMap((user, index) => user.selected ? [{ index, sha256: wire[index] }] : []),
    scope_digest: digest(wire) };
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
      // The middleware admits only a managed provider. Its existing grant and
      // Agent marker gate this final transport seam; no shared headers mutate.
      if (context && headers.get("x-hiroute-agent") === "dsh" && (headers.has("x-hiroute-token") || headers.has("authorization"))) {
        let proof;
        try { if (context.wire.purpose === "main") proof = userProof(context, init); } catch { /* Optional proof miss. */ }
        let encoded = JSON.stringify({ ...context.wire, ...(proof ? { input: proof } : {}) });
        if (Buffer.byteLength(encoded) > 4096) encoded = JSON.stringify(context.minimum);
        if (context.minimum.session_id) headers.set("x-deepseek-harness-session-id", context.minimum.session_id);
        headers.set("x-hiroute-routing-context", encoded);
        if (context.parent) headers.set("x-deepseek-harness-parent-session-id", context.parent);
        next = { ...init, headers };
      }
    } catch { /* Optional formatting must not replace the provider response. */ }
    return original.call(this, input, next);
  };
  globalThis[symbol] = storage;
  return storage;
}

function requestContext(ctx, options) {
  if (typeof options.provider !== "string" || !/^hiroute-(main-|worker(?:$|-))/.test(options.provider)) return;
  const nativePurpose = options.purpose;
  const purpose = nativePurpose === undefined || nativePurpose === "main" ? "main" :
    ["compaction", "session-title"].includes(nativePurpose) ? nativePurpose : "unknown";
  const nativeSession = options.sessionId;
  const sessionId = id(nativeSession) ? nativeSession : undefined;
  if (!sessionId && !["compaction", "session-title"].includes(purpose)) return;
  // Purpose/session exist before any optional session, event or checkpoint read.
  const minimum = Object.freeze({ version: 2, agent: "dsh", purpose, ...(sessionId ? { session_id: sessionId } : {}) });
  const wire = { ...minimum };
  let session, current, parent, users;
  try { if (sessionId) session = ctx.sessions?.get(sessionId); } catch { /* Optional native lookup. */ }
  try {
    current = currentTurn(session);
    if (current) wire.turn_id = current.turn;
  } catch { /* Bounded native events are optional. */ }
  try {
    const candidate = session?.header?.parentSession;
    if (id(candidate)) parent = candidate;
  } catch { /* Optional parent. */ }
  try {
    const messages = options.messages;
    if (Array.isArray(messages) && messages.length <= MAX_MESSAGES) {
      const checkpoint = messages.findLast(message => message.source?.kind === "compact-checkpoint")?.source.compactionId;
      if (id(checkpoint)) wire.checkpoint = checkpoint;
    }
  } catch { /* Optional checkpoint cannot erase explicit purpose. */ }
  try {
    if (purpose === "main") users = Object.freeze([false, true]
      .map(instructionUsers => nativeUsers(options, current, instructionUsers)).filter(Boolean));
  } catch { /* Optional current-event proof. */ }
  return Object.freeze({ minimum, wire: Object.freeze(wire), parent, users });
}

export function apply(ctx) {
  const storage = transport();
  // Web sessions can own an isolated LLM service. Cordis otherwise filters this
  // root-loaded listener before it can bind main or auxiliary call metadata.
  // Request/provider and authenticated transport gates still limit the wrapper.
  ctx.on("llm/stream", (options, next) => {
    let context;
    try { context = requestContext(ctx, options); } catch { /* Unusable call identity stays observed. */ }
    // Clear an enclosing managed call even for an unmanaged nested invocation.
    const stream = storage.run(context, next);
    const iterator = storage.run(context, () => stream[Symbol.asyncIterator]());
    return {
      [Symbol.asyncIterator]() { return this; },
      next(value) { return storage.run(context, () => iterator.next(value)); },
      return(value) { return storage.run(context, () => iterator.return?.(value) ?? Promise.resolve({ done: true, value })); },
      throw(error) { return storage.run(context, () => iterator.throw ? iterator.throw(error) : Promise.reject(error)); },
    };
  }, { global: true });
}
