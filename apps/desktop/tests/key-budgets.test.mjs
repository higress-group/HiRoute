import test from 'node:test';
import assert from 'node:assert/strict';
import { budgetAssignment, budgetDraft, moneyMicros } from '../src/features/key-budgets/types.ts';
import { buildKeyBudgetEdits, createDraft, validateDraftBudgets } from '../src/features/models/model-draft.ts';

test('allowances default off and exact decimal micros never round a monetary amount', () => {
  assert.deepEqual(budgetAssignment(budgetDraft()), { intent: 'unlimited' });
  assert.equal(moneyMicros('0.000001'), 1); assert.equal(moneyMicros('100.000001'), 100000001);
  for (const input of ['1e3', '-1', '0.0000001', 'NaN', 'Infinity', '1000001']) assert.throws(() => moneyMicros(input));
  const draft = { ...budgetDraft(), enabled: true, amount: '100', initial: '25' };
  const assignment = budgetAssignment(draft);
  assert.equal(assignment.policy.renewal.mode, 'reset'); assert.equal(assignment.policy.renewal.period, 'monthly');
  assert.equal(assignment.initial_remaining_micros, 25000000);
  assert.throws(() => budgetAssignment({ ...draft, initial: '101' }));
  assert.throws(() => budgetAssignment({ ...draft, timezone: 'Mars/Olympus' }));
  assert.throws(() => budgetAssignment({ ...draft, renewalMode: 'accumulate', cap: '99' }));
  assert.equal(budgetAssignment({ ...draft, renewalMode: 'accumulate', cap: '200' }).policy.renewal.balance_cap_micros, 200000000);
});

test('saved budgets keep identity and revision; rotation and untouched drafts do not reset them', () => {
  const definition = { budget_id: 'budget/old', revision: 7, display_name: 'API allowance', policy: { currency: 'USD', amount_micros: 100000000, renewal: null } };
  const draft = budgetDraft({ definition });
  const assignment = budgetAssignment({ ...draft, amount: '120' });
  assert.equal(assignment.budget_id, definition.budget_id); assert.equal(assignment.expected_revision, 7);
  assert.equal(assignment.initial_remaining_micros, null);
  const source = { source_id: 'source/a', state: 'ready', models: [], keys: [{ key_id: 'key/a', generation: 2, enabled: true }] };
  const modelDraft = createDraft(source); modelDraft.keys[0].replacement = 'replacement';
  assert.deepEqual(buildKeyBudgetEdits(modelDraft, []), []);
  modelDraft.keys[0].budget = draft;
  assert.deepEqual(buildKeyBudgetEdits(modelDraft, []), [{ key: { kind: 'saved', key_id: 'key/a' }, assignment: budgetAssignment(draft) }]);
  modelDraft.keys[0].budget.amount = 'invalid'; assert.throws(() => validateDraftBudgets(modelDraft));
});

test('new key budgets bind each protected candidate exactly and sharing is explicit', () => {
  const modelDraft = { keys: [{ draftId: 'first', removed: false }, { draftId: 'second', removed: false, budget: { ...budgetDraft(), enabled: true, mode: 'once', amount: '5' } }] };
  const edits = [{ action: 'add', input_candidate: { candidate_ref: 'candidate/first' } }, { action: 'add', input_candidate: { candidate_ref: 'candidate/second' } }];
  const result = buildKeyBudgetEdits(modelDraft, edits);
  assert.equal(result.length, 1); assert.deepEqual(result[0].key, { kind: 'input', candidate_ref: 'candidate/second' });
  assert.throws(() => budgetAssignment({ ...budgetDraft(), enabled: true, mode: 'shared' }));
  assert.deepEqual(budgetAssignment({ ...budgetDraft(), enabled: true, mode: 'shared', sharedId: 'budget/shared', sharedRevision: 3 }), { intent: 'share', budget_id: 'budget/shared', expected_revision: 3 });
});
