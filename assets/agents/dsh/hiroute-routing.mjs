// Cordis llm/stream middleware: request-local native metadata, independent of pi-ai caching.
import { AsyncLocalStorage } from "node:async_hooks";
export const inject = ["sessions"];
const symbol = Symbol.for("hiroute.dsh.routing-context.v1");
const id = (value) => typeof value === "string" && value.length > 0 &&
  Buffer.byteLength(value) <= 128 && !/[\x00-\x1f\x7f-\x9f]/.test(value);

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
        const { parent, ...wire } = context;
        headers.set("x-hiroute-routing-context", JSON.stringify(wire));
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
      if (/^hiroute-(main-|worker(?:$|-))/.test(options.provider) && id(options.sessionId)) {
        // Read the native event log by bounded index: compacted surface messages
        // may contain only a synthetic checkpoint and omit the real user's ID.
        const session = ctx.sessions?.get(options.sessionId);
        let turn;
        if (session && Number.isSafeInteger(session.seq)) {
          for (let i = session.seq - 1, floor = Math.max(0, session.seq - 512); i >= floor; --i) {
            const event = session.eventAt(i);
            if (event?.type === "turn/start") { turn = String(event.data.turn); break; }
          }
        }
        const checkpoint = options.messages.findLast((message) => message.source?.kind === "compact-checkpoint")?.source.compactionId;
        context = {
          version: 1, agent: "dsh", session_id: String(options.sessionId),
          ...(id(session?.header?.parentSession) ? { parent: session.header.parentSession } : {}),
          purpose: options.purpose ?? "main", ...(id(turn) ? { turn_id: turn } : {}),
          ...(id(checkpoint) ? { checkpoint } : {}),
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
