import { test } from 'node:test';
import assert from 'node:assert/strict';
import { qualityExecutionKey, qualityModelRows, qualityNativeModelName, qualityReasoningLabel } from '../src/features/plan-quality-state.ts';

const configured = (branch, model, profile = 'high') => ({ plan_revision: 1, group: 'regular', candidate_index: 0, branch_id: branch, model_configuration_id: model, reasoning_profile_id: profile, display_name: model + ' readable' });
const summary = (branch, model, profile = 'high', digest = profile) => ({
  execution: { group: 'regular', candidate_index: 0, plan_revision: 1, selected_branch_id: null, executed_branch_id: branch, model_configuration_id: model, profile_digest: digest, attribution: 'single' },
  reasoning_profile_id: profile, native_model: model, scored_stage_count: 1, unrated_stage_count: 0, average_score: .8,
});

test('published order survives a higher score and the same model in other groups or reasoning profiles', () => {
  const models = [configured('economy', 'a', 'low'), { ...configured('economy', 'b'), candidate_index: 1 }, configured('primary', 'a'), { ...configured('primary', 'a', 'medium'), candidate_index: 1 }];
  const at = (stat, index) => ({ ...stat, execution: { ...stat.execution, candidate_index: index } });
  const stats = [at(summary('primary', 'a', 'medium'), 1), summary('primary', 'a'), at(summary('economy', 'b'), 1), { ...summary('economy', 'a', 'low'), average_score: 0 }];
  const rows = qualityModelRows(models, stats);
  assert.deepEqual(rows.map(row => [row.branch, row.configured.model_configuration_id, row.summary.reasoning_profile_id]), [
    ['economy', 'a', 'low'], ['economy', 'b', 'high'], ['primary', 'a', 'high'], ['primary', 'a', 'medium'],
  ]);
  assert.equal(rows[0].summary.average_score, 0);
  assert.equal(new Set(rows.map(row => row.key)).size, 4);
});

test('distinct execution digests remain separate and unused models acquire no invented score', () => {
  const rows = qualityModelRows([configured('primary', 'a'), { ...configured('primary', 'unused'), candidate_index: 1 }], [summary('primary', 'a', 'high', 'digest/old'), summary('primary', 'a', 'high', 'digest/new')]);
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

test('regular and primary groups keep separate samples for the same exact model', () => {
  const normal = configured('writing', 'a');
  const upgraded = { ...normal, group: 'primary' };
  const regularSummary = summary('writing', 'a');
  const upgradeSummary = { ...summary('writing', 'a'), execution: { ...regularSummary.execution, group: 'primary' }, average_score: 0 };
  const rows = qualityModelRows([normal, upgraded], [upgradeSummary, regularSummary]);
  assert.equal(rows.length, 2);
  assert.equal(rows[0].summary, regularSummary);
  assert.equal(rows[1].summary, upgradeSummary);
  assert.notEqual(rows[0].key, rows[1].key);
  assert.equal(qualityModelRows([normal, upgraded], [regularSummary])[1].summary, undefined);
});

test('a source refresh does not duplicate published candidates with their retained execution identities', () => {
  const models = ['writing', 'reviewing'].flatMap(branch => [
    { ...configured(branch, 'model/runtime-fallback/current-flash', 'low'), display_name: 'Qwen 3.8 Flash' },
    { ...configured(branch, 'model/runtime-fallback/current-glm', 'max'), group: 'primary', display_name: 'GLM-5.3' },
  ]);
  const stats = models.map((model, index) => ({
    ...summary(model.branch_id, 'model/runtime-fallback/recorded-' + index, model.reasoning_profile_id),
    execution: { ...summary(model.branch_id, 'model/runtime-fallback/recorded-' + index).execution, group: model.group },
    native_model: index % 2 ? 'glm-5.3' : 'qwen3.8-flash', average_score: index / 10,
  }));
  const rows = qualityModelRows(models, stats.toReversed());
  assert.equal(rows.length, 4, 'Each configured candidate must own its observed row, without an empty duplicate');
  rows.forEach((row, index) => {
    assert.equal(row.configured, models[index]);
    assert.equal(row.summary, stats[index], 'The exact retained summary and drill-down identity must be preserved');
  });
});

test('published positions keep providers, revisions and profiles separate after source refresh', () => {
  const current = configured('writing', 'current/provider-a', 'low');
  const otherProvider = { ...current, model_configuration_id: 'current/provider-b', candidate_index: 1 };
  const observed = summary('writing', 'recorded/provider-a', 'low', 'digest/first');
  const refreshed = summary('writing', 'refreshed/provider-a', 'low', 'digest/refreshed');
  const prior = { ...observed, execution: { ...observed.execution, plan_revision: 0 } };
  const otherPosition = { ...observed, execution: { ...observed.execution, candidate_index: 2 } };
  const differentReasoning = summary('writing', 'recorded/provider-a', 'high', 'digest/high');
  const rows = qualityModelRows([current, otherProvider], [prior, otherPosition, differentReasoning, observed, refreshed]);
  assert.equal(rows.length, 6);
  assert.deepEqual(rows.slice(0, 2).map(row => row.summary), [observed, refreshed]);
  assert.equal(rows[2].configured, otherProvider);
  assert.equal(rows[2].summary, undefined, 'An unused provider must not acquire another candidate’s observations');
  assert.deepEqual(rows.slice(3).map(row => row.summary), [prior, otherPosition, differentReasoning]);
  assert(rows.slice(3).every(row => !row.configured));
  assert.equal(new Set(rows.map(row => row.key)).size, rows.length);
});
