import assert from 'node:assert/strict';
import test from 'node:test';
import { NewSubscriptionApiSetup } from '../src/features/subscriptions/new-api.ts';
import { observeSaveAttempt } from '../src/features/subscriptions/save-attempt.ts';
import { networkDraft } from '../src/features/model-connections/native-network.ts';

const op = { operation_id: 'operation/api-created', state: 'succeeded', sequence: 1, cancellable: false };
const model = id => ({ upstream_model_id: id, model_ref: `model/${id}`, membership: 'observed', selectable: true });
function fixture(provider = 'codex') {
  const optionId = provider === 'codex' ? 'openai.platform.global.v1' : 'anthropic.platform.global.v1';
  const ids = provider === 'codex' ? ['gpt-sol', 'gpt-luna'] : ['claude-sonnet', 'claude-opus'];
  const source = { source_id: 'source/subscription', revision: 4, provenance: 'connector_owned', models: ids.map(model), connection_identity: { connection_option_id: `${provider}.subscription.global.v1` } };
  const unrelated = { source_id: 'source/unrelated', display_name: 'Other API', keys: ['existing-key'], target: { authority: 'other.example' }, models: [model('unrelated')] };
  const snapshot = { sources: [source, unrelated], revisions: { target: 12, dependencies: {} }, subscription_api_links: [], routes: ['original-user-route'] };
  const calls = { check: [], preview: [], apply: [], find: [], result: [], associate: [], release: [], cancel: [], uncertain: 0, cleared: 0, registrations: [] };
  const candidate = { producer: 'native', provenance: 'registered', candidate: { candidate_ref: 'candidate/api', candidate_revision: 1 }, fact_state: 'complete', models: ids.map(model) };
  const checked = { authentication: 'verified', directory: 'available', inference: 'not_run', candidate, observed_model_ids: [...ids] };
  const api = { source_id: 'source/api', provenance: 'registered', state: 'ready', revision: 1, display_name: 'API', connection_identity: { connection_option_id: optionId }, models: [] };
  let latestChange, active = true;
  const ports = {
    backend: {
      registerProtectedInput: async secret => { calls.registrations.push(secret); return { input_candidate: { candidate_ref: 'protected/input', candidate_revision: 1 } }; },
      releaseProtectedInput: async input => calls.release.push(input),
      cancelModelConnectionCheck: async id => calls.cancel.push(id),
      listConnectionOptions: async () => ({ catalog: { catalog_binding_id: 'catalog/test' }, options: [{ connection_option_id: optionId, registered_check_available: true, origin: 'native_api' }] }),
      checkRegisteredModelConnection: async (request, network) => {
        calls.check.push({ request, network });
        checked.candidate.correlation = { candidate_ref: candidate.candidate.candidate_ref, edit_revision: request.edit_revision, check_id: request.check_id, input_digest: 'sha256:check' };
        checked.input_digest = 'sha256:check';
        return structuredClone(checked);
      },
      previewComputeSave: async (change, network) => { calls.preview.push({ change, network }); latestChange = change; return { spec: { command_id: 'compute.connection.apply', desired_state: change }, accept_digest: 'sha256:preview', expected_revisions: snapshot.revisions }; },
      applyComputeSave: async request => {
        calls.apply.push(request);
        api.models = latestChange.selected_model_refs.map(ref => model(ref.slice(6)));
        snapshot.sources.push(api); return { operation: op };
      },
      getComputeSaveResult: async operation => { calls.result.push(operation); return { disposition: 'saved', source_id: api.source_id, saved_revision: 1, bindings: [], operation: op }; },
    },
    read: async () => structuredClone(snapshot),
    find: async key => { calls.find.push(key); return { disposition: 'saved', source_id: api.source_id, bindings: [], operation: op }; },
    associate: async (id, apiId, revision, current) => {
      calls.associate.push({ id, apiId, revision });
      assert.equal(current(), true);
      snapshot.subscription_api_links = [{ source_id: id, api_source_id: apiId }];
      source.revision += 1; return 'saved';
    },
    onOperation() {}, onUncertain: () => { calls.uncertain += 1; }, current: () => active,
  };
  const setup = new NewSubscriptionApiSetup(provider, ports);
  const connect = (secret = 'fixture-key') => setup.connect({ source_id: source.source_id, revision: 4 }, secret, networkDraft(), () => { calls.cleared += 1; });
  return { source, snapshot, unrelated, checked, api, ports, calls, setup, connect, ids, optionId, inactive: () => { active = false; } };
}

