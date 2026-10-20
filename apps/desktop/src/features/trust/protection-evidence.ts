import type { RoutingSummary } from '../RoutingObservations';
import type { TrustReport } from './types';

export type ReleasedProtectionEvidence = { kind: 'available'; count: number } | {
  kind: 'unavailable'; reason: 'missing' | 'partial' | 'plan' | 'revision' | 'time' | 'branch';
};

/** Decisions and scored stages are separate populations, even when their scope agrees. */
export function releasedProtectionEvidence(report: TrustReport, summary: RoutingSummary | null): ReleasedProtectionEvidence {
  const unavailable = (reason: Extract<ReleasedProtectionEvidence, { kind: 'unavailable' }>['reason']): ReleasedProtectionEvidence => ({ kind: 'unavailable', reason });
  if (!summary) return unavailable('missing');
  if (summary.coverage !== 'complete' || summary.scan_limited) return unavailable('partial');
  if (summary.plan_id !== report.plan_id || summary.session_id !== null) return unavailable('plan');
  const revision = report.revision === 'active' ? report.active_revision : report.revision === 'all' ? null : report.revision;
  if (summary.plan_revision !== revision) return unavailable('revision');
  if (summary.covered_from_ms !== report.from_ms || summary.covered_to_ms !== report.to_ms) return unavailable('time');
  if (!summary.cohorts.length) return unavailable('missing');
  const branches = new Set(report.branches.map(branch => branch.branch_id));
  if (summary.cohorts.some(({ cohort }) => !branches.has(cohort.scope_id))) return unavailable('branch');
  let count = 0;
  for (const { counts } of summary.cohorts) {
    if (!Number.isSafeInteger(counts.suppressed) || counts.suppressed < 0) return unavailable('partial');
    count += counts.suppressed;
  }
  return Number.isSafeInteger(count) ? { kind: 'available', count } : unavailable('partial');
}
