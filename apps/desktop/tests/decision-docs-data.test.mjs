import assert from 'node:assert/strict';
import test from 'node:test';
import { decisionDocsData, queryDocsSamples } from '../decision-docs-data.ts';
import { qualityModelRows } from '../src/features/plan-quality-state.ts';

const now = Date.UTC(2026, 9, 8, 9, 40);
test('documentation preserves configured identities and bounded stage facts', () => {
  const { models, plan, customPlan, samples, services } = decisionDocsData('zh', now);
  assert.equal(plan.desired.strategy.classifier.service.id, services[0].id);
  assert.equal(plan.desired.strategy.judgment.degree.simple_threshold_millis, 800);
  assert.equal(plan.desired.strategy.primary_fallback, undefined);
  assert.equal(customPlan.desired.strategy.routing.branches.length, 2);
  assert.deepEqual(models.map(model => model.model_configuration_id), ['qwen-flash', 'qwen-max']);
  const review = customPlan.desired.strategy.routing.branches.find(branch => branch.id === 'review');
  assert.deepEqual(review.primary_candidates, [{ binding_id: 'qwen-max', reasoning: { kind: 'profile', profile: 'high' } }]);
  const reviewMain = samples.find(sample => sample.segment_id === 'docs-review-main');
  assert.equal(reviewMain.model_configuration_id, 'published-qwen-max');
  assert.equal(reviewMain.native_model, 'qwen3.8-max');
  assert.equal(reviewMain.reasoning_profile_id, 'high');
  for (const sample of samples) {
    assert.equal(sample.plan_id, customPlan.agent_plan_id);
    assert.equal(sample.plan_revision, customPlan.agent_plan_revision);
    assert(sample.first_at_ms < sample.last_at_ms && sample.last_at_ms < now);
    assert(sample.first_at_ms > now - 7 * 86400_000);
    assert.equal(sample.execution_evidence_available, false);
    if (sample.assessment) {
      assert(sample.assessment.target_through_ordinal <= sample.last_observed_turn_ordinal);
      assert(sample.assessment.score >= 0 && sample.assessment.score <= 1);
      assert.equal(sample.assessment.reason, undefined);
      assert.equal(sample.assessment.evidence_available, false);
    }
  }
  assert(samples.some(s => s.assessment?.target_through_ordinal < s.last_observed_turn_ordinal));
});

test('partial assessments stay unrated, and configured/observed slots appear only once', () => {
  const { samples } = decisionDocsData('en', now);
  const { summary } = queryDocsSamples(samples, {});
  assert.equal(summary.scored_stage_count, 3);
  assert.equal(summary.unrated_stage_count, 2);
  assert.equal(summary.models.length, 4);
  const maxPrimary = summary.models.filter(model => model.native_model === 'qwen3.8-max' && model.reasoning_profile_id === 'high');
  assert.deepEqual(maxPrimary.map(model => model.execution.executed_branch_id).sort(), ['review', 'writing'], 'The same model/profile in different task branches must keep distinct execution identities');
  const writing = summary.models.find(m => m.reasoning_profile_id === 'medium');
  assert.equal(writing.average_score, .88, 'Partial .37 must not contribute to the average');
  assert.equal(writing.unrated_stage_count, 1);
  const configured = summary.models.map(m => ({ plan_revision: 3, branch_id: m.execution.executed_branch_id,
    group: m.execution.group, candidate_index: 0, model_configuration_id: 'editor-id', display_name: m.native_model,
    reasoning_profile_id: m.reasoning_profile_id }));
  const rows = qualityModelRows(configured, summary.models);
  assert.equal(rows.length, 4, 'Different editor and publication IDs must not manufacture extra rows');
  assert(rows.every(row => row.configured && row.summary));
});

test('documentation drill-down applies execution, competence, scope and time filters', () => {
  const { samples } = decisionDocsData('en', now);
  assert.equal(queryDocsSamples(samples, { competence: 'below_floor' }).samples.length, 1);
  assert.equal(queryDocsSamples(samples, { competence: 'meets_floor' }).samples.length, 2);
  assert.equal(queryDocsSamples(samples, { unrated_only: true }).samples.length, 2);
  assert.equal(queryDocsSamples(samples, { plan_revision: 2 }).samples.length, 0);
  assert.equal(queryDocsSamples(samples, { plan_id: 'unrelated' }).samples.length, 0);
  assert.equal(queryDocsSamples(samples, { session_id: 'session-review' }).samples.length, 2);
  const { summary } = queryDocsSamples(samples, {});
  assert.equal(queryDocsSamples(samples, { execution: summary.models[0].execution }).samples.length, 1);
  assert.equal(queryDocsSamples(samples, { from_ms: now }).samples.length, 0);
  assert.equal(queryDocsSamples(samples, { to_ms: samples.at(-1).last_at_ms }).samples.length, 0);
  assert.deepEqual(queryDocsSamples(samples, { limit: 2 }).samples, samples.slice(0, 2));
  assert.throws(() => queryDocsSamples(samples, { cursor: 'unexpected' }), /CURSOR_UNSUPPORTED/);
});

test('language changes labels, not the synthetic execution facts', () => {
  const zh = decisionDocsData('zh', now), en = decisionDocsData('en', now);
  const facts = data => data.samples.map(({branch_execution, ...s}) => ({ ...s, group: branch_execution.group, floor: branch_execution.policy.floor_millis }));
  assert.deepEqual(facts(zh), facts(en));
  assert.deepEqual(zh.models, en.models);
  assert.notEqual(zh.plan.desired.display_name, en.plan.desired.display_name);
});