test('inline new API uses provider defaults and only exact saved subscription model IDs', async () => {
  for (const provider of ['codex', 'claude']) {
    const f = fixture(provider), original = structuredClone(f.unrelated);
    f.checked.candidate.models.push(model('not-selected'));
    f.checked.observed_model_ids.push('not-selected');
    assert.equal((await f.connect()).state, 'saved');
    assert.deepEqual(f.api.models.map(item => item.upstream_model_id), f.ids);
    assert.equal(f.calls.check[0].request.connection_option_id, f.optionId);
    assert.equal(f.calls.check[0].request.inference_model_id, null);
    assert.deepEqual(f.calls.check[0].request.models, []);
    assert.deepEqual(f.calls.preview[0].change.selected_model_refs, f.ids.map(id => `model/${id}`));
    assert.equal(f.calls.check[0].network.policy.mode, 'direct');
    assert.equal(f.calls.associate[0].revision, 4);
    assert.equal(f.calls.cleared, 1);
    assert.deepEqual(f.unrelated, original); assert.deepEqual(f.snapshot.routes, ['original-user-route']);
  }
});

test('failed API check retains subscription and protected key for API-only retry', async () => {
  const f = fixture(), original = structuredClone(f.source);
  f.checked.authentication = 'rejected';
  assert.equal((await f.connect()).state, 'failed');
  assert.deepEqual(f.source, original); assert.equal(f.calls.apply.length, 0); assert.equal(f.calls.associate.length, 0);
  assert.equal(f.setup.hasCredential, true);
  f.checked.authentication = 'verified';
  assert.equal((await f.connect('')).state, 'saved');
  assert.equal(f.calls.registrations.length, 1);
  assert.equal(f.calls.apply.length, 1);
});

test('catalog-only IDs cannot create an API; partial exact matches are reported', async () => {
  const f = fixture(); f.checked.candidate.models[0].membership = 'catalog';
  f.checked.candidate.models[1].selectable = false; f.checked.observed_model_ids = [];
  assert.equal((await f.connect()).reason, 'models_unavailable'); assert.equal(f.calls.apply.length, 0);
  const partial = fixture(); partial.checked.candidate.models.pop(); partial.checked.observed_model_ids.pop();
  const answer = await partial.connect();
  assert.equal(answer.state, 'saved'); assert.deepEqual(answer.missing_model_ids, [partial.ids[1]]);
});

test('lost API Apply reply resolves the original receipt without another Check or creation', async () => {
  const f = fixture(), apply = f.ports.backend.applyComputeSave;
  f.ports.backend.applyComputeSave = async request => { await apply(request); throw new Error('reply lost'); };
  assert.equal((await f.connect()).state, 'pending');
  assert.equal((await f.connect('')).state, 'saved');
  assert.equal(f.calls.apply.length, 1); assert.equal(f.calls.check.length, 1);
  assert.equal(f.calls.find[0], f.calls.apply[0].idempotency_key);
  assert.equal(f.calls.registrations.length, 1);
});

test('pending API result is observed in place; terminal failure requests key reentry', async () => {
  const f = fixture();
  f.ports.backend.getComputeSaveResult = async operation => { f.calls.result.push(operation); return { disposition: 'pending', bindings: [], operation }; };
  assert.equal((await f.connect()).state, 'pending');
  assert.equal((await f.connect('')).state, 'pending');
  assert.equal(f.calls.apply.length, 1); assert.equal(f.calls.check.length, 1);
  f.ports.backend.getComputeSaveResult = async () => ({ disposition: 'failed', bindings: [], operation: op });
  assert.equal((await f.connect('')).reason, 'save_failed');
  assert.equal((await f.connect('')).reason, 'key_required');
});

