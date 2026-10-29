import assert from 'node:assert/strict';
import test from 'node:test';
import { apiSyncKey, synchronizeSubscriptionApi } from '../src/features/subscriptions/api-sync.ts';

const link = { source_id: 'source/sub', api_source_id: 'source/api' };
const model = (id, extra = {}) => ({ upstream_model_id: id, model_ref: `model/${id}`, binding_id: `binding/${id}`, membership: 'observed', selectable: true, ...extra });
function fixture() {
  const subscription = { source_id: link.source_id, revision: 4, provenance: 'connector_owned', state: 'ready', connection_identity: { connection_option_id: 'codex.subscription.global.v1' }, models: [model('gpt-sol'), model('gpt-luna')] };
  const api = { source_id: link.api_source_id, revision: 8, provenance: 'user_configured', state: 'ready', models: [model('unrelated')], keys: [{ key_id: 'existing-key' }], target: { authority: 'api.example' } };
  const snapshot = { sources: [subscription, api], subscription_api_links: [link], revisions: { target: 11, dependencies: {} }, routes: ['existing-user-route'] };
  const checked = { authentication: 'verified', directory: 'available', observed_model_ids: ['gpt-sol', 'gpt-luna'], candidate: { existing_source_id: api.source_id, producer: 'native', candidate: { candidate_ref: 'candidate/api', candidate_revision: 1 }, validation: { validation_ref: 'validation/api' }, models: [model('gpt-sol'), model('gpt-luna')] } };
  const calls = { read: 0, check: [], preview: [], apply: [], find: [], renew: 0, operations: [], uncertain: 0 };
  let change;
  const operation = { operation_id: 'op_api', state: 'running', sequence: 1, cancellable: false };
  const backend = {
    read: async () => { calls.read++; return structuredClone(snapshot); },
    check: async request => { calls.check.push(request); return structuredClone(checked); },
    preview: async value => { calls.preview.push(value); change = value; return { spec: { test: true }, accept_digest: 'sha256:test', expected_revisions: value.expected_revisions }; },
    apply: async request => {
      calls.apply.push(request);
      for (const ref of change.selected_model_refs) api.models.push(model(ref.slice('model/'.length), { binding_id: `binding/api-${ref}` }));
      api.revision++; return { operation };
    },
    result: async () => ({ disposition: 'saved', source_id: api.source_id, operation }),
    findSave: async key => { calls.find.push(key); return null; },
    renewFailed: async () => { calls.renew++; subscription.revision++; },
    onOperation: op => calls.operations.push(op), onUncertain: () => calls.uncertain++,
    current: () => true, checkId: () => 'check-api',
  };
  return { snapshot, subscription, api, checked, calls, backend, operation, run: () => synchronizeSubscriptionApi(link, backend) };
}

test('checked exact API models append without changing subscription, keys, endpoint or user routes', async () => {
  const f = fixture(); const before = structuredClone(f.snapshot);
  const result = await f.run();
  assert.deepEqual(result.items.map(m => m.state), ['imported', 'imported']);
  assert.equal(result.api_revision, 9);
  assert.deepEqual(f.subscription, before.sources[0]);
  assert.deepEqual(f.api.models[0], before.sources[1].models[0]);
  assert.deepEqual(f.api.keys, before.sources[1].keys);
  assert.deepEqual(f.api.target, before.sources[1].target);
  assert.deepEqual(f.snapshot.routes, before.routes);
  assert.deepEqual(f.calls.preview[0].selected_model_refs, ['model/gpt-sol', 'model/gpt-luna']);
  assert.deepEqual(f.calls.preview[0].edit, { action: 'append_models' });
  assert.deepEqual(f.calls.check[0], { source_id: link.api_source_id, expected_source_revision: 8, edit_revision: 1, check_id: 'check-api' });
  await f.run();
  assert.equal(f.calls.apply.length, 1); assert.equal(f.calls.check.length, 1);
});

test('complete API absence is unsupported, partial absence stays pending and names never substitute for IDs', async () => {
  for (const directory of ['available', 'empty', 'partial']) {
    const f = fixture(); f.checked.directory = directory;
    f.checked.observed_model_ids = ['GPT-SOL'];
    f.checked.candidate.models = [model('GPT-SOL', { display_name: 'gpt-sol' })];
    const result = await f.run();
    assert.deepEqual(result.items.map(m => m.state), [directory === 'partial' ? 'pending' : 'unsupported', directory === 'partial' ? 'pending' : 'unsupported']);
    assert.equal(f.calls.apply.length, 0);
  }
});

test('API catalog declarations do not inherit subscription entitlements; incomplete capabilities stay pending', async () => {
  const f = fixture(); f.checked.observed_model_ids = [];
  f.checked.candidate.models = [model('gpt-sol', { membership: 'catalog' }), model('gpt-luna', { selectable: false })];
  const result = await f.run();
  assert.deepEqual(result.items.map(m => m.state), ['unsupported', 'pending']);
  assert.equal(f.calls.apply.length, 0);
});

test('partial API success retains unsupported subscription selections and retries only remaining IDs', async () => {
  const f = fixture(); f.checked.candidate.models.pop(); f.checked.observed_model_ids.pop();
  const first = await f.run();
  assert.deepEqual(first.items.map(m => m.state), ['imported', 'unsupported']);
  f.checked.candidate.models.push(model('gpt-luna')); f.checked.observed_model_ids.push('gpt-luna');
  const second = await f.run();
  assert.deepEqual(second.items.map(m => m.state), ['imported', 'imported']);
  assert.deepEqual(f.calls.preview[1].selected_model_refs, ['model/gpt-luna']);
  assert.notEqual(f.calls.apply[0].idempotency_key, f.calls.apply[1].idempotency_key);
});

