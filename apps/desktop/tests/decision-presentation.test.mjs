import { test } from 'node:test';
import assert from 'node:assert/strict';
import { initialAuthMode, protectedInput, sameEndpointOrigin, testKey, decisionReferences } from '../src/features/decision-services/presentation.ts';
import { protocolExamples, protocolCurl } from '../src/features/decision-services/protocol-examples.ts';
import { defaultJudgment, judgmentIssue } from '../src/features/decision-services/types.ts';
import { readFileSync } from 'node:fs';

test('long judgment prompts remain editable and the wire schema has no per-prompt quota', () => {
  const judgment = structuredClone(defaultJudgment);
  judgment.degree.instructions = '决策条件\n'.repeat(5000);
  judgment.competence.criteria[2] = 'Competence standard\n'.repeat(1000);
  assert.equal(judgmentIssue(judgment, 'global', true, 'zh'), null);
  judgment.degree.instructions = '   ';
  assert.match(judgmentIssue(judgment, 'global', true, 'zh').selector, /degree-instructions/);
  const { schemas } = JSON.parse(readFileSync(new URL('../../../decision-extensions/api/decision.openapi.json', import.meta.url), 'utf8')).components;
  for (const definition of [schemas.OrdinalDefinition, schemas.CategoricalDefinition, schemas.AssessmentTarget]) {
    assert.deepEqual(definition.properties.instructions, { type: 'string', minLength: 1 });
  }
  assert.equal(schemas.OrdinalDefinition.properties.levels.items.properties.id.maxLength, 128);
});

test('custom auth does not infer stored schemes, and only new bearer input is prefixed', () => {
  const header = { kind: 'custom', auth_header: { name: 'Authorization', value_secret_ref: 'protected/r1' } };
  assert.equal(initialAuthMode(header), 'header');
  assert.equal(protectedInput(header, 'header', ''), null);
  assert.equal(protectedInput(header, 'header', 'Basic example'), 'Basic example');
  assert.equal(protectedInput(header, 'bearer', 'example'), 'Bearer example');
  assert.equal(protectedInput({ ...header, kind: 'system_one' }, 'bearer', 'example'), 'example');
  assert.equal(protectedInput({ kind: 'custom' }, 'none', 'abandoned'), null);
});
test('credentials are origin scoped and test status is revision scoped', () => {
  assert.equal(sameEndpointOrigin('https://example.test/a', 'https://example.test/b'), true);
  for (const target of ['https://elsewhere.test/b', 'http://example.test/a', 'https://example.test:444/a', 'incomplete']) assert.equal(sameEndpointOrigin('https://example.test/a', target), false);
  assert.notEqual(testKey({ id: 'same', revision: 1 }), testKey({ id: 'same', revision: 2 }));
});
test('references retain each published and draft pin, excluding inactive routing modes', () => {
  const classifier = revision => ({ kind: 'decision_service', service: { id: 'decision/one', revision } });
  const plans = [{ agent_plan_id: 'plan', desired: { display_name: 'Published', strategy: { routing: { classifier: classifier(1) } } } }];
  const drafts = [{ draft_id: 'draft', editor: { display_name: 'Draft', mode: 'custom_branches', branch_routing: { classifier: classifier(2) } } }, { draft_id: 'inactive', editor: { mode: 'fixed_model', branch_routing: { classifier: classifier(2) } } }];
  assert.deepEqual(decisionReferences('decision/one', plans, drafts).map(ref => [ref.key, ref.revision]), [['plan', 1], ['draft', 2]]);
});
test('protocol examples separate the new task decision from its prior assessment target', () => {
  const { first, assessment } = protocolExamples;
  assert.equal(first.request.assessment_target, null);
  assert.equal(first.response.assessment, undefined);
  assert.equal(first.request.decision.kind, 'ordinal');
  assert.equal(assessment.request.assessment_target.from, 0);
  assert.equal(assessment.request.visible_conversation[0].branch_id, undefined);
  assert.equal(assessment.response.decision.choice, 'review');
  for (const key of ['first', 'assessment']) {
    assert.doesNotMatch(protocolCurl(key), /\n\+/);
    const request = JSON.parse(protocolCurl(key).split("--data-raw '")[1].slice(0, -1));
    assert.deepEqual(Object.keys(request).sort(), ['assessment_target', 'decision', 'history_partial', 'latest_user', 'visible_conversation']);
    assert.deepEqual(request, protocolExamples[key].request);
  }
});
