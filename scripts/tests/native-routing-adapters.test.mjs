import assert from "node:assert/strict";
import { test } from "node:test";
import "./pi-routing-adapter.test.mjs";
import { HiRouteRouting } from "../../assets/agents/opencode/hiroute-routing.mjs";
import { apply as dshRouting } from "../../assets/agents/dsh/hiroute-routing.mjs";
import { createHash } from "node:crypto";

test("OpenCode reads completed native checkpoints, real turns and isolates concurrent auxiliary calls", async () => {
  const entries = [
    { info: { role: "user", id: "01" }, parts: [{ type: "text" }] },
    { info: { role: "user", id: "01c" }, parts: [{ type: "compaction", auto: true }] },
    { info: { role: "assistant", id: "02", parentID: "01c", summary: true, time: { completed: 1 } }, parts: [] },
    { info: { role: "user", id: "03" }, parts: [{ type: "text", synthetic: true, metadata: { compaction_continue: true } }] },
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
  assert.equal(main.continuation_requires_existing_turn, true);
  assert.equal(JSON.parse(output[3]["x-hiroute-routing-context"]).session_id, "child");
  assert.deepEqual(await headers(await create(), input("build")), output[0]);
  entries.push({ info: { role: "user", id: "05" }, parts: [{ type: "text" }] });
  const next = input("build"); next.message.id = "05";
  await hook["chat.message"](next, { message: { id: "05" }, parts: entries.at(-1).parts });
  const nextContext = JSON.parse((await headers(hook, next))["x-hiroute-routing-context"]);
  assert.equal(nextContext.turn_id, "05");
  assert.equal(nextContext.continuation_requires_existing_turn, false);
  const restarted = await create();
  assert.equal(JSON.parse((await headers(restarted, next))["x-hiroute-routing-context"])
    .continuation_requires_existing_turn, undefined, "a cold plugin must not claim an unseen prompt");
  await restarted["chat.message"](next, { message: { id: "05" }, parts: entries.at(-1).parts });
  assert.equal(JSON.parse((await headers(restarted, next))["x-hiroute-routing-context"])
    .continuation_requires_existing_turn, false);
  // A queued newer real user cannot change the identity of an earlier in-flight continuation.
  const earlier = JSON.parse((await headers(hook, input("build")))["x-hiroute-routing-context"]);
  assert.equal(earlier.turn_id, "01");
  assert.equal(earlier.continuation_requires_existing_turn, true);
  const broken = await HiRouteRouting({ client: { session: { messages: async () => { throw Error("native read failed"); } } } });
  assert.deepEqual(await headers(broken, next), {});
  const hung = await HiRouteRouting({ client: { session: { messages: () => new Promise(() => {}) } } });
  const start = performance.now();
  assert.deepEqual(await headers(hung, next), {});
  assert.ok(performance.now() - start < 300);
  assert.deepEqual(await headers(hook, { ...next, model: { providerID: "foreign" } }), {});
});

test("OpenCode requires an exact per-call native user and never admits unknown synthetic parts as new input", async () => {
  const real = { info: { role: "user", id: "01" }, parts: [{ type: "text" }] };
  let entries = [real];
  const hooks = await HiRouteRouting({ client: { session: {
    messages: async () => ({ data: entries }),
  } } });
  const read = async id => {
    const output = { headers: { native: "retained" } };
    await hooks["chat.headers"]({ agent: "build", sessionID: "session",
      model: { providerID: "hiroute-main-test" }, message: { id } }, output);
    return output.headers;
  };
  assert.deepEqual(await read("missing"), { native: "retained" });
  assert.deepEqual(await read(undefined), { native: "retained" });
  assert.deepEqual(await read(""), { native: "retained" });
  entries = [{ info: { role: "user", id: "03" }, parts: [{ type: "text", synthetic: true }] }];
  assert.equal(JSON.parse((await read("03"))["x-hiroute-routing-context"]).turn_id, "03");
  for (const parts of [
    [], [{ type: "compaction", auto: true }], [{ type: "text", synthetic: true }],
    [{ type: "text", ignored: true }], [{ type: "unknown" }],
    [{ type: "compaction" }, { type: "text", synthetic: true }],
  ]) {
    entries = [real, { info: { role: "user", id: "03" }, parts }];
    const context = JSON.parse((await read("03"))["x-hiroute-routing-context"]);
    assert.equal(context.turn_id, "03");
    assert.equal(context.continuation_requires_existing_turn, undefined);
  }
  entries = [real, { info: { role: "user", id: "03" }, parts: [
    { type: "text", synthetic: true }, { type: "file" },
  ] }];
  assert.equal(JSON.parse((await read("03"))["x-hiroute-routing-context"])
    .continuation_requires_existing_turn, undefined, "a copied overflow user is not a newly observed prompt");
});

test("OpenCode binds typed compaction to its source and leaves late child input observed", async () => {
  const entries = [
    { info: { role: "user", id: "01" }, parts: [{ type: "text" }] },
    { info: { role: "user", id: "02" }, parts: [{ type: "compaction", auto: true }] },
    { info: { role: "user", id: "03" }, parts: [{ type: "text" }] },
    { info: { role: "assistant", id: "04", parentID: "02", summary: true, time: { completed: 1 } }, parts: [] },
    { info: { role: "user", id: "05" }, parts: [{ type: "text", synthetic: true, metadata: { compaction_continue: true } }] },
    { info: { role: "user", id: "06" }, parts: [{ type: "text", synthetic: true }] },
    { info: { role: "user", id: "07" }, parts: [{ type: "text" }] },
  ];
  const hooks = await HiRouteRouting({ client: { session: { messages: async () => ({ data: entries }) } } });
  const input = id => ({ sessionID: "session", agent: "build", model: { providerID: "hiroute-main-test" }, message: { id } });
  const read = async id => {
    const output = { headers: {} };
    await hooks["chat.headers"](input(id), output);
    return JSON.parse(output.headers["x-hiroute-routing-context"]);
  };
  assert.equal((await read("05")).turn_id, "01", "queued B cannot own A's auto-continue");
  assert.equal((await read("05")).continuation_requires_existing_turn, true);
  assert.equal((await read("06")).turn_id, "06");
  assert.equal((await read("06")).continuation_requires_existing_turn, undefined);
  await hooks["chat.message"](input("07"), { message: { id: "07" }, parts: entries.at(-1).parts });
  assert.equal((await read("07")).continuation_requires_existing_turn, false);
  entries.splice(0, 3);
  assert.equal((await read("05")).turn_id, undefined, "missing source cannot be replaced by the latest user");
  assert.equal((await read("05")).continuation_requires_existing_turn, true, "known synthetic summary keeps capture guard");
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
    const sendMatching = async current => {
      const actualBody = JSON.stringify({ input: current.messages.slice(1).map(m => ({ role: m.role, content: m.content })) });
      const actualNext = async function* () { yield await fetch("http://127.0.0.1/v1/responses", {
        method: "POST", body: actualBody, headers: { "x-hiroute-agent": "dsh", "x-hiroute-token": "synthetic" },
      }); };
      assert.equal((await hook(current, actualNext).next()).value, "native-result");
      assert.equal(sent.at(-1).body, actualBody);
      return JSON.parse(new Headers(sent.at(-1).headers).get("x-hiroute-routing-context"));
    };
    // A proven user earlier in this turn cannot authorize silently discarding
    // a later envelope whose native source semantics are unknown.
    for (const source of [{ kind: "future-user-source" }, undefined]) {
      const unknown = { ...user("future", "UNKNOWN-INTENT"), source };
      assert.equal((await sendMatching({ ...options, messages: [...options.messages, unknown] })).user_turns, undefined);
    }
    // Adjacent-Agent relays are native synthetic envelopes. The child's actual
    // task uses source.kind=user and is covered by the real-user group above.
    const relay = { ...user("relay", "PEER-RELAY", "agent-message"), source: {
      kind: "agent-message", form: "relay", senderSessionId: "peer",
    } };
    assert.equal((await sendMatching({ ...options, messages: [...options.messages, relay] })).user_turns, undefined);
    const settlement = { ...user("settled", "NATIVE-SETTLEMENT", "subagent-settled"), source: {
      kind: "subagent-settled", form: "notice", senderSessionId: "peer", summary: "native completion",
    } };
    assert.equal((await sendMatching({ ...options, messages: [...options.messages, settlement] })).user_turns, undefined);
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
    assert.equal((await sendMatching({ ...resumed, messages: [...resumed.messages, settlement] })).user_turns,
      undefined, "late child output must not acquire the newer human's proof");
    const fresh = user("u4", "FRESH-D");
    events.push({ type: "turn/start", data: { turn: 3 } }, { type: "user/message", data: fresh });
    const recovered = await sendMatching({ ...resumed, messages: [...resumed.messages, settlement, fresh] });
    assert.equal(recovered.user_turns.at(-1).turn_id, "3", "a later real input recovers native binding");
  } finally { globalThis.fetch = original; delete globalThis[symbol]; }
});

