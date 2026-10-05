import nodeTest from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import {
  mkdtempSync,
  realpathSync,
  mkdirSync,
  writeFileSync,
  readFileSync,
  existsSync,
  rmSync,
  copyFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline";
import { once } from "node:events";
const test = (name, fn) => nodeTest(name, { timeout: 5000 }, fn);

// A selected official-package shape with controlled failures, not native product evidence.
const sdk = `
import {appendFileSync,readFileSync} from 'node:fs';
const receipt = event => appendFileSync(process.env.FIXTURE_RECEIPT, event+'\\n');
const secret = 'fixture-private-token-do-not-emit';
export const CURRENT_SESSION_VERSION = 3;
export const createReadTool = () => {};
export const createBashTool = () => {};
export class SettingsManager {
  static create() {return new SettingsManager();}
  static fromStorage() {return new SettingsManager();}
  getGlobalSettings() {return {};}
  getProjectSettings() {return {};}
}
export class DefaultResourceLoader {async reload() {} getSkills() {return {skills:[]};}}
export class ModelRuntime {
  static async create() {return new ModelRuntime();}
  registerProvider(provider, value) {this.models=value.models.map(model=>({...model,provider,api:value.api,baseUrl:value.baseUrl}));}
  setRuntimeApiKey() {}
  getModels() {return this.models;}
}
export class SessionManager {
  static create() {return new SessionManager();}
  static open(file) {receipt('open');const manager=new SessionManager();manager.file=file;return manager;}
  getSessionId() {return 'fixture-session';}
  getSessionFile() {return this.file;}
  setSessionFile(file) {this.file=file;}
  buildSessionContext() {return {messages:[{}]};}
}
export class AgentSession {
  constructor() {this.sessionId='fixture-session';this.messages=[];}
  async setModel(model) {this.model=model;}
  subscribe(handler) {this.handler=handler;}
  async prompt() {
    receipt('prompt');
    if(process.env.FIXTURE_FAILURE==='history') {this.messages=readFileSync('native-pi.jsonl','utf8').trim().split('\\n').map(JSON.parse).filter(e=>e.type==='message').map(e=>e.message);return;}
    if(process.env.FIXTURE_FAILURE==='reject') throw new Error(secret);
    if(process.env.FIXTURE_FAILURE==='terminal') {this.messages=[{role:'assistant',stopReason:'error',errorMessage:secret}];return;}
    return new Promise((resolve,reject)=>{this.pending=reject;});
  }
  clearQueue() {}
  async abort() {receipt('abort');this.pending?.(new Error(secret));}
  dispose() {receipt('dispose');}
}
export async function createAgentSession() {
  receipt('creating');
  await new Promise(resolve=>setTimeout(resolve, Number(process.env.FIXTURE_CREATE_DELAY||0)));
  receipt('created');
  return {session:new AgentSession()};
}
`;

function fixture(t, env = {}) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "hiroute-pi-bridge-test-")));
  const packageRoot = join(root, "package");
  mkdirSync(packageRoot);
  const cli = join(packageRoot, "cli.mjs");
  writeFileSync(cli, "");
  writeFileSync(
    join(packageRoot, "package.json"),
    JSON.stringify({
      name: "@earendil-works/pi-coding-agent",
      version: "99.0.0-fixture",
      bin: { pi: "cli.mjs" },
      exports: "./sdk.mjs",
    }),
  );
  if (env.FIXTURE_FAILURE !== "sdk_missing") writeFileSync(join(packageRoot, "sdk.mjs"), sdk);
  const bridge = join(root, "bridge.mjs");
  copyFileSync(
    process.env.HIROUTE_TEST_PI_BRIDGE ||
      fileURLToPath(new URL("./pi_worker_bridge.mjs", import.meta.url)),
    bridge,
  );
  copyFileSync(
    fileURLToPath(
      new URL("../../../../integrations/src/agents/pi_sdk_contract.mjs", import.meta.url),
    ),
    join(root, "pi-sdk-contract.mjs"),
  );
  const receipt = join(root, "receipt");
  const child = spawn(process.execPath, [bridge, cli], {
    cwd: root,
    stdio: ["pipe", "pipe", "pipe"],
    env: {
      PATH: process.env.PATH,
      HOME: root,
      PI_CODING_AGENT_DIR: root,
      HIROUTE_PI_SESSION_ROOT: root,
      HIROUTE_RUN_TOKEN: "fixture-private-token-do-not-emit",
      HIROUTE_PI_ROUTE: JSON.stringify({
        minimumNode: [22, 19],
        provider: "managed",
        endpoint: "http://127.0.0.1:1/v1",
        model: { id: "frozen", contextWindow: 16384, maxTokens: 4096 },
      }),
      FIXTURE_RECEIPT: receipt,
      ...env,
    },
  });
  let output = "";
  let stderr = "";
  child.stdout.on("data", (data) => {
    output += data;
  });
  child.stderr.on("data", (data) => {
    stderr += data;
  });
  const exit = once(child, "exit");
  const replies = new Map();
  const waiting = new Map();
  createInterface({ input: child.stdout }).on("line", (line) => {
    const response = JSON.parse(line);
    if (response.id === undefined) return;
    replies.set(response.id, response);
    waiting.get(response.id)?.(response);
  });
  t.after(async () => {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    await exit;
    assert.doesNotMatch(output + stderr, /fixture-private-token-do-not-emit/);
    rmSync(root, { recursive: true, force: true });
  });
  let id = 0;
  return {
    root,
    child,
    exit,
    events: () => (existsSync(receipt) ? readFileSync(receipt, "utf8").trim().split("\n") : []),
    send(method, params, notification = false) {
      const requestId = ++id;
      const response = new Promise((resolve, reject) => {
        if (notification) {
          resolve();
          return;
        }
        const timer = setTimeout(() => reject(new Error("missing ACP response: " + method)), 2000);
        waiting.set(requestId, (value) => {
          clearTimeout(timer);
          resolve(value);
        });
      });
      child.stdin.write(
        JSON.stringify({
          jsonrpc: "2.0",
          ...(notification ? {} : { id: requestId }),
          method,
          params,
        }) + "\n",
      );
      return response;
    },
    async ready() {
      assert.equal(
        (await this.send("initialize", { protocolVersion: 1 })).result.protocolVersion,
        1,
      );
      return this.send("session/new", { cwd: root, mcpServers: [] });
    },
    async event(value) {
      const deadline = Date.now() + 2000;
      while (!this.events().includes(value)) {
        if (Date.now() > deadline) throw new Error("missing native event: " + value);
        await new Promise((resolve) => setTimeout(resolve, 5));
      }
    },
  };
}

