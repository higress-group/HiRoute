import assert from 'node:assert/strict';
import test from 'node:test';
import { matchingModelRoutes, matchingModelSources, modelRouteAuthoring, modelRouteEditor } from '../src/features/models/model-route.ts';

const model = (binding_id, upstream_model_id = 'gpt-6.1-sol', availability = 'available') => ({ binding_id, upstream_model_id, display_name: 'Same label', presentation: { availability } });
const current = model('binding/subscription');
const source = (source_id, models, state = 'ready') => ({ source_id, models, state });
const plan = { agent_plan_id: 'plan/sol', model_alias: 'gpt-6.1-sol', head: { status: 'enabled', head_revision: 7 }, desired: { mode: 'fixed_model', display_name: 'Sol', strategy: { candidates: [{ binding_id: current.binding_id, reasoning: { kind: 'profile', profile: 'low' } }] } } };

test('candidate suggestions use exact model IDs and distinct ready bindings, not labels', () => {
  const snapshot = { sources: [source('subscription', [current]), source('api', [model('binding/api'), model('binding/wrong', 'other-id'), model('binding/unavailable', undefined, 'unavailable')]), source('disabled', [model('binding/disabled')], 'disabled'), source('duplicate', [model('binding/api')])] };
  assert.deepEqual(matchingModelSources(snapshot, current).map(item => item.model.binding_id), ['binding/api']);
  assert.deepEqual(matchingModelRoutes('gpt-6.1-sol', [plan, { ...plan, head: { status: 'deleted' } }, { ...plan, model_alias: 'other', desired: { display_name: 'Same label' } }]), [plan]);
});

test('new model routes prefill exact ID, fixed mode and explicitly ordered candidates', () => {
  const request = { modelId: 'gpt-6.1-sol', bindingIds: [current.binding_id, 'binding/api', 'binding/api'] };
  assert.deepEqual(modelRouteAuthoring(request), { mode: 'fixed_model', display_name: request.modelId, candidates: [{ binding_id: current.binding_id }, { binding_id: 'binding/api' }] });
  const editor = { display_name: request.modelId, purpose: '', candidates: [{ binding_id: current.binding_id, reasoning: { kind: 'profile', profile: 'high' } }, { binding_id: 'binding/api' }], delegation_enabled: false };
  const value = modelRouteEditor({ authoring: { editor }, candidates: [{ binding_id: 'binding/api', reasoning: { kind: 'budget', minimum_tokens: 1024 } }] }, request);
  assert.equal(value.custom_alias, request.modelId);
  assert.equal(value.purpose, '');
  assert.deepEqual(value.candidates.map(item => item.reasoning), [{ kind: 'profile', profile: 'high' }, { kind: 'budget', tokens: 1024 }]);
  assert.equal(editor.candidates[1].reasoning, undefined, 'options are not mutated');
});

test('existing fixed route appends without changing original order, reasoning, alias or settings', () => {
  const request = { modelId: 'gpt-6.1-sol', bindingIds: ['binding/api', current.binding_id], plan };
  assert.deepEqual(modelRouteAuthoring(request), { plan_id: plan.agent_plan_id, candidates: [...plan.desired.strategy.candidates, { binding_id: 'binding/api' }] });
  const editor = { purpose: 'Original purpose', custom_alias: 'gpt-6.1-sol', limits: { maximum_attempts: 2 }, candidates: [...plan.desired.strategy.candidates, { binding_id: 'binding/api' }] };
  assert.deepEqual(modelRouteEditor({ authoring: { editor, expected_head_revision: 7 }, candidates: [] }, request), editor);
});

test('changed route heads and other routing modes cannot be silently overwritten', () => {
  const request = { modelId: 'gpt-6.1-sol', bindingIds: ['binding/api'], plan };
  assert.throws(() => modelRouteEditor({ authoring: { expected_head_revision: 8 } }, request), { code: 'CHANGE_PREVIEW_STALE' });
  assert.throws(() => modelRouteEditor({ authoring: { expected_head_revision: 7 } }, { ...request, plan: { ...plan, desired: { mode: 'smart_saving' } } }), { code: 'CHANGE_PREVIEW_STALE' });
});
