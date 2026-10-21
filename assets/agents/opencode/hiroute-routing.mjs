// Uses the native per-call hook and bounded optional SDK reads, never a global phase flag.
export const HiRouteRouting = async ({ client }) => ({
  "chat.headers": async (input, output) => {
    try {
      if (!/^hiroute-(main-|worker(?:$|-))/.test(input.model.providerID)) return;
      const controller = new AbortController();
      let timer;
      let result;
      try {
        result = await Promise.race([
          client.session.messages({ path: { id: input.sessionID }, query: { limit: 128 }, signal: controller.signal }),
          new Promise((resolve) => { timer = setTimeout(() => { controller.abort(); resolve({}); }, 50); }),
        ]);
      } finally { clearTimeout(timer); }
      if (!Array.isArray(result.data)) return;
      const entries = result.data;
      if (typeof input.message?.id !== "string" || !input.message.id) return;
      const current = entries.find(({ info }) =>
        info.role === "user" && info.id === input.message.id
      );
      if (!current || !Array.isArray(current.parts)) return;
      const realUser = ({ info, parts }) => info.role === "user" && Array.isArray(parts) &&
        parts.some((part) => ["text", "file", "agent", "subtask"].includes(part.type) &&
          !part.synthetic && !part.ignored);
      const summary = entries.findLast(({ info }) =>
        info.role === "assistant" && info.summary && info.time?.completed && !info.error
      );
      // Native automatic-compaction followups are synthetic user messages.
      // The preceding real user retains the logical turn; actual new input has its own ID.
      const user = entries.findLast((entry) =>
        realUser(entry) && entry.info.id <= current.info.id
      );
      if (!user) return;
      const context = {
        version: 1, agent: "opencode", session_id: input.sessionID,
        purpose: ["title", "summary", "compaction"].includes(input.agent) ? input.agent : "main",
        continuation_requires_existing_turn: !realUser(current) || current.info.id !== user.info.id,
        ...(user?.info.id ? { turn_id: user.info.id } : {}),
        ...(summary?.info.id ? { checkpoint: summary.info.id } : {}),
      };
      const encoded = JSON.stringify(context);
      if (new TextEncoder().encode(encoded).length > 4096) return;
      output.headers["x-hiroute-agent"] = "opencode";
      output.headers["x-hiroute-routing-context"] = encoded;
    } catch {
      // Missing metadata degrades history; native model execution remains unchanged.
    }
  },
});
