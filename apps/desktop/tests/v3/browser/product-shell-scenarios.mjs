import { scenario, scenarioCatalog, runScenarios } from './scenario-selection.mjs';
import { productShellRequired } from './scenario-requirements.mjs';
import { extensionDecisionScenarios } from './extension-decision-scenarios.mjs';
import { cProtocolScenarios } from './c-protocol-scenarios.mjs';
import { decisionReviewScenarios } from './decision-review-scenarios.mjs';
import { nativeConnectionScenarios } from './native-connection-scenarios.mjs';
import { presentationScenarios } from './presentation-interactions.mjs';
import { trustProgressScenarios } from './trust-progress-scenarios.mjs';
import { homeStartupScenarios } from './home-startup-scenarios.mjs';
import { trialModelNavigationScenarios } from './trial-model-navigation-scenarios.mjs';
import { operationRecoveryScenarios } from './operation-recovery-scenarios.mjs';
const c = () => window.productShell;
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const assert = (condition, message) => { if (!condition) throw new Error(message); };
const visible = element => Boolean(element?.getClientRects().length);
const all = selector => [...document.querySelectorAll(selector)].filter(visible);
const text = () => document.body.innerText;
const button = label => all('button').find(element => element.textContent.trim() === label);
const calls = command => c().commands.filter(item => item.command === command);
async function until(predicate, label, timeout = 6000) {
  const deadline = Date.now() + timeout;
  while (!predicate()) { if (Date.now() > deadline) throw new Error(`Timed out: ${label}; ${text().slice(-500)}`); await pause(25); }
  await pause(30);
}
async function click(label) { await until(() => button(label), label); button(label).click(); await pause(40); }
async function deviceScan() {
  await click('模型'); await click('添加模型');
  await until(() => all('.add-choice').some(item => item.textContent.includes('扫描本机')), 'scan choice');
  all('.add-choice').find(item => item.textContent.includes('扫描本机')).click();
  await until(() => button('设置订阅连接代理') && !button('设置订阅连接代理').disabled, 'ready device scan');
}
async function newBailianAPI() {
  await click('添加模型');
  await until(() => all('.add-choice').some(item => item.textContent.includes('新建 API 接入')), 'new API entry');
  all('.add-choice').find(item => item.textContent.includes('新建 API 接入')).click();
  await until(() => all('.connection-provider-card').some(item => item.querySelector('strong')?.textContent === '百炼'), 'Bailian provider');
  all('.connection-provider-card').find(item => item.querySelector('strong')?.textContent === '百炼').click();
  await until(() => all('.connection-product-card').some(item => item.querySelector('strong')?.textContent.includes('Coding Plan')), 'Bailian Coding Plan');
  all('.connection-product-card').find(item => item.querySelector('strong')?.textContent.includes('Coding Plan')).click();
  await until(() => all('[role="dialog"] input[type="password"]').length === 1 && !button('检查并继续')?.disabled, 'visible API configuration');
  return all('[role="dialog"] input[type="password"]')[0];
}
async function fresh(setup = () => {}) {
  c().reset(); setup(); await until(() => visible(document.querySelector('.sidebar-health .status-dot.good')), 'ready DesktopApp');
}
function setInput(input, value) {
  assert(input, 'Expected an editable input');
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}
const nativeField = name => all('.decision-connection-form')[0]?.querySelector(`[data-native-field="${name}"]`);
function choose(select, value) { select.value = value; select.dispatchEvent(new Event('change', { bubbles: true })); }
const decisionOrder = () => c().commands.map(item => item.command).filter(command => ['save_decision_service_v2', 'decision_services_v2', 'test_decision_service_v2'].includes(command));
// Synthetic daemon state for the V2 decision commands; a submitted key becomes an opaque reference.
// Retained revisions keep their sources related. Saves must name exactly the newly selected source,
// deletions exactly the related sources, so an over- or under-registering request is rejected.
const sharedSourceOf = service => service?.connection.kind === 'native' && service.connection.authentication.kind === 'shared_source' ? service.connection.authentication.source_id : null;
function decisionStore(services = [], sources = []) {
  const store = { services: structuredClone(services), sources: structuredClone(sources), retained: structuredClone(services), rejected: [], test: () => ({ outcome: 'passed' }) };
  for (const source of sources) {
    if (c().management.sources.some(item => item.source_id === source.source_id)) continue;
    const native = structuredClone(c().management.sources.find(item => item.authentication.kind === 'bearer'));
    c().management.sources.push({ ...native, source_id: source.source_id, display_name: source.display_name,
      models: native.models.map((model, index) => ({ ...model, model_ref: source.source_id + '/model/' + index, binding_id: source.source_id + '/binding/' + index })),
      keys: native.keys.map((key, index) => ({ ...key, key_id: source.source_id + '/key/' + index, model_statuses: [] })),
      target: { ...native.target, authority: source.endpoint_host } });
  }
  const related = id => [...new Set(store.retained.filter(item => item.id === id).map(sharedSourceOf).filter(Boolean))];
  store.view = id => {
    const service = store.services.find(item => item.id === id);
    if (!service) return undefined;
    const view = c().decisionView(service, store.sources);
    // The fixture projection resolves a source summary (or its missing form) from current sources.
    const summary = sourceId => c().decisionView({ id, revision: 0, name: '', connection: { kind: 'native', authentication: { kind: 'shared_source', source_id: sourceId } } }, store.sources).source;
    return { ...view, related_sources: related(id).map(summary) };
  };
  c().handlers.decision_services_v2 = () => ({ services: store.services.map(service => store.view(service.id)) });
  c().handlers.decision_connection_sources = () => ({ sources: structuredClone(store.sources) });
  c().handlers.save_decision_service_v2 = ({ input }) => {
    const { change, secret } = input;
    if ((store.services.find(item => item.id === change.id)?.revision ?? 0) !== change.expected_revision) throw { code: 'DECISION_REVISION_CONFLICT' };
    const closure = change.service ? [sharedSourceOf(change.service)].filter(Boolean) : related(change.id);
    const submitted = change.expected_source_revisions.map(item => item.source_id);
    if (submitted.length !== closure.length || closure.some(id => !submitted.includes(id))) {
      store.rejected.push({ expected: closure, submitted });
      throw { code: 'FIXTURE_SOURCE_CLOSURE_MISMATCH' };
    }
    if (change.expected_source_revisions.some(item => store.sources.find(source => source.source_id === item.source_id)?.revision !== item.revision)) throw { code: 'DECISION_SOURCE_REVISION_CONFLICT' };
    store.services = store.services.filter(item => item.id !== change.id);
    if (change.service) {
      const saved = structuredClone(change.service);
      const header = saved.connection.kind === 'native' ? saved.connection.authentication.auth_header : saved.connection.auth_header;
      if (header && secret) header.value_secret_ref = `protected/${saved.id}/r${saved.revision}`;
      store.services.push(saved); store.retained.push(structuredClone(saved));
    } else store.retained = store.retained.filter(item => item.id !== change.id);
    return { operation_id: 'operation/decision', accepted_digest: 'sha256:' + '5'.repeat(64), state: 'succeeded' };
  };
  c().handlers.test_decision_service_v2 = ({ input }) => {
    const view = store.view(input.id);
    if (!view || view.service.revision !== input.revision || view.configuration_digest !== input.expected_configuration_digest) throw { code: 'DECISION_REVISION_CONFLICT' };
    // `store.test` may change credentials mid-call; the result still names the state it admitted.
    return { schema: 'hiroute.decision-service-test-result/v2', failure_code: null, duration_millis: 20, ...store.test(input),
      tested_configuration_digest: view.configuration_digest, tested_credential_state_revision: view.credential_state_revision };
  };
  return store;
}
const operation = (state = 'running', id = 'operation/shell') => ({ operation_id: id, state, sequence: state === 'succeeded' ? 2 : 1, cancellable: false, safe_error_code: null });
const feedback = () => document.querySelector('[data-operation-phase]');
async function pending() {
  await fresh(() => {
    c().desktop.pending = { plan_id: 'plan/daily', operation_id: 'operation/shell', idempotency_key: 'key/shell' };
    c().handlers.observe_operation = () => operation();
  });
  await until(() => feedback()?.dataset.operationState === 'running', 'observed operation');
}
async function routing() {
  await click('智能路由');
  await until(() => all('.master-list .list-row').length, 'saved routes');
  all('.master-list .list-row')[0].click();
  await until(() => document.querySelector('.plan-identity-fields input'), 'route editor');
}
const scenarios = [
  ...trialModelNavigationScenarios(scenario),
  scenario('desktop.operation.hidden-summaries-defer', ['operation-feedback'], 'Terminal confirmation does not scan a hidden Agent page or unrelated Home summaries', async () => {
    await fresh();
    await click('Agent');
    await until(() => all('.native-list .list-row').length > 0 && !text().includes('正在读取本地 Agent'), 'Agent page loaded');
    await click('会话');
    const reads = calls('agent_snapshot').length;
    const summaries = calls('agent_home_summary').length;
    c().desktop.pending = { plan_id: 'plan/daily', operation_id: null, idempotency_key: 'key/deferred-summaries' };
    c().handlers.observe_operation = () => operation('succeeded', 'operation/deferred-summaries');
    window.dispatchEvent(new Event('focus'));
    await until(() => feedback()?.dataset.operationPhase === 'succeeded', 'terminal operation confirmed');
    await pause(150);
    assert(calls('agent_snapshot').length === reads, 'Hidden summary or Agent page performed a full scan');
    await click('首页');
    await until(() => calls('agent_home_summary').length === summaries + 1, 'Invalidated summary reread when opened');
    assert(calls('agent_snapshot').length === reads, 'Home summary requested full Agent settings');
  }),
  ...homeStartupScenarios({ until, control: c }),
  scenario('desktop.routing.associations-remain-unknown-while-loading', ['routing-editor'], 'The visible routing editor does not describe missing or partial Agent associations as unused', async () => {
    let release;
    await fresh(() => { c().handlers.agent_model_associations = () => new Promise(resolve => { release = resolve; }); });
    await routing();
    await until(() => release && text().includes('正在读取 Agent 关联'), 'association read is pending');
    assert(!text().includes('尚未被任何 Agent 用作模型路由'), 'Loading was presented as no associations');
    release({ complete: false, agents: [] });
    await until(() => text().includes('部分 Agent 关联暂不可用'), 'partial association result');
    assert(!text().includes('尚未被任何 Agent 用作模型路由'), 'Partial coverage was presented as no associations');
  }),
  ...operationRecoveryScenarios({ fresh, until, control: c, calls }),
  ...trustProgressScenarios({ fresh, until, control: c }),
  ...nativeConnectionScenarios({ fresh, click, until, text, calls, control: c }),
  ...extensionDecisionScenarios({ fresh, routing, click, until, all, calls, control: c }),
  ...cProtocolScenarios({ fresh, routing, click, until, all, text, calls, setInput, control: c }),
  ...decisionReviewScenarios({ fresh, routing, click, until, all, text, calls, setInput, nativeField, choose, decisionStore, control: c }),
  ...presentationScenarios,
  scenario('desktop.routing.recovered-mapping-session', ['routing-editor', 'sessions'], 'A resumed Agent mapping opens the observed session through the shell and releases its dialog', async () => {
    const sessionId = 'session/recovered-mapping';
    for (const action of ['watch', 'list', 'dirty-watch']) {
    await fresh(() => {
      c().handlers.preview_agent_settings = ({ input }) => {
        const agent = c().agents.agents.find(item => item.context_id === input.spec.context_id);
        agent.settings = { ...agent.settings, state: 'configured', current_selection: structuredClone(input.spec.model.settings), operation_id: 'operation/map', operation_state: 'succeeded' };
        return { preview: { applicable: true, blockers: [] }, mutation: { state: 'applied', operation: operation('succeeded', 'operation/map') } };
      };
      c().handlers.observation_read = payload => {
        const intent = payload.request.intent;
        if (intent.view === 'sessions' && intent.query.plan_id) return { sessions: [{ session_id: sessionId }], next_cursor: null };
        return c().fixtureResponse('observation_read', payload);
      };
    });
    await routing();
    if (action === 'dirty-watch') { setInput(document.querySelector('.plan-identity-fields input'), 'Retained route draft'); await pause(40); }
    await click('映射 Agent'); await click('检查并映射 Agent'); await click('刷新结果');
    await until(() => text().includes('发现映射确认后使用此路由的会话'), 'verified mapping and observed activity');
    assert(button('观察此会话'), 'Recovered mapping has no Watch this session action');
    await click(action === 'list' ? '查看会话时间线' : '观察此会话');
    if (action === 'dirty-watch') {
      await until(() => all('dialog[open]').length, 'unsaved route guard');
      await click('继续编辑');
      assert(button('观察此会话') && document.querySelector('.plan-identity-fields input').value === 'Retained route draft', 'Cancelled navigation lost the mapping or route draft');
      assert(!calls('observation_read').some(call => call.payload.request.intent.view === 'timeline' && call.payload.request.intent.query.session_id === sessionId), 'Cancelled navigation opened the session');
      await click('观察此会话'); await click('不保存');
    }
    await until(() => calls('observation_read').some(call => call.payload.request.intent.view === 'timeline' && (action === 'list' || call.payload.request.intent.query.session_id === sessionId)), 'requested session view opened');
    assert(!all('[role="dialog"]').length, 'Mapping dialog remained over Sessions');
    assert(!document.querySelector('.sidebar')?.inert && document.body.style.overflow !== 'hidden', 'Mapping dialog kept navigation inert or scroll locked');
    await click('智能路由');
    assert(!all('.trial-mapping').length, 'Returning to Routing resurrected the mapping dialog');
    assert(calls('preview_agent_settings').length === 1, 'Watching activity submitted the mapping twice');
    }
  }),
  scenario('desktop.routing.two-context-conflict', ['routing-editor'], 'A rival context revision refuses stale publication and preserves the current draft', async () => {
    await fresh(); await routing();
    const name = document.querySelector('.plan-identity-fields input');
    const draftName = 'Retained after another tab changes the route';
    setInput(name, draftName); await pause(40);
    const rival = structuredClone(c().desktop);
    const plan = rival.catalog.plans[0]; plan.head.head_revision += 1;
    plan.desired.display_name = 'Saved by a second context';
    c().handlers.desktop_snapshot = () => structuredClone(rival);
    c().handlers.preview_plan_editor = ({ input }) => {
      assert(input.expected_head_revision !== undefined, 'Revision was omitted from the draft');
      throw { kind: 'native', code: 'REVISION_CONFLICT' };
    };
    const reads = calls('desktop_snapshot').length;
    await click('保存草稿');
    await until(() => text().includes('保留') && text().includes('另一会话'), 'revision conflict copy');
    assert(calls('desktop_snapshot').length > reads, 'Conflict did not reload current facts');
    assert(document.querySelector('.plan-identity-fields input').value === draftName, 'Rival revision discarded the local draft');
    assert(calls('preview_plan_editor').length === 1, 'Stale publication was retried automatically');
  }),
  scenario('desktop.models.subscription-login.callback-modes', ['subscription-proxy', 'subscription-login'], 'Automatic callback opens the browser; manual fallback explains the conflict before opening and keeps callback text protected', async () => {
    const browser = window.fixtureTransport === 'console';
    const originalClick = HTMLAnchorElement.prototype.click;
    let opened = [];
    // Console navigation is a browser anchor, never native IPC. Observe the real host
    // boundary while keeping this synthetic authorization URL inside the fixture.
    if (browser) HTMLAnchorElement.prototype.click = function () {
      opened.push({ url: this.href, target: this.target, rel: this.rel });
    };
    const openCount = () => browser ? opened.length : calls('open_external_url').length;
    try {
    for (const language of (window.cpaCallbackLanguages ?? ['zh', 'en'])) {
      for (const provider of ['codex', 'claude']) {
        for (const manual of [false, true]) {
          let session;
          opened = [];
          await fresh(() => {
            session = { provider, login_ref: `login/fixture-${provider}`, status: 'pending',
              authorization_url: 'https://example.invalid/oauth/authorize',
              ...(manual ? { reason_code: 'SUBSCRIPTION_LOGIN_MANUAL_CALLBACK' } : {}) };
            c().handlers.manage_subscription_login = ({ request }) => ({ schema: 'hiroute.subscription-login-result/v1',
              sessions: request.action === 'list' ? [] : [{ ...session, ...(request.action === 'cancel' ? { status: 'cancelled' } : {}) }] });
            c().handlers.open_external_url = () => undefined;
            c().handlers.submit_subscription_login_callback = () => {
              session = { ...session, status: 'authorized', reason_code: undefined,
                candidate: { candidate_ref: 'candidate/cpa/fixture', candidate_revision: 1 } };
              return { schema: 'hiroute.subscription-login-result/v1', sessions: [session] };
            };
          });
          localStorage.setItem('hiroute.language', language); c().remount();
          const label = (zh, en) => language === 'zh' ? zh : en;
          await click(label('模型', 'Models')); await click(label('添加模型', 'Add models'));
          await until(() => all('.add-choice').some(item => item.textContent.includes(label('连接订阅', 'Connect subscription'))), 'subscription sign-in entry');
          all('.add-choice').find(item => item.textContent.includes(label('连接订阅', 'Connect subscription'))).click();
          const name = provider === 'codex' ? 'Codex' : 'Claude Code';
          await until(() => all('.oc-status-row').some(row => row.querySelector('strong')?.textContent === name && !row.querySelector('button')?.disabled), 'ready provider sign-in');
          all('.oc-status-row').find(row => row.querySelector('strong')?.textContent === name).querySelector('button').click();
          await until(() => { const manualAction = button(label('手动完成授权', 'Complete authorization manually')); return manualAction && !manualAction.disabled; }, 'completed sign-in action and pending callback mode');
          assert(openCount() === (manual ? 0 : 1), 'Manual conflict opened a browser before guidance or automatic mode did not open it');
          assert(text().includes(manual ? label('无法自动接收浏览器授权', 'Automatic callback reception is unavailable') : label('HiRoute 会自动接收结果', 'HiRoute will receive the result automatically')), 'Callback mode guidance missing');
          if (manual) { await click(label('打开登录页面', 'Open sign-in page')); assert(openCount() === 1, 'Explicit manual browser action did not dispatch'); }
          if (browser) {
            assert(calls('open_external_url').length === 0, 'Console navigation invoked native IPC');
            assert(opened.length === 1 && opened[0].url === session.authorization_url
              && opened[0].target === '_blank' && opened[0].rel.includes('noreferrer'), 'Console authorization anchor lost its URL or navigation boundary');
          }
          await click(label('手动完成授权', 'Complete authorization manually'));
          await until(() => all('input[type="password"]').length === 1, 'protected manual callback form');
          const dialog = all('[role="dialog"]')[0];
          const input = all('input[type="password"]')[0];
          assert(dialog.scrollWidth <= dialog.clientWidth + 1, 'Sign-in dialog overflows horizontally');
          assert(input.getBoundingClientRect().width > 150, 'Callback field is squeezed');
          assert(text().includes(label('返回页面无法打开时', 'even if the return page cannot load')), 'Manual copy lacks the unavailable-page instruction');
          setInput(input, 'synthetic-callback-material');
          await click(label('提交授权', 'Submit authorization'));
          await until(() => button(label('检查并选择模型', 'Check and choose models')), 'confirmed authorized state');
          assert(calls('submit_subscription_login_callback').length === 1 && calls('submit_subscription_login_callback')[0].payload.callback === 'synthetic-callback-material', 'Manual material did not enter the native protected command exactly once');
          assert(!all('input[type="password"]').length, 'Submitted callback form retained its value');
          assert(!JSON.stringify(calls('manage_subscription_login')).includes('synthetic-callback-material'), 'Callback material entered ordinary control requests');
          assert(!calls('check_subscription').length && !calls('apply_compute_save').length, 'Login automatically checked or saved a connection');
          // Leave the empty fallback form visible for production-style screenshots.
          if (provider === 'claude' && manual) {
            session = { ...session, login_ref: `login/fixture-${provider}-next`, status: 'pending', reason_code: 'SUBSCRIPTION_LOGIN_MANUAL_CALLBACK', candidate: undefined };
            all('.oc-status-row').find(row => row.querySelector('strong')?.textContent === name).querySelector('button').click();
            await until(() => button(label('手动完成授权', 'Complete authorization manually')), 'fallback form after status observation');
            await click(label('手动完成授权', 'Complete authorization manually'));
          }
        }
      }
    }
    } finally { HTMLAnchorElement.prototype.click = originalClick; }
  }),
  scenario('desktop.models.subscription-scan.append-to-login', ['subscription-proxy', 'subscription-login'], 'Independent sign-in from an append scan creates a new connection instead of retaining the previous append target', async () => {
    let previous;
    await fresh(() => {
      previous = JSON.stringify(c().management.sources.find(source => source.source_id === 'source/codex/subscription'));
      const borrowed = c().fixtureResponse('compute_subscriptions', {}).candidates[0];
      const pending = { ...borrowed, existing_source_id: undefined, display_name: 'New independent account', candidate: { candidate_ref: 'candidate/independent-new-account', candidate_revision: 1 } };
      const base = c().fixtureResponse('check_subscription', {});
      const checked = { ...base.checked_candidate, existing_source_id: undefined, display_name: pending.display_name,
        candidate: { ...pending.candidate, candidate_revision: 2 },
        models: [{ ...base.checked_candidate.models[0], model_ref: 'model-ref/independent-new-account' }],
      };
      c().handlers.compute_subscriptions = () => ({ discovery_state: 'complete', candidates: [borrowed, pending] });
      c().handlers.manage_subscription_login = ({ request }) => ({ schema: 'hiroute.subscription-login-result/v1', sessions: request.provider === 'codex'
        ? [{ provider: 'codex', login_ref: 'login/independent-new-account', status: 'authorized', candidate: pending.candidate }]
        : [] });
      c().handlers.check_subscription = () => ({ ...base, candidate: checked.candidate, checked_candidate: checked });
    });
    await click('模型'); await click('向此接入添加模型');
    await until(() => button('返回') && text().includes('Codex 本机订阅'), 'append subscription detail');
    await click('返回'); await click('连接订阅（推荐）');
    await click('检查并选择模型');
    assert(all('[role="dialog"] h2').some(item => item.textContent === '连接订阅') && !document.querySelector('[data-hermes-source-scope]'), 'Independent subscription check leaked the device-scan title or Hermes controls');
    await click('检查订阅');
    await until(() => button('保存接入'), 'new account checked');
    await click('保存接入');
    await until(() => calls('preview_compute_save').length || document.querySelector('[data-error-code]'), 'save outcome');
    assert(calls('preview_compute_save').length === 1, 'New independent account kept the previous append target and cannot save');
    const change = calls('preview_compute_save')[0].payload.change;
    assert(!change.edit && change.subject.candidate.candidate_ref === 'candidate/independent-new-account', 'New login was saved as an append to the old connection');
    assert(JSON.stringify(change.selected_model_refs) === '["model-ref/independent-new-account"]', 'New login selection reused old membership');
    await until(() => calls('apply_compute_save').length === 1, 'new connection apply');
    assert(JSON.stringify(c().management.sources.find(source => source.source_id === 'source/codex/subscription')) === previous, 'Previous connection was changed');
  }),
  scenario('desktop.models.subscription-scan.recommended-login', ['subscription-proxy', 'subscription-login'], 'Scan explains native credential refresh and opens independent sign-in without authorizing or saving', async () => {
    for (const language of ['zh', 'en']) {
      for (const previousScale of [1, 1.5, 2]) {
        await fresh(() => { c().handlers.manage_subscription_login = async () => { await pause(150); return { schema: 'hiroute.subscription-login-result/v1', sessions: [] }; }; });
        const saved = JSON.stringify(c().management);
        localStorage.setItem('hiroute.language', language);
        localStorage.setItem('hiroute.text-scale', String(previousScale));
        c().remount();
        const label = (zh, en) => language === 'zh' ? zh : en;
        await click(label('模型', 'Models')); await click(label('添加模型', 'Add models'));
        const scanLabel = label('扫描本机', 'Scan this device');
        await until(() => all('.add-choice').some(item => item.textContent.includes(scanLabel)), 'scan choice');
        all('.add-choice').find(item => item.textContent.includes(scanLabel)).click();
        const entryLabel = label('连接订阅（推荐）', 'Connect subscription (recommended)');
        await until(() => button(entryLabel), 'recommended independent sign-in entry');
        assert(document.querySelector('.hr-ui').dataset.textScale === '1', 'Scan retained a previous enlarged text preference');
        const note = document.querySelector('.subscription-reuse-note');
        assert(note.textContent.includes(label('依赖 Codex / Claude Code', 'depends on Codex / Claude Code')) && note.textContent.includes(label('自动刷新登录凭据', 'automatically refresh sign-in credentials')), 'Scan does not explain both credential refresh owners');
        assert(!text().includes('自动续期'), 'Paid-subscription renewal copy remains');
        const entry = button(entryLabel);
        entry.scrollIntoView({ block: 'center' }); await pause(40);
        const bounds = note.getBoundingClientRect();
        const buttonBounds = entry.getBoundingClientRect();
        const footer = document.querySelector('.modal-foot').getBoundingClientRect();
        assert(note.scrollWidth <= note.clientWidth + 1 && entry.scrollWidth <= entry.clientWidth + 1, 'Refresh guidance or recommendation overflows horizontally');
        assert(buttonBounds.left >= bounds.left && buttonBounds.right <= bounds.right && buttonBounds.bottom <= footer.top + 1, 'Recommendation is outside the note or obscured by the footer');
        assert(footer.bottom <= innerHeight + 1, 'Fixed scan actions left the viewport');
        for (const copy of note.querySelectorAll('p')) {
          const range = document.createRange(); range.selectNodeContents(copy);
          assert([...range.getClientRects()].every(rect => rect.left >= bounds.left - 1 && rect.right <= bounds.right + 1), 'Refresh guidance text is clipped');
        }
        const closes = calls('close_subscription_check').length;
        await click(entryLabel);
        await until(() => all('[role="dialog"] h2').some(item => item.textContent === label('连接订阅', 'Connect subscription')), 'independent sign-in dialog');
        await until(() => calls('manage_subscription_login').length > 0, 'existing login session listing');
        await until(() => all('.oc-scan-detail .btn-primary').some(item => !item.disabled), 'login session list completed');
        await pause(40);
        assert(text().includes(label('自动刷新登录凭据', 'automatically refresh sign-in credentials')), 'Sign-in copy does not explain automatic credential refresh');
        const intro = document.querySelector('.v3-select-intro');
        const body = intro.closest('.modal-body');
        assert(intro.getBoundingClientRect().top >= body.getBoundingClientRect().top - 1, 'Initial login focus scrolled the credential-refresh explanation out of view');
        assert(calls('close_subscription_check').length === closes + 1, 'Scan check intent was not released');
        assert(calls('manage_subscription_login').every(item => item.payload.request.action === 'list'), 'Opening the recommendation automatically started authorization');
        assert(calls('check_subscription').length === 0 && calls('apply_compute_save').length === 0 && calls('subscription_proxy_apply').length === 0 && JSON.stringify(c().management) === saved, 'Navigation mutated a connection or proxy');
        all('summary').find(item => item.textContent.includes(label('复用本机登录', 'Reuse a local sign-in'))).click(); await pause(40);
        await click(label('扫描本机登录', 'Find local sign-ins'));
        await until(() => button(entryLabel), 'return to fresh device scan');
      }
    }
  }),
  scenario('desktop.models.subscription-proxy.inline-cancel', ['subscription-proxy'], 'Proxy editing stays in the scan and cancelling retains its results and selected subscription', async () => {
    const saved = { config: { revision: 'fixture', policy: { mode: 'manual', url: 'http://proxy.example:8080', no_proxy: 'example.org' } }, applied: true };
    await fresh(() => { c().handlers.subscription_proxy_status = () => structuredClone(saved); });
    await click('设置');
    assert(!text().includes('订阅连接代理') && calls('subscription_proxy_status').length === 0, 'Settings still owns the proxy entry');
    await deviceScan();
    await until(() => button('调整模型'), 'scanned subscription');
    assert(document.querySelector('[data-hermes-source-scope]'), 'Device scan lost its optional Hermes profile entry');
    const dialog = document.querySelector('[role="dialog"]');
    const list = document.querySelector('.oc-scan-list');
    await click('设置订阅连接代理');
    await until(() => document.querySelector('[aria-label="订阅代理地址"]'), 'saved manual proxy');
    assert(document.querySelector('[role="dialog"]') === dialog && document.querySelector('.oc-scan-list') === list, 'Proxy entry replaced the scan');
    assert(all('.subscription-proxy-scan-content button').every(item => item.matches(':disabled')), 'Scan actions remain enabled during proxy editing');
    assert(text().includes('全部订阅及订阅检查') && text().includes('正在执行的请求和任务可能中断'), 'Scope or restart impact missing');
    assert(getComputedStyle(document.querySelector('.subscription-proxy-control')).display === 'flex', 'Proxy form layout is missing from the production stylesheet');
    assert(getComputedStyle(document.querySelector('.subscription-proxy-scan-content')).borderTopWidth === '0px', 'Disabled scan inherited an unintended fieldset border');
    const panelBounds = document.querySelector('.subscription-proxy-panel').getBoundingClientRect();
    const dialogBounds = dialog.getBoundingClientRect();
    assert(panelBounds.left >= dialogBounds.left && panelBounds.right <= dialogBounds.right, 'Proxy form overflows the scan dialog');
    for (const label of all('.subscription-proxy-control .field-label')) {
      const range = document.createRange(); range.selectNodeContents(label.firstChild);
      const labelText = range.getBoundingClientRect();
      const control = label.querySelector('input, select').getBoundingClientRect();
      assert(labelText.height <= parseFloat(getComputedStyle(label).lineHeight) * 2, 'Proxy field label wraps into a narrow column');
      assert(control.top >= labelText.bottom && control.left >= panelBounds.left && control.right <= panelBounds.right, 'Proxy field collides with its label or panel edge');
    }
    assert(document.querySelector('[aria-label="订阅代理地址"]').placeholder === 'http://127.0.0.1:端口', 'Proxy placeholder exposes a fixed port');
    setInput(document.querySelector('[aria-label="订阅代理地址"]'), 'http://draft.example:9000');
    await click('取消');
    assert(document.querySelector('.oc-scan-list') === list && !button('调整模型').matches(':disabled'), 'Cancel lost or locked the scan');
    assert(calls('subscription_proxy_apply').length === 0, 'Cancel applied a draft');
    await click('调整模型');
    const detail = document.querySelector('.oc-scan-detail');
    assert(all('[role="dialog"] h2').some(item => item.textContent === '连接订阅') && !document.querySelector('[data-hermes-source-scope]'), 'Selected subscription leaked device-scan controls');
    await click('设置订阅连接代理');
    await until(() => document.querySelector('[aria-label="订阅代理地址"]')?.value === saved.config.policy.url, 'discarded draft and saved readback');
    document.querySelector('[role="dialog"]').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await until(() => !document.querySelector('.subscription-proxy-panel'), 'Escape collapses editor');
    assert(document.querySelector('.oc-scan-detail') === detail && document.querySelector('[role="dialog"]') === dialog, 'Escape lost the selected subscription');
    await click('返回');
    await until(() => document.querySelector('.oc-scan-list'), 'returned to device scan');
    assert(all('[role="dialog"] h2').some(item => item.textContent === '扫描本机') && document.querySelector('[data-hermes-source-scope]'), 'Returning from a subscription did not restore the device-scan context');
  }),
  scenario('desktop.models.subscription-proxy.read-retry', ['subscription-proxy'], 'An unread proxy policy cannot be overwritten and retry loads the saved mode', async () => {
    let attempts = 0;
    await fresh(() => { c().handlers.subscription_proxy_status = () => {
      if (++attempts === 1) throw new Error('SUBSCRIPTION_PROXY_UNAVAILABLE');
      return { config: { revision: 'fixture', policy: { mode: 'direct' } }, applied: false };
    }; });
    await deviceScan(); await click('设置订阅连接代理');
    await until(() => button('重试读取'), 'read failure');
    assert(button('保存并重启 HiRoute').disabled && !document.querySelector('[aria-label="订阅代理模式"]'), 'Unknown policy can be edited or saved');
    button('保存并重启 HiRoute').click(); await pause(40);
    assert(calls('subscription_proxy_apply').length === 0, 'Read failure overwrote the saved policy');
    await click('重试读取');
    await until(() => document.querySelector('[aria-label="订阅代理模式"]')?.value === 'direct', 'retried saved policy');
    assert(!button('保存并重启 HiRoute').disabled && text().includes('等待应用或服务就绪'), 'Retry did not restore editing or misreported application');
  }),
  scenario('desktop.models.subscription-proxy.save-retry-return', ['subscription-proxy'], 'Save failure retains the draft and a successful restart returns to a fresh scan with saved configuration', async () => {
    let saved = { config: { revision: 'fixture', policy: { mode: 'inherit' } }, applied: true };
    let fail = true;
    await fresh(() => {
      c().handlers.subscription_proxy_status = () => structuredClone(saved);
      c().handlers.subscription_proxy_apply = ({ policy }) => {
        if (fail) throw new Error('SUBSCRIPTION_PROXY_INVALID_OR_UNAVAILABLE');
        saved = { config: { revision: 'next', policy }, applied: true };
        c().remount();
      };
    });
    await deviceScan(); await click('设置订阅连接代理');
    await until(() => document.querySelector('[aria-label="订阅代理模式"]'), 'proxy modes');
    const select = document.querySelector('[aria-label="订阅代理模式"]');
    assert([...select.options].map(item => item.value).join(',') === 'inherit,direct,manual', 'Existing modes changed');
    select.value = 'manual'; select.dispatchEvent(new Event('change', { bubbles: true })); await pause(40);
    setInput(document.querySelector('[aria-label="订阅代理地址"]'), 'http://proxy.example:8080');
    setInput(document.querySelector('[aria-label="订阅代理绕过列表"]'), 'example.org');
    await click('保存并重启 HiRoute');
    await until(() => text().includes('输入已保留'), 'failed save');
    assert(document.querySelector('[aria-label="订阅代理地址"]').value === 'http://proxy.example:8080' && document.querySelector('[aria-label="订阅代理绕过列表"]').value === 'example.org', 'Save failure discarded input');
    assert(!localStorage.getItem('hiroute.subscription-proxy-return'), 'Failed save left a restart hint');
    const scans = calls('compute_subscriptions').length;
    fail = false; await click('保存并重启 HiRoute');
    await until(() => document.querySelector('.oc-scan-list') && !document.querySelector('.subscription-proxy-panel'), 'restart returned to scan');
    assert(calls('compute_subscriptions').length > scans && !localStorage.getItem('hiroute.subscription-proxy-return'), 'Restart reused stale scan or retained navigation hint');
    assert(calls('subscription_proxy_apply').length === 2 && calls('check_subscription').length === 0, 'Save was duplicated or restart automatically checked a subscription');
    assert(JSON.stringify(calls('subscription_proxy_apply')[1].payload.policy) === JSON.stringify(saved.config.policy), 'Saved policy changed');
    await click('设置订阅连接代理');
    await until(() => document.querySelector('[aria-label="订阅代理地址"]')?.value === saved.config.policy.url, 'saved proxy on reopen');
  }),
  scenario('desktop.models.selection-survives-save-refresh', ['model-connections'], 'Renaming a selected connection does not jump back to the previous save result', async () => {
    let a; let b; let beforeB; let latestChange;
    await fresh(() => {
      b = c().management.sources.find(source => source.source_id === 'source/bailian/coding');
      a = structuredClone(b); a.source_id = 'source/selection-a'; a.display_name = '接入 A';
      a.models = a.models.map(model => ({ ...model, model_ref: `${model.model_ref}/a`, binding_id: `${model.binding_id}/a` }));
      c().management.sources = [a, b]; beforeB = JSON.stringify(b);
      c().handlers.get_compute_save_result = payload => ({ ...c().fixtureResponse('get_compute_save_result', payload), source_id: b.source_id });
      c().handlers.preview_compute_save = payload => { latestChange = payload.change; return c().fixtureResponse('preview_compute_save', payload); };
      c().handlers.apply_compute_save = payload => {
        if (latestChange?.subject.kind === 'saved_source' && latestChange.subject.source_id === a.source_id && latestChange.edit?.action === 'rename') {
          a.display_name = latestChange.edit.display_name; a.revision += 1;
        }
        return c().fixtureResponse('apply_compute_save', payload);
      };
    });
    await click('模型');
    setInput(await newBailianAPI(), 'synthetic-selection-key');
    await click('检查并继续');
    await until(() => all('.model-result-row input[type="checkbox"]').length, 'model choices');
    all('.model-result-row input[type="checkbox"]')[0].click(); await pause(40);
    await click('保存连接');
    await until(() => !document.querySelector('[role="dialog"]') && calls('get_compute_save_result').length, 'completed initial save');
    await click('按接入');
    await until(() => document.querySelector('.models-feature .detail-identity h2')?.textContent === b.display_name, 'saved connection focus');
    all('.models-feature .master-list .list-row').find(row => row.textContent.includes('接入 A')).click(); await pause(40);
    await click('重命名');
    await until(() => document.querySelector('[role="dialog"] input'), 'rename field');
    setInput(document.querySelector('[role="dialog"] input'), '接入 A 已改名');
    const snapshots = calls('compute_management_snapshot').length;
    await click('保存名称');
    await until(() => calls('compute_management_snapshot').length > snapshots && !document.querySelector('[role="dialog"]'), 'rename refresh');
    assert(document.querySelector('.models-feature .detail-identity h2')?.textContent === '接入 A 已改名', 'Refresh jumped to the previous saved connection');
    assert(JSON.stringify(b) === beforeB, 'Another connection changed while renaming A');
  }),
  scenario('desktop.models.connection-template-directory-layout', ['model-connections'], 'The production stylesheet renders the complete searchable provider and product directory', async () => {
    await fresh(); await click('模型'); await click('添加模型');
    await until(() => all('button').some(item => item.textContent.includes('新建 API 接入')), 'new connection entry');
    all('button').find(item => item.textContent.includes('新建 API 接入')).click();
    await until(() => all('.connection-provider-card').length === 13, 'complete provider directory');
    assert(document.querySelector('.connection-directory-intro').textContent.includes('19 种接入方式'), 'Directory omits supported products');
    assert(getComputedStyle(document.querySelector('.connection-provider-grid')).display === 'grid', 'Production provider layout missing');
    for (const row of all('.connection-provider-card')) {
      assert(getComputedStyle(row).display === 'grid' && getComputedStyle(row).alignItems === 'center', 'Provider card is not aligned');
      assert(row.scrollWidth <= row.clientWidth + 1, 'Provider card overflows horizontally');
    }
    setInput(document.querySelector('.connection-directory input[type="search"]'), '百炼');
    await until(() => all('.connection-provider-card').length === 1 && all('.connection-provider-card')[0].textContent.includes('百炼'), 'filtered provider');
    all('.connection-provider-card')[0].click();
    await until(() => all('.connection-product-card').length === 3, 'all Bailian products');
    for (const row of all('.connection-product-card')) {
      assert(getComputedStyle(row).display === 'flex' && getComputedStyle(row).alignItems === 'center', 'Product row is not aligned');
      assert(getComputedStyle(row.querySelector('.row-main')).display === 'grid', 'Product description is not on its own line');
    }
    const close = all('[role="dialog"] button[aria-label="取消添加"]')[0];
    assert(close, 'Template dialog close control missing');
    close.click();
    await until(() => !document.querySelector('[role="dialog"]'), 'closed template dialog');
  }),
  scenario('desktop.models.connection-rename', ['model-connections'], 'Rename is scoped to one saved connection without replacing its models or keys', async () => {
    await fresh(() => { c().management.sources = [c().management.sources.find(source => source.source_id === 'source/bailian/coding')]; });
    await click('模型'); await click('按接入'); await click('重命名');
    await until(() => document.querySelector('[role="dialog"] input'), 'rename input');
    setInput(document.querySelector('[role="dialog"] input'), '团队百炼'); await click('保存名称');
    await until(() => calls('preview_compute_save').length === 1, 'rename preview');
    const change = calls('preview_compute_save')[0].payload.change;
    assert(change.edit.action === 'rename' && change.edit.display_name === '团队百炼', 'Rename intent missing');
    assert(change.subject.source_id === 'source/bailian/coding' && change.selected_model_refs.length === 0 && change.key_edits.length === 0, 'Rename replaced a model or credential selection');
  }),
  scenario('desktop.models.delete-reference-block', ['model-connections'], 'A disabled route reference remains visible and prevents Apply', async () => {
    await fresh(() => {
      c().management.sources = [c().management.sources.find(source => source.source_id === 'source/bailian/coding')];
      const plan = c().desktop.catalog.plans[0]; plan.head.status = 'disabled';
      c().handlers.preview_compute_save = payload => ({ ...c().fixtureResponse('preview_compute_save', payload), affected_plan_refs: [plan.agent_plan_id] });
    });
    await click('模型'); await click('移除模型');
    await until(() => text().includes('仍被引用，请先调整引用再删除。'), 'reference blocker');
    assert(text().includes('这是最后一个模型，将一并删除所属接入。'), 'Last-model consequence was hidden');
    const remove = all('[role="dialog"] button').find(item => item.textContent.trim() === '删除接入');
    assert(remove?.disabled, 'Referenced connection could be deleted');
    remove.click(); await pause(50);
    assert(calls('apply_compute_save').length === 0, 'Disabled button dispatched Apply');
    assert(calls('preview_compute_save')[0].payload.change.edit.action === 'delete', 'Last model removal did not use explicit delete');
    const deleteInDialog = () => all('[role="dialog"] button').find(item => item.textContent.trim() === '删除接入');
    c().handlers.refresh_route_references = () => { throw new Error('publication unavailable'); };
    await click('更新引用状态');
    await until(() => text().includes('引用状态暂未更新'), 'failed reference refresh');
    assert(button('更新引用状态') && !button('更新引用状态').disabled, 'Failed refresh lost its retry entry');
    assert(deleteInDialog()?.disabled, 'Failed refresh released a reference');
    c().handlers.refresh_route_references = () => undefined;
    await click('更新引用状态');
    await until(() => !document.querySelector('[role="dialog"]'), 'refresh closes stale preview');
    await click('移除模型');
    await until(() => text().includes('仍被引用，请先调整引用再删除。'), 'fresh disabled reference');
    assert(deleteInDialog()?.disabled, 'Checkpoint bypassed a disabled route');
    assert(calls('preview_compute_save').length === 2, 'Reopen reused an old deletion preview');
    assert(calls('apply_compute_save').length === 0, 'Reference refresh automatically deleted configuration');
    // A separately completed publication can release historical references. Its
    // new deletion preview is still required before the user explicitly deletes.
    c().handlers.refresh_route_references = () => {
      c().handlers.preview_compute_save = payload => ({ ...c().fixtureResponse('preview_compute_save', payload), affected_plan_refs: [] });
    };
    await click('更新引用状态');
    await until(() => !document.querySelector('[role="dialog"]'), 'completed historical reference refresh');
    await click('移除模型');
    await until(() => deleteInDialog() && !deleteInDialog().disabled, 'new unreferenced preview');
    assert(calls('apply_compute_save').length === 0, 'Refresh was mistaken for delete consent');
  }),
  scenario('desktop.models.append-preserves-existing', ['model-connections'], 'Appending selects only new models and retains a disabled connection', async () => {
    await fresh(() => {
      c().management.sources = [c().management.sources.find(source => source.source_id === 'source/bailian/coding')];
      c().management.sources[0].state = 'disabled';
      c().handlers.check_saved_model_connection = payload => {
        const checked = c().fixtureResponse('check_saved_model_connection', payload);
        checked.candidate.models.push({ ...checked.candidate.models[0], model_ref: 'model-ref/new', upstream_model_id: 'new-model', display_name: 'New model' });
        return checked;
      };
    });
    await click('模型'); await click('向此接入添加模型');
    await until(() => all('[role="dialog"] input[type="checkbox"]').length === 2, 'append inventory');
    const checks = all('[role="dialog"] input[type="checkbox"]');
    assert(checks[0].disabled && checks[0].checked, 'Existing model was editable as a removal');
    checks[1].click(); await click('添加 1 个模型');
    await until(() => calls('preview_compute_save').length === 1, 'append preview');
    const change = calls('preview_compute_save')[0].payload.change;
    assert(change.edit.action === 'append_models' && change.intent === 'save_disabled', 'Append lost explicit action or disabled state');
    assert(JSON.stringify(change.selected_model_refs) === '["model-ref/new"]' && change.key_edits.length === 0, 'Append modified existing model/key selection');
  }),
  scenario('desktop.models.remove-only-selected', ['model-connections'], 'Removing one model retains the connection and sends exactly the selected saved model', async () => {
    await fresh(() => { c().management.sources = [c().management.sources[0]]; });
    await click('模型'); await click('移除模型');
    await until(() => all('[role="dialog"] button').some(item => item.textContent.trim() === '移除模型' && !item.disabled), 'remove preview');
    const change = calls('preview_compute_save')[0].payload.change;
    assert(change.edit.action === 'remove_models' && change.selected_model_refs.length === 1 && change.key_edits.length === 0, 'Remove scope replaced connection or keys');
    all('[role="dialog"] button').find(item => item.textContent.trim() === '移除模型').click();
    await until(() => calls('apply_compute_save').length === 1, 'single remove apply');
  }),
  scenario('desktop.models.tool-check-selection', ['model-connections'], 'A tool check retains the chosen model with its fresh reference without selecting new inventory', async () => {
    await fresh(); await click('模型');
    setInput(await newBailianAPI(), 'synthetic-tool-check-input');
    c().handlers.check_registered_model_connection = payload => {
      const result = c().fixtureResponse('check_registered_model_connection', payload);
      if (payload.request.inference_model_id) {
        result.inference = 'verified';
        result.candidate.models = result.candidate.models.map(model => ({ ...model, model_ref: `${model.model_ref}/checked` }));
      }
      return result;
    };
    await click('检查并继续');
    await until(() => all('.model-result-row input[type="checkbox"]').length === 2, 'inventory');
    assert(all('.model-result-row input:checked').length === 0, 'Inventory selected models without consent');
    all('.model-result-row input[type="checkbox"]')[0].click(); await pause(40);
    await click('验证工具调用（仅所选模型，可能计费）');
    await until(() => calls('check_registered_model_connection').length === 2 && button('保存连接'), 'tool check completed');
    assert(all('.model-result-row input:checked').length === 1, 'Tool check lost the explicit model selection');
    assert(!button('保存连接').disabled, 'Successful tool check disabled save');
    await click('保存连接');
    await until(() => calls('preview_compute_save_v3').length === 1, 'current save preview');
    const preview = JSON.stringify(calls('preview_compute_save_v3')[0].payload);
    assert(preview.includes('model-ref/qwen3-coder-plus/checked') && !preview.includes('model-ref/qwen3-max/checked'), 'Save did not use only the selected fresh model reference');
  }),
  scenario('desktop.routing.disabled-header', ['routing-editor'], 'A stopped route stays visibly disabled while its editor has unsaved changes', async () => {
    await fresh(() => {
      c().desktop.catalog.plans[0].head.status = 'disabled';
    });
    await routing();
    const header = () => document.querySelector('.plan-editor .editor-header').textContent;
    assert(header().includes('已停用') && !header().includes('已启用'), 'Stopped route is labelled active');
    setInput(document.querySelector('.plan-identity-fields input'), 'Stopped route edit'); await pause(40);
    assert(header().includes('已停用') && header().includes('有未发布更改'), 'Editing hid the stopped call state');
  }),
  scenario('desktop.quality.evidence-return', ['routing-editor', 'sessions'], 'Quality evidence names the exact request, preserves it on refresh and returns to the same filtered performance view', async () => {
    const session = 'session/quality-link';
    const selectedId = 'request/quality-selection';
    let failExactOnce = false;
    const now = Date.now();
    const identity = { plan_revision: 8, selected_branch_id: 'smart_saving', executed_branch_id: 'smart_saving', model_configuration_id: 'model/evidence', profile_digest: 'profile/evidence', attribution: 'single', group: 'primary', candidate_index: 0 };
    const sample = { ...identity, segment_id: 'stage/evidence', session_id: session, plan_id: 'plan/daily-coding',
      native_model: 'quality-evidence-model', reasoning_profile_id: 'low', first_turn_ordinal: 1, last_observed_turn_ordinal: 1,
      first_at_ms: now - 1000, last_at_ms: now - 1000, history_partial: false,
      first_request_id: selectedId, last_request_id: selectedId, execution_evidence_available: true,
      branch_execution: { group: 'primary', candidate_index: 0, policy: { name: 'Smart saving', floor_millis: 500 } },
      selection: { execution_group: 'regular', selection_reason: 'simple_task', simple_probability: .95, simple_threshold_millis: 800 },
      assessment: { score: .5, target_from_ordinal: 1, target_through_ordinal: 1, partial: false, evidence_available: false },
    };
    const rows = ['request/old-history', selectedId].map((id, index) => ({ request_id: id, session_id: session, started_at_ms: now - 2000 + index * 1000, final_native_model: index ? 'quality-evidence-model' : 'earlier-model', content_completeness: 'complete', within_request_fallback: false }));
    const reads = view => calls('observation_read').filter(call => call.payload.request?.intent?.view === view);
    const query = call => call.payload.request.intent.query;
    await fresh(() => {
      c().handlers.observation_read = payload => {
        const { view, query: q } = payload.request.intent;
        if (view === 'plan_quality') return { samples: [sample], summary: { models: [{ execution: identity, native_model: 'quality-evidence-model', reasoning_profile_id: 'low', scored_stage_count: 1, unrated_stage_count: 0, average_score: .5 }], scored_stage_count: 1, unrated_stage_count: 0, session_count: 1, available_revisions: [8] }, next_cursor: null };
        if (view === 'sessions') return { sessions: [{ session_id: session, agent_id: '', request_count: 2, fallback_request_count: 0, last_request_at_ms: now - 1000, correlation_kind: 'agent_supplied', content_completeness: 'complete' }], next_cursor: null };
        if (view === 'timeline') {
          if (q.request_id && failExactOnce) { failExactOnce = false; throw { code: 'LOCAL_SERVICE_UNAVAILABLE' }; }
          return { requests: q.request_id ? rows.filter(row => row.request_id === q.request_id) : rows, next_cursor: null };
        }
        if (view === 'catalog') return { contents: [{ content_id: q.request_id, role: 'user', kind: 'text', state: 'complete', direction: 'request_input', media_type: 'text/plain', message_occurrence_id: 'user' }], transcript_roots: [], roots_partial: false, next_cursor: null };
        if (view === 'content') return { state: 'complete', chunks: [{ text: q.content_id === selectedId ? 'SELECTED_REQUEST_WITH_REPLAYED_HISTORY' : 'EARLIER_REQUEST', original_byte_offset: 0 }], next_cursor: null };
        return c().fixtureResponse('observation_read', payload);
      };
    });
    await routing(); await click('模型表现');
    await until(() => all('[data-model-configuration="model/evidence"] .quality-view-stages').length, 'observed model');
    all('[data-model-configuration="model/evidence"] .quality-view-stages')[0].click();
    await until(() => all('.quality-score-filter').length, 'stage filter');
    const filter = all('.quality-score-filter')[0]; filter.value = 'high'; filter.dispatchEvent(new Event('change', { bubbles: true }));
    await until(() => query(reads('plan_quality').at(-1)).competence === 'meets_floor' && all('.quality-assessment-details summary').length, 'filtered stage');
    all('.quality-assessment-details summary')[0].click(); await click('查看选择请求');
    await until(() => text().includes('SELECTED_REQUEST_WITH_REPLAYED_HISTORY'), 'exact selected request body');
    assert(reads('timeline').some(call => query(call).request_id === selectedId), 'Quality link omitted the request identity');
    assert(all('.session-request-focus')[0]?.dataset.requestId === selectedId, 'Exact evidence has no visible request focus');
    assert(all('.session-request-focus')[0].textContent.includes('quality-evidence-model'), 'Focus omits the actual model');
    const beforeRefresh = reads('sessions').length;
    window.dispatchEvent(new Event('focus'));
    await until(() => reads('sessions').length > beforeRefresh && all('.session-request-focus')[0]?.dataset.requestId === selectedId && text().includes('SELECTED_REQUEST_WITH_REPLAYED_HISTORY'), 'foreground refresh retains exact request');
    await click('返回模型表现');
    await until(() => all('.quality-score-filter')[0]?.value === 'high', 'return retains stage filter');
    assert(query(reads('plan_quality').at(-1)).plan_revision === 8 && query(reads('plan_quality').at(-1)).from_ms > 0, 'Return lost current revision or seven-day scope');
    failExactOnce = true;
    await click('查看选择请求');
    await until(() => all('.session-detail-feedback[data-error-code]').length, 'exact request read failure');
    const beforeRetry = reads('timeline').length;
    await click('重试');
    await until(() => !all('.session-detail-feedback[data-error-code]').length && all('.session-request-focus')[0]?.textContent.includes('quality-evidence-model'), 'retry preserves exact evidence');
    assert(reads('timeline').slice(beforeRetry).some(call => query(call).request_id === selectedId), 'Retry silently opened the complete session');
    await click('查看完整会话');
    await until(() => !all('.session-request-focus').length && all('.request-timeline').length, 'explicit full-session navigation');
  }),
  scenario('desktop.models.return-from-decisions', ['model-connections', 'decision-services'], 'Returning from a decision connection save uses the current configuration revision for credential changes', async () => {
    let store;
    await fresh(() => {
      store = decisionStore();
      const save = c().handlers.save_decision_service_v2;
      c().handlers.save_decision_service_v2 = payload => { const result = save(payload); c().management.revisions.target += 1; return result; };
      c().handlers.preview_compute_save = ({ change }) => {
        if (change.expected_revisions.target !== c().management.revisions.target) throw { code: 'REVISION_CONFLICT' };
        return c().fixtureResponse('preview_compute_save', { change });
      };
    });
    await click('模型');
    await until(() => all('.oc-models .master-list button').length, 'general models');
    await click('决策模型'); await click('添加决策模型');
    await until(() => nativeField('credential'), 'independent setup after an empty connection read');
    setInput(nativeField('credential'), 'synthetic-decision-key');
    await click('仅保存'); await until(() => store.services.length && !all('.decision-connection-form').length, 'saved decision connection');
    await click('通用模型');
    await until(() => all('.oc-models .master-list button').some(item => item.textContent.includes('Qwen3-Coder-Plus')), 'Bailian model');
    all('.oc-models .master-list button').find(item => item.textContent.includes('Qwen3-Coder-Plus')).click(); await pause(40);
    await click('管理凭据'); await click('保存凭据');
    await until(() => calls('preview_compute_save').length > 0, 'credential preview');
    assert(calls('preview_compute_save').at(-1).payload.change.expected_revisions.target === c().management.revisions.target, 'Credential change reused the revision from before the decision save');
    await until(() => text().includes('接入凭据已保存'), 'credential save completed');
  }),
  scenario('desktop.routing.follow-up-preference', ['routing-editor', 'decision-services'], 'Both editors preserve authoritative compatibility values without exposing or generating a follow-up control', async () => {
    await fresh(); await routing(); await click('新建智能路由');
    const fields = all('.plan-identity-fields input');
    setInput(fields[0], 'Continuous routing'); setInput(fields[1], 'Check routing without a follow-up setting');
    for (const label of ['按难度分流', '自定义分支', '按难度分流', '自定义分支']) {
      all('.mode-card').find(item => item.querySelector('strong')?.textContent === label).click();
      await pause(40);
      assert(!all('.plan-editor details').some(item => /追问设置|Follow-up settings/.test(item.textContent)), 'An empty retired follow-up section remains');
      assert(!all('.plan-editor label').some(item => /追问时允许换模型|Allow model switching on follow-up/.test(item.textContent)), 'The retired control is still offered');
    }
    const defaultBranch = all('.plan-editor details').find(item => item.querySelector('summary')?.textContent.includes('默认分支'));
    assert(defaultBranch, 'Default branch control was removed with the preference');
    defaultBranch.querySelector('summary').click(); await pause(40);
    assert(defaultBranch.querySelector('select')?.options.length === 2, 'Default branches are not editable');
    await click('保存草稿');
    await until(() => calls('preview_plan_editor').length > 0, 'draft without follow-up setting');
    const editor = calls('preview_plan_editor').at(-1).payload.input.editor;
    const seed = c().fixtureResponse('plan_editor_options', { input: { authoring: {} } }).authoring.editor;
    assert(editor.smart.reselect_on_user_message === seed.smart.reselect_on_user_message && Object.hasOwn(editor.smart, 'reselect_on_user_message') === Object.hasOwn(seed.smart, 'reselect_on_user_message'), 'Frontend changed the authoritative compatibility value');
    assert(!('reselect_on_user_message' in editor.branch_routing), 'New frontend branches generate the retired setting');
    for (const mode of ['smart_saving', 'custom_branches']) {
      for (const legacy of [true, false, undefined]) {
        const label = `${mode}, legacy=${legacy}`;
        await fresh(() => {
          const plan = c().desktop.catalog.plans[0];
          if (mode === 'custom_branches') {
            const smart = plan.desired.strategy;
            plan.desired.mode = mode;
            plan.desired.strategy = { mode: 'branches', routing: {
              classifier: { kind: 'decision_service', service: { id: 'decision/fixture', revision: 1, name: 'Fixture extension', connection: { kind: 'custom', endpoint: 'https://example.test/decision', timeout_ms: 10000 } } },
              default_branch_id: 'primary', judgment: structuredClone(smart.judgment),
              branches: [
                { id: 'economy', name: 'Economy', condition: 'Simple', judgment: null, candidates: structuredClone(smart.economy), primary_candidates: [] },
                { id: 'primary', name: 'Primary', condition: 'Complex', judgment: null, candidates: structuredClone(smart.primary), primary_candidates: [] },
              ],
            } };
          }
          const settings = mode === 'custom_branches' ? plan.desired.strategy.routing : plan.desired.strategy;
          if (legacy === undefined) delete settings.reselect_on_user_message;
          else settings.reselect_on_user_message = legacy;
        });
        await routing();
        setInput(all('.plan-identity-fields input')[1], `Edit saved route: ${label}`);
        await click('保存草稿');
        await until(() => calls('preview_plan_editor').length > 0 && !button('保存草稿').disabled, `saved draft: ${label}`);
        const saved = calls('preview_plan_editor').at(-1).payload.input.editor;
        const settings = mode === 'custom_branches' ? saved.branch_routing : saved.smart;
        assert(settings.reselect_on_user_message === legacy && Object.hasOwn(settings, 'reselect_on_user_message') === (legacy !== undefined), `Editing changed the retired value: ${label}`);
        assert(!all('.plan-editor label').some(item => /追问时允许换模型|Allow model switching on follow-up/.test(item.textContent)), `A saved route exposes the retired control: ${label}`);
      }
    }
  }),
  scenario('desktop.routing.judgment-settings', ['routing-editor', 'decision-services'], 'Advanced judgment starts collapsed; branch copy/reset stays independent and draft payload retains settings', async () => {
    await fresh(); await routing();
    assert(!all('.plan-editor details').some(item => item.querySelector('summary')?.textContent.includes('追问设置')), 'A saved legacy plan exposes the retired preference');
    await click('决策模型');
    const field = id => document.querySelector(`[data-decision-field="${id}"]`);
    const smartDetails = field('smart-simple-threshold').closest('details');
    assert(!smartDetails.open, 'Smart judgment is not collapsed by default');
    smartDetails.querySelector('summary').click(); await pause(40);
    assert(field('smart-simple-threshold').value === '0.8' && field('smart-floor').value === '0.5', 'Default thresholds differ from the published defaults');
    assert(!field('smart-simple').closest('details').open && !field('smart-criterion-0').closest('details').open, 'Prompt details start expanded');
    setInput(field('smart-simple-threshold'), '0.7'); await pause(40);
    await click('保存草稿');
    await until(() => calls('preview_plan_editor').length > 0 && !button('保存草稿').disabled, 'smart draft request');
    const smart = calls('preview_plan_editor').at(-1).payload.input.editor.smart;
    assert(smart.judgment.degree.simple_threshold_millis === 700 && !('branch_routing' in calls('preview_plan_editor').at(-1).payload.input.editor), 'Smart saving was serialized as task branches');
    all('.mode-card').find(item => item.querySelector('strong')?.textContent === '自定义分支').click();
    await until(() => all('.branch-routing-card').length === 2, 'custom branch editor');
    assert(all('.branch-routing-card').every(card => !card.querySelector('details').open), 'Branch advanced fields start expanded');
    assert(!field('branch-0-simple-threshold'), 'Single-group branch exposes a degree threshold');
    const defaults = field('global-floor').closest('details');
    defaults.querySelector('summary').click(); await pause(40);
    setInput(field('global-floor'), '0.6'); await pause(40);
    const first = all('.branch-routing-card')[0];
    first.querySelector('details > summary').click(); await pause(40);
    assert(field('branch-0-floor').disabled || field('branch-0-floor').closest('fieldset').disabled, 'Following defaults is editable before copying');
    [...first.querySelectorAll('button')].find(item => item.textContent === '单独调整').click(); await pause(40);
    assert(field('branch-0-floor').value === '0.6', 'Copy did not capture current plan defaults');
    setInput(field('branch-0-floor'), '0.4'); await pause(40);
    setInput(field('global-floor'), '0.7'); await pause(40);
    assert(field('branch-0-floor').value === '0.4' && field('branch-1-floor').value === '0.7', 'Independent branch changed with plan defaults');
    await click('保存草稿');
    await until(() => calls('preview_plan_editor').at(-1).payload.input.editor.mode === 'custom_branches', 'custom draft request');
    const saved = calls('preview_plan_editor').at(-1).payload.input.editor;
    assert(saved.branch_routing.branches[0].judgment.competence.floor_millis === 400 && saved.branch_routing.branches[1].judgment == null, 'Whole-setting override identity was lost');
    assert(saved.smart.judgment.degree.simple_threshold_millis === 700, 'Changing modes lost parked smart judgment');
    await until(() => !button('保存草稿').disabled, 'draft action finished');
    await click('恢复计划默认');
    assert(field('branch-0-floor').value === '0.7', 'Restore did not resume following plan defaults');
  }),
  scenario('desktop.models.branch-route-references', ['model-connections', 'routing-editor'], 'Model references include regular and upgrade branch candidates and open the correct published route', async () => {
    await fresh(() => {
      const base = structuredClone(c().desktop.catalog.plans[0]);
      const selection = { binding_id: 'binding/codex/gpt-6-astra' };
      const makePlan = (id, name, upgrade) => ({ ...structuredClone(base), agent_plan_id: id, model_alias: id.replace('/', '-'), desired: { ...base.desired, display_name: name, mode: 'custom_branches', strategy: { mode: 'branches', routing: {
        classifier: { kind: 'decision_service', service: { id: 'decision/fixture', revision: 1, name: 'Fixture extension', connection: { kind: 'custom', endpoint: 'https://example.test/decision', timeout_ms: 10000 } } }, default_branch_id: 'primary', judgment: structuredClone(base.desired.strategy.judgment), reselect_on_user_message: false,
        branches: [{ id: 'economy', name: 'Economy', condition: 'Simple', judgment: null, candidates: upgrade ? [{ binding_id: 'binding/bailian/qwen-coder' }] : [selection], primary_candidates: upgrade ? [selection] : [] }, { id: 'primary', name: 'Primary', condition: 'Complex', judgment: null, candidates: [{ binding_id: 'binding/bailian/qwen-coder' }], primary_candidates: [] }],
      } } } });
      const regular = makePlan('plan/branch-regular', 'Branch regular model use', false);
      const upgrade = makePlan('plan/branch-upgrade', 'Branch upgrade model use', true);
      const disabled = { ...makePlan('plan/disabled', 'Disabled branch model use', false), head: { ...base.head, status: 'disabled' } };
      c().desktop.catalog.plans = [regular, upgrade, disabled];
    });
    await click('模型');
    await until(() => all('.oc-models .master-list button').some(item => item.textContent.includes('GPT-6-Astra')), 'model row');
    all('.oc-models .master-list button').find(item => item.textContent.includes('GPT-6-Astra')).click(); await pause(40);
    const refs = () => all('.oc-models .v3-linked');
    await until(() => text().includes('用于这些路由'), 'model route references');
    assert(refs().some(item => item.textContent.includes('Branch regular model use')), 'Published regular branch model reference is missing');
    assert(refs().some(item => item.textContent.includes('Branch upgrade model use')), 'Published upgrade branch model reference is missing');
    assert(refs().some(item => item.textContent.trim() === 'Disabled branch model use（已停用）'), 'Disabled reference is missing or presented without its stopped state');
    assert(refs().filter(item => item.textContent.includes('Disabled branch model use')).length === 1, 'Disabled reference is duplicated');
    await click('Branch regular model use');
    await until(() => document.querySelector('.plan-identity-fields input')?.value === 'Branch regular model use', 'referenced route opened');
  }),
  scenario('desktop.decisions.hidden-invalid-field', ['decision-services'], 'An invalid collapsed field and its error scroll above the sticky form actions', async () => {
    const saved = { id: 'decision/invalid-field', revision: 1, name: 'Connection with a collapsed timeout', connection: { kind: 'system_one', provider: 'bailian-token-plan', model: 'decision-model-preview', endpoint: 'https://example.test/systemone', timeout_ms: 10000, auth_header: { name: 'Authorization', value_secret_ref: 'protected/r1' } } };
    await fresh(() => { decisionStore([saved]); });
    await click('模型'); await click('决策模型'); await click('编辑');
    const form = all('.decision-connection-form')[0];
    const timeout = form.querySelector('[data-native-field="timeout"]');
    const details = timeout.closest('details');
    details.querySelector('summary').click(); await pause(40);
    setInput(timeout, '0'); await pause(40);
    details.querySelector('summary').click(); await pause(40);
    const pane = form.closest('.detail-pane'); pane.scrollTop = pane.scrollHeight;
    await click('仅保存');
    await until(() => document.activeElement === timeout && details.open, 'invalid field expanded and focused');
    const field = timeout.closest('.field').getBoundingClientRect();
    const footer = form.querySelector('.decision-form-actions').getBoundingClientRect();
    assert(field.top >= pane.getBoundingClientRect().top && field.bottom <= footer.top, 'Invalid field or error is hidden behind the sticky actions');
    assert(calls('save_decision_service_v2').length === 0, 'Invalid timeout was submitted');
  }),
  scenario('desktop.decisions.save-test-revision', ['decision-services'], 'Save and test reads back the exact revision, tests its digest, maps failures to repairs and never resubmits a key', async () => {
    let store;
    await fresh(() => { store = decisionStore(); store.test = () => ({ outcome: 'failed', failure_code: 'DECISION_INPUT_REJECTED' }); });
    await click('模型'); await click('决策模型'); await click('添加决策模型');
    const form = () => all('.decision-connection-form')[0];
    const input = nativeField;
    await until(() => input('credential'), 'independent setup after an empty connection read');
    assert(all('.decision-mode')[1].getAttribute('aria-pressed') === 'true', 'An empty connection read did not default to independent setup');
    assert(all('.decision-provider img').length >= 2, 'Provider brand logos are missing');
    assert(!input('timeout').closest('details').open, 'Advanced defaults overwhelm initial form');
    setInput(input('credential'), 'synthetic-test-key'); await pause(30);
    input('timeout').closest('details').querySelector('summary').click(); await pause(30);
    assert(input('credential').value === 'synthetic-test-key', 'Disclosure lost the credential');
    await click('保存并测试'); await until(() => text().includes('配置已保存，测试失败'), 'saved failure feedback');
    const saved = calls('save_decision_service_v2').at(-1).payload.input;
    assert(saved.change.expected_revision === 0 && saved.change.service.revision === 1 && saved.change.service.schema === 'hiroute.decision-service/v2', 'New connection did not use the V2 revision transaction');
    assert(saved.secret === 'synthetic-test-key' && saved.change.service.connection.authentication.kind === 'independent', 'Independent key was not sent once as the save secret');
    assert(decisionOrder().slice(-4).join() === 'save_decision_service_v2,decision_services_v2,test_decision_service_v2,decision_services_v2', 'Test did not follow readback of the saved revision or skipped the post-test readback');
    const tested = calls('test_decision_service_v2').at(-1).payload.input;
    assert(tested.revision === 1 && tested.id === saved.change.service.id && tested.expected_configuration_digest === store.view(saved.change.service.id).configuration_digest, 'Test used the unsaved editor or an old digest');
    assert(text().includes('请核对模型、完整地址和协议') && !text().includes('DECISION_INPUT_REJECTED') && !text().includes('sha256:'), 'Failure is not actionable or exposes codes and digests by default');
    await click('编辑决策模型'); await click('更换');
    setInput(input('credential'), 'replacement-key'); await pause(30);
    all('.decision-address-value button')[0].click(); await pause(30);
    setInput(input('endpoint'), 'https://other.example.test/v1/decisions'); await pause(30);
    assert(input('credential').value === '' && text().includes('地址的来源已变化，请重新填写 API Key。'), 'Endpoint origin change retained credentials');
    const before = calls('save_decision_service_v2').length;
    await click('仅保存');
    assert(calls('save_decision_service_v2').length === before, 'Missing credential was submitted');
    setInput(input('credential'), 'replacement-key'); await pause(30);
    await click('仅保存'); await until(() => !form(), 'saved edited revision');
    assert(calls('test_decision_service_v2').length === 1 && text().includes('本次打开尚未测试'), 'Old test status leaked to new revision or save-only tested');
    store.test = () => ({ outcome: 'passed' });
    await click('测试连接'); await until(() => text().includes('连接测试通过'), 'test repaired connection');
    assert(calls('test_decision_service_v2').at(-1).payload.input.revision === 2, 'Repair tested a stale revision');
    await click('编辑'); setInput(input('name'), 'Saved while readback fails'); await pause(40);
    const read = c().handlers.decision_services_v2;
    c().handlers.decision_services_v2 = () => { throw new Error('DAEMON_UNAVAILABLE'); };
    await click('保存并测试'); await until(() => text().includes('已保存 — 需要刷新'), 'acknowledged save readback failure');
    assert(!form() && calls('test_decision_service_v2').length === 2, 'Readback failure retained a resubmittable secret or ran a stale test');
    const testButton = () => all('.decision-test-actions button')[0];
    assert(testButton().disabled, 'Readback failure allowed testing an old snapshot');
    c().handlers.decision_services_v2 = read;
    await click('刷新'); await until(() => text().includes('Saved while readback fails') && !testButton().disabled, 'readback recovery');
  }),
  scenario('desktop.decisions.shared-source', ['decision-services', 'model-connections'], 'A ready API connection defaults to reuse without choosing it; bundles, pending keys, detours and stale tests stay scoped', async () => {
    let store;
    const sourceId = 'source/openai/decisions';
    await fresh(() => { store = decisionStore([], [c().decisionSource(sourceId)]); });
    await click('模型'); await click('决策模型'); await click('添加决策模型');
    const input = nativeField;
    const mode = index => all('.decision-mode')[index];
    await until(() => mode(0)?.getAttribute('aria-pressed') === 'true', 'reuse default after a ready read');
    assert(input('source').value === '' && !input('model'), 'The form chose an API connection for the user');
    choose(input('source'), sourceId); await pause(40);
    assert(input('model').value === 'gpt-6-luna' && input('name').value === 'gpt-6-luna · Decision API', 'Suggested settings or default name are missing');
    assert(!input('credential') && text().includes('使用 API 接入的凭据'), 'Reuse asked for a key');
    setInput(input('model'), 'gpt-6-luna-mini'); setInput(input('name'), 'Team decisions'); await pause(40);
    await click('使用建议设置');
    assert(input('model').value === 'gpt-6-luna' && input('name').value === 'Team decisions', 'Suggested settings overwrote the edited name');
    setInput(input('model'), 'gpt-6-luna-mini'); await pause(40);
    mode(1).click(); await pause(40);
    setInput(input('credential'), 'pending-independent-key'); await pause(40);
    mode(0).click(); await pause(40);
    assert(text().includes('未保存的 Key 已清除') && input('model').value === 'gpt-6-luna-mini' && input('source').value === sourceId, 'Mode switch kept a pending key or replaced the shared bundle');
    mode(1).click(); await pause(40);
    assert(input('credential').value === '', 'Returning to independent setup restored a cleared key');
    mode(0).click(); await pause(40);
    await click('保存并测试'); await until(() => text().includes('连接测试通过'), 'shared save, readback and test');
    const saved = calls('save_decision_service_v2').at(-1).payload.input;
    const auth = saved.change.service.connection.authentication;
    assert(saved.secret === null && auth.kind === 'shared_source' && auth.source_id === sourceId && auth.source_lineage_digest === store.sources[0].lineage_digest, 'Shared save carried a key or the wrong source lineage');
    assert(JSON.stringify(saved.change.expected_source_revisions) === JSON.stringify([{ source_id: sourceId, revision: 1 }]), 'Shared save omitted the source revision precondition');
    assert(decisionOrder().slice(-4).join() === 'save_decision_service_v2,decision_services_v2,test_decision_service_v2,decision_services_v2', 'Test did not follow readback of the saved revision or skipped the post-test readback');
    assert(calls('test_decision_service_v2').at(-1).payload.input.expected_configuration_digest === store.view(saved.change.id).configuration_digest, 'Test did not use the read-back digest');
    assert(!text().includes('sha256:') && calls('preview_plan_editor').length === 0, 'Digest shown outside diagnostics or save published a route');
    await click('编辑'); setInput(input('name'), 'Edited during detour'); await pause(40);
    await click('管理 API 接入');
    await until(() => button('返回决策模型 — 草稿已保留'), 'detour banner');
    store.sources[0] = { ...store.sources[0], credential_state_revision: 'sha256:' + '9'.repeat(64) };
    const reads = calls('decision_connection_sources').length;
    await click('返回决策模型 — 草稿已保留');
    await until(() => calls('decision_connection_sources').length > reads && input('name')?.value === 'Edited during detour', 'detour return refreshes sources and keeps the draft');
    assert(mode(0).getAttribute('aria-pressed') === 'true' && input('source').value === sourceId && input('model').value === 'gpt-6-luna-mini', 'Detour return changed mode, source or bundle');
    await until(() => all('.decision-test-state.stale').length === 1, 'key rotation marks the earlier pass stale');
    assert(calls('test_decision_service_v2').length === 1, 'Returning from the detour retested implicitly');
    await click('取消'); await click('放弃修改');
    await until(() => text().includes('上次测试通过 — 凭据或配置已变化'), 'stale detail state');
    store.test = () => ({ outcome: 'failed', failure_code: 'DECISION_AUTHENTICATION_REJECTED' });
    await click('重新测试'); await until(() => text().includes('连接测试失败'), 'shared authentication failure');
    assert(button('管理 API 接入') && button('编辑决策模型') && !text().includes('DECISION_AUTHENTICATION_REJECTED'), 'Shared authentication failure lacks repairs or exposes raw codes');
    assert(!button('重试'), 'Authentication failure offered a blind retry');
    await click('管理 API 接入');
    await until(() => button('返回决策模型 — 草稿已保留'), 'repair detour banner');
    store.sources[0] = { ...store.sources[0], state: 'disabled' };
    await click('返回决策模型 — 草稿已保留');
    await until(() => all('.decision-source-callout').length && text().includes('没有可用的 Key 或已停用'), 'disabled source repair');
  }),
  scenario('desktop.decisions.source-closure', ['decision-services', 'model-connections'], 'A save registers only the newly selected API connection, independent setup registers none, and only deletion names every related connection', async () => {
    let store;
    const a = 'source/team-a', b = 'source/team-b';
    const record = { schema: 'hiroute.decision-service/v2', id: 'decision/closure', revision: 1, name: 'Closure decisions', connection: { kind: 'native', provider: 'openai', wire_protocol: 'openai_decisions', model: 'gpt-6-luna', endpoint: 'https://api.openai.test/v1/decisions', timeout_ms: 10000, authentication: { kind: 'shared_source', source_id: a, source_lineage_digest: c().decisionSource(a).lineage_digest } } };
    await fresh(() => { store = decisionStore([record], [c().decisionSource(a, { display_name: 'Team A', revision: 3 }), c().decisionSource(b, { display_name: 'Team B', revision: 7 })]); });
    await click('模型'); await click('决策模型');
    const input = nativeField, sent = () => calls('save_decision_service_v2').at(-1).payload.input.change;
    const sources = () => JSON.stringify(sent().expected_source_revisions);
    await click('编辑'); await until(() => input('source')?.value === a, 'shared edit');
    choose(input('source'), b); await pause(40);
    await click('仅保存'); await until(() => store.services[0].revision === 2, 'A -> B save');
    assert(sources() === JSON.stringify([{ source_id: b, revision: 7 }]), `A -> B registered other sources: ${sources()}`);
    assert(store.view(record.id).related_sources.map(item => item.source_id).join() === `${a},${b}`, 'Retained revision no longer relates its source');
    await click('编辑'); await until(() => input('source')?.value === b, 'shared edit on B');
    all('.decision-mode')[1].click(); await until(() => input('credential'), 'independent setup');
    setInput(input('credential'), 'synthetic-closure-key'); await pause(40);
    await click('仅保存'); await until(() => store.services[0].revision === 3, 'shared -> independent save');
    assert(sent().service.connection.authentication.kind === 'independent' && sources() === '[]', `Independent save registered sources: ${sources()}`);
    all('summary').find(item => item.textContent.includes('详情与更多操作')).click(); await pause(40); await click('删除决策模型'); await click('删除');
    await until(() => !store.services.length, 'whole-service deletion');
    assert(sent().service === null && sources() === JSON.stringify([{ source_id: a, revision: 3 }, { source_id: b, revision: 7 }]), `Deletion did not name every related source: ${sources()}`);
    assert(!store.rejected.length, `The store rejected a request shape: ${JSON.stringify(store.rejected)}`);
  }),
  scenario('desktop.decisions.test-freshness', ['decision-services', 'model-connections'], 'A pass counts only against credential state read back afterwards; a failed readback leaves it unverified and nothing retests automatically', async () => {
    let store;
    const sourceId = 'source/openai/decisions';
    const record = { schema: 'hiroute.decision-service/v2', id: 'decision/fresh', revision: 1, name: 'Fresh decisions', connection: { kind: 'native', provider: 'openai', wire_protocol: 'openai_decisions', model: 'gpt-6-luna', endpoint: 'https://api.openai.test/v1/decisions', timeout_ms: 10000, authentication: { kind: 'shared_source', source_id: sourceId, source_lineage_digest: c().decisionSource(sourceId).lineage_digest } } };
    await fresh(() => { store = decisionStore([record], [c().decisionSource(sourceId)]); });
    await click('模型'); await click('决策模型');
    const rotate = n => { store.sources[0] = { ...store.sources[0], credential_state_revision: 'sha256:' + String(n).repeat(64) }; };
    const state = () => document.querySelector('.decision-test-result')?.dataset.testState;
    const tests = () => calls('test_decision_service_v2').length;
    store.test = () => { rotate(8); return { outcome: 'passed' }; };
    await click('测试连接'); await until(() => state() && state() !== 'testing', 'test with rotation');
    assert(state() === 'stale' && text().includes('凭据或配置已变化') && !text().includes('连接测试通过'), `Key rotation during the test still showed a current pass: ${state()}`);
    store.test = () => ({ outcome: 'passed' });
    await click('重新测试'); await until(() => state() === 'passed', 'test on unchanged state');
    const detour = async () => { await click('编辑'); await click('管理 API 接入'); await until(() => button('返回决策模型 — 草稿已保留'), 'detour'); };
    const back = async () => { const reads = calls('decision_services_v2').length; await click('返回决策模型 — 草稿已保留'); await until(() => calls('decision_services_v2').length > reads, 'return readback'); await pause(60); await click('取消'); };
    await detour(); await back();
    await until(() => state() === 'passed', 'unchanged readback keeps the pass');
    const read = c().handlers.decision_services_v2;
    await detour(); c().handlers.decision_services_v2 = () => { throw { code: 'DAEMON_UNAVAILABLE' }; }; await back();
    await until(() => state() === 'unverified', 'failed readback leaves the pass unverified');
    assert(text().includes('未能读取当前凭据状态') && !text().includes('连接测试通过') && all('.decision-test-state.stale').length === 1, 'Failed readback still presented a current pass');
    assert(tests() === 2, 'Returning from management retested implicitly');
    c().handlers.decision_services_v2 = read;
    await click('刷新'); await until(() => state() === 'passed', 'successful readback confirms the admitted state');
    assert(tests() === 2, 'Recovery retested implicitly');
    // An older activation read must not overwrite state refreshed after an explicit new test.
    await detour();
    const earlier = structuredClone(read());
    rotate(9);
    let defer = true;
    const releaseEarlier = [];
    c().handlers.decision_services_v2 = () => defer
      ? new Promise(resolve => { releaseEarlier.push(() => resolve(earlier)); }) : read();
    await back(); defer = false;
    await click(button('测试连接') ? '测试连接' : '重新测试');
    await until(() => tests() === 3 && state() === 'passed', 'test against the current Key');
    releaseEarlier.forEach(release => release()); await pause(120);
    assert(state() === 'passed', 'An older read overwrote the state confirmed by a newer test');
    assert(tests() === 3, 'Overlapping refreshes retested implicitly');
  }),
  scenario('desktop.decisions.source-read-states', ['decision-services'], 'Slow or failed connection reads never block independent setup, and a late read never overrides an explicit choice', async () => {
    let release;
    await fresh(() => {
      decisionStore();
      c().handlers.decision_connection_sources = () => new Promise(resolve => { release = () => resolve({ sources: [c().decisionSource('source/late')] }); });
    });
    await click('模型'); await click('决策模型'); await click('添加决策模型');
    await until(() => text().includes('正在读取 API 接入…') && button('改为独立配置'), 'loading escape');
    await click('改为独立配置');
    assert(nativeField('credential'), 'Loading escape did not open independent setup');
    release(); await until(() => calls('decision_connection_sources').length && !text().includes('正在读取 API 接入…'), 'late source read');
    assert(all('.decision-mode')[1].getAttribute('aria-pressed') === 'true' && nativeField('credential'), 'A late read overrode the explicit independent choice');
    let fail = true;
    await fresh(() => {
      decisionStore();
      c().handlers.decision_connection_sources = () => { if (fail) throw { code: 'DAEMON_UNAVAILABLE' }; return { sources: [c().decisionSource('source/retried')] }; };
    });
    await click('模型'); await click('决策模型'); await click('添加决策模型');
    await until(() => text().includes('暂时无法读取 API 接入。'), 'source read error');
    assert(button('重试') && button('独立配置') && !all('.decision-mode').some(item => item.getAttribute('aria-pressed') === 'true'), 'Read error chose a mode or lacks recovery');
    fail = false; await click('重试');
    await until(() => all('.decision-mode')[0]?.getAttribute('aria-pressed') === 'true', 'retry defaults to reuse after a ready read');
    assert(nativeField('source').value === '', 'Retry chose an API connection for the user');
  }),
  scenario('desktop.decisions.selector-labels', ['decision-services', 'routing-editor'], 'Route selectors accept V1 and V2 records, distinguish same-name models and keep the classifier kind exact', async () => {
    const legacy = { id: 'decision/legacy', revision: 1, name: 'Legacy decisions', connection: { kind: 'system_one', provider: 'bailian-token-plan', model: 'qwen-decision', endpoint: 'https://legacy.example.test/systemone', timeout_ms: 10000, auth_header: { name: 'Authorization', value_secret_ref: 'protected/legacy' } } };
    const shared = (id, sourceId, model) => ({ schema: 'hiroute.decision-service/v2', id, revision: 1, name: 'Decisions', connection: { kind: 'native', provider: 'openai', wire_protocol: 'openai_decisions', model, endpoint: 'https://api.openai.test/v1/decisions', timeout_ms: 10000, authentication: { kind: 'shared_source', source_id: sourceId, source_lineage_digest: c().decisionSource(sourceId).lineage_digest } } });
    await fresh(() => { decisionStore([legacy, shared('decision/a', 'source/team-a', 'gpt-6-luna'), shared('decision/b', 'source/team-a', 'gpt-6-sol')], [c().decisionSource('source/team-a', { display_name: 'Team API' })]); });
    await routing(); await click('决策模型');
    const select = () => all('.decision-selector select')[0];
    await until(() => select()?.options.length === 4, 'decision choices');
    const labels = [...select().options].slice(1).map(option => option.textContent);
    assert(new Set(labels).size === 3 && labels.includes('Legacy decisions'), `Choices are ambiguous or omit V1: ${labels.join(' | ')}`);
    assert(labels.includes('Decisions · gpt-6-luna · Team API') && labels.includes('Decisions · gpt-6-sol · Team API'), `Same-name choices lack model and source: ${labels.join(' | ')}`);
    choose(select(), 'decision/b'); await pause(40);
    await click('保存草稿');
    await until(() => calls('preview_plan_editor').length > 0, 'draft with V2 classifier');
    const classifier = calls('preview_plan_editor').at(-1).payload.input.editor.smart.classifier;
    assert(classifier.kind === 'decision_service_v2' && classifier.service.id === 'decision/b' && classifier.service.revision === 1, 'V2 selection was not serialized with its exact kind and revision');
    choose(select(), 'decision/legacy'); await pause(40);
    await until(() => !button('保存草稿').disabled, 'draft action finished');
    await click('保存草稿');
    await until(() => calls('preview_plan_editor').at(-1).payload.input.editor.smart.classifier.kind === 'decision_service', 'V1 selection keeps its kind');
  }),
  scenario('desktop.decisions.route-detour', ['decision-services', 'routing-editor'], 'Adding a custom extension preserves the route and selects its exact saved version without publication', async () => {
    let store;
    await fresh(() => { store = decisionStore(); });
    await routing();
    const routeName = () => all('.plan-identity-fields input')[0];
    setInput(routeName(), 'Preserved route draft'); await pause(40);
    await click('自定义扩展'); await click('接入自定义扩展');
    assert(!document.querySelector('dialog[open]') && text().includes('路由草稿已保留'), 'Detour tried to discard the route');
    const form = () => all('.decision-connection-form')[0];
    const input = name => form()?.querySelector(`[name="${name}"]`);
    setInput(input('endpoint'), 'https://extension.example.test/decision');
    const auth = form().querySelector('select'); auth.value = 'bearer'; auth.dispatchEvent(new Event('change', { bubbles: true })); await pause(40);
    setInput(input('credential'), 'extension-token');
    setInput(input('name'), 'Review extension'); await pause(40);
    await click('查看接入协议'); await click('完成');
    assert(input('credential').value === 'extension-token', 'Protocol dialog cleared authentication');
    await click('仅保存');
    await until(() => routeName()?.value === 'Preserved route draft', 'return to original draft');
    assert(calls('save_decision_service_v2').at(-1).payload.input.secret === 'Bearer extension-token', 'Custom bearer auth was not serialized correctly');
    assert(all('.decision-selector select')[0]?.value === store.services[0].id, 'Saved extension was not selected');
    assert(calls('preview_plan_editor').length === 0 && calls('test_decision_service_v2').length === 0, 'Connection save published the route or save-only ran a test');
    await click('启发式规则'); await click('自定义扩展');
    assert(all('.decision-selector select')[0]?.value === store.services[0].id, 'Changing decision method lost the selected extension');
    await click('决策模型');
    assert(all('.decision-selector select')[0].options.length === 1, 'Extension leaked into decision model choices');
    await click('添加决策模型'); await click('返回路由'); await click('放弃修改');
    assert(routeName()?.value === 'Preserved route draft', 'Cancelling detour discarded the route draft');
  }),
  scenario('desktop.decisions.nested-detour', ['decision-services', 'routing-editor', 'model-connections'], 'Route -> decision model -> API connection -> decision model -> route returns explicitly at each level, keeps every non-secret bundle and still guards discards', async () => {
    let store;
    const sourceId = 'source/openai/decisions';
    await fresh(() => { store = decisionStore([], [c().decisionSource(sourceId, { display_name: 'Team OpenAI' })]); });
    await routing();
    const routeName = () => all('.plan-identity-fields input')[0];
    setInput(routeName(), 'Nested route draft'); await pause(40);
    await click('决策模型'); await click('添加决策模型');
    assert(!document.querySelector('dialog[open]') && text().includes('路由草稿已保留'), 'Opening the decision model tried to discard the route');
    const input = nativeField, mode = index => all('.decision-mode')[index];
    await until(() => mode(0)?.getAttribute('aria-pressed') === 'true', 'reuse default');
    choose(input('source'), sourceId); await pause(40);
    setInput(input('model'), 'gpt-6-luna-mini'); setInput(input('name'), 'Nested decisions'); await pause(40);
    mode(1).click(); await until(() => input('credential'), 'independent bundle');
    setInput(input('model'), 'independent-nested-model'); setInput(input('credential'), 'nested-pending-key'); await pause(40);
    mode(0).click(); await pause(40);
    await click('管理 API 接入');
    await until(() => button('返回决策模型 — 草稿已保留') && !all('.decision-connection-form').length, 'API connection management with explicit return');
    await until(() => all('.detail-identity').some(item => item.textContent.includes('Team OpenAI')), 'the selected API connection is open');
    await click('首页');
    await until(() => document.querySelector('dialog.discard-dialog[open]') && button('继续编辑'), 'leaving the detour is guarded');
    await click('继续编辑');
    assert(button('返回决策模型 — 草稿已保留'), 'Keep editing left the detour');
    await click('返回决策模型 — 草稿已保留');
    await until(() => input('name')?.value === 'Nested decisions', 'decision model draft after the API connection');
    assert(mode(0).getAttribute('aria-pressed') === 'true' && input('source').value === sourceId && input('model').value === 'gpt-6-luna-mini', 'Shared bundle changed across the detour');
    assert(button('返回路由') && text().includes('路由草稿已保留'), 'The route return was lost inside the nested detour');
    mode(1).click(); await until(() => input('credential'), 'independent bundle after detour');
    assert(input('model').value === 'independent-nested-model' && input('credential').value === '', 'Independent non-secret bundle lost or pending key resurrected');
    mode(0).click(); await pause(40);
    await click('首页');
    await until(() => document.querySelector('dialog.discard-dialog[open]') && button('继续编辑'), 'decision model discard guard');
    await click('继续编辑');
    assert(input('name')?.value === 'Nested decisions', 'Keep editing lost the decision draft');
    await click('仅保存');
    await until(() => routeName()?.value === 'Nested route draft', 'explicit return to the route draft');
    const saved = calls('save_decision_service_v2').at(-1).payload.input;
    assert(saved.secret === null && saved.change.service.connection.model === 'gpt-6-luna-mini' && JSON.stringify(saved.change.expected_source_revisions) === JSON.stringify([{ source_id: sourceId, revision: 1 }]), 'Nested save submitted the wrong bundle, a key or the wrong source closure');
    assert(all('.decision-selector select')[0]?.value === store.services[0].id, 'Saved decision model was not selected in the route');
    assert(calls('preview_plan_editor').length === 0 && calls('test_decision_service_v2').length === 0 && !store.rejected.length, 'Nested flow published, tested or sent a rejected request');
    await click('首页');
    await until(() => document.querySelector('dialog.discard-dialog[open]') && button('继续编辑'), 'route draft discard guard after return');
    await click('继续编辑');
    assert(routeName()?.value === 'Nested route draft', 'Keep editing lost the route draft');
  }),
  scenario('desktop.decisions.discard-navigation', ['decision-services'], 'Discarding decision edits clears retained editors and credentials across navigation', async () => {
    const saved = { id: 'decision/saved', revision: 1, name: 'Saved decision', connection: {
      kind: 'system_one', provider: 'bailian-token-plan', model: 'decision-model-preview',
      endpoint: 'https://example.test/systemone', timeout_ms: 10000,
      auth_header: { name: 'Authorization', value_secret_ref: 'decision/saved/r1' },
    } };
    await fresh(() => { decisionStore([saved]); });
    await click('模型'); await click('决策模型');
    const editor = () => all('.decision-connection-form')[0];
    await until(() => text().includes(saved.name), 'saved decision');
    await click('编辑');
    assert(editor().querySelector('button[type="submit"]').disabled, 'An untouched V1 record opened as V2 is dirty');
    await click('取消');
    for (const creating of [false, true]) {
      if (creating) { await click('添加决策模型'); await until(() => nativeField('credential'), 'new independent setup'); }
      else { await click('编辑'); await click('更换'); }
      setInput(nativeField('name'), 'Unsaved decision');
      setInput(nativeField('credential'), 'synthetic-discard-value');
      await click('通用模型'); await click('继续编辑');
      assert(nativeField('name').value === 'Unsaved decision', 'Keep editing lost the name');
      assert(nativeField('credential').value === 'synthetic-discard-value', 'Keep editing lost the credential');
      await click('首页'); await click('放弃修改'); await click('模型');
      await until(() => !editor() && text().includes(saved.name), 'discard restores saved detail');
      await click('编辑'); await click('更换');
      assert(nativeField('credential').value === '', 'Discard retained the unsaved credential');
      assert(editor().querySelector('button[type="submit"]').disabled, 'Discard left a dirty editor');
      await click('取消'); await click('首页');
      assert(!document.querySelector('dialog[open]'), 'Discarded editor prompted again');
      await click('模型');
    }
    assert(calls('save_decision_service_v2').length === 0, 'Discard persisted an edit');
  }),
  scenario('desktop.settings.fixed-scale', ['desktop-presentation'], 'Settings omit text size and normalize old enlarged preferences without losing language or appearance', async () => {
    await fresh(); await click('设置');
    const titleSize = getComputedStyle(all('.main .page-title')[0]).fontSize;
    for (const previousScale of ['1.5', '2']) {
      localStorage.setItem('hiroute.language', 'zh');
      localStorage.setItem('hiroute.theme', 'dark');
      localStorage.setItem('hiroute.text-scale', previousScale);
      c().remount(); await click('设置');
      assert(document.querySelector('.hr-ui').dataset.textScale === '1' && getComputedStyle(all('.main .page-title')[0]).fontSize === titleSize, 'Previous text size is still applied by the real shell');
      assert(localStorage.getItem('hiroute.text-scale') === '1', 'Previous scale was not normalized in storage');
      assert(!text().includes('文字大小') && !['100%', '150%', '200%'].some(label => button(label)), 'Text size controls remain in Settings');
      assert(button('深色').getAttribute('aria-pressed') === 'true', 'Normalizing text changed the appearance preference');
      await click('English'); await click('Light');
      c().remount(); await click('Settings');
      assert(document.querySelector('.hr-ui').lang === 'en' && document.querySelector('.hr-ui').dataset.theme === 'light', 'Language or appearance did not survive remount');
      assert(document.querySelector('.hr-ui').dataset.textScale === '1' && localStorage.getItem('hiroute.text-scale') === '1', 'Remount restored the enlarged scale');
      assert(!text().includes('Text size') && !['100%', '150%', '200%'].some(label => button(label)), 'English Settings still expose text size controls');
    }
  }),
  scenario('desktop.operation.dismiss-observes', ['operation-feedback'], 'Closing feedback leaves the operation observable and never cancels it', async () => {
    await pending();
    const before = calls('observe_operation').length;
    feedback().querySelector('button[aria-label="关闭"]').click();
    await until(() => !feedback(), 'feedback closed');
    await until(() => calls('observe_operation').length > before, 'observation continues after closing');
    assert(!c().commands.some(({ command }) => /cancel|stop_observ/.test(command)), 'Closing feedback cancelled or stopped the operation');
  }),
  scenario('desktop.operation.identity-retry-converges', ['operation-feedback'], 'A mismatched observation stays neutral and retry converges on the accepted operation', async () => {
    await pending();
    const before = calls('observe_operation').length;
    c().handlers.observe_operation = () => operation('succeeded', 'operation/unrelated');
    await until(() => calls('observe_operation').length > before, 'first mismatched observation');
    assert(!feedback().dataset.observationErrorCode, 'A transient mismatch skipped the recovery grace period');
    await until(() => feedback()?.dataset.operationPhase === 'unverified', 'unconfirmed result becomes retryable', 12000);
    assert(feedback().dataset.observationErrorCode === 'OPERATION_IDENTITY_MISMATCH', 'Mismatch was not diagnosed');
    assert(feedback().querySelector('.oc-spinner') && !feedback().querySelector('.callout.bad'), 'Unknown result was presented as terminal failure');
    c().handlers.observe_operation = () => operation('succeeded');
    c().desktop.pending = null;
    const reads = calls('desktop_snapshot').length;
    await click('重新查询');
    await until(() => feedback()?.dataset.operationPhase === 'succeeded', 'retry observes the accepted identity');
    await until(() => calls('desktop_snapshot').length > reads, 'success refreshes actual state');
    await until(() => !feedback(), 'success feedback dismisses itself', 6500);
  }),
  scenario('desktop.operation.repeated-submission', ['operation-feedback'], 'Repeated accepted and unverified Agent saves each resume operation observation', async () => {
    for (const knownIdentity of [true, false]) {
      await fresh(() => {
        const agent = c().agents.agents.find(item => item.agent_id === 'agent_codex_default');
        agent.configuration_state = 'not_configured';
        agent.settings.current_selection = null;
        agent.settings.state = 'not_configured';
        c().agents.plans.plans = c().agents.plans.plans.filter(plan => plan.head.status === 'enabled').slice(0, 1);
        c().handlers.preview_agent_settings = () => ({ preview: { applicable: true, blockers: [] }, mutation: { state: knownIdentity ? 'applied' : 'response_unknown', operation: knownIdentity ? operation() : null } });
      });
      await click('Agent');
      await until(() => all('.native-list .list-row').some(item => item.textContent.includes('Codex')), 'Codex row');
      all('.native-list .list-row').find(item => item.textContent.includes('Codex')).click();
      await until(() => document.querySelector('[data-agent-facet="model"]'), 'model configuration');
      document.querySelector('[data-agent-facet="model"]').click();
      for (let attempt = 0; attempt < 2; attempt++) {
        if (attempt && knownIdentity) document.querySelector('[data-agent-facet="model"]').click();
        await until(() => document.querySelector('[role="dialog"] button[type="submit"]')?.disabled === false, 'save enabled');
        const before = calls('observe_operation').length;
        document.querySelector('[role="dialog"] button[type="submit"]').click();
        await until(() => calls('preview_agent_settings').length === attempt + 1, 'save submitted');
        await until(() => calls('observe_operation').length > before, `${knownIdentity ? 'accepted' : 'unverified'} save ${attempt + 1} is observed`, 4000);
      }
    }
  }),
  scenario('desktop.routing.qoder-budget-checkpoint', ['routing-editor', 'qoder'], 'An accepted publication that fails its Qoder budget checkpoint explains model reconnection and retains edits', async () => {
    await fresh(() => {
      c().handlers.preview_plan_editor = () => ({ state: 'applied', operation: operation() });
      c().handlers.observe_operation = () => ({ ...operation('rolled_back'), sequence: 2, safe_error_code: 'QODER_MODEL_BUDGET_CONFLICT' });
    });
    await routing();
    const contextMode = document.querySelector('.plan-context-mode');
    assert(contextMode && visible(contextMode), 'Context window mode is not reachable');
    contextMode.value = 'custom';
    contextMode.dispatchEvent(new Event('change', { bubbles: true }));
    await until(() => visible(document.querySelector('.plan-context-window-field')), 'custom context window');
    const contextWindow = document.querySelector('.plan-context-window-field');
    setInput(contextWindow, '32000');
    await until(() => document.querySelector('[data-codex-capability-summary]')?.textContent.includes('32K'), 'model options for the edited context window');
    await click('发布更改');
    await until(() => feedback()?.dataset.operationPhase === 'failed', 'accepted publication checkpoint failure');
    const guidance = feedback().querySelector('[data-error-code="QODER_MODEL_BUDGET_CONFLICT"]');
    assert(guidance?.textContent.includes('停用 Qoder 的模型路由') && guidance.textContent.includes('再发布此计划并重新配置模型路由') && guidance.textContent.includes('任务协作无需停用'), 'Checkpoint failure lacks independent model recovery steps');
    const publication = calls('preview_plan_editor').at(-1)?.payload.input;
    assert(publication?.action === 'publish' && publication.editor.limits.context_window_tokens === 32000, 'The accepted request did not publish the edited budget');
    assert(contextWindow.value === '32000' && !contextWindow.disabled && !button('发布更改').disabled, 'Checkpoint failure discarded or locked the local edit');
    assert(calls('preview_agent_settings').length === 0 && calls('check_agent_live').length === 0, 'Checkpoint failure implicitly changed an Agent connection or called a model');
  }),
  scenario('desktop.routing.additional-budget-consumers', ['routing-editor', 'pi', 'dsh', 'opencode'], 'Both admission and checkpoint failures identify installed additional consumers and keep the route draft', async () => {
    for (const checkpoint of [false, true]) {
      await fresh(() => {
        const plan = c().desktop.catalog.plans[0];
        const base = c().agents.agents[0];
        for (const [id, mode] of [['agent_pi_default', 'pi_additional'], ['agent_dsh_default', 'dsh_additional'], ['agent_opencode_default', 'opencode_additional']]) {
          c().agents.agents.push({ ...base, agent_id: id, context_id: `agent-context/${mode}/sha256:abc`, settings: { ...base.settings, current_selection: { mode, allowed_plan_ids: [plan.agent_plan_id] } } });
        }
        const conflict = { schema: 'hiroute.agent-model-budget-conflict/v1', blockers: [
          { agent_kind: 'pi', context_id: 'agent-context/pi/sha256:abc', plan_id: plan.agent_plan_id, model_alias: plan.model_alias, reason: 'capacity_reduced', declared: { context_window_tokens: 262144, max_output_tokens: 235929 }, proposed: { context_window_tokens: 262144, max_output_tokens: 131072 } },
          { agent_kind: 'deepseek_harness', context_id: 'agent-context/dsh/sha256:abc', plan_id: plan.agent_plan_id, model_alias: plan.model_alias, reason: 'capacity_reduced', declared: { context_window_tokens: 262144, max_output_tokens: 235929 }, proposed: { context_window_tokens: 262144, max_output_tokens: 131072 } },
          { agent_kind: 'opencode', context_id: 'agent-context/opencode/sha256:abc', plan_id: plan.agent_plan_id, model_alias: plan.model_alias, reason: 'capacity_reduced', declared: { context_window_tokens: 262144, max_output_tokens: 235929 }, proposed: { context_window_tokens: 262144, max_output_tokens: 131072 } },
        ] };
        c().handlers.preview_plan_editor = checkpoint ? () => ({ state: 'applied', operation: operation() }) : () => { throw { source: 'backend', envelope: { error: { code: 'REVISION_CONFLICT', model_budget_conflict: conflict } } }; };
        c().handlers.observe_operation = () => ({ ...operation('rolled_back'), sequence: 2, safe_error_code: 'AGENT_MODEL_BUDGET_CONFLICT', model_budget_conflict: conflict });
      });
      await routing();
      const usage = document.querySelector('.plan-editor .agent-card-list');
      assert(usage?.textContent.includes('Pi') && usage.textContent.includes('DeepSeek Harness') && usage.textContent.includes('OpenCode') && !usage.textContent.includes('Qoder'), 'Where it is used omits an installed additional consumer or includes an unrelated Qoder');
      const name = document.querySelector('.plan-identity-fields input');
      const draftName = 'Budget retained draft with a long route name';
      setInput(name, draftName);
      await until(() => document.querySelector('[data-codex-capability-state=available]'), 'current model options before publication');
      const header = document.querySelector('.plan-editor .editor-header').getBoundingClientRect();
      const publish = button('发布更改').getBoundingClientRect();
      assert(publish.left >= header.left - 1 && publish.right <= header.right + 1,
        'The retained draft title pushes publication outside the editor');
      await click('发布更改');
      await until(() => (checkpoint ? feedback() : document.querySelector('.plan-editor [role="alert"]'))?.textContent.includes('最大输出 235,929 → 131,072'), 'typed output-budget rejection');
      const message = (checkpoint ? feedback() : document.querySelector('.plan-editor [role="alert"]')).textContent;
      assert(message.includes('Pi') && message.includes('DeepSeek Harness') && message.includes('OpenCode') && !message.includes('Qoder') && message.includes('任务协作无需停用'), 'Recovery guidance misattributes the constraint');
      assert(name.value === draftName && !name.disabled && !button('发布更改').disabled, 'Budget rejection discarded or locked the draft');
      assert(calls('preview_agent_settings').length === 0 && calls('check_agent_live').length === 0, 'Budget rejection implicitly changed native routes or called a model');
    }
  }),
  scenario('desktop.home.reactivation-usage', ['home', 'sessions'], 'Returning home refreshes usage alongside recent activity without remounting the app', async () => {
    let input = 2400;
    const usage = () => document.querySelector('.v3-usage-body')?.textContent ?? '';
    await fresh(() => {
      c().handlers.observation_read = payload => {
        const result = c().fixtureResponse('observation_read', payload);
        if (payload.request.intent.view !== 'home_value' || payload.request.intent.query.session_id) return result;
        return { ...result, usage: result.usage.map(metric => metric.metric === 'input' ? { ...metric, known_sum: input } : metric) };
      };
    });
    await until(() => usage().includes('2,400'), 'initial usage');
    await click('会话');
    input = 9322197;
    const before = calls('observation_read').filter(call => call.payload.request.intent.view === 'home_value').length;
    await click('首页');
    await until(() => usage().includes('9,322,197'), 'new usage on ordinary home navigation');
    assert(calls('observation_read').filter(call => call.payload.request.intent.view === 'home_value').length > before, 'Returning home reused the old summary');
    assert(usage().includes('尚未计价') && !usage().includes('2,400'), 'Refresh fabricated a price or retained the old total');
  }),
  scenario('desktop.routing.reactivation-models', ['routing-editor'], 'Returning to an existing route reloads saved model choices and scopes quality to active models', async () => {
    await fresh(); await routing();
    await click('模型表现');
    await until(() => document.querySelector('.quality-model-scope')?.textContent.includes('Qwen'), 'active model scope');
    assert(document.querySelector('.quality-model-scope').textContent.includes('Qwen'), 'Active route models are absent');
    assert(!document.querySelector('.quality-model-filter'), 'Quality asks users to type internal model IDs');
    const before = calls('compute_management_snapshot').length;
    const optionsBefore = calls('plan_editor_options').length;
    await click('首页');
    const revisedName = 'Saved renamed model';
    c().management.sources.find(source => source.source_id === 'source/bailian/coding').models[0].display_name = revisedName;
    c().handlers.plan_editor_options = payload => {
      const options = c().fixtureResponse('plan_editor_options', payload);
      return { ...options, candidates: options.candidates.map(candidate => candidate.binding_id === 'binding/bailian/qwen-coder' ? { ...candidate, display_name: revisedName } : candidate) };
    };
    await click('智能路由');
    await until(() => calls('compute_management_snapshot').length > before && calls('plan_editor_options').length > optionsBefore, 'fresh saved choices after reactivation');
    assert(document.querySelector('.plan-identity-fields input'), 'Returning lost the existing editor');
    await until(() => document.querySelector('.quality-model-scope')?.textContent.includes(revisedName), 'latest saved model name is visible');
  }),
  scenario('desktop.models.documentation-retains-input', ['model-connections'], 'Model readiness stays bounded and a failed documentation link preserves input', async () => {
    await fresh(); await click('模型');
    await until(() => all('button').some(item => item.textContent.includes('GPT-6-Astra')), 'model row');
    all('button').find(item => item.textContent.includes('GPT-6-Astra')).click();
    await until(() => text().includes('接入就绪'), 'readiness status');
    assert(text().includes('不代表上游推理或工具调用已验证'), 'Local readiness claims upstream inference verification');
    const key = await newBailianAPI();
    setInput(key, 'synthetic-input-preserved'); await pause(40);
    const link = all('[role="dialog"] a').find(item => item.textContent.includes('获取 API Key'));
    assert(link?.target === '_blank' && link.rel.includes('noreferrer') && link.href.startsWith('https://'), 'Web documentation link is unavailable');
    if (window.fixtureTransport === 'console') {
      const prevent = event => { assert(!event.defaultPrevented, 'Browser anchor was intercepted'); event.preventDefault(); };
      document.addEventListener('click', prevent, { once: true }); link.click(); await pause(30);
      assert(key.value === 'synthetic-input-preserved' && calls('open_external_url').length === 0, 'Browser documentation navigation lost input or invoked native IPC');
      return;
    }
    c().handlers.open_external_url = () => { throw new Error('BROWSER_UNAVAILABLE'); };
    link.click(); await until(() => text().includes('当前表单内容已保留'), 'native browser failure feedback');
    assert(calls('open_external_url').at(-1)?.payload.url === link.href, 'Native command did not receive the displayed documentation URL');
    assert(key.value === 'synthetic-input-preserved', 'Opening documentation lost the API key input');
    // Prevent an actual external navigation while observing the normal web default action.
    const internals = window.__TAURI_INTERNALS__;
    let preventedByProduct;
    const observeClick = event => { preventedByProduct = event.defaultPrevented; event.preventDefault(); };
    document.addEventListener('click', observeClick);
    try { delete window.__TAURI_INTERNALS__; link.click(); }
    finally { window.__TAURI_INTERNALS__ = internals; document.removeEventListener('click', observeClick); }
    assert(preventedByProduct === false && calls('open_external_url').length === 1, 'Web preview does not retain ordinary anchor behavior');
  }),
  scenario('desktop.routing.classifier-protocol', ['routing-editor'], 'Classifier choice and protocol copy/save expose visible outcomes', async () => {
    await fresh(); await routing();
    const external = button('决策模型');
    assert(external, 'Decision model choice is missing'); external.click(); await pause(40);
    assert(external.getAttribute('aria-pressed') === 'true', 'Decision choice is not visibly selected');
    const local = button('启发式规则');
    local.click(); await pause(40);
    assert(local.getAttribute('aria-pressed') === 'true' && external.getAttribute('aria-pressed') === 'false', 'Heuristic choice did not replace the decision selection');
    await click('自定义扩展');
    c().handlers.decision_services_v2 = () => ({ services: [] });
    await click('接入自定义扩展');
    assert(!document.querySelector('dialog[open]'), 'Connection detour discarded the route');
    await click('查看接入协议');
    await click('复制 curl');
    await until(() => c().clipboard.length === 1, 'curl copied');
    const curl = c().clipboard[0];
    assert(curl.includes('https://classifier.example/v1/decisions'), 'Curl targets the wrong API');
    for (const field of ['route', 'latest_user', 'visible_conversation', 'history_partial', 'preceding_stage']) assert(curl.includes(`"${field}"`), `Curl lacks ${field}`);
    assert(all('.classifier-protocol pre')[1].textContent.includes('regular') && !all('.classifier-protocol pre')[1].textContent.includes('assessment'), 'First decision fabricated an assessment');
    await click('自定义分支 · 分类并评分');
    assert(all('.classifier-protocol pre')[0].textContent.includes('"from": 0'), 'Assessment request has no prior stage');
    assert(all('.classifier-protocol pre')[1].textContent.includes('review') && text().includes('评分只属于同类别的前一实际阶段'), 'Assessment targets the new branch');
    if (window.fixtureTransport === 'console') {
      await click('保存 OpenAPI'); await until(() => text().includes('已开始下载') || text().includes('保存失败'), 'browser download outcome');
      if (text().includes('保存失败')) await c().callHost('save_classifier_openapi');
      assert(c().downloads.length === 1, 'Browser did not produce exactly one Blob download');
      const document = JSON.parse(await c().downloads[0].text());
      assert(document.openapi === '3.1.0' && document.paths['/v1/decisions'], 'Browser download differs from the canonical OpenAPI');
      assert(calls('save_classifier_openapi').length === 0, 'Browser requested a native save command');
      return;
    }
    let finish;
    c().handlers.save_classifier_openapi = () => new Promise(resolve => { finish = resolve; });
    await click('保存 OpenAPI');
    assert(button('保存 OpenAPI').disabled && text().includes('请选择保存位置'), 'Native save did not show a pending state');
    finish('cancelled'); await until(() => text().includes('已取消保存'), 'cancelled save outcome');
    c().handlers.save_classifier_openapi = () => 'saved';
    await click('保存 OpenAPI'); await until(() => text().includes('已保存：hiroute-decision.openapi.json'), 'saved file outcome');
    assert(calls('save_classifier_openapi').length === 2, 'Save did not use exactly one native command per click');
  }),
];
export const productShellScenarioCatalog = scenarioCatalog(scenarios);
export const runProductShellScenarios = (selection = { requiredIds: productShellRequired }) => runScenarios(scenarios, selection);