test('API missing, disabled, failed authentication and transport failure leave subscription intact', async () => {
  for (const kind of ['missing', 'disabled', 'unauthorized', 'network', 'wrong-source', 'connector']) {
    const f = fixture(); const before = structuredClone(f.subscription);
    if (kind === 'missing') f.snapshot.sources.pop();
    if (kind === 'disabled') f.api.state = 'disabled';
    if (kind === 'unauthorized') f.checked.authentication = 'failed';
    if (kind === 'network') f.backend.check = async () => { throw new Error('unavailable'); };
    if (kind === 'wrong-source') f.checked.candidate.existing_source_id = 'source/other';
    if (kind === 'connector') f.checked.candidate.producer = 'cpa';
    const result = await f.run();
    assert.ok(result.reason); assert.equal(f.calls.apply.length, 0);
    assert.deepEqual(f.subscription, before);
  }
});

test('a disabled subscription can independently import into its ready API', async () => {
  const f = fixture(); f.subscription.state = 'disabled';
  assert.deepEqual((await f.run()).items.map(m => m.state), ['imported', 'imported']);
  assert.equal(f.subscription.state, 'disabled');
});

test('changes to association, subscription revision or API revision during check prevent stale writes', async () => {
  for (const kind of ['link', 'subscription', 'api', 'cancelled']) {
    const f = fixture(); const original = f.backend.check;
    f.backend.check = async request => {
      const checked = await original(request);
      if (kind === 'link') f.snapshot.subscription_api_links = [];
      if (kind === 'subscription') f.subscription.revision++;
      if (kind === 'api') f.api.revision++;
      if (kind === 'cancelled') f.backend.current = () => false;
      return checked;
    };
    assert.ok((await f.run()).reason); assert.equal(f.calls.apply.length, 0);
  }
});

test('an existing pending operation is observed without checking, renewing or submitting again', async () => {
  const f = fixture();
  f.backend.findSave = async () => ({ disposition: 'pending', operation: f.operation, source_id: f.api.source_id });
  assert.equal((await f.run()).reason, 'save_pending');
  assert.equal(f.calls.check.length, 0); assert.equal(f.calls.apply.length, 0); assert.equal(f.calls.renew, 0);
  assert.equal(f.calls.uncertain, 1); assert.deepEqual(f.calls.operations, [f.operation]);
});

test('terminal failed operation gets one explicit retry with a new source revision and key', async () => {
  const f = fixture(); const keys = [];
  f.backend.findSave = async key => { keys.push(key); return keys.length === 1 ? { disposition: 'failed', operation: f.operation } : null; };
  assert.deepEqual((await f.run()).items.map(m => m.state), ['imported', 'imported']);
  assert.equal(f.calls.renew, 1); assert.notEqual(keys[0], keys[1]); assert.equal(f.calls.apply.length, 1);
  const repeated = fixture(); repeated.backend.findSave = async () => ({ disposition: 'failed' });
  assert.equal((await repeated.run()).reason, 'save_failed'); assert.equal(repeated.calls.renew, 1);
});

test('lost Apply/result reply keeps a recoverable pending import without a duplicate request', async () => {
  for (const phase of ['apply', 'result']) {
    const f = fixture();
    f.backend[phase] = async () => { throw new Error('reply lost'); };
    assert.equal((await f.run()).reason, 'save_pending'); assert.equal(f.calls.uncertain, 1);
    f.backend.findSave = async () => ({ disposition: 'pending', operation: f.operation });
    await f.run(); assert.equal(f.calls.check.length, 1); assert.equal(f.calls.renew, 0);
  }
});

test('saved operation requires matching API identity and membership readback', async () => {
  for (const kind of ['wrong-source', 'no-membership']) {
    const f = fixture();
    if (kind === 'wrong-source') f.backend.result = async () => ({ disposition: 'saved', source_id: 'source/other' });
    else f.backend.apply = async () => ({ operation: f.operation });
    assert.equal((await f.run()).reason, 'save_failed');
  }
});

test('lookup of completed import requires readback and never replays Apply', async () => {
  const f = fixture(); f.backend.findSave = async () => ({ disposition: 'saved', source_id: f.api.source_id, operation: f.operation });
  assert.equal((await f.run()).reason, 'save_failed');
  assert.equal(f.calls.check.length, 0); assert.equal(f.calls.apply.length, 0);
});

test('sync keys distinguish encoded separators, identities and source/API revisions', () => {
  const f = fixture(); const base = apiSyncKey(f.subscription, f.api);
  assert.match(base, /^[a-zA-Z0-9._:-]{1,200}$/);
  for (const id of ['source_sub', 'source:sub', 'source_2f_sub']) assert.notEqual(base, apiSyncKey({ ...f.subscription, source_id: id }, f.api));
  assert.notEqual(base, apiSyncKey({ ...f.subscription, revision: 5 }, f.api));
  assert.notEqual(base, apiSyncKey(f.subscription, { ...f.api, revision: 9 }));
  assert.throws(() => apiSyncKey({ ...f.subscription, source_id: 'x'.repeat(201) }, f.api));
});
