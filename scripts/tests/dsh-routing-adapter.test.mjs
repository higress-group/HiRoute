import assert from "node:assert/strict";
import { test } from "node:test";
import { createHash } from "node:crypto";
import { apply } from "../../assets/agents/dsh/hiroute-routing.mjs";

const symbol = Symbol.for("hiroute.dsh.routing-context.v2");
const hash = value => createHash("sha256").update(JSON.stringify(value)).digest("hex");
const text = value => ({ type: "text", text: value });
const user = (id, value, kind = "user") => ({ id, role: "user", source: { kind },
  content: typeof value === "string" ? [text(value)] : value });
const turn = number => ({ type: "turn/start", data: { turn: number } });
const event = message => ({ type: "user/message", data: message });
const managed = { "x-hiroute-agent": "dsh", "x-hiroute-token": "synthetic-grant" };
const metadata = init => {
  const value = new Headers(init?.headers).get("x-hiroute-routing-context");
  return value ? JSON.parse(value) : undefined;
};

async function fixture(run, options = {}) {
  const original = globalThis.fetch;
  const sent = [];
  const fetch = options.fetch ?? function (input, init) {
    sent.push({ input, init, receiver: this });
    return Promise.resolve("native-result");
  };
  globalThis.fetch = fetch;
  delete globalThis[symbol];
  const events = options.events ?? [];
  let hook;
  const ctx = { sessions: { get: () => ({ get seq() { return events.length; },
    eventAt: index => events[index], header: { parentSession: "parent" } }) },
    on: (_name, value, registration = {}) => {
      // Cordis filters service events by the dispatching LLM's scope. Web
      // session services may be isolated from the root that loads this plugin.
      hook = options.isolatedLlm && !registration.global
        ? (_request, next) => next()
        : value;
    } };
  apply(ctx);
  const request = (messages = [], extra = {}) => ({ provider: "hiroute-main-fixture", sessionId: "session", messages, ...extra });
  const send = async (native, wire, protocol = "responses", extra = {}) => {
    const body = typeof wire === "string" ? wire : JSON.stringify({ [protocol === "responses" ? "input" : "messages"]: wire });
    const init = { method: "POST", body, headers: managed, ...extra };
    const stream = hook(native, async function* () {
      yield await globalThis.fetch("http://127.0.0.1/v1/" + protocol, init);
    });
    assert.equal((await stream.next()).value, "native-result");
    assert.equal(sent.at(-1).init.body, body, "adapter changed actual execution bytes");
    return metadata(sent.at(-1).init);
  };
  try { await run({ ctx, events, hook, request, send, sent }); }
  finally { globalThis.fetch = original; delete globalThis[symbol]; }
}

test("DSH managed main and auxiliary calls retain metadata in an isolated LLM scope", async () => {
  const current = user("current", "CURRENT");
  await fixture(async ({ request, send }) => {
    for (const purpose of [undefined, "session-title", "compaction"]) {
      const context = await send(request([current], { purpose }), [{ role: "user", content: "CURRENT" }]);
      assert.equal(context?.session_id, "session");
      assert.equal(context?.purpose, purpose ?? "main");
      if (purpose === undefined) assert.deepEqual(context.input.messages, [{ index: 0, sha256: hash(["CURRENT"]) }]);
      else assert.equal(context.input, undefined);
    }
    assert.equal(await send(request([current], { provider: "foreign" }), [{ role: "user", content: "CURRENT" }]), undefined,
      "an isolated foreign provider acquired managed routing evidence");
  }, { isolatedLlm: true, events: [turn(1), event(current)] });
});

test("DSH Desktop 0.2.1-rc.3 keeps its real input before native skill and time context", async () => {
  const current = user("current", "Inspect the export service");
  const context = [user("runtime", "WORKSPACE", "runtime-context"),
    user("skills", "AVAILABLE SKILLS", "skill-catalog"), user("clock", "CURRENT TIME", "time-context")];
  await fixture(async ({ request, send }) => {
    const messages = [{ role: "system", content: [text("SYSTEM")] }, current, ...context];
    const wire = messages.slice(1).map(message => ({ role: "user", content: message.content }));
    for (let retry = 0; retry < 2; ++retry) {
      const observed = await send(request(messages), wire);
      assert.deepEqual(observed.input?.messages, [{ index: 0, sha256: hash(["Inspect the export service"]) }]);
      assert.equal(observed.input.scope_digest, hash(wire.map(message => hash(message.content.map(p => p.text)))));
    }
    const unknown = user("unknown", "LATE NOTICE", "future-context");
    assert.equal((await send(request([...messages, unknown]), [...wire, { role: "user", content: "LATE NOTICE" }])).input, undefined);
  }, { isolatedLlm: true, events: [turn(1), event(current), ...context.map(event)] });
});

