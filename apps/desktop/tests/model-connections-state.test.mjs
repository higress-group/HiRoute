import assert from 'node:assert/strict';
import test from 'node:test';

import {
  buildSaveChange,
  buildCheckDraft,
  clientIdempotencyKey,
  defaultSelectedModelRefs,
  modelCanBeSelected,
  modelSaveCompleted,
  saveEligibility,
  savedSourceConnectionFields,
  selectedModelRefsForSave,
  selectedModelRefsAfterRecheck,
  explicitApiModelSelection,
  blankModel,
  declareCapability,
  applySuggestedCapabilities,
  displayCapabilities,
  toWireModel,
  withRegisteredModels,
} from '../src/features/model-connections/state.ts';

test('associated API automatic save uses only observed selectable IDs selected in the subscription', () => {
  const checked = { candidate: { producer: 'native', models: [
    { upstream_model_id: 'gpt-sol', model_ref: 'fresh-sol', selectable: true, membership: 'observed' },
    { upstream_model_id: 'GPT-SOL', model_ref: 'different-case', selectable: true },
    { upstream_model_id: 'gpt-luna', model_ref: 'blocked', selectable: false },
    { upstream_model_id: 'unselected', model_ref: 'unrelated', selectable: true, membership: 'observed' },
    { upstream_model_id: 'catalog-only', model_ref: 'catalog', selectable: true, membership: 'catalog' },
  ] } };
  assert.deepEqual([...explicitApiModelSelection(checked, ['gpt-sol', 'gpt-luna', 'missing', 'catalog-only'])], ['fresh-sol']);
  assert.deepEqual([...explicitApiModelSelection(checked, [])], []);
  assert.deepEqual([...selectedModelRefsAfterRecheck(null, new Set(), checked)], []);
});

test('tool recheck keeps explicit model identities with fresh refs and drops unavailable choices', () => {
  const checked = (ref, revision, models) => ({ candidate: {
    candidate: { candidate_ref: ref }, correlation: { edit_revision: revision }, models,
  } });
  const model = (id, ref, selectable = true) => ({ upstream_model_id: id, model_ref: ref, selectable });
  const before = checked('account-a', 3, [model('chosen', 'old'), model('removed', 'old-removed'), model('blocked', 'old-blocked')]);
  const after = checked('account-a', 3, [model('chosen', 'fresh'), model('new', 'new-ref'), model('blocked', 'fresh-blocked', false)]);
  const selected = new Set(['old', 'old-removed', 'old-blocked']);
  assert.deepEqual([...selectedModelRefsAfterRecheck(before, selected, after)], ['fresh']);
  assert.deepEqual([...selectedModelRefsAfterRecheck(null, selected, after)], []);
  assert.deepEqual([...selectedModelRefsAfterRecheck(before, selected, checked('account-b', 3, after.candidate.models))], []);
  assert.deepEqual([...selectedModelRefsAfterRecheck(before, selected, checked('account-a', 4, after.candidate.models))], []);
  assert.deepEqual([...selected], ['old', 'old-removed', 'old-blocked']);
});

test('a blank model keeps every fact unknown and offers editable suggestions', () => {
  const model = blankModel('unknown-model');
  for (const key of ['tool', 'vision', 'streaming', 'context_tokens', 'max_output_tokens', 'native_reasoning']) {
    assert.equal(model.capabilities[key].value, null, key);
    assert.equal(model.capabilities[key].basis, 'unknown', key);
  }
  assert.ok(model.suggested_capabilities);
  assert.equal(displayCapabilities(model).context_tokens.value, 200_000);
  assert.equal(displayCapabilities(model).vision.value, false);
});

test('declaring one capability preserves unrelated facts and suggestions', () => {
  const model = blankModel('unknown-model');
  const declared = declareCapability(model, 'vision', { value: true, basis: 'user_declared' });
  assert.equal(declared.capabilities.vision.value, true);
  assert.equal(declared.capabilities.vision.basis, 'user_declared');
  assert.equal(declared.capabilities.tool.basis, 'unknown');
  assert.deepEqual(declared.cleared_capabilities, []);
  assert.ok(declared.suggested_capabilities);
});

test('apply suggestions fills only untouched unknown facts', () => {
  const model = declareCapability(blankModel('unknown-model'), 'vision', { value: true, basis: 'user_declared' });
  const applied = applySuggestedCapabilities(model);
  assert.equal(applied.capabilities.tool.value, true);
  assert.equal(applied.capabilities.vision.value, true);
  assert.equal(applied.capabilities.context_tokens.value, 200_000);
});

