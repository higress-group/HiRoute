import { useEffect, useState } from 'react';
import { observationRead } from '../observation-client';
import { safeDiagnosticCode } from '../../error-code';
import type { TrustReport } from './types';

export type TrustGrouping = 'model' | 'executor';

/** The complete trust report for one model performance scope. */
export function usePlanTrust({ planId, revision, fromMs, toMs, active, groupBy, reload }: {
  planId?: string | null; revision: 'active' | 'all' | number; fromMs?: number; toMs: number; active: boolean; groupBy: TrustGrouping; reload: number;
}) {
  const [retry, setRetry] = useState(0);
  const scope = JSON.stringify([planId, revision, fromMs, toMs, active, groupBy, reload, retry]);
  const [result, setResult] = useState<{ scope: string; report: TrustReport | null; error: string } | null>(null);
  useEffect(() => {
    if (!active || !planId) return;
    let current = true;
    setResult({ scope, report: null, error: '' });
    observationRead<TrustReport>('plan_trust', { plan_id: planId, revision, from_ms: fromMs, to_ms: toMs, group_by: groupBy })
      .then(report => { if (current) setResult({ scope, report, error: '' }); })
      .catch(cause => { if (current) setResult({ scope, report: null, error: safeDiagnosticCode(cause, 'OBSERVATION_UNAVAILABLE') }); });
    return () => { current = false; };
  }, [scope]);
  // Effect cleanup happens after rendering. Never expose the previous read's
  // revision/window even for that render while its replacement is pending.
  return { report: result?.scope === scope ? result.report : null,
    error: result?.scope === scope ? result.error : '', retry: () => setRetry(n => n + 1) };
}
