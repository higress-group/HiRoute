import assert from "node:assert/strict";
import { test } from "node:test";
import { HiRouteRouting } from "../../assets/agents/opencode/hiroute-routing.mjs";

const text = (value = "repeated input", extra = {}) => ({ type: "text", text: value, ...extra });
const user = (id, created, parts = [text()], sessionID = "session") =>
  ({ info: { role: "user", id, sessionID, time: { created } }, parts });
const summary = (id, created, parentID, extra = {}) => ({ info: { role: "assistant", id,
  sessionID: "session", summary: true, finish: "stop", parentID,
  time: { created, completed: created + 1 }, ...extra }, parts: [] });
const continuation = (id, created) => user(id, created,
  [text("native continuation", { synthetic: true, metadata: { compaction_continue: true } })]);
const input = (id = "current", agent = "build", sessionID = "session", providerID = "hiroute-main-test") =>
  ({ sessionID, agent, model: { providerID }, message: { id } });
const context = output => JSON.parse(output.headers["x-hiroute-routing-context"]);
const minimal = (id = "current", session = "session") => ({ version: 2, agent: "opencode", purpose: "main",
  session_id: session, turn_id: id, input: { kind: "observed_batch", event_id: id } });
async function fixture(entries = [user("current", 1)], read) {
  const calls = [];
  const hooks = await HiRouteRouting({ client: { session: { messages(options) {
    calls.push(options);
    return read ? read(options, calls.length) : Promise.resolve({ data: entries });
  } } } });
  const headers = async (value = input(), output = { headers: { native: "retained" } }) => {
    await hooks["chat.headers"](value, output);
    return output;
  };
  const created = async (entry, value = input(entry.info.id)) =>
    hooks["chat.message"](value, { message: structuredClone(entry.info), parts: structuredClone(entry.parts) });
  return { hooks, calls, headers, created };
}

for (const purpose of ["title", "summary", "compaction"]) {
  test(`OpenCode ${purpose} retains minimal routing without reading SDK history`, async () => {
    const f = await fixture([], () => {
      assert.fail("known auxiliaries must not invoke optional SDK history");
    });
    const output = await f.headers(input("current", purpose));
    assert.deepEqual(context(output), { version: 2, agent: "opencode", purpose,
      session_id: "session", turn_id: "current" });
    assert.equal(f.calls.length, 0, "known auxiliaries never invoke optional history");
    assert.equal(output.headers.native, "retained");
  });
}

test("OpenCode known purpose survives unavailable optional IDs and managed gating stays exact", async () => {
  const f = await fixture();
  const output = await f.headers(input(undefined, "compaction", "\ninvalid"));
  assert.equal(context(output).purpose, "compaction");
  assert.equal(context(output).session_id, undefined);
  for (const provider of ["foreign", "hiroute-main", "hiroute-workerish", undefined]) {
    const value = input(); value.model.providerID = provider;
    assert.deepEqual((await f.headers(value)).headers, { native: "retained" });
  }
  for (const provider of ["hiroute-main-plan", "hiroute-worker", "hiroute-worker-plan"]) {
    assert.equal(context(await f.headers(input("current", "custom-agent", "session", provider))).purpose, "main");
  }
});

for (const fault of ["reject", "synchronous throw", "timeout", "current outside page"]) {
  test(`OpenCode ordinary ${fault} retains immutable minimal routing and later recovers`, async () => {
    let failing = true;
    const f = await fixture([], () => {
      if (!failing) return { data: [user("current", 1)] };
      if (fault === "reject") return Promise.reject(Error("injected SDK rejection"));
      if (fault === "synchronous throw") throw Error("injected SDK throw");
      if (fault === "timeout") return new Promise(() => {});
      return { data: [user("other", 1)] };
    });
    const output = { headers: { native: "retained" } };
    const pending = f.hooks["chat.headers"](input(), output);
    assert.deepEqual(context(output), minimal(), "publish before awaiting SDK");
    await pending;
    assert.deepEqual(context(output), minimal());
    failing = false;
    assert.deepEqual(context(await f.headers()), minimal());
    assert.equal(output.headers.native, "retained");
  });
}

test("OpenCode uses one bounded page and aborts without accumulating ignored timeouts", async () => {
  const pending = [];
  const f = await fixture([], options => new Promise(resolve => pending.push({ options, resolve })));
  const start = performance.now();
  const outputs = await Promise.all(Array.from({ length: 24 }, (_, i) => f.headers(input(`event-${i}`))));
  assert.ok(performance.now() - start < 400);
  assert.equal(f.calls.length, 4, "at most four unresolved SDK calls own slots");
  for (const { options } of pending) {
    assert.deepEqual(options.path, { id: "session" });
    assert.deepEqual(options.query, { limit: 128 });
    assert.equal(options.signal.aborted, true);
  }
  for (let i = 0; i < outputs.length; ++i) assert.deepEqual(context(outputs[i]), minimal(`event-${i}`));
  for (const request of pending) request.resolve({ data: [continuation("event-0", 1)] });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(context(outputs[0]), minimal("event-0"), "late SDK data cannot rewrite a finished request");
  const recovery = f.headers();
  assert.equal(f.calls.length, 5, "settled slots become available again");
  pending.at(-1).resolve({ data: [user("current", 1)] });
  assert.deepEqual(context(await recovery), minimal());
});

test("OpenCode freezes each native identity before asynchronous enrichment", async () => {
  let resolve;
  const f = await fixture([], () => new Promise(done => { resolve = done; }));
  const value = input();
  const output = { headers: {} };
  const pending = f.hooks["chat.headers"](value, output);
  value.sessionID = "child"; value.message.id = "new-message"; value.agent = "title";
  value.model.providerID = "foreign";
  resolve({ data: [user("current", 1)] });
  await pending;
  assert.deepEqual(context(output), minimal());
  assert.equal(f.calls[0].path.id, "session");
});

