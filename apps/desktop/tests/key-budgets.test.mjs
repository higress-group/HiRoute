import test from 'node:test';
import assert from 'node:assert/strict';
import { budgetAssignment, budgetDraft, moneyMicros } from '../src/features/key-budgets/types.ts';
import { changeRenewalPeriod, changeRenewalSchedule, renewalMonthDays, renewalSchedule } from '../src/features/key-budgets/renewal-schedule.ts';
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

const scheduleDraft = (period, anchor, timezone = 'Asia/Shanghai') => ({ ...budgetDraft(), enabled: true, amount: '100', period, anchor, timezone });

test('period controls retain saved original anchors when opening or selecting the same rule', () => {
  for (const [period, anchor] of [['daily', '2030-10-01T08:00'], ['weekly', '2030-10-07T09:00'], ['monthly', '2026-01-31T00:00'], ['yearly', '2024-02-29T10:00']]) {
    const policy = budgetAssignment(scheduleDraft(period, anchor)).policy;
    const definition = { budget_id: 'budget/saved', revision: 4, display_name: 'Credits', policy };
    const saved = budgetDraft({ definition });
    const selected = changeRenewalPeriod(saved, period, new Date('2026-02-10T00:00:00Z'));
    assert.equal(selected, saved);
    assert.equal(changeRenewalSchedule(saved, { time: renewalSchedule(saved).time }), saved);
    assert.deepEqual(budgetAssignment({ ...selected, amount: '120' }).policy.renewal, policy.renewal);
  }
});

test('daily and weekly changes use the allowance time zone instead of a hidden future date', () => {
  const now = new Date('2026-03-02T00:30:00Z');
  const draft = scheduleDraft('monthly', '2030-12-31T09:00', 'America/Los_Angeles');
  assert.equal(changeRenewalPeriod(draft, 'daily', now).anchor, '2026-03-01T09:00');
  assert.equal(changeRenewalPeriod({ ...draft, timezone: 'Asia/Tokyo' }, 'daily', now).anchor, '2026-03-02T09:00');
  const weekly = { ...draft, period: 'weekly' };
  assert.equal(changeRenewalSchedule(weekly, { weekday: 1 }, now).anchor, '2026-02-23T09:00');
  assert.equal(changeRenewalSchedule({ ...weekly, timezone: 'Asia/Tokyo' }, { weekday: 1 }, now).anchor, '2026-03-02T09:00');
  assert.equal(renewalSchedule(changeRenewalSchedule(weekly, { weekday: 0 }, now)).weekday, 0);
});

test('monthly day 31 remains the original day through short months and year boundaries', () => {
  const draft = scheduleDraft('monthly', '2026-01-30T09:00');
  const february = changeRenewalSchedule(draft, { day: 31 }, new Date('2027-02-10T00:00:00Z'));
  assert.equal(february.anchor, '2027-01-31T09:00');
  const april = changeRenewalSchedule(february, { time: '08:30' }, new Date('2027-04-20T00:00:00Z'));
  assert.equal(april.anchor, '2027-03-31T08:30');
  const january = changeRenewalSchedule(draft, { day: 31 }, new Date('2027-01-10T00:00:00Z'));
  assert.equal(january.anchor, '2027-01-31T09:00');
  assert.equal(budgetAssignment(april).policy.renewal.anchor_local, '2027-03-31T08:30:00');
});

test('yearly dates allow leap day without delaying all renewals to the next leap year', () => {
  const now = new Date('2027-02-10T00:00:00Z');
  const february = changeRenewalSchedule(scheduleDraft('yearly', '2026-01-31T09:00'), { month: 2 }, now);
  assert.equal(renewalMonthDays(2), 29);
  assert.equal(february.anchor, '2024-02-29T09:00');
  assert.equal(changeRenewalSchedule(february, { day: 28 }, now).anchor, '2027-02-28T09:00');
  const april = changeRenewalSchedule(scheduleDraft('yearly', '2026-01-31T09:00'), { month: 4 }, now);
  assert.equal(april.anchor, '2027-04-30T09:00');
});

test('incomplete time or time-zone edits remain invalid without crashing or discarding the draft', () => {
  const draft = scheduleDraft('monthly', '2026-01-31T09:00');
  const blank = changeRenewalSchedule(draft, { time: '' });
  assert.equal(renewalSchedule(blank).time, '');
  assert.throws(() => budgetAssignment(blank));
  assert.equal(renewalSchedule(changeRenewalPeriod(blank, 'weekly')).time, '');
  const invalidZone = changeRenewalPeriod({ ...draft, timezone: 'Asia/' }, 'daily');
  assert.equal(invalidZone.timezone, 'Asia/');
  assert.throws(() => budgetAssignment(invalidZone));
});

test('new recurring allowances retain the current day at month-end without Date overflow', () => {
  const now = new Date(2026, 9, 31, 12);
  const draft = budgetDraft(undefined, now);
  assert.equal(draft.anchor, '2026-10-31T00:00');
  assert.equal(now.getDate(), 31);
  assert.equal(now.getHours(), 12);
});