test('clearing a suggested field keeps it unknown and blocks silent refill', () => {
  const model = declareCapability(blankModel('unknown-model'), 'tool', { value: null, basis: 'unknown' });
  assert.deepEqual(model.cleared_capabilities, ['tool']);
  assert.equal(displayCapabilities(model).tool.value, null);
  const applied = applySuggestedCapabilities(model);
  assert.equal(applied.capabilities.tool.value, null);
});

test('wire model keeps declarations and never serializes suggestions', () => {
  const model = declareCapability(blankModel('wire-model'), 'vision', { value: true, basis: 'user_declared' });
  const wire = toWireModel(model);
  assert.equal(wire.capabilities.vision.value, true);
  assert.equal(wire.capabilities.tool.basis, 'unknown');
  assert.equal('suggested_capabilities' in wire, false);
  assert.equal('client_id' in wire, false);
});

test('only a terminal saved Operation can close a model save as successful', () => {
  for (const disposition of ['pending', 'needs_input', 'conflict', 'failed']) {
    assert.equal(modelSaveCompleted({ disposition }), false, disposition);
  }
  assert.equal(modelSaveCompleted({ disposition: 'saved' }), true);
});

test('save idempotency keys satisfy domain and native recovery constraints', () => {
  const key = clientIdempotencyKey('model-save');
  assert.match(key, /^[A-Za-z0-9._:-]{1,200}$/);
  assert.equal(key.length, 64);
  assert.ok(key.startsWith('model-save:'));
  assert.ok(!key.includes('/'));
});

test('native user check maps UI provenance to the exact public wire draft', () => {
  const draft = {
    entry_kind: 'custom_api', candidate_ref: null, lineage_ref: 'lineage/native/test',
    display_name: 'Native API', existing_source_id: null, edit_revision: 7,
    check_id: 'check/7', base_url: 'http://127.0.0.1:51026/v1', base_kind: 'api_root',
    request_path_override: null, inventory_path_override: null, protocol: 'responses',
    protocol_profile_id: 'profile/custom/responses', protocol_profile_revision: 1,
    authentication: { kind: 'none' },
    provenance: { kind: 'user_configured', configuration_revision: 4 },
    qualification: { free_access: null, evidence_ref: null },
    models: [{
      client_id: 'model/client-only', upstream_model_id: 'manual-native-model',
      display_name: 'Manual Native Model', catalog_configuration_id: null,
      membership: 'user_declared', capabilities: {
        tool: { value: false, basis: 'user_declared' },
        vision: { value: false, basis: 'user_declared' },
        streaming: { value: true, basis: 'user_declared' },
        context_tokens: { value: 32768, basis: 'user_declared' },
        max_output_tokens: { value: 4096, basis: 'user_declared' },
        native_reasoning: { value: { kind: 'fixed', profile: 'provider-default' }, basis: 'user_declared' },
      },
    }],
  };
  const wire = buildCheckDraft(draft);
  assert.equal(wire.configuration_revision, 4);
  assert.equal(wire.models[0].upstream_model_id, 'manual-native-model');
  assert.equal('client_id' in wire.models[0], false);
  assert.equal('entry_kind' in wire, false);
  assert.equal('provenance' in wire, false);
  assert.equal('qualification' in wire, false);
});

test('native check keeps opaque provider IDs distinct from display text and normalization', () => {
  const ids = ['内网模型', '外网模型', 'Vendor/Model@2026?revision#1', 'Model A',
    'é', 'e\u0301', '\ufeff模型\ufeff', '\ufeff', '模型🧠', '🧠'.repeat(128), '界'.repeat(170) + 'ab'];
  const draft = {
    entry_kind: 'custom_api', provenance: { kind: 'user_configured', configuration_revision: 1 },
    qualification: { free_access: null, evidence_ref: null },
    models: ids.map(id => blankModel(id, '中文显示名称')),
  };
  const wire = buildCheckDraft(draft);
  assert.deepEqual(wire.models.map(model => model.upstream_model_id), ids);
  assert.ok(wire.models.every(model => model.display_name === '中文显示名称'));
  assert.equal(new Set(wire.models.map(model => model.upstream_model_id)).size, ids.length);
  assert.equal(new TextEncoder().encode(wire.models.at(-1).upstream_model_id).length, 512);
});

