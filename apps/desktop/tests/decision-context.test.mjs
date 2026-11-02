import test from 'node:test';
import assert from 'node:assert/strict';
import { contextDraft, contextErrors, contextPolicy, contextSignature, typesafeDestination } from '../src/features/decision-services/context-state.ts';
import { chooseMode, chooseSource, currentBundle, editBundle, newNativeDraft } from '../src/features/decision-services/form-state.ts';

const base = { endpoint: 'https://api.typesafe.ai/v1/systemone', model: 'jev-1.14.0', wire_protocol: 'system_one' };
test('context intent preserves parked custom values without applying them in automatic mode', () => {
  const context = { mode: 'automatic', window: '60000', question: '30000', typesafe_long_context: 'disabled' };
  assert.deepEqual(contextPolicy({ ...base, context }), { mode: 'automatic', typesafe_long_context: 'disabled' });
  assert.deepEqual(contextPolicy({ ...base, context: { ...context, mode: 'custom' } }), { mode: 'custom', window_tokens: 60000, per_question_tokens: 30000, typesafe_long_context: 'disabled' });
  for (const window of ['0', '-1', '1.5', '4294967296', '1e5', '']) assert.ok(contextErrors({ ...base, context: { ...context, mode: 'custom', window } }, 'en').context_window);
  assert.ok(contextErrors({ ...base, context: { ...context, mode: 'custom', question: '60001' } }, 'en').context_question);
  assert.notEqual(contextSignature(base), contextSignature({ ...base, endpoint: base.endpoint + '/proxy' }));
  assert.notEqual(contextSignature(base), contextSignature({ ...base, context: { ...context, mode: 'custom' } }));
});
test('window drafts belong to their selected source or independent connection', () => {
  const a = { source_id: 'a', lineage_digest: 'sha256:a', suggestion: base }, b = { source_id: 'b', lineage_digest: 'sha256:b', suggestion: base };
  const context = { mode: 'custom', window: '65536', question: '32000', typesafe_long_context: 'disabled' };
  let draft = chooseSource(newNativeDraft('decision/new'), a, 'a');
  draft = editBundle(draft, a, { context });
  draft = chooseSource(draft, b, 'b');
  assert.equal(contextDraft(currentBundle(draft, b)).mode, 'automatic');
  draft = chooseMode(draft, 'independent', null);
  assert.equal(contextDraft(currentBundle(draft, null)).mode, 'automatic');
  draft = chooseSource(draft, a, 'a');
  assert.deepEqual(contextDraft(currentBundle(draft, a)), context);
});
test('TypeSafe toggle is shown only for the exact official codec and URL', () => {
  assert.equal(typesafeDestination(base), true);
  for (const endpoint of ['https://api.typesafe.ai.evil.test/v1/systemone', 'http://api.typesafe.ai/v1/systemone', 'https://api.typesafe.ai:444/v1/systemone', 'https://api.typesafe.ai/v1/systemone?q=1', 'https://api.typesafe.ai/v1/systemone?', 'https://api.typesafe.ai/v1/systemone#', 'https://api.typesafe.ai/v1/systemone#x', 'https://user@api.typesafe.ai/v1/systemone', 'https://proxy.test/v1/systemone']) assert.equal(typesafeDestination({ ...base, endpoint }), false, endpoint);
  assert.equal(typesafeDestination({ ...base, wire_protocol: 'openai_decisions' }), false);
});
