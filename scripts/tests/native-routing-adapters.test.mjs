import assert from "node:assert/strict";
import { test } from "node:test";
import piRouting from "../../assets/agents/pi/hiroute-routing.mjs";
import { HiRouteRouting } from "../../assets/agents/opencode/hiroute-routing.mjs";
import { apply as dshRouting } from "../../assets/agents/dsh/hiroute-routing.mjs";
import { createHash } from "node:crypto";

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
  const hung = await HiRouteRouting({ client: { session: { messages: () => new Promise(() => {}) } } });
  const start = performance.now();
  assert.deepEqual(await headers(hung, next), {});
  assert.ok(performance.now() - start < 300);
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

test("DSH binds native semantic users to exact wire positions without changing model bytes", async () => {
  const original = globalThis.fetch;
  const symbol = Symbol.for("hiroute.dsh.routing-context.v1");
  const sent = [];
  globalThis.fetch = async (_url, init) => { sent.push(init); return "native-result"; };
  delete globalThis[symbol];
  const text = value => [{ type: "text", text: value }];
  const user = (id, value, kind = "user") => ({ id, role: "user", content: text(value), source: { kind } });
  const events = [{ type: "turn/start", data: { turn: 1 } },
    ...[user("u1", "USER-A"), user("u2", "USER-B"), user("r", "POLICY", "runtime-context")]
      .map(data => ({ type: "user/message", data }))];
  let hook;
  const session = { get seq() { return events.length; }, eventAt: i => events[i], header: { parentSession: "parent" } };
  dshRouting({ sessions: { get: () => session }, on: (_event, callback) => { hook = callback; } });
  const options = { provider: "hiroute-main-test", sessionId: "child", messages: [
    { id: "sys", role: "system", source: { kind: "system-prompt" }, content: text("SYSTEM") },
    user("u1", "USER-A"), user("u2", "USER-B"), user("r", "POLICY", "runtime-context"),
  ] };
  const body = JSON.stringify({ model: "alias", input: options.messages.slice(1).map(m => ({ role: m.role,
    content: m.content.map(p => ({ type: "input_text", text: p.text })) })) });
  const next = async function* () { yield await fetch("http://127.0.0.1/v1/responses", {
    method: "POST", body, headers: { "x-hiroute-agent": "dsh", "x-hiroute-token": "synthetic" },
  }); };
  try {
    assert.equal((await hook(options, next).next()).value, "native-result");
    assert.equal(sent[0].body, body);
    const headers = new Headers(sent[0].headers);
    assert.equal(headers.get("x-deepseek-harness-parent-session-id"), "parent");
    const metadata = JSON.parse(headers.get("x-hiroute-routing-context"));
    const digest = value => createHash("sha256").update(JSON.stringify([value])).digest("hex");
    assert.deepEqual(metadata.user_turns, [{ turn_id: "1", messages: [
      { index: 0, sha256: digest("USER-A") }, { index: 1, sha256: digest("USER-B") },
    ] }]);
    assert.equal(metadata.users, undefined);
    // A compacted surface without the current real user supplies no usable proof.
    const compacted = { ...options, messages: [user("cp", "summary", "compact-checkpoint")] };
    assert.equal((await hook(compacted, next).next()).value, "native-result");
    assert.equal(JSON.parse(new Headers(sent.at(-1).headers).get("x-hiroute-routing-context")).user_turns, undefined);
    // A real body mismatch and unknown image conversion are conservative misses.
    const changed = { ...options, messages: options.messages.map(m => m.id === "u1" ? { ...m, content: text("different") } : m) };
    assert.equal((await hook(changed, next).next()).value, "native-result");
    assert.equal(JSON.parse(new Headers(sent.at(-1).headers).get("x-hiroute-routing-context")).user_turns, undefined);
    const image = { ...options, messages: [user("u1", "USER-A"), { ...user("u2", "USER-B"), content: [{ type: "image", attachment: {} }] }] };
    assert.equal((await hook(image, next).next()).value, "native-result");
    assert.equal(JSON.parse(new Headers(sent.at(-1).headers).get("x-hiroute-routing-context")).user_turns, undefined);
    // A resumed real turn keeps both prior semantic users and the new input,
    // preserving the intervening runtime-context ordinal without blessing it.
    const nextUser = user("u3", "USER-C");
    events.push({ type: "turn/start", data: { turn: 2 } }, { type: "user/message", data: nextUser });
    const resumed = { ...options, messages: [...options.messages,
      { role: "assistant", content: text("completed") }, nextUser] };
    const resumedBody = JSON.stringify({ messages: resumed.messages.slice(1).map(m => ({ role: m.role, content: m.content })) });
    const resumedNext = async function* () { yield await fetch("http://127.0.0.1/v1/messages", {
      method: "POST", body: resumedBody, headers: { "x-hiroute-agent": "dsh", "x-hiroute-token": "synthetic" },
    }); };
    assert.equal((await hook(resumed, resumedNext).next()).value, "native-result");
    assert.equal(sent.at(-1).body, resumedBody);
    assert.deepEqual(JSON.parse(new Headers(sent.at(-1).headers).get("x-hiroute-routing-context")).user_turns,
      [...metadata.user_turns, { turn_id: "2", messages: [{ index: 3, sha256: digest("USER-C") }] }]);
  } finally { globalThis.fetch = original; delete globalThis[symbol]; }
});
