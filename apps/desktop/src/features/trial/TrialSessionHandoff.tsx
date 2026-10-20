import { useEffect, useState } from 'react';
import { observationRead } from '../observation-client';

/** Observes Plan activity after this flow confirmed Apply. A Plan match alone
 * does not prove which Agent sent the request, nor that the session is new. */
export function TrialSessionHandoff({ planId, confirmedAt, language, onWatch }: {
  planId: string; confirmedAt: number; language: 'zh' | 'en'; onWatch?(sessionId: string): void;
}) {
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [retry, setRetry] = useState(0);
  const t = (zh: string, en: string) => language === 'zh' ? zh : en;
  useEffect(() => {
    if (sessionId) return;
    const controller = new AbortController(); let inFlight = false;
    async function read() {
      if (inFlight || document.hidden || controller.signal.aborted) return;
      inFlight = true;
      try {
        const page = await observationRead<{ sessions: { session_id: string }[] }>('sessions', {
          plan_id: planId, from_ms: confirmedAt, to_ms: Math.max(Date.now() + 1, confirmedAt + 1), limit: 1,
          cursor: null, session_id: null, agent_id: null, native_model: null, outcome: null, only_model_switch: false,
        }, controller.signal);
        if (!Array.isArray(page?.sessions)) throw new Error('OBSERVATION_INVALID_RESPONSE');
        if (!controller.signal.aborted) {
          setFailed(false);
          const match = page.sessions[0]?.session_id;
          if (match) setSessionId(current => current ?? match);
        }
      } catch { if (!controller.signal.aborted) setFailed(true); }
      finally { inFlight = false; }
    }
    void read();
    const timer = window.setInterval(() => void read(), 5000);
    const visible = () => { if (!document.hidden) void read(); };
    document.addEventListener('visibilitychange', visible);
    return () => { controller.abort(); window.clearInterval(timer); document.removeEventListener('visibilitychange', visible); };
  }, [planId, confirmedAt, sessionId, retry]);
  return <div className="trial-session-handoff">
    {sessionId ? <><p role="status">{t('发现映射确认后使用此路由的会话。可以打开时间线观察模型表现。', 'Found session activity using this route after mapping was confirmed. Open its timeline to watch model performance.')}</p>{onWatch && <button className="btn btn-primary" onClick={() => onWatch(sessionId)}>{t('观察此会话', 'Watch this session')}</button>}</>
      : failed ? <div className="callout warn" role="alert"><span>{t('暂时无法查找试用会话。可以重试，或从会话列表进入。', 'Trial sessions could not be checked. Retry or open the session list.')}</span><button className="btn" onClick={() => setRetry(n => n + 1)}>{t('重试', 'Retry')}</button></div>
        : <p role="status">{t('正在等待使用此路由的会话…', 'Waiting for session activity using this route…')}</p>}
  </div>;
}
