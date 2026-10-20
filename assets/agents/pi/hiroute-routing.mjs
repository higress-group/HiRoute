// Native Pi core routing metadata; no tools, credentials or network clients.
const managed = (model) => /^hiroute-(main-|worker(?:$|-))/.test(model?.provider ?? "");
export default function hirouteRouting(pi) {
  const compacting = new Set();
  pi.on("cache_warming_decision", (event, ctx) => {
    try { if (managed(event.model ?? ctx.model)) return { action: "stop" }; } catch {}
  });
  pi.on("session_before_compact", (_event, ctx) => {
    try { if (managed(ctx.model)) compacting.add(ctx.sessionManager.getSessionId()); } catch {}
  });
  for (const name of ["session_compact", "session_compact_failed"]) {
    pi.on(name, (_event, ctx) => {
      try { compacting.delete(ctx.sessionManager.getSessionId()); } catch {}
    });
  }
  pi.on("before_provider_headers", (event, ctx) => {
    try {
      if (!managed(event.model ?? ctx.model)) return;
      const session = ctx.sessionManager.getSessionId();
      const entries = ctx.sessionManager.getBranch();
      const checkpoint = entries.findLast((entry) => entry.type === "compaction");
      const user = entries.findLast((entry) => entry.type === "message" && entry.message?.role === "user");
      const context = {
        version: 1, agent: "pi", session_id: session,
        purpose: compacting.has(session) ? "compaction" : "main",
        ...(user?.id ? { turn_id: user.id } : {}),
        ...(checkpoint?.id ? { checkpoint: checkpoint.id } : {}),
      };
      const encoded = JSON.stringify(context);
      if (Buffer.byteLength(encoded) > 4096) return;
      event.headers["x-session-affinity"] = session;
      // Responses retains the same native identity even when caching is off.
      event.headers["x-client-request-id"] = session;
      event.headers["x-hiroute-agent"] = "pi";
      event.headers["x-hiroute-routing-context"] = encoded;
    } catch {
      // The provider's static Agent marker makes a missing context request-scoped.
    }
  });
}
