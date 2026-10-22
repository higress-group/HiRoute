import assert from "node:assert/strict";
import { test } from "node:test";
import "./pi-routing-adapter.test.mjs";
import { HiRouteRouting } from "../../assets/agents/opencode/hiroute-routing.mjs";
import "./dsh-routing-adapter.test.mjs";

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
