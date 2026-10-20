import test from 'node:test';
import assert from 'node:assert/strict';
import { planQualityWindow } from '../src/features/plan-quality-window.ts';
import { releasedProtectionEvidence } from '../src/features/trust/protection-evidence.ts';

const DAY = 86_400_000;
for (const period of ['seven_days', 'all', 'one_day']) {
  test(`${period} keeps independently timed reads in the same retained interval`, () => {
    const now = 20 * DAY;
    const scope = planQualityWindow(period, now);
    const effectiveFrom = queryNow => Math.max(scope.from, queryNow - 7 * DAY + 1);
    const report = { plan_id: 'trial', revision: 'active', active_revision: 8, from_ms: effectiveFrom(now + 10), to_ms: scope.to, branches: [{ branch_id: 'coding' }] };
    const summary = { plan_id: 'trial', session_id: null, plan_revision: 8, covered_from_ms: effectiveFrom(now + 500), covered_to_ms: scope.to, coverage: 'complete', scan_limited: false, cohorts: [{ cohort: { scope_id: 'coding' }, counts: { suppressed: 5 } }] };
    assert.deepEqual(releasedProtectionEvidence(report, summary), { kind: 'available', count: 5 });
    assert.equal(scope.to, now + 1);
    if (period === 'one_day') assert.equal(scope.from, now - DAY);
    else assert.equal(scope.from, now - 7 * DAY + 60_000);
    assert.equal(releasedProtectionEvidence(report, { ...summary, covered_from_ms: report.from_ms + 1 }).kind, 'unavailable');
  });
}

test('a window captured beyond its read margin still fails closed after retention moves', () => {
  const now = 20 * DAY;
  const scope = planQualityWindow('all', now);
  const report = { plan_id: 'trial', revision: 'active', active_revision: 8, from_ms: scope.from, to_ms: scope.to, branches: [{ branch_id: 'coding' }] };
  const summary = { plan_id: 'trial', session_id: null, plan_revision: 8, covered_from_ms: now - 7 * DAY + 60_001, covered_to_ms: scope.to, coverage: 'complete', scan_limited: false, cohorts: [{ cohort: { scope_id: 'coding' }, counts: { suppressed: 5 } }] };
  assert.deepEqual(releasedProtectionEvidence(report, summary), { kind: 'unavailable', reason: 'time' });
  assert.equal(planQualityWindow('all', 1000).from, 0);
});