test("OpenCode compaction references use exact native page positions with nonmonotonic IDs", async () => {
  const entries = [user("z-A", 10), user("y-parent", 20, [{ type: "compaction", auto: true }]),
    user("a-B", 30), summary("zz-summary", 40, "y-parent"), continuation("b-continue", 50)];
  const f = await fixture(entries);
  const result = context(await f.headers(input("b-continue")));
  assert.equal(result.turn_id, "b-continue", "a source reference cannot rename this native activation");
  assert.equal(result.input.kind, "existing_turn");
  assert.equal(result.input.event_id, "b-continue");
  assert.equal(result.checkpoint, "zz-summary", "summary ID may sort after current ID");
  assert.deepEqual(result.continuation, { kind: "compression", from_turn_id: "z-A", to_checkpoint: "zz-summary" });
  assert.equal(context(await f.headers(input("a-B"))).checkpoint, undefined, "a future summary cannot enrich queued B");
});

test("OpenCode exact parent wins over queued users, other summaries and model-context order", async () => {
  const entries = [user("A", 10), user("parent-A", 20, [{ type: "compaction" }]), user("B", 30),
    summary("summary-A", 40, "parent-A"), continuation("current", 50),
    user("parent-B", 60, [{ type: "compaction" }]), summary("future-summary", 70, "parent-B")];
  const f = await fixture(entries);
  assert.equal(context(await f.headers()).continuation.from_turn_id, "A");
  const transformed = [entries[1], entries[3], entries[0], entries[2], entries[4]];
  const changed = await fixture(transformed);
  assert.deepEqual(context(await changed.headers()), minimal(), "transformed model array cannot masquerade as ordered SDK page");
  entries[3].info.parentID = "missing-parent";
  const missing = context(await f.headers());
  assert.equal(missing.input.kind, "existing_turn");
  assert.equal(missing.continuation, undefined);
  assert.equal(missing.checkpoint, undefined);
});

for (const defect of ["error", "unfinished", "missing finish", "wrong parent", "source outside page", "parent outside page"]) {
  test(`OpenCode ${defect} loses source proof while retaining typed continuation guard`, async () => {
    const entries = [user("A", 10), user("parent", 20, [{ type: "compaction" }]),
      summary("summary", 30, "parent"), continuation("current", 40)];
    if (defect === "error") entries[2].info.error = { name: "APIError" };
    if (defect === "unfinished") delete entries[2].info.time.completed;
    if (defect === "missing finish") delete entries[2].info.finish;
    if (defect === "wrong parent") entries[2].info.parentID = "foreign";
    if (defect === "source outside page") entries.shift();
    if (defect === "parent outside page") entries.splice(0, 2);
    const f = await fixture(entries);
    const result = context(await f.headers());
    assert.equal(result.input.kind, "existing_turn");
    assert.equal(result.turn_id, "current");
    assert.equal(result.continuation, undefined);
  });
}

test("OpenCode exact current lookup rejects duplicate, oversized, cross-session and malformed pages", async () => {
  for (const entries of [[], [user("other", 1)], [user("current", 1), user("current", 2)],
    [user("current", 1, [text()], "foreign-session")], [user("current", NaN)],
    Array.from({ length: 129 }, (_, i) => user(i === 128 ? "current" : `other-${i}`, i))]) {
    const f = await fixture(entries);
    assert.deepEqual(context(await f.headers()), minimal());
    assert.equal(f.calls.length, 1, "no retry or pagination loop");
  }
});

test("OpenCode unknown, copied overflow, multipart and late child input remain observed", async () => {
  for (const parts of [[], [text("synthetic child", { synthetic: true })],
    [text("marker string", { synthetic: true, metadata: { compaction_continue: "true" } })],
    [text("ignored", { ignored: true })], [{ type: "unknown" }],
    [text(), { type: "file", mime: "image/png" }], [text("first"), text("second")],
    [text("marker", { synthetic: true, metadata: { compaction_continue: true } }), text("ordinary")]]) {
    const entry = user("current", 1, parts);
    const before = structuredClone(entry);
    const f = await fixture([entry]);
    await f.created(entry);
    assert.deepEqual(context(await f.headers()), minimal());
    assert.deepEqual(entry, before, "public hooks never write transcript markers");
  }
});

test("OpenCode callback cache only vetoes incompatible metadata and cold/evicted callbacks keep routing", async () => {
  const entry = user("current", 1);
  const f = await fixture([entry]);
  await f.created(entry);
  entry.parts = continuation("current", 1).parts;
  assert.equal(context(await f.headers()).input.kind, "observed_batch");
  const cold = await fixture([entry]);
  assert.equal(context(await cold.headers()).input.kind, "existing_turn");
  for (let i = 0; i < 257; ++i) await f.created(user(`prompt-${i}`, i));
  assert.equal(context(await f.headers()).input.kind, "existing_turn");
  entry.parts = [text()];
  assert.deepEqual(context(await f.headers()), minimal(), "cold/evicted ordinary calls retain fresh A/B availability");
});

test("OpenCode actual child/fork identity stays independent of parent/cache affinity", async () => {
  for (const session of ["parent", "child", "fork"]) {
    const f = await fixture([user("same-text-distinct-event", 1, [text()], session)]);
    const output = await f.headers(input("same-text-distinct-event", "build", session), {
      headers: { "x-opencode-parent-session-id": "parent", "x-session-affinity": "parent" },
    });
    assert.equal(context(output).session_id, session);
    assert.equal(output.headers["x-opencode-parent-session-id"], "parent");
    assert.equal(output.headers["x-session-affinity"], "parent");
  }
});
