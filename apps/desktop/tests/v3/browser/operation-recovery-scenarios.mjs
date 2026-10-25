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
