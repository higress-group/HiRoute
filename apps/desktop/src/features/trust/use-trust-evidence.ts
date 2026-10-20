import { useEffect, useState } from 'react';
import { observationRead } from '../observation-client';
import type { TrustReport } from './types';

export function useTrustEvidence(planId: string | undefined, active: boolean, refreshVersion: number) {
  const [data, setData] = useState<TrustReport | null>(null);
  const [error, setError] = useState(false);
  useEffect(() => {
    if (!planId || !active) return;
    let current = true;
    setData(null); setError(false);
    observationRead<TrustReport>('plan_trust', { plan_id: planId, revision: 'active' })
      .then(value => { if (current) setData(value); })
      .catch(() => { if (current) setError(true); });
    return () => { current = false; };
  }, [planId, active, refreshVersion]);
  return { data, error };
}