function stage(response) {
  return response.error?.data?.stage;
}

test("startup dependency failure reaches the ACP caller without raw exception text", async (t) => {
  const f = fixture(t, { FIXTURE_FAILURE: "sdk_missing" });
  assert.equal(stage(await f.send("initialize", { protocolVersion: 1 })), "sdk_load");
  await f.exit;
});

test("only one native session is created when creation requests overlap", async (t) => {
  const f = fixture(t, { FIXTURE_CREATE_DELAY: "80" });
  await f.send("initialize", { protocolVersion: 1 });
  const first = f.send("session/new", { cwd: f.root, mcpServers: [] });
  await f.event("creating");
  assert.equal(stage(await f.send("session/new", { cwd: f.root, mcpServers: [] })), "busy");
  assert.equal((await first).result.sessionId, "fixture-session");
  assert.equal(f.events().filter((event) => event === "created").length, 1);
});

for (const failure of ["reject", "terminal"])
  test("native " + failure + " is an execution failure, never success", async (t) => {
    const f = fixture(t, { FIXTURE_FAILURE: failure });
    await f.ready();
    assert.equal(
      stage(
        await f.send("session/prompt", {
          sessionId: "fixture-session",
          prompt: [{ type: "text", text: "run" }],
        }),
      ),
      "prompt",
    );
  });

test("cancellation overtakes a prompt and does not require completed history", async (t) => {
  const f = fixture(t);
  await f.ready();
  const result = f.send("session/prompt", {
    sessionId: "fixture-session",
    prompt: [{ type: "text", text: "run" }],
  });
  await f.event("prompt");
  await f.send("session/cancel", { sessionId: "fixture-session" }, true);
  assert.equal((await result).result.stopReason, "cancelled");
  f.child.stdin.end();
  await f.exit;
  assert.equal(f.events().filter((event) => event === "dispose").length, 1);
});

