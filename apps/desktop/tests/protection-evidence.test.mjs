import { test } from 'node:test';
import assert from 'node:assert/strict';
import { releasedProtectionEvidence } from '../src/features/trust/protection-evidence.ts';

const report = { plan_id: 'trial', revision: 'active', active_revision: 8, from_ms: 100, to_ms: 200, branches: [{ branch_id: 'coding' }] };
const cohort = (scope_id, suppressed) => ({ cohort: { scope_id }, counts: { suppressed } });
const summary = () => ({ plan_id: 'trial', session_id: null, plan_revision: 8, covered_from_ms: 100, covered_to_ms: 200, coverage: 'complete', scan_limited: false, cohorts: [cohort('coding', 2), cohort('coding', 3)] });

test('complete matching decision cohorts retain their separate release count, including zero', () => {
  assert.deepEqual(releasedProtectionEvidence(report, summary()), { kind: 'available', count: 5 });
  assert.deepEqual(releasedProtectionEvidence(report, { ...summary(), cohorts: [cohort('coding', 0)] }), { kind: 'available', count: 0 });
});

test('unknown and partial populations do not become zero', () => {
  for (const value of [null, { ...summary(), cohorts: [] }]) assert.deepEqual(releasedProtectionEvidence(report, value), { kind: 'unavailable', reason: 'missing' });
  for (const value of [{ ...summary(), coverage: 'partial' }, { ...summary(), scan_limited: true }]) assert.deepEqual(releasedProtectionEvidence(report, value), { kind: 'unavailable', reason: 'partial' });
});

test('Plan, session, published revision, time and branch identities must all match', () => {
  for (const [patch, reason] of [
    [{ plan_id: 'other' }, 'plan'], [{ session_id: 'one-session' }, 'plan'],
    [{ plan_revision: 7 }, 'revision'], [{ plan_revision: null }, 'revision'],
    [{ covered_from_ms: 101 }, 'time'], [{ covered_to_ms: 201 }, 'time'],
    [{ cohorts: [cohort('coding', 2), cohort('unrelated', 100)] }, 'branch'],
  ]) assert.deepEqual(releasedProtectionEvidence(report, { ...summary(), ...patch }), { kind: 'unavailable', reason });
});

test('all-revision and explicit historical selections agree only with their corresponding read scope', () => {
  assert.deepEqual(releasedProtectionEvidence({ ...report, revision: 'all' }, { ...summary(), plan_revision: null }), { kind: 'available', count: 5 });
  assert.deepEqual(releasedProtectionEvidence({ ...report, revision: 7 }, { ...summary(), plan_revision: 7 }), { kind: 'available', count: 5 });
  assert.equal(releasedProtectionEvidence({ ...report, revision: 7 }, summary()).kind, 'unavailable');
});
