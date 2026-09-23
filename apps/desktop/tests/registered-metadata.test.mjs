import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { registeredModelCandidates, templateEndpointForProtocol } from '../src/features/model-connections/registered-metadata.ts';

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
  assert.ok(!ids('zhipu.general.cn.v1').includes('glm-5.3'));
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
  };
  const selected = templateEndpointForProtocol(draft, tokenPlan, 'messages');
  assert.equal(`${selected.base_url}${selected.request_path}`,
    'https://token-plan.cn-beijing.maas.aliyuncs.com/apps/anthropic/v1/messages');
  assert.equal(templateEndpointForProtocol({ ...draft, request_path_override: '/custom/messages' }, tokenPlan, 'messages'), null);
  assert.equal(templateEndpointForProtocol(draft, tokenPlan, 'responses')?.request_path,
    '/compatible-mode/v1/responses');
});
