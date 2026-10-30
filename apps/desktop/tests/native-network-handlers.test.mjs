import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';
import { saveFailureDefinitelyPreAdmission } from '../src/features/subscriptions/model.ts';
import { manualProxyError, networkDraft, networkEdit, networkPolicyEqual } from '../src/features/model-connections/native-network.ts';

// Exercise the production IPC handlers with injected faults, without mounting a UI.
// TSX cannot be stripped by Node, so select the handler declarations before stripping.
function handlers(file, start, end) {
  const source = readFileSync(new URL('../src/features/' + file, import.meta.url), 'utf8');
  const first = source.indexOf('  ' + start), last = source.indexOf('  ' + end, first + 1);
  assert(first >= 0 && last > first, `Missing production handler: ${start}`);
  return stripTypeScriptTypes(source.slice(first, last));
}
function bind(source, dependencies, result) {
  return new Function(...Object.keys(dependencies), source + '\nreturn ' + result)(...Object.values(dependencies));
}
const editor = 'model-connections/NativeSourceNetworkEditor.tsx';
const editorHandlers = handlers(editor, 'async function finish(', 'async function retryConsumerRefresh(')
  + handlers(editor, 'async function apply()', 'function reload()');
const backendFailure = { source: 'backend', envelope: { error: { code: 'application.error.daemon_unavailable' }, operation: null } };

function sourceSave(failCommand) {
  const calls = [], submission = { current: null }, operation = { operation_id: 'operation/accepted', state: 'running', sequence: 1, cancellable: false };
  const state = { pending: false, error: '', busy: false, refreshed: 0 };
  const current = { source_id: 'source/a', policy: { mode: 'inherit' }, network_revision: 2 };
  let fail = true;
  const invoke = async (command, input) => {
    calls.push({ command, input });
    if (fail && command === failCommand) { fail = false; throw backendFailure; }
    if (command === 'apply_native_source_network') return { operation };
    if (command === 'get_compute_save_result') return { disposition: 'saved' };
    if (command === 'native_source_network') return current;
    throw Error('Unexpected IPC command: ' + command);
  };
  const actions = bind(editorHandlers, {
    invoke, submission, saveFailureDefinitelyPreAdmission, networkDraft,
    sourceId: 'source/a', language: 'en', zh: false, mutable: true,
    live: { current: true }, working: { current: false },
    preview: { preview: { accept_digest: 'digest/accepted', spec: {}, expected_revisions: {} } },
    clientIdempotencyKey: () => 'idempotency/original', safeConnectionErrorCode: () => 'DAEMON_UNAVAILABLE',
    setBusy: value => { state.busy = value; }, setPending: value => { state.pending = value; },
    setError: value => { state.error = value; }, setView: value => { state.view = value; },
    setDraft: value => { state.draft = value; }, setPreview: () => {}, setSaved: () => {},
    setConsumerRefreshFailed: value => { state.consumerRefreshFailed = value; },
    onSaved: async () => { state.refreshed++; },
  }, '{ apply, refreshPending }');
  return { ...actions, state, calls, submission, operation, current };
}

for (const failedRead of ['get_compute_save_result', 'native_source_network']) {
  test(`a failed ${failedRead} retains the accepted operation and refresh never reapplies`, async () => {
    const flow = sourceSave(failedRead);
    await flow.apply();
    assert.equal(flow.submission.current.operation, flow.operation);
    assert.equal(flow.submission.current.key, 'idempotency/original');
    assert.equal(flow.submission.current.digest, 'digest/accepted');
    assert.equal(flow.state.pending, true);
    assert.equal(flow.state.busy, false);
    assert.match(flow.state.error, /not confirmed/);
    assert.doesNotMatch(flow.state.error, /not submitted/);
    await flow.apply();
    await flow.refreshPending();
    assert.equal(flow.calls.filter(call => call.command === 'apply_native_source_network').length, 1);
    assert(flow.calls.filter(call => call.command === 'get_compute_save_result').every(call => call.input.operation === flow.operation));
    assert.equal(flow.submission.current, null);
    assert.equal(flow.state.pending, false);
    assert.equal(flow.state.view, flow.current);
    assert.equal(flow.state.draft.expected_network_revision, 2);
    assert.equal(flow.state.refreshed, 1);
  });
}

test('a rejected dispatch remains safely retryable without pretending it was admitted', async () => {
  const flow = sourceSave('apply_native_source_network');
  await flow.apply();
  assert.equal(flow.submission.current, null);
  assert.equal(flow.state.pending, false);
  assert.match(flow.state.error, /not submitted/);
  assert.deepEqual(flow.calls.map(call => call.command), ['apply_native_source_network']);
});

