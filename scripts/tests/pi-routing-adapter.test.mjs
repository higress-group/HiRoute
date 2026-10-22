import assert from "node:assert/strict";
import { test } from "node:test";
import piRouting from "../../assets/agents/pi/hiroute-routing.mjs";

const user = id => ({ type: "message", id, message: { role: "user", content: "repeated input" } });
function fixture(api = "openai-responses") {
  const hooks = new Map(), calls = [], registrations = [];
  const model = { provider: "hiroute-main-test", api };
  let branch = [user("user-1")], session = "session";
  const providers = new Map(), providerConfigs = new Map();
  const original = { tag: "native", streamSimple(model, transcript, options) {
    const result = { model, transcript, options, receiver: this };
    calls.push(result); return result;
  } };
  providers.set(model.provider, original);
  const ctx = { model, modelRegistry: {
    getAll: () => [model], getProvider: name => providers.get(name),
    getRegisteredProviderConfig: name => providerConfigs.get(name),
  }, sessionManager: {
    getSessionId: () => session, getBranch: () => branch,
    buildSessionProjection: () => ({ entries: branch.map(sourceEntry => ({ sourceEntry,
      messages: sourceEntry.type === "message" ? [sourceEntry.message] : [{ role: "compactionSummary" }],
    })) }),
  } };
  const pi = { on(name, callback) { hooks.set(name, callback); },
    registerProvider(name, config) {
      registrations.push({ name, config }); providerConfigs.set(name, config);
      providers.set(name, { ...providers.get(name), ...config });
    },
  };
  const emit = (name, event = {}) => hooks.get(name)?.(event, ctx);
  const call = (options = {}, callModel = model, transcript = { messages: [] }) =>
    providers.get(model.provider).streamSimple(callModel, transcript,
      { sessionId: session, signal: new AbortController().signal, ...options });
  const context = result => JSON.parse(result.options.headers["x-hiroute-routing-context"]);
  piRouting(pi); emit("session_start");
  return { pi, ctx, hooks, calls, registrations, providers, providerConfigs, model, original, emit, call, context,
    set branch(value) { branch = value; }, get branch() { return branch; },
    set session(value) { session = value; }, get session() { return session; } };
}

for (const api of ["openai-responses", "anthropic-messages"]) {
  test(`Pi delegates ${api} once, retaining native options and headers at the provider seam`, () => {
    const f = fixture(api), signal = new AbortController().signal;
    const options = { signal, sessionId: "session", apiKey: "synthetic", headers: { existing: "keep" },
      onPayload() {}, onResponse() {}, transformHeaders() {}, maxRetries: 3 };
    const transcript = { messages: [{ role: "user", content: "original payload" }] };
    const result = f.call(options, f.model, transcript);
    assert.equal(f.calls.length, 1); assert.equal(result.receiver, f.original);
    assert.equal(result.transcript, transcript); assert.equal(result.model, f.model);
    for (const key of ["signal", "apiKey", "onPayload", "onResponse", "transformHeaders", "maxRetries"])
      assert.equal(result.options[key], options[key]);
    assert.deepEqual(options.headers, { existing: "keep" });
    assert.equal(result.options.headers.existing, "keep");
    assert.deepEqual(f.context(result), { version: 2, agent: "pi", purpose: "main", session_id: "session",
      turn_id: "user-1", branch: f.context(result).branch, input: { kind: "observed_batch", event_id: "user-1" } });
    assert.equal(result.options.headers["x-session-affinity"], "session");
    assert.equal(result.options.headers["x-client-request-id"], "session");
    assert.equal(f.hooks.has("before_provider_headers"), false);
  });
}