test('registered model joins deduplicate only exact IDs and retain a distinct FEFF-prefixed model', () => {
  const prefixed = blankModel('\ufeff模型', '已编辑名称');
  const models = withRegisteredModels([prefixed], [
    { upstream_model_id: '\ufeff模型', display_name: '重复目录名称' },
    { upstream_model_id: '模型', display_name: '无前缀名称' },
    { upstream_model_id: '模型\ufeff', display_name: '有后缀名称' },
  ]);
  assert.deepEqual(models.map(model => model.upstream_model_id), ['\ufeff模型', '模型', '模型\ufeff']);
  assert.equal(models[0], prefixed);
  assert.deepEqual(models.map(model => model.display_name), ['已编辑名称', '无前缀名称', '有后缀名称']);
  const added = withRegisteredModels([], [{ upstream_model_id: '\ufeff', display_name: '仅有格式字符的 ID' }]);
  assert.equal(buildCheckDraft({ provenance: { kind: 'user_configured', configuration_revision: 1 }, models: added }).models[0].upstream_model_id, '\ufeff');
});

test('native check preserves every endpoint and the saved-source edit revision', () => {
  const draft = {
    entry_kind: 'custom_api', provenance: { kind: 'user_configured', configuration_revision: 2 },
    qualification: { free_access: null, evidence_ref: null }, models: [],
    existing_source_id: 'source/a', expected_source_revision: 7,
    base_url: 'https://open.example.test/v1', protocol: 'responses',
    authentication: { kind: 'bearer' },
    additional_endpoints: [{ base_url: 'https://messages.example.test', base_kind: 'api_root',
      request_path_override: '/v1/messages', inventory_path_override: null,
      protocol: 'messages', protocol_profile_id: 'profile/custom/messages',
      protocol_profile_revision: 1, authentication: { kind: 'api_key_header', header: 'x-api-key' } }],
  };
  const wire = buildCheckDraft(draft);
  assert.equal(wire.expected_source_revision, 7);
  assert.deepEqual(wire.additional_endpoints, draft.additional_endpoints);
  assert.equal('entry_kind' in wire, false);
});

test('saved source edit restores exact endpoint settings and IPv6 URLs', () => {
  const target = (authority, port, protocol, profile, revision, path) => ({
    scheme: 'http', authority, port, request_path: path,
    upstream_protocol: protocol, protocol_profile_id: profile, protocol_profile_revision: revision,
  });
  const fields = savedSourceConnectionFields({
    display_template_id: 'template/custom', inventory_path: '/custom/models',
    target: target('::1', 8123, 'responses', 'profile/custom/responses', 3, '/v1/responses'),
    authentication: { kind: 'bearer' },
    additional_native_endpoints: [{
      target: target('::1', 8124, 'messages', 'profile/custom/messages', 4, '/v1/messages'),
      authentication: { kind: 'api_key_header', header: 'x-api-key' },
      recheck: { inventory_path: '/anthropic/models' },
    }],
  });
  assert.equal(fields.base_url, 'http://[::1]:8123');
  assert.equal(fields.display_template_id, 'template/custom');
  assert.equal(fields.inventory_path_override, '/custom/models');
  assert.equal(fields.protocol_profile_id, 'profile/custom/responses');
  assert.equal(fields.protocol_profile_revision, 3);
  assert.equal(fields.additional_endpoints[0].base_url, 'http://[::1]:8124');
  assert.equal(fields.additional_endpoints[0].inventory_path_override, '/anthropic/models');
  assert.equal(fields.additional_endpoints[0].protocol_profile_revision, 4);
});

function resultWith({ factState, reachability, authentication }) {
  const model = {
    model_ref: 'model/user-declared',
    upstream_model_id: 'declared-model',
    display_name: 'Declared model',
    membership: 'user_declared',
    selectable: false,
    reason: 'model_connections.connection_check_required',
  };
  return {
    candidate: {
      candidate: { candidate_ref: 'candidate/native/test', candidate_revision: 3 },
      correlation: {
        candidate_ref: 'candidate/native/test',
        edit_revision: 7,
        check_id: 'check/7',
        input_digest: 'sha256:input',
      },
      producer: 'native',
      provenance: 'user_configured',
      display_name: 'Custom API',
      models: [model],
      input_state: factState === 'pending_credential' ? 'missing' : 'provided',
      fact_state: factState,
      issues: [],
    },
    target: {
      scheme: 'https', authority: 'api.example.test', port: 443,
      request_path: '/v1/responses', upstream_protocol: 'responses',
      protocol_profile_id: 'profile/custom/responses', protocol_profile_revision: 1,
    },
    inventory_path: '/v1/models',
    reachability,
    authentication,
    directory: 'not_run',
    protocol: 'selected',
    inference: 'not_run',
    checked_model_count: 0,
    invalid_model_count: 0,
    pages_read: 0,
    checked_at_unix_ms: 1,
    input_digest: 'sha256:input',
    issues: [],
  };
}