const runCheck = handlers('model-connections/ModelConnectionForm.tsx', 'async function runCheck(', 'function cancelActiveCheck()');
for (const [name, ready, network] of [
  ['unreadable saved settings', false, networkDraft()],
  ['empty manual proxy', true, { ...networkDraft(), mode: 'manual' }],
  ['invalid manual proxy', true, { ...networkDraft(), mode: 'manual', proxy_url: 'socks5://proxy.invalid' }],
]) {
  test(`manual-model checking returns to the editable connection for ${name}`, async () => {
    const state = { screen: 'manual', reveal: 0 };
    const value = { display_name: 'Connection', models: [{ upstream_model_id: 'model-one' }], network };
    const input = { value: 'retained-key' }, draftRef = { current: value };
    const check = bind(runCheck, {
      phase: 'editing', props: { mutable: true }, custom: true, draftRef,
      registeredCandidates: [], preservedModelIds: { current: new Set(['model-one']) },
      withCapabilityFallback: value => value, manualModelIdError: () => null,
      password: { current: input }, networkReady: ready, networkDraft, manualProxyError,
      setError: value => { state.error = value; }, setScreen: value => { state.screen = value; },
      setNetworkReveal: update => { state.reveal = update(state.reveal); },
      backend: new Proxy({}, { get() { throw Error('An invalid network must not issue provider or credential calls'); } }),
    }, 'runCheck');
    await check(true);
    assert.equal(state.screen, 'connection');
    assert.equal(state.reveal, 1);
    assert.equal(draftRef.current, value);
    assert.equal(input.value, 'retained-key');
  });
}

const decisions = 'decision-services/DecisionServicesPage.tsx';
const decisionSave = handlers(decisions, 'async function save(', 'async function testSelected()')
  + handlers(decisions, 'function networkChanged(', 'const selected =');
const inheritedChange = handlers('decision-services/InheritedSourceNetwork.tsx', 'function networkChanged(', 'useEffect(');
const editorChange = handlers(editor, 'function changeDraft(', 'useEffect(');

for (const editDuringTest of [false, true]) {
  test(`decision save/test ${editDuringTest ? 'retains a newer network edit' : 'returns to its unchanged caller'}`, async () => {
    const state = { testStarted: false, used: [], networkDirty: false }, useGeneration = { current: 0 };
    let finishTest;
    const pendingTest = new Promise(resolve => { finishTest = resolve; });
    const service = { id: 'decision/a', revision: 1 };
    const flow = bind(decisionSave, {
      editing: { key: service.id, isNew: true, expected: 0 }, busy: false, sources: {}, editingView: null,
      t: (_, en) => en, expectedSourceRevisions: () => [{ source_id: 'source/a', revision: 1 }],
      invoke: async () => ({ state: 'succeeded' }), refresh: async () => [{ service }], useGeneration,
      onChanged: () => {}, setError: () => {}, setNotice: () => {}, setEditing: () => {}, setSelectedId: () => {},
      setUnverifiedSave: () => {}, setSavedBeforeTest: () => {}, setBusy: () => {}, testKey: service => service.id,
      test: () => { state.testStarted = true; return pendingTest; },
      setNetworkDirty: value => { state.networkDirty = value; }, onUse: value => { state.used.push(value); },
      planErrorCode: () => 'UNKNOWN', reconcile: async () => false, decisionFailureHelp: () => '',
    }, '{ save, networkChanged }');
    const work = flow.save(service, null, true, ['source/a']);
    for (let step = 0; !state.testStarted && step < 20; step++) await Promise.resolve();
    assert.equal(state.testStarted, true);
    if (editDuringTest) {
      const forward = bind(inheritedChange, { setDirty: () => {}, callback: { current: flow.networkChanged } }, 'networkChanged');
      const edit = bind(editorChange, {
        setDraft: () => {}, setSaved: () => {}, setError: () => {},
        dirtyCallback: { current: forward }, view: { policy: { mode: 'direct' } }, networkEdit, networkPolicyEqual,
      }, 'changeDraft');
      edit(networkDraft({ mode: 'inherit' }, 1));
      assert.equal(state.networkDirty, true, 'Dirty state reaches the caller before a later promise can settle');
      assert.equal(useGeneration.current, 1);
    }
    finishTest(true);
    await work;
    assert.deepEqual(state.used, editDuringTest ? [] : [service]);
  });
}
