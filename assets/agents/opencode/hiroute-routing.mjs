// Public per-call hooks; optional SDK evidence never owns native execution.
const identity = value => typeof value === "string" && /^[\x20-\x7e]{1,128}$/.test(value);
const managed = provider => /^hiroute-(main-|worker(?:$|-))/.test(provider ?? "");
const key = (session, message) => `${session}\0${message}`;
const ordinaryParts = parts => Array.isArray(parts) && parts.some(part =>
  ["text", "file", "agent", "subtask"].includes(part?.type) && !part.synthetic && !part.ignored);
const completed = info => info?.role === "assistant" && info.summary === true &&
  Number.isFinite(info.time?.completed) && info.time.completed > 0 &&
  typeof info.finish === "string" && !["", "error", "unknown"].includes(info.finish) && !info.error;

// OpenCode 1.19.3's private compaction_continue capability. This exact typed
// shape is optional producer evidence, not a stable public source/origin API.
const compactionContinue = parts => Array.isArray(parts) && parts.length > 0 &&
  parts.some(part => part?.type === "text" && part.synthetic === true &&
    part.metadata?.compaction_continue === true) &&
  parts.every(part => part?.synthetic === true || part?.ignored === true);

function emit(output, context) {
  const encoded = JSON.stringify(context);
  if (new TextEncoder().encode(encoded).length > 4096) return;
  output.headers["x-hiroute-agent"] = "opencode";
  output.headers["x-hiroute-routing-context"] = encoded;
}

function pageEvidence(entries, session, currentID, created) {
  if (!Array.isArray(entries) || entries.length > 128) return;
  const ids = new Set();
  let previous = -Infinity;
  // The SDK page is chronological. Model history is deliberately reordered by
  // filterCompacted and cannot substitute for it. IDs never establish time.
  for (const { info } of entries) {
    if (!info || !identity(info.id) || ids.has(info.id) || info.sessionID !== session ||
        !Number.isFinite(info.time?.created) || info.time.created < previous) return;
    ids.add(info.id);
    previous = info.time.created;
  }
  const currentIndex = entries.findIndex(({ info }) => info.id === currentID && info.role === "user");
  if (currentIndex < 0 || !Array.isArray(entries[currentIndex].parts)) return;
  const current = entries[currentIndex];
  // A real prompt callback can veto incompatible later synthetic metadata. Its
  // absence (including cold cache/overflow) never proves human or copied input.
  const continuing = !created && compactionContinue(current.parts);
  const result = continuing ? { input: { kind: "existing_turn", event_id: currentID } } : {};
  const summaryIndex = entries.findLastIndex(({ info }, index) => index < currentIndex && completed(info));
  if (summaryIndex < 0) return result;
  const summary = entries[summaryIndex];
  const parentIndex = entries.findIndex(({ info, parts }, index) =>
    index < summaryIndex && info.role === "user" && info.id === summary.info.parentID &&
    Array.isArray(parts) && parts.some(part => part?.type === "compaction"));
  if (parentIndex < 0) return result;
  result.checkpoint = summary.info.id;
  if (!continuing) return result;
  const sourceIndex = entries.findLastIndex(({ info, parts }, index) =>
    index < parentIndex && info.role === "user" && ordinaryParts(parts));
  // Missing/truncated source loses only its reference; the known continuation
  // still cannot turn summary text into a fresh task. Gateway validates edges.
  if (sourceIndex >= 0) result.continuation = {
    kind: "compression", from_turn_id: entries[sourceIndex].info.id,
    to_checkpoint: summary.info.id,
  };
  return result;
}

export const HiRouteRouting = async ({ client }) => {
  const prompts = new Map();
  let inFlight = 0;
  const page = async session => {
    // An SDK may ignore abort. Keep its slot until it really settles, so repeated
    // deadlines cannot accumulate unbounded work or block native model requests.
    if (inFlight >= 4) return;
    const controller = new AbortController();
    let timer;
    ++inFlight;
    try {
      const pending = Promise.resolve(client.session.messages({
        path: { id: session }, query: { limit: 128 }, signal: controller.signal,
      })).then(result => result, () => undefined).finally(() => { --inFlight; });
      return await Promise.race([pending, new Promise(resolve => {
        timer = setTimeout(() => { controller.abort(); resolve(undefined); }, 50);
      })]);
    } catch {
      --inFlight; // A synchronous SDK throw never owns an unresolved slot.
    } finally { clearTimeout(timer); }
  };
  return {
    "chat.message": async (input, output) => {
      try {
        if (!managed(input.model?.providerID ?? output.message?.model?.providerID) ||
            !identity(input.sessionID) || !identity(output.message?.id) || !ordinaryParts(output.parts)) return;
        // Snapshot only callback facts; never mutate transcript parts or rely on
        // the same object surviving between independent public hooks.
        prompts.set(key(input.sessionID, output.message.id), Date.now());
        while (prompts.size > 256) prompts.delete(prompts.keys().next().value);
      } catch { /* Optional callback evidence cannot alter the native prompt. */ }
    },
    "chat.headers": async (input, output) => {
      try {
        if (!managed(input.model?.providerID)) return;
        const session = identity(input.sessionID) ? input.sessionID : undefined;
        const current = identity(input.message?.id) ? input.message.id : undefined;
        const purpose = ["title", "summary", "compaction"].includes(input.agent) ? input.agent : "main";
        // Freeze the per-call facts and publish them before any asynchronous SDK
        // work. Known auxiliaries need neither history nor a usable session.
        const minimal = Object.freeze({ version: 2, agent: "opencode", purpose,
          ...(session ? { session_id: session } : {}),
          ...(current ? { turn_id: current } : {}),
          ...(purpose === "main" ? { input: { kind: "observed_batch", ...(current ? { event_id: current } : {}) } } : {}),
        });
        emit(output, minimal);
        if (purpose !== "main" || !session || !current) return;
        const promptKey = key(session, current);
        const createdAt = prompts.get(promptKey);
        const created = createdAt !== undefined && Date.now() - createdAt <= 3600_000;
        if (createdAt !== undefined && !created) prompts.delete(promptKey);
        const result = await page(session);
        const extra = pageEvidence(result?.data, session, current, created);
        if (extra) emit(output, { ...minimal, ...extra });
      } catch { /* Minimal context survives; native model execution stays unchanged. */ }
    },
  };
};