const revisions = { target: 4, dependencies: {} };

for (const scenario of [
  {
    name: 'missing key',
    result: resultWith({ factState: 'pending_credential', reachability: 'not_run', authentication: 'not_run' }),
    readyReason: 'credential_required',
  },
  {
    name: 'connection failure without a selectable declaration',
    result: resultWith({ factState: 'complete', reachability: 'transport_failed', authentication: 'unknown' }),
    readyReason: 'model_required',
  },
]) {
  test(`${scenario.name} keeps declared model refs for a disabled draft`, () => {
    const selected = new Set(defaultSelectedModelRefs(scenario.result));
    const disabledRefs = selectedModelRefsForSave(scenario.result, selected, false);
    const readyRefs = selectedModelRefsForSave(scenario.result, selected, true);

    assert.deepEqual(disabledRefs, ['model/user-declared']);
    assert.deepEqual(readyRefs, []);
    assert.equal(scenario.result.candidate.models[0].selectable, false);
    assert.deepEqual(saveEligibility(scenario.result, false), { allowed: true, reason: null });
    assert.deepEqual(saveEligibility(scenario.result, true), { allowed: false, reason: scenario.readyReason });

    const change = buildSaveChange({
      result: scenario.result,
      expectedRevisions: revisions,
      selectedModelRefs: disabledRefs,
      enable: false,
      protectedInput: null,
    });
    assert.equal(change.intent, 'save_disabled');
    assert.deepEqual(change.selected_model_refs, ['model/user-declared']);
  });
}

test('disabled selection exception rejects capability gaps and CPA candidates', () => {
  for (const candidate of [
    { producer: 'native', reason: 'model_connections.capability_required' },
    { producer: 'cpa', reason: 'model_connections.connection_check_required' },
  ]) {
    const result = resultWith({
      factState: 'complete',
      reachability: 'transport_failed',
      authentication: 'unknown',
    });
    result.candidate.producer = candidate.producer;
    result.candidate.models[0].reason = candidate.reason;
    const model = result.candidate.models[0];

    assert.equal(modelCanBeSelected(result, model, false), false);
    assert.deepEqual(defaultSelectedModelRefs(result), []);
    assert.deepEqual(selectedModelRefsForSave(result, new Set([model.model_ref]), false), []);
  }
});

test('mixed result selects complete model without admitting capability-required model', () => {
  const result = resultWith({
    factState: 'complete',
    reachability: 'reachable',
    authentication: 'verified',
  });
  result.candidate.models[0].reason = 'model_connections.capability_required';
  result.candidate.models.push({
    model_ref: 'model/complete',
    upstream_model_id: 'complete-model',
    display_name: 'Complete model',
    membership: 'user_declared',
    selectable: true,
  });

  assert.deepEqual(defaultSelectedModelRefs(result), ['model/complete']);
  const selected = new Set(result.candidate.models.map(model => model.model_ref));
  assert.deepEqual(selectedModelRefsForSave(result, selected, true), ['model/complete']);
  assert.deepEqual(selectedModelRefsForSave(result, selected, false), ['model/complete']);
});


test('optional directory failure permits valid models but explicit inference and auth failures do not', () => {
  const result = resultWith({ factState: 'complete', reachability: 'transport_failed', authentication: 'unknown' });
  result.candidate.models[0].selectable = true;
  assert.deepEqual(saveEligibility(result, true), { allowed: true, reason: null });
  result.inference = 'failed';
  assert.deepEqual(saveEligibility(result, true), { allowed: false, reason: 'explicit_failure' });
  result.inference = 'not_run';
  result.authentication = 'rejected';
  assert.deepEqual(saveEligibility(result, true), { allowed: false, reason: 'explicit_failure' });
});


test('save after a repeated check uses the current revision of the same protected slot', () => {
  const result = resultWith({ factState: 'complete', reachability: 'not_run', authentication: 'unknown' });
  result.candidate.candidate = { candidate_ref: 'candidate/protected', candidate_revision: 4 };
  const build = protectedInput => buildSaveChange({ result, expectedRevisions: revisions, selectedModelRefs: ['model/user-declared'], enable: true, protectedInput });
  assert.deepEqual(build({ candidate_ref: 'candidate/protected', candidate_revision: 1 }).key_edits[0].input_candidate, result.candidate.candidate);
  const other = { candidate_ref: 'candidate/another-slot', candidate_revision: 2 };
  assert.deepEqual(build(other).key_edits[0].input_candidate, other);
});