test('saved API is reused when association fails or its response is unknown', async () => {
  for (const failure of ['throw', 'pending']) {
    const f = fixture(), associate = f.ports.associate;
    f.ports.associate = async () => { if (failure === 'throw') throw new Error('unavailable'); return 'uncertain'; };
    const first = await f.connect(); assert.equal(first.state, failure === 'throw' ? 'failed' : 'pending');
    f.ports.associate = associate;
    assert.equal((await f.connect('')).state, 'saved');
    assert.equal(f.calls.apply.length, 1); assert.equal(f.calls.check.length, 1);
  }
});

test('source revision change before API Apply cannot create or associate a stale source', async () => {
  const f = fixture(), check = f.ports.backend.checkRegisteredModelConnection;
  f.ports.backend.checkRegisteredModelConnection = async (...args) => { const result = await check(...args); f.source.revision++; return result; };
  assert.equal((await f.connect()).reason, 'source_changed');
  assert.equal(f.calls.apply.length, 0); assert.equal(f.calls.associate.length, 0);
});

test('closing releases only unconsumed credentials and cancels the exact Check', async () => {
  const f = fixture(); f.checked.authentication = 'rejected'; await f.connect();
  await f.setup.dispose();
  assert.equal(f.calls.release.length, 1); assert.equal(f.calls.cancel[0], f.calls.check[0].request.check_id);
  const pending = fixture(); pending.ports.backend.getComputeSaveResult = async () => ({ disposition: 'pending', bindings: [] });
  await pending.connect(); await pending.setup.dispose(); assert.equal(pending.calls.release.length, 0); assert.equal(pending.calls.cancel.length, 0);
});

test('late credential registration after closing is released without clearing newer input', async () => {
  const f = fixture(); let release;
  f.ports.backend.registerProtectedInput = () => new Promise(resolve => { release = resolve; });
  const run = f.connect();
  await new Promise(resolve => setTimeout(resolve, 0)); await f.setup.dispose();
  release({ input_candidate: { candidate_ref: 'late/input', candidate_revision: 1 } });
  assert.equal((await run).state, 'pending'); assert.equal(f.calls.cleared, 0); assert.equal(f.calls.release.length, 1);
  assert.equal(f.calls.check.length, 0);
});

test('concurrent submit shares one in-flight attempt', async () => {
  const f = fixture(); await Promise.all([f.connect(), f.connect()]);
  assert.equal(f.calls.apply.length, 1); assert.equal(f.calls.check.length, 1); assert.equal(f.calls.associate.length, 1);
});

test('absent receipt replays the same sealed request and key', async () => {
  const request = { idempotency_key: 'same-key', spec: { frozen: true } };
  const attempt = { request, submitted: true }, applied = [];
  const ports = { find: async () => null, apply: async value => { applied.push(value); return { operation: op }; }, result: async () => ({ disposition: 'saved', source_id: 'source/api', bindings: [] }), onUncertain() {}, onOperation() {}, current: () => true };
  assert.equal((await observeSaveAttempt(attempt, ports)).disposition, 'saved');
  assert.equal(applied[0], request);
});

test('a rejected replay cannot release an earlier uncertain save or replace its request', async () => {
  const f = fixture(), create = f.ports.backend.applyComputeSave, requests = [];
  f.ports.backend.applyComputeSave = async request => {
    requests.push(request);
    if (requests.length === 1) { await create(request); throw new Error('reply lost'); }
    throw { code: 'CONFIRMATION_STALE' };
  };
  f.ports.find = async key => {
    f.calls.find.push(key);
    return f.calls.find.length === 1 ? null : { disposition: 'saved', source_id: f.api.source_id, bindings: [], operation: op };
  };
  assert.equal((await f.connect()).state, 'pending');
  assert.equal((await f.connect('')).state, 'pending');
  assert.equal(f.setup.awaitingSave, true);
  await f.setup.dispose();
  assert.equal(f.calls.release.length, 0); assert.equal(f.calls.cancel.length, 0);
  assert.equal((await f.connect('')).state, 'saved');
  assert.equal(requests.length, 2); assert.equal(requests[0], requests[1]);
  assert.equal(f.calls.apply.length, 1); assert.equal(f.calls.check.length, 1);
  assert.deepEqual(f.calls.find, [requests[0].idempotency_key, requests[0].idempotency_key]);
});