test("Pi compaction signal survives serial calls, retries and elapsed old TTL with no terminal callbacks", () => {
  const f = fixture(), compact = new AbortController(), realNow = Date.now;
  try {
    f.emit("session_before_compact", { signal: compact.signal, branchEntries: f.branch });
    assert.equal(f.context(f.call()).purpose, "main", "concurrent main has its own signal");
    Date.now = () => realNow() + 900_000;
    for (let i = 0; i < 3; ++i) {
      const result = f.context(f.call({ signal: compact.signal, sessionId: "native-summary-random-id" }));
      assert.equal(result.purpose, "compaction"); assert.equal(result.session_id, "session");
    }
    compact.abort();
    assert.equal(f.context(f.call({ signal: compact.signal })).purpose, "compaction");
    assert.equal(f.context(f.call()).purpose, "main");
    assert.equal(f.hooks.has("session_compact"), false); assert.equal(f.hooks.has("session_compact_failed"), false);
  } finally { Date.now = realNow; }
});

test("Pi tree summaries mark only their requested operation signal, without phase cleanup", () => {
  const f = fixture();
  for (const preparation of [{ userWantsSummary: false, entriesToSummarize: f.branch },
    { userWantsSummary: true, entriesToSummarize: [] }]) {
    const signal = new AbortController().signal;
    f.emit("session_before_tree", { preparation, signal });
    assert.equal(f.context(f.call({ signal })).purpose, "main");
  }
  const signal = new AbortController().signal;
  f.emit("session_before_tree", { preparation: { userWantsSummary: true, entriesToSummarize: f.branch }, signal });
  assert.equal(f.context(f.call({ signal })).purpose, "summary");
  // Extension-provided/cancelled summaries make no call; neither poisons normal requests.
  assert.equal(f.context(f.call()).purpose, "main");
  assert.equal(f.context(f.call({ signal })).purpose, "summary");
});

test("Pi refuses a different SDK session and never uses missing signal as main provenance", () => {
  const f = fixture();
  const wrong = f.call({ sessionId: "another-session", headers: { native: "keep" } });
  assert.deepEqual(wrong.options.headers, { native: "keep" });
  assert.equal(f.call({ sessionId: undefined }).options.headers, undefined);
  assert.equal(f.context(f.call({ signal: undefined })).purpose, "unknown");
});

test("Pi snapshots ordinary and summary cursors before asynchronous execution and navigation", async () => {
  const f = fixture(), signal = new AbortController().signal;
  f.emit("session_before_compact", { signal, branchEntries: f.branch });
  const main = f.call();
  f.branch.push({ type: "message", id: "assistant-branch-2", message: { role: "assistant" } });
  f.session = "replacement-session";
  f.emit("session_start");
  assert.equal(f.context(main).session_id, "session");
  assert.equal(f.context(main).branch.leaf_id, "user-1");
  const summary = f.context(f.call({ signal }));
  assert.equal(summary.session_id, "session"); assert.equal(summary.branch.leaf_id, "user-1");
  const next = f.context(f.call());
  assert.equal(next.session_id, "replacement-session"); assert.equal(next.branch.leaf_id, "assistant-branch-2");
});

test("Pi branch leaf and ordered path distinguish shared users and repeated text", () => {
  const f = fixture();
  f.branch = [user("same-user"), { type: "message", id: "left", message: { role: "assistant" } }];
  const left = f.context(f.call());
  f.branch = [user("same-user"), { type: "message", id: "right", message: { role: "assistant" } }];
  const right = f.context(f.call());
  assert.equal(left.turn_id, right.turn_id);
  assert.notEqual(left.branch.leaf_id, right.branch.leaf_id);
  assert.notEqual(left.branch.path_digest, right.branch.path_digest);
  f.branch.reverse(); assert.notEqual(f.context(f.call()).branch.path_digest, right.branch.path_digest);
  f.branch = [user("same-text-new-instance")];
  assert.notEqual(f.context(f.call()).turn_id, left.turn_id);
});

