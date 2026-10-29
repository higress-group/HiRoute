import { scenario } from './scenario-selection.mjs';

export function trustProgressScenarios({ fresh, until, control }) {
  const c = control;
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const attention = () => document.querySelector('.trust-attention');
  return [
    scenario('desktop.home.trust-waits-for-recovery', ['home-trust-progress'], 'Suggestions wait for actual service recovery instead of warning during startup', async () => {
      let snapshots = 0; let suggestions = 0;
      c().reset();
      {
        c().handlers.desktop_snapshot = payload => {
          snapshots += 1;
          const value = structuredClone(c().fixtureResponse('desktop_snapshot', payload));
          value.service.recovery_ready = false;
          value.service.mutation_available = false;
          return value;
        };
        c().handlers.observation_read = payload => {
          if (payload.request.intent.view === 'plan_trust_suggestions') { suggestions += 1; throw { code: 'RECOVERY_REQUIRED' }; }
          return c().fixtureResponse('observation_read', payload);
        };
      }
      await until(() => snapshots > 0 && attention()?.textContent.includes('正在读取路由证据'), 'startup recovery waiting state');
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      assert(suggestions === 0, 'Suggestions queried before recovery admitted reads');
      assert(!attention().querySelector('[role="alert"]'), 'Unfinished startup appeared as a suggestion failure');
    }),
    scenario('desktop.home.trust-pending-continues', ['home-trust-progress'], 'Budget-deferred suggestions remain pending and continue automatically in both languages', async () => {
      for (const language of ['zh', 'en']) {
        let reads = 0;
        await fresh(() => {
          localStorage.setItem('hiroute.language', language);
          c().handlers.observation_read = payload => {
            if (payload.request.intent.view !== 'plan_trust_suggestions') return c().fixtureResponse('observation_read', payload);
            return { suggestions: [], unavailable: [], pending: ++reads === 1 ? ['plan/daily'] : [] };
          };
        });
        await until(() => reads === 1 && attention(), 'first suggestion batch');
        assert(!attention().querySelector('[role="alert"]'), 'Deferred work produced a warning');
        assert(!attention().textContent.includes(language === 'zh' ? '当前没有路由调整建议' : 'No routing adjustments are suggested'), 'Pending suggestions were reported as empty');
        assert(attention().querySelector('[role="status"]')?.textContent.includes(language === 'zh' ? '继续读取' : 'Continuing'), 'Pending progress is not disclosed');
        if (language === 'en') {
          const previousVisibility = Object.getOwnPropertyDescriptor(document, 'visibilityState');
          let visibility = 'hidden';
          try {
            Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visibility });
            document.dispatchEvent(new Event('visibilitychange'));
            await new Promise(resolve => setTimeout(resolve, 1250));
            assert(reads === 1, 'Hidden client continued pending suggestion reads');
            visibility = 'visible'; document.dispatchEvent(new Event('visibilitychange'));
          } finally {
            if (previousVisibility) Object.defineProperty(document, 'visibilityState', previousVisibility);
            else delete document.visibilityState;
          }
        }
        await until(() => reads >= 2 && attention().textContent.includes(language === 'zh' ? '当前没有路由调整建议' : 'No routing adjustments are suggested'), 'automatic suggestion continuation');
        assert(reads === 2 && !attention().querySelector('[role="alert"]'), 'Completed continuation retried or retained a warning');
      }
    }),
    scenario('desktop.home.trust-error-and-legacy-retry', ['home-trust-progress'], 'Actual read errors stay actionable and a legacy complete response still clears them', async () => {
      let reads = 0;
      await fresh(() => {
        c().handlers.observation_read = payload => {
          if (payload.request.intent.view !== 'plan_trust_suggestions') return c().fixtureResponse('observation_read', payload);
          if (++reads === 1) throw { code: 'OBSERVATION_UNAVAILABLE' };
          return { suggestions: [], unavailable: [] };
        };
      });
      await until(() => attention()?.querySelector('[role="alert"]'), 'actual suggestion read error');
      const retry = attention().querySelector('button');
      assert(retry && retry.textContent.includes('重试'), 'Actual error lost Retry');
      retry.click();
      await until(() => attention().textContent.includes('当前没有路由调整建议'), 'legacy complete response');
      assert(reads === 2 && !attention().querySelector('[role="alert"]'), 'Legacy retry did not complete');
    }),
  ];
}
