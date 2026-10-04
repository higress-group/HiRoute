import { scenario, scenarioCatalog, runScenarios } from './scenario-selection.mjs';
import { productShellRequired } from './scenario-requirements.mjs';
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
async function fresh(setup = () => {}) {
  c().reset(); setup(); await until(() => text().includes('仅在本机运行'), 'ready DesktopApp');
}
function setInput(input, value) {
  assert(input, 'Expected an editable input');
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
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
  scenario('desktop.settings.persisted-scale', ['desktop-presentation'], 'Settings apply and persist all supported text scales through the shell', async () => {
    await fresh(); await click('设置');
    for (const [label, value] of [['150%', '1.5'], ['200%', '2'], ['100%', '1']]) {
      await click(label);
      assert(button(label).getAttribute('aria-pressed') === 'true', 'Scale choice did not become selected');
      assert(document.querySelector('.hr-ui').dataset.textScale === value, 'Shell did not apply the selected scale');
      assert(localStorage.getItem('hiroute.text-scale') === value, 'Selected scale was not persisted');
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
  scenario('desktop.routing.reactivation-models', ['routing-editor'], 'Returning to an existing route reloads saved model choices and scopes quality to active models', async () => {
    await fresh(); await routing();
    await until(() => document.querySelector('.quality-model-scope'), 'active model scope');
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
    await click('添加模型');
    await until(() => all('button').some(item => item.textContent.includes('添加 API')), 'add API entry');
    all('button').find(item => item.textContent.includes('添加 API')).click();
    await until(() => document.querySelector('[role="dialog"] input[type="password"]'), 'API key input');
    const key = document.querySelector('[role="dialog"] input[type="password"]');
    setInput(key, 'synthetic-input-preserved'); await pause(40);
    const link = all('a').find(item => item.textContent.includes('官方说明与获取 Key'));
    assert(link?.target === '_blank' && link.rel.includes('noreferrer') && link.href.startsWith('https://'), 'Web documentation link is unavailable');
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
    const custom = all('.classifier-choice').find(item => item.textContent.includes('自定义分类服务'));
    assert(custom, 'Custom classifier choice is missing'); custom.click(); await pause(40);
    assert(custom.getAttribute('aria-pressed') === 'true' && visible(custom.querySelector('svg')), 'Custom choice is not visibly selected');
    const local = all('.classifier-choice').find(item => item.textContent.includes('内置规则'));
    local.click(); await pause(40);
    assert(local.getAttribute('aria-pressed') === 'true' && custom.getAttribute('aria-pressed') === 'false', 'Built-in choice did not replace the custom selection');
    custom.click(); await pause(40);
    await click('查看接入协议');
    await click('复制 curl');
    await until(() => c().clipboard.length === 1, 'curl copied');
    const curl = c().clipboard[0];
    assert(curl.includes('https://classifier.example/v1/decisions'), 'Curl targets the wrong API');
    for (const field of ['branches', 'latest_user', 'visible_conversation', 'history_partial', 'assessment_from']) assert(curl.includes(`"${field}"`), `Curl lacks ${field}`);
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
