import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { customApiPrefillOptions, metadataProviderCandidates, metadataProviderOption, registeredModelCandidates, sortMetadataProviders, sortRegisteredOptions, templateEndpointForProtocol } from '../src/features/model-connections/registered-metadata.ts';

const bundle = JSON.parse(readFileSync(new URL('../../../assets/release-facts/current/bundle/model-data.json', import.meta.url)));
const registry = JSON.parse(readFileSync(new URL('../../../assets/release-facts/current/bundle/connector-registry.json', import.meta.url)));
const profiles = new Map(registry.endpoint_profiles.map(profile => [profile.endpoint_profile_id, profile]));
const models = new Map(bundle.data.models.map(model => [model.model_configuration_id, model.display_name]));

function option(id) {
  const registered = registry.connection_options.find(item => item.connection_option_id === id);
  const profile = profiles.get(registered.endpoint_profile_id);
  const known_models = Object.fromEntries(bundle.data.model_endpoint_capabilities
    .filter(capability => capability.endpoint_profile_id === registered.endpoint_profile_id)
    .map(capability => [capability.upstream_model_id, models.get(capability.model_configuration_id)]));
  return { ...registered, endpoints: profile.protocol_endpoints, known_models };
}

function ids(id, modifiedOption = null) {
  return registeredModelCandidates(modifiedOption ?? option(id), bundle.metadata_catalog)
    .map(candidate => candidate.upstream_model_id);
}

test('every built-in API option offers product-scoped metadata or an executable catalog model', () => {
  for (const registered of registry.connection_options.filter(item => item.origin !== 'agent_subscription')) {
    assert.ok(ids(registered.connection_option_id).length > 0, registered.connection_option_id);
  }
});

test('GLM Flash is scoped to both Zhipu and Z.AI API products', () => {
  for (const id of ['zhipu.coding-plan.cn.v1', 'zhipu.general.cn.v1', 'zai.coding-plan.global.v1', 'zai.general.global.v1']) {
    assert.ok(ids(id).includes('glm-5.3-flash'), id);
  }
  assert.ok(ids('zhipu.general.cn.v1').includes('glm-5.3'));
});

test('DeepSeek direct API and Bailian plans retain their own callable IDs', () => {
  assert.ok(ids('deepseek.official.global.v1').includes('deepseek-flash'));
  assert.ok(!ids('deepseek.official.global.v1').includes('deepseek-v4.1-flash'));
  assert.ok(!ids('deepseek.official.global.v1').includes('deepseek-v4-flash'));
  assert.ok(ids('bailian.payg.cn.v1').includes('deepseek-v4.1-flash'));
  assert.ok(ids('bailian.token-plan.cn.v1').includes('deepseek-v4.1-flash'));
  assert.ok(!ids('bailian.coding-plan.cn.v1').includes('deepseek-v4.1-flash'));
});

test('Token Plan OpenAI and Anthropic endpoints resolve to the same plan product', () => {
  const tokenPlan = option('bailian.token-plan.cn.v1');
  const messages = tokenPlan.endpoints.find(endpoint => endpoint.protocol === 'messages');
  assert.ok(messages);
  assert.ok(ids(tokenPlan.connection_option_id, { ...tokenPlan, endpoints: [{ ...messages, stable_preference: 0 }] })
    .includes('deepseek-v4.1-flash'));
  assert.ok(!ids(tokenPlan.connection_option_id, { ...tokenPlan, endpoints: [{ ...messages, base_url: 'https://unmatched.example.test' }] })
    .includes('deepseek-v4.1-flash'));
});

test('switching a Token Plan template to Anthropic selects its documented endpoint', () => {
  const tokenPlan = option('bailian.token-plan.cn.v1');
  const chat = tokenPlan.endpoints.find(endpoint => endpoint.protocol === 'chat_completions');
  assert.ok(chat);
  const draft = {
    display_template_id: tokenPlan.connection_option_id,
    base_url: chat.base_url,
    request_path_override: chat.request_path,
    inventory_path_override: chat.inventory_path ?? null,
    protocol: chat.protocol,
    authentication: chat.authentication_semantics,
  };
  const selected = templateEndpointForProtocol(draft, tokenPlan, 'messages');
  assert.equal(`${selected.base_url}${selected.request_path}`,
    'https://token-plan.cn-beijing.maas.aliyuncs.com/apps/anthropic/v1/messages');
  assert.equal(templateEndpointForProtocol({ ...draft, request_path_override: '/custom/messages' }, tokenPlan, 'messages'), null);
  assert.equal(templateEndpointForProtocol(draft, tokenPlan, 'responses')?.request_path,
    '/compatible-mode/v1/responses');
  assert.equal(templateEndpointForProtocol({ ...draft, authentication: { kind: 'none' } }, tokenPlan, 'messages'), null);
});

