import { scenario } from './scenario-selection.mjs';

export function homeStartupScenarios({ until, control }) {
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const settled = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  return [
    scenario('desktop.home.stale-startup-poll', ['home-startup'], 'An earlier startup poll cannot clear a newer read failure report', async () => {
      const c = control();
      let releasePoll; let reports = 0;
      c.reset();
      c.handlers.startup_status = () => ++reports === 1
        ? new Promise(resolve => { releasePoll = resolve; })
        : { state: 'failed', code: 'DAEMON_START_FAILED', recovery_available: false };
      c.handlers.desktop_snapshot = () => { throw { code: 'RESIDENT_UNAVAILABLE' }; };
      try {
        await until(() => releasePoll && document.querySelector('.startup-recovery')?.dataset.errorCode === 'DAEMON_START_FAILED', 'current startup failure');
        releasePoll({ state: 'ready', recovery_available: false });
        await settled();
        assert(document.querySelector('.startup-recovery')?.dataset.errorCode === 'DAEMON_START_FAILED', 'Earlier startup poll cleared the newer failure report');
      } finally {
        c.reset();
        releasePoll?.({ state: 'ready', recovery_available: false });
        await settled();
      }
    }),
    scenario('desktop.home.stale-startup-failure', ['home-startup'], 'An old startup report cannot replace a newer successful Home read', async () => {
      const c = control();
      let releasePoll; let releaseFailure; let reads = 0; let reports = 0;
      c.reset();
      c.handlers.startup_status = () => new Promise(resolve => {
        if (++reports === 1) releasePoll = resolve;
        else releaseFailure = resolve;
      });
      c.handlers.desktop_snapshot = () => {
        if (++reads === 1) throw { code: 'RESIDENT_UNAVAILABLE' };
        return structuredClone(c.desktop);
      };
      try {
        await until(() => releaseFailure && document.querySelector('.home-read-error'), 'initial read failure and pending startup report');
        window.dispatchEvent(new Event('focus'));
        await until(() => reads === 2 && document.querySelector('.sidebar-health .status-dot.good'), 'newer successful Home refresh');
        releaseFailure({ state: 'failed', code: 'DAEMON_START_FAILED', recovery_available: false });
        await settled();
        assert(!document.querySelector('.startup-recovery'), 'Old startup failure replaced the newer successful read');
        assert(!document.querySelector('.home-read-error'), 'Old failure replaced current Home data');
      } finally {
        c.reset();
        releasePoll?.({ state: 'ready', recovery_available: false });
        releaseFailure?.({ state: 'ready', recovery_available: false });
        await settled();
      }
    }),
    scenario('desktop.home.stale-startup-success', ['home-startup'], 'An old successful Home read cannot clear a newer startup failure', async () => {
      const c = control();
      let releasePoll; let releaseSnapshot; let reads = 0; let reports = 0;
      c.reset();
      c.handlers.startup_status = () => ++reports === 1
        ? new Promise(resolve => { releasePoll = resolve; })
        : { state: 'failed', code: 'DAEMON_START_FAILED', recovery_available: false };
      c.handlers.desktop_snapshot = () => {
        if (++reads === 1) return new Promise(resolve => { releaseSnapshot = resolve; });
        throw { code: 'RESIDENT_UNAVAILABLE' };
      };
      try {
        await until(() => releaseSnapshot && releasePoll, 'pending old Home snapshot');
        window.dispatchEvent(new Event('focus'));
        await until(() => document.querySelector('.startup-recovery')?.dataset.errorCode === 'DAEMON_START_FAILED', 'newer startup failure');
        releaseSnapshot(structuredClone(c.desktop));
        await settled();
        assert(document.querySelector('.startup-recovery')?.dataset.errorCode === 'DAEMON_START_FAILED', 'Old success cleared the newer startup failure');
        assert(document.querySelector('.home-read-error'), 'Old success cleared current failed Home data');
      } finally {
        c.reset();
        releasePoll?.({ state: 'ready', recovery_available: false });
        releaseSnapshot?.(structuredClone(c.desktop));
        await settled();
      }
    }),
  ];
}
