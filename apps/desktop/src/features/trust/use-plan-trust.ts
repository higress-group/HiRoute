import { useEffect, useState } from 'react';
import { observationRead } from '../observation-client';
import { safeDiagnosticCode } from '../../error-code';
import type { TrustReport } from './types';

export type TrustGrouping = 'model' | 'executor';

/** The complete trust report for one model performance scope. */
export function usePlanTrust({ planId, revision, fromMs, toMs, active, groupBy, reload }: {
  planId?: string | null; revision: 'active' | 'all' | number; fromMs?: number; toMs: number; active: boolean; groupBy: TrustGrouping; reload: number;
}) {
  const [report, setReport] = useState<TrustReport | null>(null);
  const [error, setError] = useState('');
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    if (!active || !planId) return;
    let current = true;
    setReport(null); setError('');
    observationRead<TrustReport>('plan_trust', { plan_id: planId, revision, from_ms: fromMs, to_ms: toMs, group_by: groupBy })
      .then(value => { if (current) setReport(value); })
      .catch(cause => { if (current) setError(safeDiagnosticCode(cause, 'OBSERVATION_UNAVAILABLE')); });
    return () => { current = false; };
  }, [planId, revision, fromMs, toMs, active, groupBy, reload, retry]);
  return { report, error, retry: () => setRetry(n => n + 1) };
}
