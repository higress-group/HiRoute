import { scenario } from './scenario-selection.mjs';

// Only transport timing is controlled. The real shell owns observation, pending
// recovery and every Home read; no component state or callback is patched.
export function operationRecoveryScenarios({ fresh, until, control, calls }) {
  const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const feedback = () => document.querySelector('[data-operation-phase]');
  const ready = () => document.querySelector('.sidebar-health .status-dot.good');
  const terminal = id => ({ operation_id: id, state: 'succeeded', sequence: 4, cancellable: false, safe_error_code: null });
  return [
    scenario('desktop.operation.immediate-rollback-failure-detail', ['operation-feedback'], 'An ordinary Agent save displays enriched terminal failure without cold pending recovery', async () => {
      await fresh(() => {
        const c = control();
        const agent = c.agents.agents.find(item => item.agent_id === 'agent_codex_default');
        agent.configuration_state = 'not_configured';
        agent.settings.current_selection = null;
        agent.settings.state = 'not_configured';
        c.agents.plans.plans = c.agents.plans.plans.filter(plan => plan.head.status === 'enabled').slice(0, 1);
        // This is the product-session response shape asserted by the native save regression.
        c.handlers.preview_agent_settings = () => ({ preview: { applicable: true, blockers: [] }, mutation: {
          state: 'submitted', operation: {
            operation_id: 'operation/immediate-failure', state: 'rolled_back', sequence: 9, cancellable: false,
            safe_error_code: 'ADAPTER_FAILURE', failure_detail: {
              schema: 'hiroute.operation-failure/v1', step: 'apply_agent_artifacts', code: 'ADAPTER_FAILURE', cause: 'permission_denied',
            },
          },
        } });
      });
      const visible = selector => [...document.querySelectorAll(selector)].filter(node => node.getClientRects().length);
      visible('button').find(node => node.textContent.trim() === 'Agent').click();
      await until(() => visible('.native-list .list-row').some(node => node.textContent.includes('Codex')), 'Agent list');
      visible('.native-list .list-row').find(node => node.textContent.includes('Codex')).click();
      await until(() => document.querySelector('[data-agent-facet="model"]'), 'model facet');
      document.querySelector('[data-agent-facet="model"]').click();
      await until(() => document.querySelector('[role="dialog"] button[type="submit"]')?.disabled === false, 'ordinary save enabled');
      document.querySelector('[role="dialog"] button[type="submit"]').click();
      await until(() => feedback()?.querySelector('[data-error-code="ADAPTER_FAILURE"]'), 'immediate failure detail');
      assert(feedback().innerText.includes('准备 Agent 文件') && feedback().innerText.includes('访问被拒绝'), 'Ordinary save lost its failure guidance');
      assert(calls('preview_agent_settings').length === 1, 'Failure replayed the save');
      assert(calls('observe_operation').length === 0, 'Enriched terminal response needs no frontend recovery poll');
    }),
    scenario('desktop.operation.rollback-failure-detail', ['operation-feedback'], 'Rollback retains the actual failed phase and cause-specific recovery guidance', async () => {
      await fresh(() => {
        const c = control();
        c.desktop.pending = { plan_id: 'plan/daily', operation_id: 'operation/failed-artifact', idempotency_key: 'key/failed-artifact' };
        c.handlers.observe_operation = () => ({
          operation_id: 'operation/failed-artifact', state: 'rolled_back', sequence: 9, cancellable: false,
          safe_error_code: 'ADAPTER_FAILURE', failure_detail: {
            schema: 'hiroute.operation-failure/v1', step: 'apply_agent_artifacts', code: 'ADAPTER_FAILURE',
            cause: 'permission_denied', adapter_context: 'private-native-label',
          },
        });
      });
      await until(() => feedback()?.dataset.operationPhase === 'failed' && ready(), 'actionable rollback feedback');
      const message = feedback().innerText;
      assert(message.includes('准备 Agent 文件') && message.includes('访问被拒绝'), 'Failed phase/cause missing from real Desktop feedback');
      assert(!message.includes('private-native-label'), 'Internal adapter context leaked into user guidance');
      assert(!message.includes('稍后重试'), 'Permission failure suggests an ineffective blind retry');
    }),
    ...[null, 'operation/earlier-draft'].map(pendingId => scenario(`desktop.operation.${pendingId ? 'stale' : 'unknown'}-hint-converges`, ['operation-feedback'], 'A resolved pending hint cannot restart observation while its Home readback is still in flight', async () => {
        let initial = true;
        await fresh(() => {
          const c = control();
          c.desktop.pending = { plan_id: 'plan/daily', operation_id: pendingId, idempotency_key: 'key/earlier-snapshot' };
          c.handlers.observe_operation = () => terminal('operation/current-publication');
          c.handlers.desktop_snapshot = async () => {
            const snapshot = structuredClone(c.desktop);
            if (initial) { initial = false; return snapshot; }
            // A real full refresh may take longer than observation. Returning a
            // fresh identity must not require winning an endless request race.
            snapshot.pending = { ...snapshot.pending, operation_id: 'operation/current-publication', idempotency_key: 'key/current-publication' };
            await pause(1600);
            return snapshot;
          };
        });
        await until(() => calls('desktop_snapshot').length >= 2 && ready() && feedback()?.dataset.operationPhase === 'succeeded', 'terminal feedback and service read converge', 4500);
        await pause(900);
        assert(calls('observe_operation').length === 1, `Resolved hint restarted observation (${pendingId ?? 'unknown identity'})`);
        assert(calls('desktop_snapshot').length === 2, 'Terminal observation repeatedly refreshed Home');
        assert(!feedback()?.textContent.includes('正在确认后台操作'), 'Completed operation returned to pending');
    })),
    scenario('desktop.operation.next-hint-remains-observable', ['operation-feedback'], 'Resolving one hint does not hide a later distinct submission or invent success for an empty lookup', async () => {
      let current = terminal('operation/first');
      await fresh(() => {
        const c = control();
        c.desktop.pending = { plan_id: 'plan/daily', operation_id: null, idempotency_key: 'key/first' };
        c.handlers.observe_operation = () => current;
      });
      await until(() => feedback()?.dataset.operationPhase === 'succeeded' && ready(), 'first operation resolved');
      const observed = calls('observe_operation').length;
      control().desktop.pending = { plan_id: 'plan/daily', operation_id: null, idempotency_key: 'key/second' };
      current = null;
      window.dispatchEvent(new Event('focus'));
      await until(() => calls('observe_operation').length > observed, 'new identity lookup');
      assert(feedback()?.dataset.operationPhase === 'pending', 'Empty lookup borrowed the previous success');
      current = terminal('operation/second');
      await until(() => feedback()?.dataset.operationPhase === 'succeeded', 'second operation resolved');
      const complete = calls('observe_operation').length;
      await pause(1000);
      assert(calls('observe_operation').length === complete, 'Second resolved hint restarted observation');
    }),
  ];
}