test("Pi retains projection tri-state as observed cursor evidence through compaction and context edits", () => {
  const f = fixture();
  assert.equal(f.context(f.call()).input.event_id, "user-1");
  f.branch.push({ type: "compaction", id: "checkpoint" });
  assert.deepEqual(f.context(f.call()).continuation, { kind: "compression", from_checkpoint: "checkpoint" });
  assert.deepEqual(f.context(f.call()).input, { kind: "observed_batch" });
  f.branch.push(user("user-2"));
  assert.equal(f.context(f.call()).continuation, undefined);
  assert.equal(f.context(f.call()).input.event_id, "user-2");
  f.branch.push({ type: "context_edit", id: "edit", targetId: "user-2" });
  assert.deepEqual(f.context(f.call()).input, { kind: "observed_batch" });
  f.branch = [user("user-2")];
  for (const projection of [undefined, { entries: [] }, { entries: [{ sourceEntry: f.branch[0], messages: [{ role: "user", synthetic: true }] }] },
    { entries: [{ sourceEntry: f.branch[0], messages: [f.branch[0].message] }, { sourceEntry: { type: "custom" }, messages: [{ role: "user" }] }] }]) {
    f.ctx.sessionManager.buildSessionProjection = () => projection;
    const metadata = f.context(f.call());
    assert.deepEqual(metadata.input, { kind: "observed_batch" });
    assert.equal(metadata.input.kind === "native_messages", false);
  }
});

test("Pi provider refresh, model selection, repeated setup and native disposal do not stack delegates", () => {
  const f = fixture();
  for (const event of ["session_start", "context", "model_select"]) f.emit(event);
  assert.equal(f.registrations.length, 1);
  piRouting(f.pi); f.emit("session_start");
  f.call(); assert.equal(f.calls.length, 1); assert.equal(f.calls[0].receiver, f.original);
  const replacement = { streamSimple(_m, _t, options) { return { options, receiver: this }; } };
  f.providers.set(f.model.provider, replacement); f.providerConfigs.delete(f.model.provider); f.emit("model_select");
  const result = f.call(); assert.equal(result.receiver, replacement); assert.equal(f.context(result).purpose, "main");
  f.emit("session_shutdown", { reason: "reload" });
  assert.equal(f.call().options.headers, undefined);
  // Pi unregisters extension providers on dispose; emulate its original registration.
  f.providers.set(f.model.provider, f.original); f.providerConfigs.delete(f.model.provider); piRouting(f.pi); f.emit("session_start");
  assert.equal(f.context(f.call()).purpose, "main"); assert.equal(f.calls.length, 2);
});

test("Pi ignores unsupported/unmanaged providers while preserving managed cache suppression", () => {
  const f = fixture("unsupported-api");
  assert.equal(f.registrations.length, 0); assert.equal(f.call().options.headers, undefined);
  f.model.api = "openai-responses"; f.model.provider = "foreign"; f.emit("session_start");
  assert.equal(f.registrations.length, 0);
  assert.equal(f.emit("cache_warming_decision", { model: f.model }), undefined);
  f.model.provider = "hiroute-worker-test";
  assert.deepEqual(f.emit("cache_warming_decision", { model: f.model }), { action: "stop" });
});

test("Pi optional failures retain known summary purpose and recover without replaying a provider", () => {
  const f = fixture(), signal = new AbortController().signal;
  const getSession = f.ctx.sessionManager.getSessionId, getBranch = f.ctx.sessionManager.getBranch;
  f.ctx.sessionManager.getSessionId = () => { throw Error("optional native session failure"); };
  f.ctx.sessionManager.getBranch = () => { throw Error("optional branch failure"); };
  f.emit("session_before_compact", { signal });
  assert.deepEqual(f.context(f.call({ signal })), { version: 2, agent: "pi", purpose: "compaction" });
  const calls = f.calls.length;
  assert.equal(f.call().options.headers, undefined); assert.equal(f.calls.length, calls + 1);
  f.ctx.sessionManager.getSessionId = getSession; f.ctx.sessionManager.getBranch = getBranch;
  assert.equal(f.context(f.call()).purpose, "main");
  f.ctx.sessionManager.buildSessionProjection = () => { throw Error("projection failure"); };
  assert.deepEqual(f.context(f.call()).input, { kind: "observed_batch" });
});