test('Kimi protocol switch carries the endpoint authentication semantics', () => {
  const kimi = option('kimi.code.cn.v1');
  const chat = kimi.endpoints.find(endpoint => endpoint.protocol === 'chat_completions');
  const messages = kimi.endpoints.find(endpoint => endpoint.protocol === 'messages');
  assert.ok(chat && messages);
  const draft = {
    display_template_id: kimi.connection_option_id,
    base_url: chat.base_url,
    request_path_override: chat.request_path,
    inventory_path_override: chat.inventory_path ?? null,
    protocol: chat.protocol,
    authentication: chat.authentication_semantics,
  };
  const selected = templateEndpointForProtocol(draft, kimi, 'messages');
  assert.deepEqual(selected?.authentication_semantics, { kind: 'api_key_header', header: 'x-api-key' });
});

const personalTokenModels = [
  'qwen3.8-max', 'qwen3.8-flash', 'qwen3.7-max', 'qwen3.7-plus', 'qwen3.6-flash',
  'deepseek-v4.1-flash', 'deepseek-v4-pro', 'deepseek-v4-pro-0813', 'deepseek-v4-flash-0731',
  'glm-5.3', 'glm-5.2',
];
const teamTokenModels = [...personalTokenModels,
  'qwen3.6-plus', 'deepseek-v4-flash', 'deepseek-v3.2', 'kimi-k2.7-code',
  'kimi-k2.6', 'kimi-k2.5', 'glm-5.1', 'glm-5', 'MiniMax-M2.5',
];

test('Token Plan includes the documented text catalogs on both endpoint protocols', () => {
  const tokenPlan = option('bailian.token-plan.cn.v1');
  for (const protocol of ['chat_completions', 'messages']) {
    const endpoint = tokenPlan.endpoints.find(value => value.protocol === protocol);
    const selected = { ...tokenPlan, endpoints: [{ ...endpoint, stable_preference: 0 }] };
    assert.deepEqual(ids(tokenPlan.connection_option_id, selected).sort(), [...teamTokenModels].sort());
    for (const [tier, expected] of [['personal', personalTokenModels], ['team', teamTokenModels]]) {
      const catalog = { ...bundle.metadata_catalog, access_products: bundle.metadata_catalog.access_products
        .filter(product => product.product_key === `bailian-token-${tier}-cn-beijing`) };
      assert.deepEqual(registeredModelCandidates(selected, catalog).map(value => value.upstream_model_id).sort(),
        [...expected].sort(), `${tier}/${protocol}`);
    }
  }
});

test('all template models enter the check draft without import or fabricated capabilities', async () => {
  const { blankModel, withRegisteredModels, buildCheckDraft } = await import('../src/features/model-connections/state.ts');
  for (const registered of registry.connection_options.filter(value => value.origin !== 'agent_subscription')) {
    const candidates = registeredModelCandidates(option(registered.connection_option_id), bundle.metadata_catalog);
    const models = withRegisteredModels([], candidates);
    assert.deepEqual(models.map(value => value.upstream_model_id), candidates.map(value => value.upstream_model_id));
    for (const model of models) {
      assert.equal(model.membership, 'user_declared');
      assert.equal(model.catalog_configuration_id, null);
      assert.ok(Object.values(model.capabilities).every(fact => fact.value === null && fact.basis === 'unknown'));
    }
    assert.deepEqual(withRegisteredModels(models, candidates), models, 'recheck does not duplicate models');
  }
  const candidates = registeredModelCandidates(option('bailian.token-plan.cn.v1'), bundle.metadata_catalog);
  const edited = { ...blankModel('glm-5.3', 'My GLM'), capabilities: {
    ...blankModel().capabilities, context_tokens: { value: 1000, basis: 'user_declared' },
  } };
  const custom = blankModel('my-account-model');
  const models = withRegisteredModels([edited, custom], candidates);
  assert.equal(models.filter(model => model.upstream_model_id === 'glm-5.3').length, 1);
  assert.equal(models[0], edited, 'explicit model edits take precedence');
  assert.equal(models[1], custom, 'account-specific manual IDs survive');
  const wire = buildCheckDraft({ provenance: { kind: 'user_configured', configuration_revision: 1 }, models });
  assert.equal(wire.models.length, teamTokenModels.length + 1);
  assert.ok(wire.models.every(model => !Object.hasOwn(model, 'client_id')));
});

test('Bailian Token Plan is first among registered API connections', () => {
  const options = registry.connection_options.filter(value => value.origin !== 'agent_subscription').map(value => option(value.connection_option_id));
  assert.deepEqual(sortRegisteredOptions(options).slice(0, 3).map(value => value.connection_option_id), [
    'bailian.token-plan.cn.v1', 'bailian.coding-plan.cn.v1', 'bailian.payg.cn.v1',
  ]);
  assert.deepEqual(sortRegisteredOptions([...options].reverse()).slice(0, 3).map(value => value.connection_option_id), [
    'bailian.token-plan.cn.v1', 'bailian.coding-plan.cn.v1', 'bailian.payg.cn.v1',
  ]);
});