test("DSH projects native pure-text blocks within each user envelope before binding exact wire text", async () => {
  const original = globalThis.fetch;
  const symbol = Symbol.for("hiroute.dsh.routing-context.v1");
  const sent = [];
  globalThis.fetch = async (_url, init) => { sent.push(init); return "native-result"; };
  delete globalThis[symbol];
  const text = value => ({ type: "text", text: value });
  const user = (id, content, kind = "user") => ({ id, role: "user", content, source: { kind } });
  const first = user("first", [text("NATIVE-TASK"), text("NATIVE-RETURN-GUIDANCE")]);
  const policy = user("policy", [text("RUNTIME-POLICY")], "runtime-context");
  const second = user("second", [text("SECOND-ENVELOPE")]);
  const events = [{ type: "turn/start", data: { turn: 1 } },
    ...[first, policy, second].map(data => ({ type: "user/message", data }))];
  let hook;
  dshRouting({ sessions: { get: () => ({ seq: events.length, eventAt: i => events[i] }) },
    on: (_name, callback) => { hook = callback; } });
  const options = { provider: "hiroute-main-test", sessionId: "child", messages: [
    { role: "system", content: [text("SYSTEM")] }, first, policy, second,
  ] };
  const digest = value => createHash("sha256").update(JSON.stringify([value])).digest("hex");
  try {
    for (const protocol of ["responses", "messages"]) {
      // dsh-llm-pi-ai 0.2.0-rc.2 flattens pure-text content with join("").
      // pi-ai 0.87.1 emits one Responses input_text block or Messages string.
      const envelope = value => ({ role: "user", content: protocol === "responses"
        ? [{ type: "input_text", text: value }] : value });
      const wire = [envelope("NATIVE-TASKNATIVE-RETURN-GUIDANCE"),
        envelope("RUNTIME-POLICY"), envelope("SECOND-ENVELOPE")];
      const send = async (messages, native = options) => {
        const body = JSON.stringify({ [protocol === "responses" ? "input" : "messages"]: messages });
        const next = async function* () { yield await fetch("http://127.0.0.1/v1/" + protocol, {
          method: "POST", body, headers: { "x-hiroute-agent": "dsh", "x-hiroute-token": "synthetic" },
        }); };
        assert.equal((await hook(native, next).next()).value, "native-result");
        assert.equal(sent.at(-1).body, body);
        return JSON.parse(new Headers(sent.at(-1).headers).get("x-hiroute-routing-context"));
      };
      assert.deepEqual((await send(wire)).user_turns, [{ turn_id: "1", messages: [
        { index: 0, sha256: digest("NATIVE-TASKNATIVE-RETURN-GUIDANCE") },
        { index: 2, sha256: digest("SECOND-ENVELOPE") },
      ] }]);
      assert.equal((await send([wire[2], wire[1], wire[0]])).user_turns, undefined);
      assert.equal((await send([envelope("NATIVE-TASKCHANGED"), ...wire.slice(1)])).user_turns, undefined);
      const reordered = { ...options, messages: options.messages.map(m => m.id === "first"
        ? { ...m, content: [...m.content].reverse() } : m) };
      assert.equal((await send(wire, reordered)).user_turns, undefined);
      // No cross-envelope joining or alternate block-layout guesses are admitted.
      assert.equal((await send([envelope("NATIVE-TASKNATIVE-RETURN-GUIDANCERUNTIME-POLICYSECOND-ENVELOPE")])).user_turns, undefined);
      const split = { role: "user", content: [text("NATIVE-TASK"), text("NATIVE-RETURN-GUIDANCE")] };
      assert.equal((await send([split, ...wire.slice(1)])).user_turns, undefined);
      const image = { ...options, messages: options.messages.map(m => m.id === "first"
        ? { ...m, content: [...m.content, { type: "image", attachment: {} }] } : m) };
      assert.equal((await send(wire, image)).user_turns, undefined);
    }
  } finally { globalThis.fetch = original; delete globalThis[symbol]; }
});