test("DSH current and older instruction-role projections require the complete wire User scope", async () => {
  const current = user("current", "CURRENT");
  await fixture(async ({ request, send }) => {
    const instruction = { role: "developer", content: [text("UPDATE")] };
    const messages = [current, instruction];
    const modern = [{ role: "user", content: "CURRENT" }, { role: "developer", content: "UPDATE" }];
    const legacy = [{ role: "user", content: "CURRENT" }, { role: "user", content: "UPDATE" }];
    for (const wire of [modern, legacy]) {
      const proof = (await send(request(messages), wire)).input;
      assert.deepEqual(proof.messages, [{ index: 0, sha256: hash(["CURRENT"]) }]);
      assert.equal((await send(request(messages), [...wire, { role: "user", content: "UNKNOWN" }])).input, undefined);
    }
  }, { events: [turn(1), event(current)] });
});

test("DSH v2 purpose/session survive every optional lookup and later recover", async () => {
  const current = user("current", "CURRENT");
  await fixture(async ({ ctx, hook, request, send, sent }) => {
    const healthy = ctx.sessions.get;
    for (const purpose of [undefined, "compaction", "session-title"]) {
      for (const fault of ["get", "seq", "eventAt", "parent", "checkpoint"]) {
        const fail = () => { throw Error("injected optional " + fault); };
        ctx.sessions.get = fault === "get" ? fail : () => ({
          get seq() { return fault === "seq" ? fail() : 2; },
          eventAt: fault === "eventAt" ? fail : i => [turn(1), event(current)][i],
          get header() { return fault === "parent" ? fail() : {}; },
        });
        const native = request([current], { purpose });
        if (fault === "checkpoint") native.messages = [current,
          { role: "user", content: [text("SUMMARY")], source: { kind: "compact-checkpoint", get compactionId() { return fail(); } } }];
        const wire = native.messages.map(message => ({ role: message.role, content: message.content }));
        const context = await send(native, wire);
        assert.equal(context.version, 2);
        assert.equal(context.session_id, "session");
        assert.equal(context.purpose, purpose ?? "main");
        assert.equal(new Headers(sent.at(-1).init.headers).get("x-deepseek-harness-session-id"), "session");
      }
    }
    ctx.sessions.get = healthy;
    assert.equal((await send(request([current]), [{ role: "user", content: "CURRENT" }])).input.kind, "native_messages");
    assert.equal((await send(request([current]), "{malformed-json")).purpose, "main", "bad optional wire proof erased minimum context");
    const knownWithoutSession = request([], { purpose: "compaction", sessionId: undefined });
    assert.deepEqual(await send(knownWithoutSession, []), { version: 2, agent: "dsh", purpose: "compaction" });
    const unknown = await send(request([current], { purpose: "future-kind" }), []);
    assert.equal(unknown.purpose, "unknown");
    assert.equal(unknown.input, undefined);
    const stream = hook(request([], { provider: "foreign" }), async function* () {
      yield await fetch("http://foreign.invalid", { headers: managed });
    });
    await stream.next();
    assert.equal(metadata(sent.at(-1).init), undefined);
  }, { events: [turn(1), event(current)] });
});

