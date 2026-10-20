import assert from "node:assert/strict";
import { test } from "node:test";
import piRouting from "../../assets/agents/pi/hiroute-routing.mjs";
import { HiRouteRouting } from "../../assets/agents/opencode/hiroute-routing.mjs";
import { apply as dshRouting } from "../../assets/agents/dsh/hiroute-routing.mjs";

test("Pi separates compaction, persists native checkpoint and contains hook failures", () => {
  const hooks = new Map();
  piRouting({ on: (name, hook) => hooks.set(name, hook) });
  let branch = [{ type: "message", id: "user-1", message: { role: "user" } }];
  const ctx = { model: { provider: "hiroute-main-test" }, sessionManager: {
    getSessionId: () => "session", getBranch: () => branch,
  } };
  const request = () => {
    const event = { headers: {} };
    hooks.get("before_provider_headers")(event, ctx);
    return JSON.parse(event.headers["x-hiroute-routing-context"]);
  };
  assert.equal(request().purpose, "main");
  assert.deepEqual(hooks.get("cache_warming_decision")({}, ctx), { action: "stop" });
  assert.equal(hooks.get("cache_warming_decision")({ model: { provider: "foreign" } }, ctx), undefined);
  hooks.get("session_before_compact")({}, ctx);
  assert.equal(request().purpose, "compaction");
  hooks.get("session_compact_failed")({}, ctx);
  assert.equal(request().purpose, "main");
  branch.push({ type: "compaction", id: "checkpoint" });
  assert.equal(request().checkpoint, "checkpoint");
  // Fresh plugin instance reads restored native history rather than process memory.
  piRouting({ on: (name, hook) => hooks.set(name, hook) });
  assert.equal(request().checkpoint, "checkpoint");
  ctx.sessionManager.getSessionId = () => { throw Error("unavailable native state"); };
  for (const name of ["session_before_compact", "session_compact", "session_compact_failed"]) {
    assert.doesNotThrow(() => hooks.get(name)({}, ctx));
  }
  const event = { headers: { existing: "retained" } };
  assert.doesNotThrow(() => hooks.get("before_provider_headers")(event, ctx));
  assert.deepEqual(event.headers, { existing: "retained" });
});

test("OpenCode reads completed native checkpoints, real turns and isolates concurrent auxiliary calls", async () => {
  const entries = [
    { info: { role: "user", id: "01" }, parts: [{ type: "text" }] },
    { info: { role: "assistant", id: "02", summary: true, time: { completed: 1 } }, parts: [] },
    { info: { role: "user", id: "03" }, parts: [{ type: "text", synthetic: true }] },
    { info: { role: "assistant", id: "04", summary: true, time: {} }, parts: [] },
  ];
  const create = async () => HiRouteRouting({ client: { session: { messages: async () => ({ data: entries }) } } });
  const input = (agent, sessionID = "parent") => ({ agent, sessionID, model: { providerID: "hiroute-main-test" }, message: { id: "03" } });
  const headers = async (hook, i) => { const output = { headers: {} }; await hook["chat.headers"](i, output); return output.headers; };
  const hook = await create();
  const output = await Promise.all([input("build"), input("compaction"), input("title"), input("build", "child")].map(i => headers(hook, i)));
  assert.deepEqual(output.map(h => JSON.parse(h["x-hiroute-routing-context"]).purpose), ["main", "compaction", "title", "main"]);
  const main = JSON.parse(output[0]["x-hiroute-routing-context"]);
  assert.equal(main.turn_id, "01");
  assert.equal(main.checkpoint, "02");
  assert.equal(JSON.parse(output[3]["x-hiroute-routing-context"]).session_id, "child");
  assert.deepEqual(await headers(await create(), input("build")), output[0]);
  entries.push({ info: { role: "user", id: "05" }, parts: [{ type: "text" }] });
  const next = input("build"); next.message.id = "05";
  assert.equal(JSON.parse((await headers(hook, next))["x-hiroute-routing-context"]).turn_id, "05");
  const broken = await HiRouteRouting({ client: { session: { messages: async () => { throw Error("native read failed"); } } } });
  assert.deepEqual(await headers(broken, next), {});
  assert.deepEqual(await headers(hook, { ...next, model: { providerID: "foreign" } }), {});
});

test("DSH request-local middleware preserves concurrent streams, restored turns and transport failures", async () => {
  const original = globalThis.fetch;
  const symbol = Symbol.for("hiroute.dsh.routing-context.v1");
  const requests = [];
  globalThis.fetch = async (_input, init) => { requests.push(new Headers(init?.headers)); return "provider-result"; };
  delete globalThis[symbol];
  let hook;
  const ctx = {
    sessions: { get: () => ({ seq: 1, eventAt: () => ({ type: "turn/start", data: { turn: 3 } }) }) },
    on: (_name, value) => { hook = value; },
  };
  try {
    dshRouting(ctx);
    const options = (sessionId, purpose) => ({ provider: "hiroute-main-test", sessionId, purpose,
      messages: [{ role: "user", source: { kind: "compact-checkpoint", compactionId: "checkpoint" } }] });
    const next = async function* () {
      await Promise.resolve();
      yield await fetch("http://localhost.invalid", { headers: { "x-hiroute-agent": "dsh", "x-hiroute-token": "grant" } });
    };
    const streams = [hook(options("main"), next), hook(options("main", "session-title"), next), hook(options("child"), next)];
    assert.deepEqual((await Promise.all(streams.map(s => s.next()))).map(r => r.value), Array(3).fill("provider-result"));
    const metadata = requests.map(h => JSON.parse(h.get("x-hiroute-routing-context")));
    assert.deepEqual(metadata.map(m => [m.session_id, m.purpose]), [["main", "main"], ["main", "session-title"], ["child", "main"]]);
    assert.equal(metadata[0].turn_id, "3");
    assert.equal(metadata[0].checkpoint, "checkpoint");
    ctx.sessions.get = () => { throw Error("optional metadata fault"); };
    assert.equal((await hook(options("main"), next).next()).value, "provider-result");
    assert.equal(requests.at(-1).has("x-hiroute-routing-context"), false);
    await fetch("http://localhost.invalid", { headers: { "x-hiroute-agent": "foreign" } });
    assert.equal(requests.at(-1).has("x-hiroute-routing-context"), false);
  } finally { globalThis.fetch = original; delete globalThis[symbol]; }
});
