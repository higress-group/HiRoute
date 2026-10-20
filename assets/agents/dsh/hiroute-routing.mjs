// Cordis llm/stream middleware: request-local native metadata, independent of pi-ai caching.
import { AsyncLocalStorage } from "node:async_hooks";
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
        headers.set("x-hiroute-routing-context", JSON.stringify(context));
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
        const user = options.messages.findLast((message) => message.role === "user");
        const turn = user?.id ?? user?.messageId;
        context = {
          version: 1, agent: "dsh", session_id: String(options.sessionId),
          purpose: options.purpose ?? "main", ...(id(turn) ? { turn_id: String(turn) } : {}),
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
