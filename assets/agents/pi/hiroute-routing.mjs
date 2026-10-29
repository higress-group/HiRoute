// Delegate native Pi providers; metadata never owns streaming or operation lifetimes.
import { createHash } from "node:crypto";

const managed = model => /^hiroute-(main-|worker(?:$|-))/.test(model?.provider ?? "");
const supported = model => ["openai-responses", "anthropic-messages"].includes(model?.api);
const identity = value => typeof value === "string" && /^[\x20-\x7e]{1,128}$/.test(value);
// Module-local ownership survives repeated setup without claiming a shared SDK registry.
const delegates = new WeakMap();

function cursor(ctx, entries = ctx.sessionManager.getBranch()) {
  const result = {};
  if (!Array.isArray(entries)) return result;
  const checkpointIndex = entries.findLastIndex(entry => entry.type === "compaction");
  const userIndex = entries.findLastIndex(entry => entry.type === "message" && entry.message?.role === "user");
  const user = entries[userIndex];
  const checkpoint = entries[checkpointIndex];
  if (identity(user?.id)) result.turn_id = user.id;
  if (identity(checkpoint?.id)) result.checkpoint = checkpoint.id;
  // A shared user is not a branch. Include every ordered native path component.
  const leaf = entries.at(-1)?.id;
  if (identity(leaf) && entries.every(entry => identity(entry.id))) {
    result.branch = {
      leaf_id: leaf,
      path_digest: createHash("sha256").update(JSON.stringify(entries.map(entry => entry.id))).digest("hex"),
    };
  }
  result.input = { kind: "observed_batch" };
  if (checkpointIndex >= userIndex && checkpointIndex >= 0) {
    result.continuation = { kind: "compression", ...(identity(checkpoint?.id) ? { from_checkpoint: checkpoint.id } : {}) };
  } else if (identity(user?.id) && !entries.some(entry => entry.type === "context_edit" && entry.targetId === user.id)) {
    try {
      const projection = ctx.sessionManager.buildSessionProjection?.();
      const latest = Array.isArray(projection?.entries) && projection.entries.findLast(({ messages }) =>
        !Array.isArray(messages) || messages.some(message => !["system", "assistant", "toolResult"].includes(message?.role)));
      if (latest?.sourceEntry?.type === "message" && latest.sourceEntry.id === user.id &&
          latest.sourceEntry.message?.role === "user" && !latest.sourceEntry.message.synthetic &&
          Array.isArray(latest.messages) && latest.messages.length === 1 &&
          latest.messages[0].role === "user" && !latest.messages[0].synthetic) {
        // Native projection is cursor evidence, never a final-wire position proof.
        result.input.event_id = user.id;
      }
    } catch { /* Keep observed input when optional projection fails. */ }
  }
  return result;
}

export default function hirouteRouting(pi) {
  const summarySignals = new WeakMap();
  let liveContext;
  let active = true;

  const enrich = (ctx, purpose, entries) => {
    const evidence = { version: 2, agent: "pi", purpose };
    try {
      const session = ctx.sessionManager.getSessionId();
      if (identity(session)) evidence.session_id = session;
    } catch { /* Known purpose survives unavailable optional session evidence. */ }
    try { Object.assign(evidence, cursor(ctx, entries)); } catch { /* Optional cursor miss. */ }
    return evidence;
  };

  const install = ctx => {
    liveContext = ctx;
    try {
      // Without this public identity seam, refresh can hide our wrapper inside
      // another facade. Leave the native provider usable rather than guessing.
      if (typeof ctx.modelRegistry.getRegisteredProviderConfig !== "function") return;
      for (const model of ctx.modelRegistry.getAll()) {
        if (!managed(model) || !supported(model)) continue;
        try {
          const provider = ctx.modelRegistry.getProvider(model.provider);
          if (typeof provider?.streamSimple !== "function") continue;
          const current = provider.streamSimple;
          // Pi recomposes its public Provider facade on refresh. The public
          // registration still holds the exact function that we registered.
          const registered = ctx.modelRegistry.getRegisteredProviderConfig(model.provider)?.streamSimple;
          const owned = delegates.get(registered ?? current);
          if (owned?.owner === install) continue;
          // Repeated setup unwraps only our own wrappers, preserving native `this`.
          const original = owned?.original ?? { provider, stream: current };
          const streamSimple = function(callModel, transcript, options) {
            let next = options;
            try {
              if (active && callModel?.provider === model.provider && supported(callModel)) {
                const summary = options?.signal && summarySignals.get(options.signal);
                let evidence = summary;
                if (!evidence) {
                  const session = liveContext.sessionManager.getSessionId();
                  if (identity(session) && options?.sessionId === session) {
                    evidence = enrich(liveContext, options.signal instanceof AbortSignal ? "main" : "unknown");
                    evidence.session_id = session;
                  }
                }
                if (evidence) {
                  const encoded = JSON.stringify(evidence);
                  if (Buffer.byteLength(encoded) <= 4096) {
                    const headers = { ...options?.headers, "x-hiroute-agent": "pi", "x-hiroute-routing-context": encoded };
                    if (evidence.session_id) {
                      headers["x-session-affinity"] = evidence.session_id;
                      headers["x-client-request-id"] = evidence.session_id;
                    }
                    // ModelRuntime already consumed transformHeaders at this seam.
                    next = { ...options, headers };
                  }
                }
              }
            } catch { /* Delegate exactly once even when optional enrichment fails. */ }
            return original.stream.call(original.provider, callModel, transcript, next);
          };
          delegates.set(streamSimple, { owner: install, original });
          pi.registerProvider(model.provider, { api: model.api, streamSimple });
        } catch { /* A failed registration cannot disable another managed provider. */ }
      }
    } catch { /* Retry discovery at the next native lifecycle event. */ }
  };

  const mark = (event, ctx, purpose, entries) => {
    install(ctx);
    try {
      if (!managed(ctx.model) || !(event.signal instanceof AbortSignal)) return;
      // Store serialized native facts, never references to mutable branch entries.
      summarySignals.set(event.signal, enrich(ctx, purpose, entries));
    } catch { /* No signal/unsupported operation remains observed. */ }
  };
  pi.on("session_start", (_event, ctx) => { active = true; install(ctx); });
  pi.on("model_select", (_event, ctx) => install(ctx));
  pi.on("context", (_event, ctx) => install(ctx));
  pi.on("session_before_compact", (event, ctx) => mark(event, ctx, "compaction", event.branchEntries));
  pi.on("session_before_tree", (event, ctx) => {
    install(ctx);
    try {
      if (event.preparation.userWantsSummary && event.preparation.entriesToSummarize.length) {
        mark(event, ctx, "summary");
      }
    } catch { /* Navigation without a native summary cannot mark another call. */ }
  });
  // Native extension disposal restores its public registrations. Retained old
  // wrappers delegate without session metadata while already-started calls finish.
  pi.on("session_shutdown", () => { active = false; liveContext = undefined; });
  pi.on("cache_warming_decision", (event, ctx) => {
    try { if (managed(event.model ?? ctx.model)) return { action: "stop" }; } catch { /* Optional model read. */ }
  });
}