test("Pi preserves native auth/provider/cancellation exceptions without retrying the delegate", () => {
  for (const message of ["auth", "provider", "cancel"]) {
    const f = fixture(), failure = Error(message); let count = 0;
    f.providers.set(f.model.provider, { streamSimple() { count++; throw failure; } });
    f.providerConfigs.delete(f.model.provider); f.emit("context");
    assert.throws(() => f.call(), error => error === failure); assert.equal(count, 1);
  }
});

test("Pi registration and option enrichment failures preserve single execution and later recovery", () => {
  const f = fixture();
  f.providers.set(f.model.provider, f.original);
  f.providerConfigs.delete(f.model.provider);
  const register = f.pi.registerProvider;
  f.pi.registerProvider = () => { throw Error("registration unavailable"); };
  f.emit("session_start");
  assert.equal(f.call().options.headers, undefined); assert.equal(f.calls.length, 1);
  f.pi.registerProvider = register; f.emit("context");
  const options = { sessionId: "session", signal: new AbortController().signal,
    get headers() { throw Error("optional header read failed"); } };
  const result = f.providers.get(f.model.provider).streamSimple(f.model, {}, options);
  assert.equal(result.options, options); assert.equal(f.calls.length, 2);
  assert.equal(f.context(f.call()).purpose, "main"); assert.equal(f.calls.length, 3);
});

test("Pi concurrent calls keep independent signals and foreign model invocations stay untouched", () => {
  const f = fixture(), summary = new AbortController().signal;
  f.emit("session_before_compact", { signal: summary, branchEntries: f.branch });
  const earlier = f.call();
  f.branch.push(user("new-user"));
  const later = f.call(), auxiliary = f.call({ signal: summary });
  assert.equal(f.context(earlier).turn_id, "user-1");
  assert.equal(f.context(later).turn_id, "new-user");
  assert.equal(f.context(auxiliary).turn_id, "user-1");
  assert.equal(f.context(auxiliary).purpose, "compaction");
  assert.equal(f.call({}, { ...f.model, provider: "hiroute-main-another" }).options.headers, undefined);
  f.emit("session_before_compact", { signal: {}, branchEntries: f.branch });
  assert.equal(f.context(f.call({ signal: {} })).purpose, "unknown");
});

test("Pi registry recomposition keeps one owned registration rather than stacking facade wrappers", () => {
  const f = fixture(), registration = f.registrations[0].config;
  for (let i = 0; i < 20; i++) {
    f.providers.set(f.model.provider, { streamSimple(...args) { return registration.streamSimple(...args); } });
    f.emit("context");
  }
  assert.equal(f.registrations.length, 1);
  assert.equal(f.context(f.call()).purpose, "main"); assert.equal(f.calls.length, 1);
  let replacementCalls = 0;
  const replacement = (_model, _transcript, options) => { replacementCalls++; return { options }; };
  f.providerConfigs.set(f.model.provider, { streamSimple: replacement });
  f.providers.set(f.model.provider, { streamSimple(...args) { return replacement(...args); } });
  f.emit("context"); f.emit("context");
  assert.equal(f.context(f.call()).purpose, "main");
  assert.equal(replacementCalls, 1); assert.equal(f.calls.length, 1, "replacement must not revive the old delegate");
  assert.equal(f.registrations.length, 2);
});

test("Pi missing public registration identity leaves native calls usable with an explicit capability miss", () => {
  const f = fixture();
  f.providers.set(f.model.provider, f.original); f.providerConfigs.clear();
  delete f.ctx.modelRegistry.getRegisteredProviderConfig;
  f.emit("session_start"); f.emit("context");
  assert.equal(f.call().options.headers, undefined); assert.equal(f.calls.length, 1);
  assert.equal(f.registrations.length, 1, "uninspectable providers must not be wrapped speculatively");
});
