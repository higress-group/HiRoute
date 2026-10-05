// HiRoute's ACP transport for the explicitly selected official Pi npm SDK.
// The daemon owns admission, credentials, process groups, retention and task state.
import { readFileSync, realpathSync, lstatSync, openSync, readSync, closeSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { loadPiSdk, assertPiCapabilities } from "./pi-sdk-contract.mjs";

process.umask(0o077);
const output = (value) => process.stdout.write(JSON.stringify(value) + "\n");
let closing = false;
const reply = (request, result) => {
  if (!closing) output({ jsonrpc: "2.0", id: request.id, result });
};
const reject = (request, stage) => {
  if (!closing && request.id !== undefined)
    output({
      jsonrpc: "2.0",
      id: request.id,
      error: {
        code: -32000,
        message: "Pi managed operation unavailable",
        data: { schema: "hiroute.pi-worker-failure/v1", stage },
      },
    });
};
let session;
let active = false;
let cancelled = false;
let inFlight;
let stopping;
async function shutdown() {
  if (stopping) return stopping;
  closing = true;
  process.stdin.destroy();
  // The daemon retains process-group authority; this also bounds a broken SDK abort.
  const deadline = setTimeout(() => process.exit(1), 2000);
  deadline.unref();
  stopping = (async () => {
    try {
      if (session) {
        session.clearQueue();
        await session.abort();
      }
      await inFlight;
    } finally {
      // Session creation may have completed after the first abort attempt.
      try {
        if (session) {
          session.clearQueue();
          await session.abort();
        }
      } finally {
        session?.dispose();
      }
    }
  })()
    .catch(() => {
      process.exitCode = 1;
    })
    .finally(() => clearTimeout(deadline));
  return stopping;
}
process.once("SIGTERM", () => {
  void shutdown().then(() => process.exit(process.exitCode ?? 0));
});
process.once("SIGINT", () => {
  void shutdown().then(() => process.exit(process.exitCode ?? 0));
});
process.stdout.on("error", () => {
  void shutdown().then(() => process.exit(1));
});
let startupStage = "configuration";
let startupFailure;
let dispatch;

function messageValid(message) {
  if (!message || !Number.isFinite(message.timestamp)) return false;
  const content = message.content;
  if (message.role === "system")
    return (
      typeof content === "string" &&
      (content.length > 0 || (message.sections && typeof message.sections === "object"))
    );
  if (message.role === "user" && typeof content === "string") return true;
  if (!["user", "assistant", "toolResult"].includes(message.role) || !Array.isArray(content))
    return false;
  for (const part of content) {
    if (!part || typeof part.type !== "string") return false;
    if (part.type === "text") {
      if (typeof part.text !== "string") return false;
    } else if (part.type === "thinking") {
      if (typeof part.thinking !== "string") return false;
    } else if (part.type === "image") {
      if (typeof part.data !== "string" || typeof part.mimeType !== "string") return false;
    } else if (part.type === "toolCall") {
      if (
        typeof part.id !== "string" ||
        typeof part.name !== "string" ||
        !part.arguments ||
        typeof part.arguments !== "object" ||
        Array.isArray(part.arguments)
      )
        return false;
    } else return false;
  }
  if (message.role === "assistant")
    return (
      typeof message.api === "string" &&
      typeof message.provider === "string" &&
      typeof message.model === "string" &&
      message.usage &&
      typeof message.usage === "object" &&
      ["stop", "length", "toolUse", "error", "aborted"].includes(message.stopReason)
    );
  if (message.role === "toolResult")
    return (
      typeof message.toolCallId === "string" &&
      typeof message.toolName === "string" &&
      typeof message.isError === "boolean"
    );
  return true;
}

function ownedFile(path, root) {
  const stat = lstatSync(path);
  if (
    !stat.isFile() ||
    stat.isSymbolicLink() ||
    stat.nlink !== 1 ||
    realpathSync(path) !== path ||
    dirname(path) !== root ||
    stat.uid !== process.getuid() ||
    (stat.mode & 0o077) !== 0
  )
    throw new Error();
  if (stat.size > 64 * 1024 * 1024) throw new Error();
  const lines = readFileSync(path, "utf8").split("\n");
  if (lines.pop() !== "") throw new Error();
  const entries = lines.map((line) => JSON.parse(line));
  if (entries.length > 100000 || lines.some((line) => Buffer.byteLength(line) > 8 * 1024 * 1024))
    throw new Error();
  if (
    entries.length < 2 ||
    !entries.slice(1).some((entry) => entry.type === "message" || entry.type === "compaction")
  )
    throw new Error();
  const seen = new Set();
  const byId = new Map();
  for (const entry of entries.slice(1)) {
    if (
      typeof entry.id !== "string" ||
      !entry.id ||
      seen.has(entry.id) ||
      !Number.isFinite(Date.parse(entry.timestamp)) ||
      (entry.parentId !== null && !seen.has(entry.parentId))
    )
      throw new Error();
    if (entry.type === "message") {
      if (!messageValid(entry.message)) throw new Error();
    } else if (entry.type === "model_change") {
      if (typeof entry.provider !== "string" || typeof entry.modelId !== "string")
        throw new Error();
    } else if (entry.type === "thinking_level_change") {
      if (typeof entry.thinkingLevel !== "string") throw new Error();
    } else if (entry.type === "compaction") {
      if (
        typeof entry.summary !== "string" ||
        !entry.summary ||
        !seen.has(entry.firstKeptEntryId) ||
        !Number.isFinite(entry.tokensBefore) ||
        entry.tokensBefore < 0
      )
        throw new Error();
    } else if (entry.type === "context_edit") {
      const target = byId.get(entry.targetId);
      if (
        target?.type !== "message" ||
        !["user", "assistant", "toolResult"].includes(target.message.role)
      )
        throw new Error();
      let ancestor = entry.parentId;
      while (ancestor !== null && ancestor !== entry.targetId)
        ancestor = byId.get(ancestor).parentId;
      if (ancestor !== entry.targetId) throw new Error();
      if (entry.replacement !== null) {
        const replacement = entry.replacement;
        if (
          !replacement ||
          typeof replacement !== "object" ||
          Array.isArray(replacement) ||
          !("content" in replacement)
        )
          throw new Error();
        let content = replacement.content;
        if (typeof content === "string" && target.message.role !== "user")
          content = [{ type: "text", text: content }];
        if (!messageValid({ ...target.message, content })) throw new Error();
      }
    } else if (entry.type === "usage") {
      if (
        typeof entry.kind !== "string" ||
        typeof entry.provider !== "string" ||
        typeof entry.model !== "string" ||
        !entry.usage ||
        typeof entry.usage !== "object"
      )
        throw new Error();
    } else throw new Error();
    seen.add(entry.id);
    byId.set(entry.id, entry);
  }
  const fd = openSync(path, "r");
  try {
    const bytes = Buffer.alloc(8192);
    const count = readSync(fd, bytes, 0, bytes.length, 0);
    const end = bytes.subarray(0, count).indexOf(10);
    if (end < 0) throw new Error();
    const header = JSON.parse(bytes.subarray(0, end).toString("utf8"));
    if (
      header.type !== "session" ||
      header.version !== 3 ||
      typeof header.id !== "string" ||
      !header.id ||
      !Number.isFinite(Date.parse(header.timestamp)) ||
      header.cwd !== process.cwd()
    )
      throw new Error();
    return header;
  } finally {
    closeSync(fd);
  }
}

try {
  const route = JSON.parse(process.env.HIROUTE_PI_ROUTE);
  const nodeVersion = process.versions.node.split(".").map(Number);
  if (
    nodeVersion.some((part, index) =>
      index === 0
        ? part < route.minimumNode[0]
        : nodeVersion[0] === route.minimumNode[0] && index === 1 && part < route.minimumNode[1],
    )
  )
    throw new Error();
  startupStage = "sdk_load";
  const { sdk, pkg } = await loadPiSdk(process.argv[2]);
  startupStage = "sdk_capability";
  assertPiCapabilities(sdk, "collaboration");
  const {
    createAgentSession,
    ModelRuntime,
    SettingsManager,
    SessionManager,
    DefaultResourceLoader,
  } = sdk;
  const agentDir = resolve(process.env.PI_CODING_AGENT_DIR);
  const root = realpathSync(process.env.HIROUTE_PI_SESSION_ROOT);
  const file = join(root, "native-pi.jsonl");
  const token = process.env.HIROUTE_RUN_TOKEN;
  delete process.env.HIROUTE_RUN_TOKEN;
  delete process.env.HIROUTE_PI_ROUTE;
  const modelId = route.provider + "/" + route.model.id;
  startupStage = "route_binding";
  const credentials = new Map();
  const runtime = await ModelRuntime.create({
    modelsPath: null,
    allowModelNetwork: false,
    refreshOnCreate: false,
    credentials: {
      async read(id) {
        return credentials.get(id);
      },
      async list() {
        return [];
      },
      async modify(id, fn) {
        const next = await fn(credentials.get(id));
        if (next !== undefined) credentials.set(id, next);
        return credentials.get(id);
      },
      async delete(id) {
        credentials.delete(id);
      },
    },
  });
  runtime.registerProvider(route.provider, {
    api: "openai-responses",
    baseUrl: route.endpoint,
    models: [route.model],
  });
  await runtime.setRuntimeApiKey(route.provider, token);
  const model = runtime.getModels(route.provider).find((item) => item.id === route.model.id);
  if (
    !model ||
    model.api !== "openai-responses" ||
    model.baseUrl !== route.endpoint ||
    model.contextWindow !== route.model.contextWindow ||
    model.maxTokens !== route.model.maxTokens
  )
    throw new Error();

  // Read native resource settings without writing them. Snapshot each scope, so reload cannot
  // undo host overrides or turn relative project Skill paths into user paths.
  startupStage = "resources";
  const raw = SettingsManager.create(process.cwd(), agentDir, {
    projectTrusted: true,
  });
  const snapshots = new Map(
    ["global", "project"].map((scope) => {
      const source = scope === "global" ? raw.getGlobalSettings() : raw.getProjectSettings();
      return [
        scope,
        JSON.stringify({
          ...source,
          extensions: [],
          httpProxy: undefined,
          cacheWarming: "off",
          retry: { enabled: false },
          compaction: {
            enabled: true,
            reserveTokens: route.model.maxTokens,
            keepRecentTokens: Math.min(Math.floor(route.model.contextWindow / 4), 20000),
            modelOverrides: {},
          },
        }),
      ];
    }),
  );
  const settingsManager = SettingsManager.fromStorage(
    {
      withLock(scope, fn) {
        const next = fn(snapshots.get(scope));
        if (next !== undefined) snapshots.set(scope, next);
      },
    },
    { projectTrusted: true },
  );
  const resourceLoader = new DefaultResourceLoader({
    cwd: process.cwd(),
    agentDir,
    settingsManager,
    noExtensions: true,
  });
  await resourceLoader.reload();

  function configuration() {
    return [
      {
        id: "model",
        name: "Model",
        type: "select",
        category: "model",
        currentValue: session.model.provider + "/" + session.model.id,
        options: [{ value: modelId, name: "Frozen Plan" }],
      },
    ];
  }
  function ready() {
    return {
      sessionId: session.sessionId,
      _meta: { agentSessionId: session.sessionId },
      configOptions: configuration(),
      modes: {
        currentModeId: "approve-all",
        availableModes: [{ id: "approve-all", name: "Approve all" }],
      },
    };
  }
  function requireSession(params) {
    if (!session || params.sessionId !== session.sessionId) throw new Error();
  }
  let initialized = false;
  dispatch = async (request) => {
    const p = request.params || {};
    let stage = "request";
    try {
      if (request.method === "initialize") {
        if (initialized || p.protocolVersion !== 1) throw new Error();
        initialized = true;
        reply(request, {
          protocolVersion: 1,
          agentInfo: { name: "hiroute-pi-sdk", version: pkg.version },
          agentCapabilities: { loadSession: true },
        });
        return;
      }
      if (!initialized) throw new Error();
      if (request.method === "session/new" || request.method === "session/load") {
        if (session || realpathSync(p.cwd) !== process.cwd() || p.mcpServers?.length)
          throw new Error();
        let manager;
        if (request.method === "session/load") {
          stage = "sdk_capability";
          assertPiCapabilities(sdk, "continue");
          stage = "history";
          const header = ownedFile(file, root);
          if (header.id !== p.sessionId) throw new Error();
          stage = "session_load";
          manager = SessionManager.open(file, root);
          if (
            manager.getSessionId() !== header.id ||
            manager.getSessionFile() !== file ||
            manager.buildSessionContext().messages.length === 0
          )
            throw new Error();
        } else {
          stage = "sdk_capability";
          assertPiCapabilities(sdk, "worker");
          stage = "session_create";
          try {
            lstatSync(file);
            throw new Error("already-exists");
          } catch (error) {
            if (error.code !== "ENOENT") throw error;
          }
          manager = SessionManager.create(process.cwd(), root);
          manager.setSessionFile(file);
        }
        stage = request.method === "session/load" ? "session_load" : "session_create";
        ({ session } = await createAgentSession({
          cwd: process.cwd(),
          agentDir,
          modelRuntime: runtime,
          model,
          thinkingLevel: "off",
          settingsManager,
          resourceLoader,
          sessionManager: manager,
          tools: ["read", "bash", "edit", "write", "grep", "find", "ls"],
        }));
        // SDK load may restore its previous model. Frozen route selection is explicit.
        stage = "route_binding";
        await session.setModel(model);
        if (session.model?.provider !== route.provider || session.model?.id !== route.model.id)
          throw new Error();
        session.subscribe((event) => {
          if (
            !closing &&
            active &&
            event.type === "message_update" &&
            event.assistantMessageEvent?.type === "text_delta"
          ) {
            output({
              jsonrpc: "2.0",
              method: "session/update",
              params: {
                sessionId: session.sessionId,
                update: {
                  sessionUpdate: "agent_message_chunk",
                  content: {
                    type: "text",
                    text: event.assistantMessageEvent.delta,
                  },
                },
              },
            });
          }
        });
        reply(request, ready());
        return;
      }
      requireSession(p);
      if (request.method === "session/set_mode") {
        if (p.modeId !== "approve-all") throw new Error();
        reply(request, {});
        return;
      }
      if (request.method === "session/set_config_option") {
        if (p.configId !== "model" || p.value !== modelId) throw new Error();
        reply(request, { configOptions: configuration() });
        return;
      }
      if (request.method === "session/cancel") {
        cancelled = true;
        try {
          session.clearQueue();
          await session.abort();
        } catch {
          process.exitCode = 1;
          void shutdown();
        }
        return;
      }
      if (request.method === "session/prompt") {
        if (active || !Array.isArray(p.prompt) || p.prompt.some((part) => part.type !== "text"))
          throw new Error();
        const text = p.prompt.map((part) => part.text).join("\n");
        if (!text || text.length > 256 * 1024) throw new Error();
        active = true;
        cancelled = false;
        try {
          stage = "prompt";
          try {
            await session.prompt(text);
          } catch (error) {
            if (!cancelled) throw error;
          }
          const last = [...session.messages].reverse().find((item) => item.role === "assistant");
          if (!cancelled && last?.stopReason !== "stop") throw new Error();
          // A cancelled prompt can legitimately have no completed assistant/history yet.
          if (!cancelled) {
            stage = "history";
            const header = ownedFile(file, root);
            if (header.id !== session.sessionId) throw new Error();
          }
          reply(request, { stopReason: cancelled ? "cancelled" : "end_turn" });
        } finally {
          active = false;
        }
        return;
      }
      throw new Error();
    } catch {
      reject(request, stage);
    }
  };
} catch {
  // Keep the transport alive long enough to return a safe correlated initialize error.
  // Raw SDK errors can contain credentials, file contents or model responses.
  startupFailure = startupStage;
}

try {
  const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
  for await (const line of input) {
    if (closing) break;
    if (Buffer.byteLength(line) > 8 * 1024 * 1024) throw new Error();
    const request = JSON.parse(line);
    if (
      !request ||
      request.jsonrpc !== "2.0" ||
      typeof request.method !== "string" ||
      (request.params !== undefined &&
        (!request.params || typeof request.params !== "object" || Array.isArray(request.params))) ||
      (request.id !== undefined &&
        !(
          (typeof request.id === "string" && request.id.length <= 1024) ||
          Number.isSafeInteger(request.id)
        ))
    )
      throw new Error();
    if (startupFailure) {
      reject(request, startupFailure);
      break;
    }
    if (request.method === "session/cancel") {
      // Cancellation must overtake a pending prompt; all other requests are exclusive.
      await dispatch(request);
    } else if (inFlight) {
      reject(request, "busy");
    } else {
      inFlight = dispatch(request).finally(() => {
        inFlight = undefined;
      });
    }
  }
} catch {
  process.exitCode = 1;
} finally {
  await shutdown();
}
