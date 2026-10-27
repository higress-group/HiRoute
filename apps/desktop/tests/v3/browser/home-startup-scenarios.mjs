import { scenario } from './scenario-selection.mjs';

export function homeStartupScenarios({ until, control }) {
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const settled = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const health = () => document.querySelector('.sidebar-health');
  const routeButton = () => [...document.querySelectorAll('button')].find(button => button.textContent.trim() === '新建智能路由' && button.getClientRects().length);
  const readyRoute = async c => {
    c.reset();
    await until(() => health()?.querySelector('.good'), 'initial healthy service');
    document.querySelector('button[aria-label="智能路由"]').click();
    await until(() => routeButton() && !routeButton().disabled, 'writable routing page');
  };
  return [
    scenario('desktop.home.service-refresh-authority', ['home-startup'], 'Refreshing service display retains health without retaining permission', async () => {
      const c = control(); let release;
      try {
        await readyRoute(c);
        c.handlers.desktop_snapshot = () => new Promise(resolve => { release = resolve; });
        window.dispatchEvent(new Event('focus'));
        await until(() => release && health()?.textContent.includes('正在更新'), 'held same-target refresh');
        assert(health().querySelector('.good'), 'Healthy display was lost during the refresh');
        assert(routeButton().disabled, 'Previous display authorized a routing mutation during refresh');
        release(structuredClone(c.desktop));
        await until(() => !routeButton().disabled && !health().textContent.includes('正在更新'), 'fresh authority');

        const notComposed = structuredClone(c.desktop);
        notComposed.service.gateway = 'not_composed';
        c.handlers.desktop_snapshot = () => notComposed;
        window.dispatchEvent(new Event('focus'));
        await until(() => health()?.textContent.includes('状态待核实') && routeButton().disabled, 'returned not-composed state');
        assert(!health().querySelector('.good'), 'A completed not-composed result inherited healthy display');

        notComposed.pending = { plan_id: 'plan/daily', operation_id: 'operation/service-refresh', idempotency_key: 'service-refresh' };
        c.handlers.observe_operation = () => ({ operation_id: 'operation/service-refresh', state: 'running', sequence: 1, cancellable: false, safe_error_code: null });
        window.dispatchEvent(new Event('focus'));
        await until(() => document.querySelector('[data-operation-state="running"]'), 'concurrent exact operation');
        assert(!health().querySelector('.good') && routeButton().disabled, 'Concurrent operation masked a completed not-composed state');
        c.handlers.observe_operation = () => null;

        c.handlers.desktop_snapshot = () => { throw { code: 'READ_UNAVAILABLE' }; };
        window.dispatchEvent(new Event('focus'));
        await until(() => health()?.textContent.includes('状态读取失败') && health().querySelector('button'), 'failed service read');
        c.handlers.desktop_snapshot = () => new Promise(resolve => { release = resolve; });
        release = null;
        health().querySelector('button').click();
        await until(() => release && health()?.textContent.includes('正在更新'), 'retrying failed read');
        assert(!health().querySelector('.good') && routeButton().disabled, 'Retry restored stale display or permission');
        const readOnly = structuredClone(c.desktop); readOnly.trusted_authority = false;
        release(readOnly);
        await until(() => health()?.textContent.includes('只读连接') && routeButton().disabled, 'read-only response');

        const recovery = structuredClone(c.desktop); recovery.service.recovery_ready = false;
        c.handlers.desktop_snapshot = () => recovery;
        window.dispatchEvent(new Event('focus'));
        await until(() => health()?.textContent.includes('状态待核实') && routeButton().disabled, 'unready recovery');
        assert(!health().querySelector('.good'), 'Unready recovery was presented as healthy');
        const unavailable = structuredClone(c.desktop); unavailable.service.gateway = 'unavailable';
        c.handlers.desktop_snapshot = () => unavailable;
        window.dispatchEvent(new Event('focus'));
        await until(() => health()?.textContent.includes('本机服务不可用') && routeButton().disabled, 'unavailable gateway');
      } finally { c.reset(); release?.(structuredClone(c.desktop)); await settled(); }
    }),
    scenario('desktop.home.service-disconnect-generation', ['home-startup'], 'Connection loss invalidates the previous display and ignores an older response', async () => {
      const c = control(); const pending = [];
      try {
        await readyRoute(c);
        c.handlers.desktop_snapshot = () => new Promise(resolve => pending.push(resolve));
        window.dispatchEvent(new Event('focus'));
        await until(() => pending.length === 1 && health()?.querySelector('.good'), 'first pending refresh');
        window.dispatchEvent(new Event('hiroute-native-connection-unavailable'));
        await until(() => pending.length === 2 && health()?.textContent.includes('本机连接需要恢复'), 'connection invalidation');
        assert(!health().querySelector('.good') && routeButton().disabled, 'Lost connection kept stale health or write authority');
        pending[0](structuredClone(c.desktop));
        await settled();
        assert(health().textContent.includes('本机连接需要恢复') && routeButton().disabled, 'Older response replaced the new connection generation');
        const readOnly = structuredClone(c.desktop); readOnly.trusted_authority = false;
        pending[1](readOnly);
        await until(() => health()?.textContent.includes('只读连接') && routeButton().disabled, 'new connection result');
      } finally { c.reset(); for (const resolve of pending) resolve(structuredClone(c.desktop)); await settled(); }
    }),
    scenario('desktop.home.settings-service-refresh', ['home-startup'], 'Settings and navigation agree while the service display refreshes', async () => {
      const c = control(); let release;
      const badge = () => [...document.querySelectorAll('.settings-group')].find(group => group.querySelector('h2')?.textContent === '服务')?.querySelector('.badge');
      try {
        await readyRoute(c);
        document.querySelector('button[aria-label="设置"]').click();
        await until(() => badge()?.classList.contains('good'), 'ready Settings service badge');
        c.handlers.desktop_snapshot = () => new Promise(resolve => { release = resolve; });
        window.dispatchEvent(new Event('focus'));
        await until(() => release && health()?.textContent.includes('正在更新'), 'held Settings refresh');
        assert(badge().classList.contains('good') && badge().textContent.includes('正在更新'), 'Settings showed a warning during a healthy refresh');
        const uncomposed = structuredClone(c.desktop); uncomposed.service.gateway = 'not_composed';
        release(uncomposed);
        await until(() => badge()?.classList.contains('warn') && badge().textContent.includes('状态待核实'), 'actual uncomposed Settings state');
      } finally { c.reset(); release?.(structuredClone(c.desktop)); await settled(); }
    }),
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