test("DSH window binds exact current activation and complete final User scope on both protocols", async () => {
  const first = user("first", [text("CHILD-TASK"), text("RETURN-GUIDANCE")]);
  const second = user("second", "SECOND");
  const policy = user("policy", "POLICY", "runtime-context");
  const initial = [{ role: "system", content: [text("SYSTEM")] }, first, policy, second];
  await fixture(async ({ request, send, sent, events }) => {
    for (const protocol of ["responses", "messages"]) {
      const envelope = value => ({ role: "user", content: protocol === "responses" ? [{ type: "input_text", text: value }] : value });
      const wire = [envelope("CHILD-TASKRETURN-GUIDANCE"), envelope("POLICY"), envelope("SECOND")];
      const context = await send(request(initial), wire, protocol);
      const digests = [hash(["CHILD-TASKRETURN-GUIDANCE"]), hash(["POLICY"]), hash(["SECOND"])];
      assert.deepEqual(context.input, { kind: "native_messages", event_id: "1", coverage: "window",
        messages: [{ index: 0, sha256: digests[0] }, { index: 2, sha256: digests[2] }], scope_digest: hash(digests) });
      assert.equal(context.user_turns, undefined);
      assert.equal(context.users, undefined);
      assert.equal(new Headers(sent.at(-1).init.headers).get("x-deepseek-harness-parent-session-id"), "parent");
      for (const changed of [
        [wire[2], wire[1], wire[0]],
        [envelope("CHILD-TASKCHANGED"), ...wire.slice(1)],
        [envelope("CHILD-TASKRETURN-GUIDANCEPOLICYSECOND")],
        [{ role: "user", content: [text("CHILD-TASK"), text("RETURN-GUIDANCE")] }, ...wire.slice(1)],
        [...wire, { role: "user", content: [text("MEDIA"), { type: "image", source: {} }] }],
        [{ role: "user", content: [{ ...text("CHILD-TASKRETURN-GUIDANCE"), provider_state: "opaque" }] }, ...wire.slice(1)],
        [{ ...wire[0], provider_state: "opaque" }, ...wire.slice(1)],
      ]) assert.equal((await send(request(initial), changed, protocol)).input, undefined);
      if (protocol === "messages") {
        const cached = [{ role: "user", content: [{ ...text("CHILD-TASKRETURN-GUIDANCE"), cache_control: { type: "ephemeral", ttl: "1h" } }] }, ...wire.slice(1)];
        assert.deepEqual((await send(request(initial), cached, protocol)).input, context.input);
      }
      const next = user("next", "NEXT");
      events.push(turn(2), event(next));
      const resumed = [...initial, { role: "assistant", content: [text("DONE")] }, next];
      const resumedWire = [...wire, { role: "assistant", content: "DONE" }, envelope("NEXT")];
      assert.deepEqual((await send(request(resumed), resumedWire, protocol)).input.messages,
        [{ index: 3, sha256: hash(["NEXT"]) }], "old User events acquired current-event positions");
      events.splice(-2);
    }
  }, { events: [turn(1), event(first), event(policy), event(second)] });
});

test("DSH recent window recovers beyond sixteen turns and 512 native events with repeated old input", async () => {
  const users = Array.from({ length: 70 }, (_, index) => user("u" + index, "REPEATED"));
  const events = users.flatMap((message, index) => [turn(index + 1), event(message),
    ...Array.from({ length: 8 }, () => ({ type: "assistant/stream", data: {} }))]);
  let reads = 0;
  await fixture(async ({ ctx, request, send }) => {
    ctx.sessions.get = () => ({ seq: events.length, eventAt: index => { ++reads; return events[index]; } });
    const wire = users.map(() => ({ role: "user", content: "REPEATED" }));
    const current = await send(request(users), wire);
    assert.equal(current.turn_id, "70");
    assert.deepEqual(current.input.messages, [{ index: 69, sha256: hash(["REPEATED"]) }]);
    assert.equal(current.input.scope_digest, hash(Array(70).fill(hash(["REPEATED"]))));
    assert.ok(reads <= 512);
    // The latest activation may itself exceed the scan. No earlier user wins.
    events.push(...Array.from({ length: 513 }, () => ({ type: "tool/complete", data: {} })));
    reads = 0;
    const miss = await send(request(users), wire);
    assert.equal(miss.input, undefined);
    assert.equal(miss.purpose, "main");
    assert.equal(reads, 512);
    const fresh = user("fresh", "FRESH");
    events.push(turn(71), event(fresh));
    const recovered = await send(request([...users, fresh]), [...wire, { role: "user", content: "FRESH" }]);
    assert.deepEqual(recovered.input.messages, [{ index: 70, sha256: hash(["FRESH"]) }]);
  }, { events });
});

