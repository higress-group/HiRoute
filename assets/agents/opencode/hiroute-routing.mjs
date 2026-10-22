// Uses the native per-call hook and bounded optional SDK reads, never a global phase flag.
export const HiRouteRouting = async ({ client }) => {
  const prompts = new Map();
  const id = value => typeof value === "string" && /^[\x20-\x7e]{1,128}$/.test(value);
  const managed = provider => /^hiroute-(main-|worker(?:$|-))/.test(provider ?? "");
  const promptKey = (session, message) => `${session}\0${message}`;
  return {
  "chat.message": async (input, output) => {
    try {
      if (!managed(input.model?.providerID ?? output.message?.model?.providerID)
          || !id(input.sessionID) || !id(output.message?.id)) return;
      // Native prompt creation emits this hook; overflow replay copies bypass
      // it. The ID is request provenance, not a claim that a human typed it.
      prompts.set(promptKey(input.sessionID, output.message.id), Date.now());
      while (prompts.size > 256) prompts.delete(prompts.keys().next().value);
    } catch { /* Optional source metadata must not alter the native prompt. */ }
  },
  "chat.headers": async (input, output) => {
    try {
      if (!managed(input.model.providerID)) return;
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
        info.role === "assistant" && info.summary && info.time?.completed && !info.error &&
        info.id <= current.info.id
      );
      const key = promptKey(input.sessionID, current.info.id);
      const created = prompts.get(key);
      const prompt = realUser(current) && created !== undefined && Date.now() - created <= 3600_000;
      if (created !== undefined && !prompt) prompts.delete(key);
      // Pin the actual typed producer marker. Other synthetic inputs include
      // late child results and must not inherit the most recent human's task.
      const compacted = current.parts.some(part =>
        part.type === "text" && part.synthetic === true && part.metadata?.compaction_continue === true
      ) && current.parts.every(part => part.synthetic === true || part.ignored === true);
      const compactionUser = summary && entries.find(entry =>
        entry.info.role === "user" && entry.info.id === summary.info.parentID &&
        entry.parts?.some(part => part.type === "compaction"));
      const user = compacted ? compactionUser && entries.findLast(entry =>
        realUser(entry) && entry.info.id <= compactionUser.info.id
      ) : current;
      const context = {
        version: 1, agent: "opencode", session_id: input.sessionID,
        purpose: ["title", "summary", "compaction"].includes(input.agent) ? input.agent : "main",
        ...(compacted ? { continuation_requires_existing_turn: true }
          : prompt ? { continuation_requires_existing_turn: false } : {}),
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
  };
};
