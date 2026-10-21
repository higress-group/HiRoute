// Native Pi core routing metadata; no tools, credentials or network clients.
const managed = (model) => /^hiroute-(main-|worker(?:$|-))/.test(model?.provider ?? "");
const identity = (value) => typeof value === "string" && /^[\x20-\x7e]{1,128}$/.test(value);
export default function hirouteRouting(pi) {
  const compacting = new Map();
  const boundary = entries => ({
    user: entries.findLast(entry => entry.type === "message" && entry.message?.role === "user")?.id,
    checkpoint: entries.findLast(entry => entry.type === "compaction")?.id,
  });
  pi.on("cache_warming_decision", (event, ctx) => {
    try { if (managed(event.model ?? ctx.model)) return { action: "stop" }; } catch { /* Retry on the next event. */ }
  });
  pi.on("session_before_compact", (event, ctx) => {
    try {
      if (!managed(ctx.model)) return;
      const session = ctx.sessionManager.getSessionId();
      if (!identity(session)) return;
      const entries = Array.isArray(event.branchEntries) ? event.branchEntries : ctx.sessionManager.getBranch();
      compacting.set(session, { ...boundary(entries), expires: Date.now() + 300_000 });
      while (compacting.size > 64) compacting.delete(compacting.keys().next().value);
    } catch { /* Optional metadata miss. */ }
  });
  for (const name of ["session_compact", "session_compact_failed"]) {
    pi.on(name, (_event, ctx) => {
      try { compacting.delete(ctx.sessionManager.getSessionId()); } catch { /* Optional metadata miss. */ }
    });
  }
  pi.on("before_provider_headers", (event, ctx) => {
    try {
      if (!managed(event.model ?? ctx.model)) return;
      const session = ctx.sessionManager.getSessionId();
      if (!identity(session)) return;
      const entries = ctx.sessionManager.getBranch();
      const checkpointIndex = entries.findLastIndex((entry) => entry.type === "compaction");
      const userIndex = entries.findLastIndex((entry) => entry.type === "message" && entry.message?.role === "user");
      const checkpoint = entries[checkpointIndex];
      const user = entries[userIndex];
      let pending = compacting.get(session);
      if (pending && (pending.user !== user?.id || pending.checkpoint !== checkpoint?.id)) {
        compacting.delete(session);
        pending = undefined;
      }
      // Only a known compacted continuation requires saved input. Unknown
      // extension/context contributions remain usable as observed input.
      let continuationRequiresExistingTurn = checkpointIndex >= userIndex && checkpointIndex >= 0
        ? true : undefined;
      if (identity(user?.id) && userIndex > checkpointIndex &&
          !entries.some((entry) => entry.type === "context_edit" && entry.targetId === user.id)) {
        const projection = ctx.sessionManager.buildSessionProjection?.();
        // This is native entry provenance, not proof of a provider payload after
        // other extensions transform it. Unknown user-like contributions stay conservative.
        const latestInput = Array.isArray(projection?.entries) && projection.entries.findLast(({ messages }) =>
          !Array.isArray(messages) || messages.some((message) =>
            !["system", "assistant", "toolResult"].includes(message?.role))
        );
        const currentInput = (
          latestInput?.sourceEntry?.type === "message" &&
          latestInput.sourceEntry.id === user.id &&
          latestInput.sourceEntry.message?.role === "user" &&
          !latestInput.sourceEntry.message.synthetic &&
          Array.isArray(latestInput.messages) && latestInput.messages.length === 1 &&
          latestInput.messages[0].role === "user" &&
          !latestInput.messages[0].synthetic
        );
        if (currentInput && !(pending && pending.expires < Date.now())) continuationRequiresExistingTurn = false;
      }
      const context = {
        version: 1, agent: "pi", session_id: session,
        purpose: pending && pending.expires >= Date.now() ? "compaction" : "main",
        ...(continuationRequiresExistingTurn !== undefined
          ? { continuation_requires_existing_turn: continuationRequiresExistingTurn } : {}),
        ...(identity(user?.id) ? { turn_id: user.id } : {}),
        ...(identity(checkpoint?.id) ? { checkpoint: checkpoint.id } : {}),
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