test("disconnect during session creation releases the eventual session", async (t) => {
  const f = fixture(t, { FIXTURE_CREATE_DELAY: "80" });
  await f.send("initialize", { protocolVersion: 1 });
  // Do not await a response after the caller has disconnected.
  f.child.stdin.write(
    JSON.stringify({
      jsonrpc: "2.0",
      id: 99,
      method: "session/new",
      params: { cwd: f.root, mcpServers: [] },
    }) + "\n",
  );
  await f.event("creating");
  f.child.stdin.end();
  await f.exit;
  assert.equal(f.events().filter((event) => event === "dispose").length, 1);
  assert.ok(f.events().includes("abort"));
});

test("malformed input aborts the active work and releases its session", async (t) => {
  const f = fixture(t);
  await f.ready();
  f.child.stdin.write(
    JSON.stringify({
      jsonrpc: "2.0",
      id: 99,
      method: "session/prompt",
      params: { sessionId: "fixture-session", prompt: [{ type: "text", text: "run" }] },
    }) + "\n",
  );
  await f.event("prompt");
  f.child.stdin.write("{invalid\n");
  await f.exit;
  assert.ok(f.events().includes("abort"));
  assert.ok(f.events().includes("dispose"));
});

test("missing exact history is rejected before SDK open or model work", async (t) => {
  const f = fixture(t);
  await f.send("initialize", { protocolVersion: 1 });
  assert.equal(
    stage(await f.send("session/load", { cwd: f.root, mcpServers: [], sessionId: "old" })),
    "history",
  );
  assert.deepEqual(f.events(), []);
});

test("a closed output pipe releases the native session without an unhandled error", async (t) => {
  const f = fixture(t);
  await f.ready();
  f.child.stdout.destroy();
  f.child.stdin.write(
    JSON.stringify({
      jsonrpc: "2.0",
      id: 99,
      method: "session/set_mode",
      params: { sessionId: "fixture-session", modeId: "approve-all" },
    }) + "\n",
  );
  await f.exit;
  assert.ok(f.events().includes("dispose"));
});

function editedHistory(f, replacement = null, targetId = "failed") {
  const timestamp = new Date().toISOString();
  const assistant = (stopReason) => ({
    role: "assistant",
    timestamp: Date.now(),
    content: [{ type: "text", text: "result" }],
    api: "openai-responses",
    provider: "managed",
    model: "frozen",
    usage: {},
    stopReason,
  });
  const entries = [
    { type: "session", version: 3, id: "fixture-session", timestamp, cwd: f.root },
    {
      type: "message",
      id: "user",
      parentId: null,
      timestamp,
      message: { role: "user", timestamp: Date.now(), content: "task" },
    },
    { type: "message", id: "failed", parentId: "user", timestamp, message: assistant("error") },
    { type: "context_edit", id: "omit", parentId: "failed", timestamp, targetId, replacement },
    { type: "message", id: "recovered", parentId: "omit", timestamp, message: assistant("stop") },
  ];
  writeFileSync(join(f.root, "native-pi.jsonl"), entries.map(JSON.stringify).join("\n") + "\n", {
    mode: 0o600,
  });
}

for (const operation of ["prompt", "load"]) {
  for (const replacement of [null, { content: "recovered content" }]) {
    test(`valid context edit survives ${operation}: ${replacement === null ? "omit" : "replace"}`, async (t) => {
      const f = fixture(t, { FIXTURE_FAILURE: "history" });
      if (operation === "prompt") await f.ready();
      else await f.send("initialize", { protocolVersion: 1 });
      editedHistory(f, replacement);
      const response =
        operation === "load"
          ? await f.send("session/load", {
              cwd: f.root,
              mcpServers: [],
              sessionId: "fixture-session",
            })
          : await f.send("session/prompt", {
              sessionId: "fixture-session",
              prompt: [{ type: "text", text: "continue" }],
            });
      assert.ok(response.result, JSON.stringify(response));
    });
  }
}
for (const [replacement, target] of [
  [undefined, "failed"],
  [{ content: 42 }, "failed"],
  [null, "missing"],
  [null, "omit"],
]) {
  test(`invalid context edit refuses native load: ${JSON.stringify([replacement, target])}`, async (t) => {
    const f = fixture(t);
    await f.send("initialize", { protocolVersion: 1 });
    editedHistory(f, replacement === undefined ? {} : replacement, target);
    assert.equal(
      stage(
        await f.send("session/load", { cwd: f.root, mcpServers: [], sessionId: "fixture-session" }),
      ),
      "history",
    );
    assert.deepEqual(f.events(), []);
  });
}
