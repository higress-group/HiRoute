import { useCallback, useEffect, useRef, useState } from 'react';
import { observationRead } from '../observation-client';
import { safeDiagnosticCode } from '../../error-code';
import type { TrustReport } from './types';

export type TrustSuggestionPlan = Pick<TrustReport, 'previous_routing' | 'plan_id' | 'active_revision' | 'announcements'> & { branches: (TrustReport['branches'][number] & { protection_count: number; identity_stabilities: string[] })[] };
export type TrustSuggestionsRead = { status: 'loading' | 'ready' | 'error'; plans: TrustSuggestionPlan[]; error: string; unavailable: { plan_id: string; reason: string }[]; pending?: string[]; refresh(): void };

/** One bounded read supplies Home, navigation and Plan-list attention. */
export function useTrustSuggestions(active: boolean, revision: number | string, homeVisible = true): TrustSuggestionsRead {
  const [plans, setPlans] = useState<TrustSuggestionPlan[]>([]);
  const [status, setStatus] = useState<TrustSuggestionsRead['status']>('loading');
  const [error, setError] = useState('');
  const [unavailable, setUnavailable] = useState<TrustSuggestionsRead['unavailable']>([]);
  const [pending, setPending] = useState<string[]>([]);
  const [retry, setRetry] = useState(0);
  const refresh = useCallback(() => setRetry(n => n + 1), []);
  const wasHomeVisible = useRef(homeVisible);
  useEffect(() => {
    if (homeVisible && !wasHomeVisible.current) refresh();
    wasHomeVisible.current = homeVisible;
  }, [homeVisible, refresh]);
  useEffect(() => {
    if (!active) return;
    const controller = new AbortController(); let inFlight = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let pendingDelay = 1000;
    async function load() {
      if (inFlight || document.visibilityState !== 'visible' || controller.signal.aborted) return;
      if (timer) window.clearTimeout(timer);
      inFlight = true;
      let nextDelay = 300_000;
      try {
        const value = await observationRead<{ suggestions: TrustSuggestionPlan[]; unavailable: TrustSuggestionsRead['unavailable']; pending?: string[] }>('plan_trust_suggestions', {}, controller.signal);
        if (!Array.isArray(value?.suggestions) || !Array.isArray(value?.unavailable)) throw new Error('OBSERVATION_INVALID_RESPONSE');
        const remaining = value.pending === undefined ? [] : value.pending;
        if (!Array.isArray(remaining) || remaining.some(id => typeof id !== 'string' || !id)) throw new Error('OBSERVATION_INVALID_RESPONSE');
        if (!controller.signal.aborted) { setPlans(value.suggestions); setUnavailable(value.unavailable); setPending(remaining); setStatus('ready'); setError(''); }
        if (remaining.length) {
          nextDelay = pendingDelay;
          pendingDelay = Math.min(10_000, pendingDelay * 2);
        } else pendingDelay = 1000;
      } catch (e) {
        if (!controller.signal.aborted) { setStatus('error'); setError(safeDiagnosticCode(e, 'OBSERVATION_UNAVAILABLE')); }
      } finally {
        inFlight = false;
        if (!controller.signal.aborted) timer = window.setTimeout(() => void load(), nextDelay);
      }
    }
    setStatus('loading'); setPending([]); void load();
    const visible = () => { if (document.visibilityState === 'visible') void load(); };
    document.addEventListener('visibilitychange', visible);
    return () => { controller.abort(); if (timer) window.clearTimeout(timer); document.removeEventListener('visibilitychange', visible); };
  }, [active, revision, retry]);
  return { status, plans, error, unavailable, pending, refresh };
}