test("DSH rejects missing or malformed native turn numbers without merging unknown user turns", async () => {
  const original = globalThis.fetch;
  const symbol = Symbol.for("hiroute.dsh.routing-context.v1");
  const sent = [];
  globalThis.fetch = async (_url, init) => { sent.push(init); return "native-result"; };
  delete globalThis[symbol];
  const users = ["FIRST", "SECOND"].map((text, i) => ({ id: "user-" + i, role: "user",
    source: { kind: "user" }, content: [{ type: "text", text }] }));
  let events = [];
  let hook;
  dshRouting({ sessions: { get: () => ({ get seq() { return events.length; }, eventAt: i => events[i] }) },
    on: (_name, callback) => { hook = callback; } });
  const options = { provider: "hiroute-main-test", sessionId: "session", messages: users };
  const body = JSON.stringify({ input: users.map(m => ({ role: m.role, content: m.content })) });
  const next = async function* () { yield await fetch("http://127.0.0.1/v1/responses", {
    method: "POST", body, headers: { "x-hiroute-agent": "dsh", "x-hiroute-token": "synthetic" },
  }); };
  const send = async () => {
    assert.equal((await hook(options, next).next()).value, "native-result");
    assert.equal(sent.at(-1).body, body);
    return JSON.parse(new Headers(sent.at(-1).headers).get("x-hiroute-routing-context"));
  };
  try {
    for (const invalid of [undefined, null, {}, [], "1", true, 0, -1, 1.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]) {
      const bad = { type: "turn/start", data: { turn: invalid } };
      events = [bad, { type: "user/message", data: users[0] },
        bad, { type: "user/message", data: users[1] }];
      const unknown = await send();
      assert.equal(unknown.turn_id, undefined);
      assert.equal(unknown.user_turns, undefined);
      // A later valid native turn cannot retroactively bless an unknown one.
      events[2] = { type: "turn/start", data: { turn: 2 } };
      assert.equal((await send()).user_turns, undefined);
    }
    events = users.map(data => ({ type: "user/message", data }));
    const missing = await send();
    assert.equal(missing.turn_id, undefined);
    assert.equal(missing.user_turns, undefined);
    events = [{ type: "turn/start", data: { turn: 1 } }, { type: "user/message", data: users[0] },
      { type: "turn/start", data: { turn: 2 } }, { type: "user/message", data: users[1] }];
    assert.deepEqual((await send()).user_turns.map(g => g.turn_id), ["1", "2"]);
  } finally { globalThis.fetch = original; delete globalThis[symbol]; }
});