test("DSH leaves model selection, late peers, settlement and unknown input observed, then a fresh turn recovers", async () => {
  const old = user("old", "OLD");
  const fresh = user("fresh", "NEW");
  await fixture(async ({ request, send, events }) => {
    for (const kind of ["model-selection", "agent-message", "subagent-settled", "future-source", undefined]) {
      const notice = { ...user("notice", "LATE-OLD-TASK", kind), source: kind ? { kind, senderSessionId: "reused-child" } : undefined };
      const messages = [old, fresh, notice];
      const wire = messages.map(message => ({ role: message.role, content: message.content }));
      const context = await send(request(messages), wire);
      assert.equal(context.input, undefined, "late source acquired latest task association: " + kind);
      assert.equal(context.turn_id, "2");
      const after = user("after", "AFTER");
      events.push(turn(3), event(after));
      const recovered = await send(request([...messages, after]), [...wire, { role: "user", content: "AFTER" }]);
      assert.deepEqual(recovered.input.messages, [{ index: 3, sha256: hash(["AFTER"]) }]);
      events.splice(-2);
    }
    const unknownPrefix = user("unknown-old", "OLD-UNKNOWN", "future-source");
    const context = await send(request([unknownPrefix, old, fresh]), [unknownPrefix, old, fresh]
      .map(message => ({ role: "user", content: message.content })));
    assert.deepEqual(context.input.messages, [{ index: 2, sha256: hash(["NEW"]) }], "old unproved input prevented a new exact window");
  }, { events: [turn(1), event(old), turn(2), event(fresh)] });
});

test("DSH accounts for later System and Developer Users plus Messages tool-only envelopes", async () => {
  const current = user("current", "TASK");
  await fixture(async ({ request, send }) => {
    const messages = [{ role: "system", content: [text("SYSTEM")] }, current,
      { role: "system", content: [text("LATER-SYSTEM")] }, { role: "developer", content: [text("LATER-DEVELOPER")] }];
    const wire = [{ role: "user", content: "TASK" },
      { role: "user", content: [{ type: "tool_result", tool_use_id: "tool", content: "RESULT" }] },
      { role: "user", content: "LATER-SYSTEM" }, { role: "user", content: "LATER-DEVELOPER" }];
    const context = await send(request(messages), wire, "messages");
    assert.equal(context.input.scope_digest, hash(["TASK", "LATER-SYSTEM", "LATER-DEVELOPER"].map(value => hash([value]))));
    assert.deepEqual(context.input.messages, [{ index: 0, sha256: hash(["TASK"]) }]);
    const explicit = await send(request(messages, { system: "EXPLICIT" }), [{ role: "user", content: "SYSTEM" }, ...wire], "messages");
    assert.deepEqual(explicit.input.messages, [{ index: 1, sha256: hash(["TASK"]) }]);
    assert.equal((await send(request(messages), [wire[0],
      { role: "user", content: [text("TEXT"), { type: "tool_result", content: "RESULT" }] }, ...wire.slice(2)], "messages")).input, undefined);
  }, { events: [turn(1), event(current)] });
});

test("DSH rejects ambiguous IDs, unsupported parts and bounds without discarding minimum purpose", async () => {
  const current = user("current", "TASK");
  await fixture(async ({ request, send, events }) => {
    for (const native of [
      [current, current],
      [{ ...current, content: [text("CHANGED")] }],
      [{ ...current, content: [text("TASK"), { type: "image", attachment: {} }] }],
      Array.from({ length: 129 }, (_, index) => index === 128 ? current : user("old" + index, "OLD")),
      [user("large", "x".repeat(2 * 1024 * 1024)), current],
    ]) {
      const context = await send(request(native), native.map(message => ({ role: message.role, content: message.content })));
      assert.equal(context.input, undefined);
      assert.equal(context.purpose, "main");
    }
    events.push(event(current));
    assert.equal((await send(request([current]), [{ role: "user", content: "TASK" }])).input, undefined);
    events.pop();
    for (const invalid of [undefined, null, {}, [], "1", true, 0, -1, 1.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]) {
      events[0] = turn(invalid);
      const unknown = await send(request([current]), [{ role: "user", content: "TASK" }]);
      assert.equal(unknown.turn_id, undefined);
      assert.equal(unknown.input, undefined);
    }
    events[0] = turn(2);
    assert.equal((await send(request([current]), [{ role: "user", content: "TASK" }])).input.kind, "native_messages");
    const many = Array.from({ length: 33 }, (_, index) => user("u" + index, "TASK"));
    events.splice(0, events.length, turn(3), ...many.map(event));
    assert.equal((await send(request(many), many.map(message => ({ role: "user", content: "TASK" })))).input, undefined);
  }, { events: [turn(1), event(current)] });
});