test('a first submission rejected before admission permits a corrected attempt', async () => {
  const attempt = { request: { idempotency_key: 'not-admitted' }, submitted: false };
  const error = { code: 'CONFIRMATION_STALE' };
  await assert.rejects(observeSaveAttempt(attempt, {
    apply: async () => { throw error; }, result: async () => { throw new Error('not admitted'); },
    find: async () => null, onOperation() {}, onUncertain() {}, current: () => true,
  }), cause => cause === error);
  assert.equal(attempt.submitted, false);
});

test('incomplete saved receipt stays pending rather than starting a duplicate creation', async () => {
  const f = fixture(); f.ports.backend.getComputeSaveResult = async () => ({ disposition: 'saved', bindings: [] });
  assert.equal((await f.connect()).state, 'pending'); assert.equal((await f.connect('')).state, 'pending');
  assert.equal(f.calls.apply.length, 1);
});


test('active association does not bypass observation of its pending operation', async () => {
  const f = fixture(); let observations = 0;
  f.ports.associate = async (id, apiId) => {
    observations++;
    f.snapshot.subscription_api_links = [{ source_id: id, api_source_id: apiId }];
    if (observations === 1) f.source.revision++;
    return observations < 3 ? 'uncertain' : 'saved';
  };
  assert.equal((await f.connect()).state, 'pending');
  assert.equal((await f.connect('')).state, 'pending');
  assert.equal((await f.connect('')).state, 'saved');
  assert.equal(observations, 3); assert.equal(f.calls.apply.length, 1);
});

test('saved API readback must remain the expected provider with matching members', async () => {
  for (const field of ['provider', 'disabled', 'membership', 'revision']) {
    const f = fixture(), read = f.ports.read;
    f.ports.read = async () => {
      if (f.calls.apply.length) {
        if (field === 'provider') f.api.connection_identity.connection_option_id = 'other.api';
        if (field === 'disabled') f.api.state = 'disabled';
        if (field === 'membership') f.api.models = [model('unrelated')];
        if (field === 'revision') f.api.revision++;
      }
      return read();
    };
    assert.equal((await f.connect()).reason, 'source_changed');
    assert.equal(f.calls.associate.length, 0);
  }
});

test('completion read retries after confirmed association and its own model sync revision', async () => {
  const f = fixture(), read = f.ports.read, associate = f.ports.associate;
  let unavailable = false;
  f.checked.candidate.models.pop(); f.checked.observed_model_ids.pop();
  f.ports.read = async () => {
    if (unavailable) { unavailable = false; throw new Error('read interrupted'); }
    return read();
  };
  f.ports.associate = async (...args) => {
    await associate(...args);
    f.api.revision++; f.api.models = f.ids.map(model);
    unavailable = true;
    return 'saved';
  };
  assert.deepEqual(await f.connect(), { state: 'pending', reason: 'readback_failed', api_source_id: f.api.source_id });
  const result = await f.connect('');
  assert.equal(result.state, 'saved'); assert.deepEqual(result.missing_model_ids, []);
  assert.equal(f.calls.apply.length, 1); assert.equal(f.calls.check.length, 1); assert.equal(f.calls.associate.length, 1);
});

test('completion read still refuses a replaced association without another write', async () => {
  const f = fixture(), read = f.ports.read, associate = f.ports.associate;
  let unavailable = false;
  f.ports.read = async () => {
    if (unavailable) { unavailable = false; throw new Error('read interrupted'); }
    return read();
  };
  f.ports.associate = async (...args) => { await associate(...args); unavailable = true; return 'saved'; };
  assert.equal((await f.connect()).state, 'pending');
  f.snapshot.subscription_api_links[0].api_source_id = 'source/other-api'; f.source.revision++;
  assert.equal((await f.connect('')).reason, 'source_changed');
  assert.equal(f.calls.apply.length, 1); assert.equal(f.calls.associate.length, 1);
});