test('Bailian Token Plan is first among custom API metadata prefills', () => {
  const providers = bundle.metadata_catalog.provider_records.filter(value =>
    value.usable_for.includes('custom-api-endpoint-prefill'));
  assert.deepEqual(sortMetadataProviders(providers).slice(0, 3).map(value => value.provider_record_key), [
    'hermes-agent/alibaba-token-plan-cn',
    'hermes-agent/alibaba-coding-plan-cn',
    'hermes-agent/alibaba-cn',
  ]);
});

test('direct custom API prefill offers every built-in product with its scoped model catalog', () => {
  const options = registry.connection_options.filter(value => value.origin !== 'agent_subscription')
    .map(value => option(value.connection_option_id));
  const providers = bundle.metadata_catalog.provider_records.filter(value =>
    value.usable_for.includes('custom-api-endpoint-prefill')
    && value.base_url_candidates.length > 0 && value.protocol_candidates.length > 0);
  const prefills = customApiPrefillOptions(options, providers);
  const templates = prefills.filter(value => value.kind === 'template');
  assert.equal(templates.length, options.length);
  assert.deepEqual(new Set(templates.map(value => value.option.connection_option_id)),
    new Set(options.map(value => value.connection_option_id)));
  assert.equal(templates[0].option.connection_option_id, 'bailian.token-plan.cn.v1');
  assert.deepEqual(registeredModelCandidates(templates[0].option, bundle.metadata_catalog)
    .map(value => value.upstream_model_id).sort(), [...teamTokenModels].sort());
  assert.deepEqual(prefills.filter(value => value.kind === 'provider').map(value => value.provider.provider_record_key),
    sortMetadataProviders(providers).map(value => value.provider_record_key));
});

test('priority providers expose their current scoped model IDs', () => {
  const expected = new Map([
    ['bailian.payg.cn.v1', ['qwen3.8-max', 'deepseek-v4.1-flash', 'deepseek-v4-pro', 'glm-5.3', 'kimi-k3']],
    ['deepseek.official.global.v1', ['deepseek-flash', 'deepseek-v4-pro']],
    ['zhipu.coding-plan.cn.v1', ['glm-5.3', 'glm-5.3-flash']],
    ['zhipu.general.cn.v1', ['glm-5.3', 'glm-5.3-flash']],
    ['kimi.open-platform.cn.v1', ['kimi-k3', 'kimi-k2.7-code', 'kimi-k2.7-code-highspeed', 'kimi-k2.6']],
    ['kimi.code.cn.v1', ['k3', 'k3-256k', 'kimi-for-coding', 'kimi-for-coding-highspeed']],
  ]);
  for (const [id, models] of expected) {
    for (const model of models) assert.ok(ids(id).includes(model), `${id}: ${model}`);
  }
  assert.ok(!ids('kimi.open-platform.cn.v1').includes('kimi-k2.5'));
  assert.ok(!ids('kimi.open-platform.cn.v1').includes('kimi-k2.7'));
  assert.ok(!ids('deepseek.official.global.v1').includes('deepseek-v4-flash'));
  assert.ok(!ids('bailian.coding-plan.cn.v1').includes('deepseek-v4.1-flash'));
});

test('custom API metadata prefill inherits the matching product scope', () => {
  const options = registry.connection_options.filter(value => value.origin !== 'agent_subscription').map(value => option(value.connection_option_id));
  const providers = new Map(bundle.metadata_catalog.provider_records.map(provider => [provider.provider_record_key, provider]));
  const tokenProvider = providers.get('hermes-agent/alibaba-token-plan-cn');
  assert.equal(metadataProviderOption(tokenProvider, options)?.connection_option_id, 'bailian.token-plan.cn.v1');
  const tokenCandidates = metadataProviderCandidates(tokenProvider, bundle.metadata_catalog, options);
  assert.deepEqual(tokenCandidates.map(value => value.upstream_model_id).sort(), [...teamTokenModels].sort());
  assert.ok(!tokenCandidates.some(value => value.upstream_model_id === 'qwen3-max-2026-01-23'));
  const deepseekProvider = providers.get('openclaw/deepseek');
  assert.deepEqual(metadataProviderCandidates(deepseekProvider, bundle.metadata_catalog, options)
    .map(value => value.upstream_model_id).sort(), ['deepseek-flash', 'deepseek-v4-pro']);
  const kimiProvider = providers.get('hermes-agent/kimi-coding-cn');
  assert.deepEqual(metadataProviderCandidates(kimiProvider, bundle.metadata_catalog, options)
    .map(value => value.upstream_model_id).sort(), ids('kimi.code.cn.v1').sort());
});
