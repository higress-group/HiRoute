import { test } from 'node:test';
import assert from 'node:assert/strict';
import { qualityExecutionKey, qualityModelRows, qualityNativeModelName, qualityReasoningLabel } from '../src/features/plan-quality-state.ts';

const configured = (branch, model, profile = 'high') => ({ branch_id: branch, model_configuration_id: model, reasoning_profile_id: profile, display_name: model + ' readable' });
const summary = (branch, model, profile = 'high', digest = profile) => ({
  execution: { plan_revision: 1, selected_branch_id: null, executed_branch_id: branch, model_configuration_id: model, profile_digest: digest, attribution: 'single' },
  reasoning_profile_id: profile, native_model: model, scored_stage_count: 1, unrated_stage_count: 0, average_score: .8,
});

test('published order survives a higher score and the same model in other groups or reasoning profiles', () => {
  const models = [configured('economy', 'a', 'low'), configured('economy', 'b'), configured('primary', 'a'), configured('primary', 'a', 'medium')];
  const stats = [summary('primary', 'a', 'medium'), summary('primary', 'a'), summary('economy', 'b'), { ...summary('economy', 'a', 'low'), average_score: 0 }];
  const rows = qualityModelRows(models, stats);
  assert.deepEqual(rows.map(row => [row.branch, row.configured.model_configuration_id, row.summary.reasoning_profile_id]), [
    ['economy', 'a', 'low'], ['economy', 'b', 'high'], ['primary', 'a', 'high'], ['primary', 'a', 'medium'],
  ]);
  assert.equal(rows[0].summary.average_score, 0);
  assert.equal(new Set(rows.map(row => row.key)).size, 4);
});

test('distinct execution digests remain separate and unused models acquire no invented score', () => {
  const rows = qualityModelRows([configured('primary', 'a'), configured('primary', 'unused')], [summary('primary', 'a', 'high', 'digest/old'), summary('primary', 'a', 'high', 'digest/new')]);
  assert.equal(rows.length, 3);
  assert.notEqual(rows[0].key, rows[1].key);
  assert.equal(rows[2].configured.model_configuration_id, 'unused');
  assert.equal(rows[2].summary, undefined);
});

test('missing reasoning evidence cannot assign a historical stage to one of multiple configured profiles', () => {
  const unknown = summary('primary', 'a', null, 'digest/unknown');
  const rows = qualityModelRows([configured('primary', 'a', 'low'), configured('primary', 'a', 'high')], [unknown]);
  assert.equal(rows.length, 3);
  assert.equal(rows[2].configured, undefined);
  assert.equal(rows[2].summary, unknown);
  const mixed = { ...unknown, execution: { ...unknown.execution, attribution: 'mixed' } };
  assert.equal(qualityModelRows([], [mixed])[0].branch, 'unattributed');
  assert.notEqual(qualityExecutionKey(mixed.execution), qualityExecutionKey(unknown.execution));
});

test('reasoning controls use readable labels and keep opaque identities out of the overview', () => {
  assert.equal(qualityReasoningLabel('budget-16000', 'zh'), '16000 tokens');
  assert.equal(qualityReasoningLabel('high', 'en'), 'high');
  assert.equal(qualityReasoningLabel('xhigh', 'zh'), 'xhigh');
  assert.equal(qualityReasoningLabel('medium', 'zh'), 'medium');
  assert.equal(qualityReasoningLabel('sha256:1234', 'zh'), '推理配置未记录');
  assert.equal(qualityNativeModelName('hiroute-codex-current/gpt-6-astra'), 'gpt-6-astra');
  assert.equal(qualityNativeModelName('Qwen/Qwen3.8-Flash'), 'Qwen/Qwen3.8-Flash');
  assert.equal(qualityNativeModelName('other-provider/gpt-6-astra'), 'other-provider/gpt-6-astra');
  assert.equal(qualityNativeModelName('hiroute-codex-current/'), 'hiroute-codex-current/');
});