test("DSH immutable snapshots isolate interleaved and nested streams, iterator factories, return and throw", async () => {
  const current = user("current", "TASK");
  await fixture(async ({ request, hook, sent, events }) => {
    const fetchCurrent = async label => {
      const value = await fetch("http://127.0.0.1/v1/responses", { headers: managed, body: label });
      return value;
    };
    const iterator = label => ({
      [Symbol.asyncIterator]() { fetchCurrent(label + "-factory"); return this; },
      async next() { await Promise.resolve(); return { done: false, value: await fetchCurrent(label + "-next") }; },
      async return(value) { await fetchCurrent(label + "-return"); return { done: true, value }; },
      async throw(error) { await fetchCurrent(label + "-throw"); throw error; },
    });
    const a = request([current], { sessionId: "A" });
    const streamA = hook(a, () => iterator("A"));
    a.sessionId = "MUTATED";
    a.purpose = "compaction";
    events.push(turn(99));
    const streamB = hook(request([], { sessionId: "B", purpose: "session-title" }), () => iterator("B"));
    await Promise.all([streamA.next(), streamB.next()]);
    assert.deepEqual(await streamA.return("closed"), { done: true, value: "closed" });
    const failure = Error("native iterator failure");
    await assert.rejects(streamB.throw(failure), error => error === failure);
    for (const item of sent) {
      const name = item.init.body[0];
      const context = metadata(item.init);
      assert.equal(context.session_id, name);
      assert.equal(context.purpose, name === "A" ? "main" : "session-title");
      assert.equal(context.turn_id, name === "A" ? "1" : "99");
    }
    const outer = hook(request([], { sessionId: "OUTER" }), async function* () {
      const nested = hook(request([], { sessionId: "INNER" }), async function* () { yield await fetchCurrent("inner"); });
      const foreign = hook(request([], { provider: "foreign", sessionId: "FOREIGN" }), async function* () { yield await fetchCurrent("foreign"); });
      await nested.next();
      await foreign.next();
      yield await fetchCurrent("outer");
    });
    await outer.next();
    assert.equal(metadata(sent.at(-3).init).session_id, "INNER");
    assert.equal(metadata(sent.at(-2).init), undefined, "unmanaged nested stream inherited outer managed context");
    assert.equal(metadata(sent.at(-1).init).session_id, "OUTER");
    const bare = hook(request([], { sessionId: "BARE" }), () => ({
      [Symbol.asyncIterator]() { return this; }, next: async () => ({ done: true }),
    }));
    assert.deepEqual(await bare.return("value"), { done: true, value: "value" });
    await assert.rejects(bare.throw(failure), error => error === failure);
  }, { events: [turn(1), event(current)] });
});

test("DSH fetch preserves receiver, Request/body/signal/cancel and original provider failures", async () => {
  const observed = [];
  const receiver = {};
  const failure = Error("provider rejected");
  const response = { actual: "provider-response" };
  let fail = false;
  await fixture(async ({ hook, request }) => {
    const signal = new AbortController().signal;
    const body = new ReadableStream({ start(controller) { controller.enqueue(new Uint8Array([1])); controller.close(); } });
    const input = new Request("http://127.0.0.1/v1/responses", { method: "POST", body, duplex: "half", signal, headers: managed });
    const init = { signal, custom: "retained" };
    const native = hook(request([], { sessionId: "FETCH" }), async function* () {
      yield await globalThis.fetch.call(receiver, input, init);
    });
    assert.equal((await native.next()).value, response);
    assert.equal(observed[0].receiver, receiver);
    assert.equal(observed[0].input, input);
    assert.equal(observed[0].init.signal, signal);
    assert.equal(observed[0].init.custom, "retained");
    assert.equal(input.bodyUsed, false, "optional proof read or consumed native Request body");
    assert.equal(metadata(observed[0].init).session_id, "FETCH");
    fail = true;
    const broken = hook(request([]), async function* () { yield await fetch("http://127.0.0.1/v1/responses", { headers: managed }); });
    await assert.rejects(broken.next(), error => error === failure);
    fail = false;
    for (const headers of [{ "x-hiroute-agent": "foreign", authorization: "grant" }, { "x-hiroute-agent": "dsh" }]) {
      const foreign = hook(request([]), async function* () { yield await fetch("http://foreign.invalid", { headers }); });
      await foreign.next();
      assert.equal(metadata(observed.at(-1).init), undefined);
    }
  }, { fetch: function (input, init) {
    observed.push({ input, init, receiver: this });
    return fail ? Promise.reject(failure) : Promise.resolve(response);
  } });
});
