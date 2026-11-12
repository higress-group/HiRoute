import { scenario } from './scenario-selection.mjs';

const busyRead = () => ({ source: 'backend', envelope: { error: {
  category: 'unavailable', code: 'OBSERVATION_UNAVAILABLE', retryable: true,
  details_schema: 'hiroute.observation.read-failure/v1',
  observation_read: { reason: 'reader_busy', retry_after_ms: 100, action: 'retry' },
} } });

export function homeStartupScenarios({ until, control }) {
  const assert = (value, message) => { if (!value) throw new Error(message); };
  const settled = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const health = () => document.querySelector('.sidebar-health');
  const routeButton = () => [...document.querySelectorAll('button')].find(button => button.textContent.trim() === '新建智能路由' && button.getClientRects().length);
  const routeUnavailable = () => [...document.querySelectorAll('.callout.warn')].find(callout => callout.textContent.includes('路由可以查看，暂时不能修改'));
  const readyRoute = async c => {
    c.reset();
    await until(() => health()?.querySelector('.good'), 'initial healthy service');
    document.querySelector('button[aria-label="智能路由"]').click();
    await until(() => routeButton() && !routeButton().disabled, 'writable routing page');
  };
  return [
    scenario('desktop.home.observation-reader-bound', ['home-startup'], 'Home shares the bounded observation reader with its other visible surfaces', async () => {
      const c = control(); const pending = []; let active = 0; let maximum = 0; let released = false;
      try {
        c.reset(); await until(() => health()?.querySelector('.good'), 'initial Home ready'); await settled();
        c.handlers.observation_read = payload => {
          const result = payload.request?.intent?.view === 'sessions'
            ? { sessions: [], next_cursor: null } : c.fixtureResponse('observation_read', payload);
          if (released) return result;
          active++; maximum = Math.max(maximum, active);
          return new Promise(resolve => pending.push(() => { active--; resolve(result); }));
        };
        c.remount();
        await until(() => active >= 2, 'concurrent Home readers');
        await settled();
        assert(maximum <= 2, 'Home bypassed the shared observation-reader bound');
      } finally {
        released = true;
        for (const release of pending.splice(0)) release();
        c.reset(); await settled();
      }
    }),
    scenario('desktop.home.observation-busy-recovers', ['home-startup'], 'A transient busy Home read completes without leaving a failed first-use warning', async () => {
      const c = control(); let attempts = 0;
      try {
        c.reset(); await until(() => health()?.querySelector('.good'), 'initial Home ready'); await settled();
        c.handlers.observation_read = payload => {
          if (payload.request?.intent?.view === 'sessions') assert(payload.request.read_failure_details === true, 'Home did not opt into typed busy errors');
          if (payload.request?.intent?.view === 'sessions' && ++attempts === 1) throw busyRead();
          return c.fixtureResponse('observation_read', payload);
        };
        c.remount();
        await until(() => attempts === 2 && !document.querySelector('.home-read-error'), 'Home recovered its busy read');
        assert(health()?.querySelector('.good'), 'A busy session read lost actual service readiness');
      } finally { c.reset(); await settled(); }
    }),
    scenario('desktop.home.observation-busy-recovers-after-repeated-failures', ['home-startup'], 'Home completes after three busy replies without showing a read error or requiring an action', async () => {
      const c = control(); const attempts = []; let errorShown = false;
      const observer = new MutationObserver(() => { errorShown ||= Boolean(document.querySelector('.home-read-error')); });
      try {
        c.reset(); await until(() => health()?.querySelector('.good'), 'initial Home ready'); await settled();
        c.handlers.observation_read = payload => {
          if (payload.request?.intent?.view === 'sessions' && payload.request.intent.query.session_id == null) {
            attempts.push(performance.now());
            if (attempts.length < 4) throw busyRead();
            return { next_cursor: null, sessions: [{ session_id: 'observation-session/recovered', agent_id: 'codex', request_count: 1, last_request_at_ms: Date.now() - 1000, content_completeness: 'complete' }] };
          }
          return c.fixtureResponse('observation_read', payload);
        };
        observer.observe(document.body, { childList: true, subtree: true });
        c.remount();
        await until(() => errorShown || (attempts.length >= 4 && document.querySelector('.v3-recent')), 'Home completed the busy read');
        assert(!errorShown, `Home exposed a read error after ${attempts.length} attempts`);
        assert(attempts.length === 4, `Unexpected Home attempt count: ${attempts.length}`);
        assert([400, 1000, 2000].every((delay, index) => attempts[index + 1] - attempts[index] >= delay - 10), 'Busy recovery was not paced');
        assert(health()?.querySelector('.good'), 'Busy recovery changed service readiness');
      } finally { observer.disconnect(); c.reset(); await settled(); }
    }),
    scenario('desktop.home.observation-terminal-no-retry', ['home-startup'], 'A non-busy Home read error remains visible without automatic replay', async () => {
      const c = control(); let attempts = 0;
      try {
        c.reset(); await until(() => health()?.querySelector('.good'), 'initial Home ready'); await settled();
        c.handlers.observation_read = payload => {
          if (payload.request?.intent?.view === 'sessions') {
            attempts++;
            throw { source: 'backend', envelope: { error: { category: 'denied', code: 'E_DENIED' } } };
          }
          return c.fixtureResponse('observation_read', payload);
        };
        c.remount();
        await until(() => document.querySelector('.home-read-error'), 'terminal session read error');
        assert(attempts === 1, 'Home replayed a non-busy read failure');
      } finally { c.reset(); await settled(); }
    }),
    scenario('desktop.home.observation-superseded-retry', ['home-startup'], 'Focus coalesces a pending read and leaving Home cancels its recovery', async () => {
      const c = control(); let attempts = 0; let rejectFirst;
      const busy = busyRead();
      try {
        c.reset(); await until(() => health()?.querySelector('.good'), 'initial Home ready'); await settled();
        c.handlers.observation_read = payload => {
          if (payload.request?.intent?.view === 'sessions' && ++attempts === 1) return new Promise((_, reject) => { rejectFirst = reject; });
          return c.fixtureResponse('observation_read', payload);
        };
        c.remount();
        await until(() => rejectFirst, 'first session read held');
        window.dispatchEvent(new Event('focus'));
        window.dispatchEvent(new Event('focus'));
        await settled();
        assert(attempts === 1, 'Focus started another read of the same Home domain');
        document.querySelector('button[aria-label="智能路由"]').click();
        await until(() => routeButton(), 'left Home');
        rejectFirst(busy); rejectFirst = null;
        await new Promise(resolve => setTimeout(resolve, 450));
        assert(attempts === 1, 'Obsolete Home read retried after navigation');
        document.querySelector('button[aria-label="首页"]').click();
        await until(() => attempts === 2 && !document.querySelector('.home-read-error'), 'current Home read completed on return');
      } finally { rejectFirst?.(busy); c.reset(); await settled(); }
    }),
    scenario('desktop.home.no-full-agent-scan', ['home-startup'], 'Cold Home uses a lightweight Agent summary', async () => {
      const c = control();
      try {
        c.reset();
        await until(() => health()?.querySelector('.good'), 'healthy service on Home');
        await settled();
        assert(!c.commands.some(call => call.command === 'agent_snapshot'), 'Home requested all Agent full settings');
        await until(() => c.commands.some(call => call.command === 'agent_home_summary'), 'lightweight Agent summary');
      } finally { c.reset(); await settled(); }
    }),
    scenario('desktop.home.saved-settings-are-not-verified', ['home-startup'], 'Home describes recorded settings neutrally until a full check', async () => {
      const c = control();
      try {
        c.reset();
        c.handlers.observation_read = payload => payload.request?.intent?.view === 'sessions'
          ? { sessions: [], next_cursor: null }
          : c.fixtureResponse('observation_read', payload);
        await until(() => document.body.innerText.includes('已保存设置，使用前检查'), 'recorded settings explanation');
        assert(!document.body.innerText.includes('配置已就绪'), 'Recorded settings were shown as verified readiness');
        assert(!c.commands.some(call => call.command === 'agent_snapshot'), 'Home requested full Agent settings');
      } finally { c.reset(); await settled(); }
    }),
    scenario('desktop.home.incomplete-agent-coverage', ['home-startup'], 'Incomplete Agent coverage cannot become a first-use claim', async () => {
      const c = control();
      try {
        c.reset();
        c.management.sources = [];
        c.desktop.catalog.plans = [];
        c.desktop.catalog.drafts = [];
        c.agents.agents = [{ agent_id: 'agent_codex_default', context_id: 'context/unready',
          configuration_state: 'registered', settings: null, status_error: null }];
        c.handlers.observation_read = payload => payload.request?.intent?.view === 'sessions'
          ? { sessions: [], next_cursor: null } : c.fixtureResponse('observation_read', payload);
        c.remount();
        await until(() => document.body.innerText.includes('暂时无法确认当前配置'), 'incomplete Agent coverage');
        assert(document.querySelector('[data-home-mode="unavailable"]'), 'Incomplete coverage was presented as first use');
        assert(!document.querySelector('.v3-steps'), 'Setup steps implied an empty Agent state');
        assert(document.body.innerText.includes('重试 Agent'), 'Incomplete coverage did not offer a retry');
      } finally { c.reset(); await settled(); }
    }),
    scenario('desktop.home.service-refresh-authority', ['home-startup'], 'Refreshing service display retains health without retaining permission', async () => {
      const c = control(); let release;
      try {
        c.reset();
        await until(() => health()?.querySelector('.good'), 'initial healthy service');
        c.handlers.desktop_snapshot = () => new Promise(resolve => { release = resolve; });
        window.dispatchEvent(new Event('focus'));
        await until(() => release && health()?.textContent.includes('正在更新'), 'held same-target refresh');
        document.querySelector('button[aria-label="智能路由"]').click();
        await until(() => routeButton()?.disabled, 'routing opened during refresh');
        assert(health().querySelector('.good'), 'Healthy display was lost during the refresh');
        assert(routeButton().disabled, 'Previous display authorized a routing mutation during refresh');
        assert(document.body.innerText.includes('正在读取路由'), 'Routing refresh did not show its loading state');
        assert(!routeUnavailable(), 'Opening routing during a healthy refresh falsely warned that the service was not ready');
        release(structuredClone(c.desktop));
        await until(() => !routeButton().disabled && !health().textContent.includes('正在更新'), 'fresh authority');
        assert(!routeUnavailable(), 'Healthy routing retained the service warning');

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
        assert(routeUnavailable(), 'Confirmed read-only routing lost its warning');

        const recovery = structuredClone(c.desktop); recovery.service.recovery_ready = false;
        c.handlers.desktop_snapshot = () => recovery;
        window.dispatchEvent(new Event('focus'));
        await until(() => health()?.textContent.includes('状态待核实') && routeButton().disabled, 'unready recovery');
        assert(!health().querySelector('.good'), 'Unready recovery was presented as healthy');
        const unavailable = structuredClone(c.desktop); unavailable.service.gateway = 'unavailable';
        c.handlers.desktop_snapshot = () => unavailable;
        window.dispatchEvent(new Event('focus'));
        await until(() => health()?.textContent.includes('本机服务不可用') && routeButton().disabled, 'unavailable gateway');
        assert(routeUnavailable(), 'Confirmed unavailable routing lost its warning');

        c.handlers.desktop_snapshot = () => structuredClone(c.desktop);
        window.dispatchEvent(new Event('focus'));
        await until(() => !routeButton().disabled && !health().textContent.includes('正在更新'), 'recovered writable routing');
        assert(!routeUnavailable(), 'Service recovery retained the routing warning');
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
    scenario('desktop.home.observation-authority-loss-masks-cached-content', ['home-startup', 'native-recovery'], 'Trusted connection loss hides resolved excerpts and rejects late activity', async () => {
      const c = control();
      const history = { sessions: [{ session_id: 'observation-session/authority-test', agent_id: 'codex', request_count: 1, last_request_at_ms: Date.now() - 1000, content_completeness: 'complete' }], next_cursor: null };
      let lost = false;
      let release;
      let rejectLate;
      try {
        c.reset();
        c.handlers.desktop_snapshot = () => ({ ...structuredClone(c.desktop), native_connection: lost ? 'recovery_required' : 'ready' });
        c.handlers.recover_native_connection = () => { lost = false; return {}; };
        c.handlers.observation_read = payload => {
          const view = payload.request.intent.view;
          if (view === 'home_value' && lost) throw { source: 'native', code: 'TRUSTED_AUTHORITY_UNAVAILABLE' };
          if (view === 'sessions') return lost ? new Promise((resolve, reject) => { release = resolve; rejectLate = reject; }) : history;
          if (view === 'timeline') return { requests: [{ request_id: 'request/authority-test' }], next_cursor: null };
          if (view === 'catalog') return { contents: [{ content_id: 'authority-test', role: 'user', kind: 'text', state: 'complete', direction: 'request_input', message_occurrence_id: 'prompt' }], next_cursor: null };
          if (view === 'content') return { state: 'available', chunks: [{ text: 'Synthetic protected prompt', original_byte_offset: 0 }], next_cursor: null };
          return c.fixtureResponse('observation_read', payload);
        };
        c.remount();
        await until(() => document.querySelector('.v3-recent')?.textContent.includes('Synthetic protected prompt'), 'resolved protected excerpt');
        lost = true;
        window.dispatchEvent(new Event('focus'));
        await until(() => release && document.querySelector('[data-connection-state="recovery_required"]'), 'trusted connection blocked');
        assert(!document.querySelector('.v3-recent'), 'Resolved observation content stayed visible after authority loss');
        document.querySelector('[data-connection-state="recovery_required"] button').click();
        await until(() => document.querySelector('.v3-recent')?.textContent.includes('Synthetic protected prompt'), 'current authority restored the excerpt');
        rejectLate({ source: 'native', code: 'TRUSTED_AUTHORITY_UNAVAILABLE' });
        release = null;
        await settled();
        assert(document.querySelector('.v3-recent')?.textContent.includes('Synthetic protected prompt'), 'The old failure hid restored content');
        assert(!document.querySelector('.native-connection-recovery'), 'The old failure invalidated the restored connection');
      } finally { c.reset(); release?.(history); await settled(); }
    }),
    scenario('desktop.home.settings-service-refresh', ['home-startup'], 'Settings and navigation agree while the service display refreshes', async () => {
      const c = control(); let release;
      const badge = () => [...document.querySelectorAll('.settings-group')].find(group => group.querySelector('h2')?.textContent === '服务')?.querySelector('.badge');
      try {
        await readyRoute(c);
        document.querySelector('button[aria-label="设置"]').click();
        await until(() => [...document.querySelectorAll('.settings-tabs button')].some(button => button.textContent === '服务与接入'), 'Settings service area');
        [...document.querySelectorAll('.settings-tabs button')].find(button => button.textContent === '服务与接入').click();
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
