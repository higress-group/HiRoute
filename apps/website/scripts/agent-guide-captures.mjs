import { evaluate, navigate, waitFor } from '../../desktop/tests/v3/visual/cdp-client.mjs';

const agentEditors = {
  'agent-guide-claude': ['claude', 'Claude Code', 'Claude 日常开发', 'Claude daily coding'],
  'agent-guide-qoder': ['qoder', 'Qoder', 'Qoder 日常开发', 'Qoder daily coding'],
  'agent-guide-pi': ['pi', 'Pi', 'Pi 日常开发', 'Pi daily coding'],
  'agent-guide-workbuddy-pi': ['pi', 'Pi', 'WorkBuddy 资料整理', 'WorkBuddy document review'],
  'agent-guide-dsh': ['dsh', 'DeepSeek Harness Web', 'DSH 分析', 'DSH analysis'],
  'agent-guide-opencode': ['opencode', 'OpenCode', 'OpenCode 代码审查', 'OpenCode code review'],
  'agent-guide-hermes': ['hermes', 'Hermes', 'Hermes 日常对话', 'Hermes daily chat'],
};
const workerPlans = {
  'agent-guide-pi-worker': ['pi', 'Pi', 'Qoder 日常开发', 'Qoder daily coding'],
  'agent-guide-dsh-worker': ['deepseek_harness', 'DeepSeek Harness', 'DSH 分析', 'DSH analysis'],
};
export const agentGuideCaptureStates = [
  ...Object.keys(agentEditors),
  'agent-guide-magpie-api', 'agent-guide-qoder-models', 'agent-guide-difficulty',
  'agent-guide-subscription-login', 'agent-guide-codex-subscription-models',
  'agent-guide-claude-subscription-models', 'agent-guide-codex-desktop',
  'agent-guide-codex-desktop-launch',
  ...Object.keys(workerPlans),
];

// Use the loopback endpoint and two models from the local catalog. Everything
// else is disposable presentation data.
const magpie = {
  address: 'http://127.0.0.1:3425/v1',
  models: [
    { id: 'qoder/dfmodel', name: 'DeepSeek-Flash' },
    { id: 'qoder/kmodel_latest', name: 'Kimi-K3.1-Pro' },
  ],
};
const viewport = { width: 1200, height: 900 };
const translate = (language, zh, en) => language === 'zh' ? zh : en;

async function agentEditor(client, language, click, [kind, label, zh, en]) {
  const id = `agent_${kind}_default`;
  const title = translate(language, zh, en);
  await evaluate(client, `(() => {
    const c = window.productShell;
    const kind = ${JSON.stringify(kind)};
    const id = ${JSON.stringify(id)};
    const plan = c.agents.plans.plans[0];
    plan.desired.display_name = ${JSON.stringify(title)};
    c.agents.plans.plans = [plan];
    c.desktop.catalog.plans = [plan];
    let agent = c.agents.agents.find(value => value.agent_id === id);
    if (kind === 'claude') {
      const selection = agent.settings.current_selection;
      selection.preset_mappings.opus = { kind: 'preserve_native' };
      selection.preset_mappings.sonnet = { kind: 'plan', plan_id: plan.agent_plan_id };
    } else {
      const target = kind === 'hermes' ? { kind: 'hermes', config_root: '/example/hermes-profile' } : undefined;
      agent = {
        agent_id: id, context_id: 'agent-context/' + kind + '/example',
        available_surfaces: [kind + '_cli'], configuration_state: 'configured',
        native_target: target, version: '', status_error: null,
        settings: {
          state: 'configured', model_verified: false, restore_point_ref: 'model-restore/example',
          native_target: target,
          current_selection: target
            ? { mode: 'hermes_default', plan_id: plan.agent_plan_id, protocol: 'responses' }
            : { mode: kind + '_additional', allowed_plan_ids: [plan.agent_plan_id], protocol: 'responses' },
          collaboration: { state: 'not_configured', restore_point_ref: null, current_selection: null },
        },
      };
    }
    c.agents.agents = [agent];
    c.remount();
  })()`);
  await click('Agent');
  await waitFor(client, `[...document.querySelectorAll('.native-list .row-title')].some(node => node.textContent.trim() === ${JSON.stringify(label)})`);
  await evaluate(client, `[...document.querySelectorAll('.native-list .list-row')].find(node => node.querySelector('.row-title')?.textContent.trim() === ${JSON.stringify(label)}).click()`);
  const button = `[data-agent-id="${id}"] button[data-agent-facet="model"]:not(:disabled)`;
  await waitFor(client, `Boolean(document.querySelector(${JSON.stringify(button)}))`);
  await evaluate(client, `document.querySelector(${JSON.stringify(button)}).click()`);
  await waitFor(client, `Boolean(document.querySelector('[role="dialog"]')?.textContent.includes(${JSON.stringify(title)}))`);
}

async function loadProduct(client, base, language, state) {
  await navigate(client, `${base}product-shell.html`, viewport);
  await waitFor(client, 'Boolean(window.productShell)');
  await evaluate(client, `import('/interaction-audit-fixtures.mjs').then(m => m.seedInteractionAudit(window.productShell, ${JSON.stringify({ language, theme: 'dark', state })}))`);
  await waitFor(client, `Boolean(document.querySelector('.sidebar-health .good') && document.querySelector('.hr-ui')?.lang === ${JSON.stringify(language === 'zh' ? 'zh-CN' : 'en')})`);
}

async function fill(client, selector, value, type = 'Input') {
  await evaluate(client, `(() => {
    const field = document.querySelector(${JSON.stringify(selector)});
    Object.getOwnPropertyDescriptor(HTML${type}Element.prototype, 'value').set.call(field, ${JSON.stringify(value)});
    field.dispatchEvent(new Event(${JSON.stringify(type === 'Select' ? 'change' : 'input')}, { bubbles: true }));
  })()`);
}

async function magpieApi(client, language, click) {
  await evaluate(client, `import('/interaction-audit-fixtures.mjs').then(m => m.openInteractionAudit('models-directory', ${JSON.stringify(language)}))`);
  await click(translate(language, '自定义 API', 'Custom API'));
  await waitFor(client, 'Boolean(document.querySelector("#mc-connection-form:not([hidden])"))');
  await fill(client, '#mc-connection-form select', 'responses', 'Select');
  await fill(client, '#mc-connection-form input[data-autofocus]', translate(language, 'Magpie 本机', 'Local Magpie'));
  await fill(client, '#mc-connection-form input[inputmode="url"]', magpie.address);
  await waitFor(client, `document.querySelector('#mc-connection-form input[inputmode="url"]')?.value === ${JSON.stringify(magpie.address)}`);
}

async function seedQoderModels(client, language, worker) {
  const [, , zh = 'Qoder 日常开发', en = 'Qoder daily coding'] = worker ?? [];
  await evaluate(client, `(() => {
    const c = window.productShell;
    const reasoning = { kind: 'discrete', parameter: 'reasoning_effort', profiles: ['low', 'high', 'max'] };
    const models = ${JSON.stringify(magpie.models)}.map(({ id, name }) => ({
      model_ref: 'model-ref/' + id, binding_id: 'binding/' + id, revision: 1,
      upstream_model_id: id, display_name: name, membership: 'observed', native_reasoning: reasoning,
      presentation: { billing_class: 'unknown', availability: 'available', price_contexts: [] },
    }));
    const source = {
      source_id: 'source/magpie', revision: 1, display_name: 'Magpie',
      provenance: 'user_configured', state: 'ready', ready_model_count: models.length,
      target: { scheme: 'http', authority: '127.0.0.1', port: 3425, request_path: '/v1/responses',
        upstream_protocol: 'responses', protocol_profile_id: 'profile/custom/responses', protocol_profile_revision: 1 },
      authentication: { kind: 'bearer' }, models, keys: [], actions: ['edit', 'recheck'],
      connection_identity: { access_kind: 'api', connection_option_id: 'option/magpie', product_label: 'Qoder' },
    };
    const plan = c.desktop.catalog.plans[0];
    plan.desired.display_name = ${JSON.stringify(translate(language, zh, en))};
    plan.desired.purpose = ${JSON.stringify(translate(language, '检查代码并说明结果', 'Inspect the code and report findings'))};
    plan.desired.strategy.economy = [{ binding_id: models[0].binding_id }];
    plan.desired.strategy.primary = [{ binding_id: models[1].binding_id }];
    plan.desired.delegation_enabled = ${Boolean(worker)};
    plan.desired.work = ${JSON.stringify(worker ? { harness: worker[0], protocol: 'responses' } : null)};
    c.desktop.catalog.plans = [plan];
    c.management.sources = [source];
    c.handlers.plan_editor_options = payload => ({
      ...c.fixtureResponse('plan_editor_options', payload),
      candidates: models.map(model => ({
        binding_id: model.binding_id, model_configuration_id: model.upstream_model_id,
        display_name: model.display_name, reasoning, billing_class: 'unknown', routable: true,
        ingress_protocols: ['responses', 'messages'], native_ingress_protocols: ['responses'],
      })),
      free_suggestions: { candidates: [], unavailable: {} },
    });
    c.remount();
  })()`);
}

async function subscriptionCapture(client, language, state, click) {
  const provider = state.includes('claude') ? 'claude' : 'codex';
  const label = provider === 'codex' ? 'Codex' : 'Claude Code';
  await evaluate(client, `(() => {
    const c = window.productShell;
    const provider = ${JSON.stringify(provider)};
    const result = c.fixtureResponse('check_subscription', {});
    const candidate = result.checked_candidate;
    const pending = { ...c.fixtureResponse('compute_subscriptions', {}).candidates[0],
      existing_source_id: undefined,
      candidate: { candidate_ref: 'candidate/guide-' + provider, candidate_revision: 1 } };
    candidate.display_name = ${JSON.stringify(label)} + ${JSON.stringify(translate(language, ' 订阅', ' subscription'))};
    pending.display_name = candidate.display_name;
    candidate.existing_source_id = undefined;
    candidate.candidate = { ...pending.candidate, candidate_revision: 2 };
    result.candidate = candidate.candidate;
    candidate.models = (provider === 'codex'
      ? [['gpt-6.1-sol', 'GPT-6.1-Sol'], ['gpt-6-astra', 'GPT-6-Astra']]
      : [['claude-sonnet-example', 'Claude Sonnet'], ['claude-opus-example', 'Claude Opus']]
    ).map(([id, name]) => ({ model_ref: 'model/' + id, upstream_model_id: id,
      display_name: name, membership: 'observed', selectable: true }));
    c.handlers.manage_subscription_login = ({ request }) => ({
      schema: 'hiroute.subscription-login-result/v1',
      sessions: ${state !== 'agent-guide-subscription-login'} && request.provider === provider
        ? [{ provider, login_ref: 'login/guide', status: 'authorized', candidate: pending.candidate }] : [],
    });
    c.handlers.compute_subscriptions = () => ({ discovery_state: 'complete', candidates: [pending] });
    c.handlers.check_subscription = () => result;
    c.handlers.get_subscription_check_result = () => result;
    c.remount();
  })()`);
  await click(translate(language, '模型', 'Models'));
  await waitFor(client, `[...document.querySelectorAll('button')].some(button =>
    button.textContent.trim() === ${JSON.stringify(translate(language, '添加模型', 'Add models'))} && !button.disabled)`);
  await click(translate(language, '添加模型', 'Add models'));
  await waitFor(client, 'Boolean(document.querySelector(".add-choice"))');
  await click(translate(language, '连接订阅', 'Connect subscription'));
  await waitFor(client, 'Boolean(document.querySelector(".oc-status-row button:not(:disabled)"))');
  if (state === 'agent-guide-subscription-login') return;
  await click(translate(language, '检查并选择模型', 'Check and choose models'));
  await waitFor(client, `[...document.querySelectorAll('button')].some(button =>
    button.textContent.trim() === ${JSON.stringify(translate(language, '检查订阅', 'Check subscription'))} && !button.disabled)`);
  await click(translate(language, '检查订阅', 'Check subscription'));
  await waitFor(client, 'Boolean(document.querySelector(".v3-catalog input[type=checkbox]"))');
  await evaluate(client, `(() => {
    for (const input of document.querySelectorAll('.v3-catalog input[type=checkbox]')) {
      if (!input.checked && !input.disabled) input.click();
    }
  })()`);
  await waitFor(client, `[...document.querySelectorAll('button')].some(button =>
    button.textContent.trim() === ${JSON.stringify(translate(language, '保存并接入', 'Save and connect'))} && !button.disabled)`);
}

async function codexDesktopCapture(client, language, state, click) {
  await evaluate(client, `(() => {
    const c = window.productShell;
    const plan = c.agents.plans.plans[0];
    plan.desired.display_name = ${JSON.stringify(translate(language, '日常开发', 'Daily coding'))};
    c.agents.plans.plans = [plan];
    c.desktop.catalog.plans = [plan];
    const desktop = c.agents.agents.find(agent => agent.agent_id === 'agent_codex_default');
    desktop.agent_id = 'agent_codex_desktop';
    desktop.context_id = 'agent-context/codex-desktop/sha256:' + 'a'.repeat(64);
    desktop.available_surfaces = ['codex_desktop'];
    desktop.codex_access = undefined;
    desktop.codex_desktop = { codex_home: '/example/codex-desktop/codex',
      user_data: '/example/codex-desktop/desktop', source_codex_home: '/example/codex',
      shared_skills: '/example/skills' };
    c.agents.agents = [desktop];
    c.remount();
  })()`);
  await click('Agent');
  await waitFor(client, `[...document.querySelectorAll('.native-list .list-row')].some(row => row.textContent.includes('Codex Desktop'))`);
  await evaluate(client, `[...document.querySelectorAll('.native-list .list-row')].find(row => row.textContent.includes('Codex Desktop')).click()`);
  await waitFor(client, 'Boolean(document.querySelector("[data-codex-desktop-launch]"))');
  if (state === 'agent-guide-codex-desktop-launch') return;
  await evaluate(client, 'document.querySelector("[data-agent-facet=model]").click()');
  await waitFor(client, 'Boolean(document.querySelector("[role=dialog] input[type=checkbox]"))');
}

export async function openAgentGuideCapture(client, base, language, state, click) {
  await loadProduct(client, base, language, state);
  if (state.includes('subscription-')) return subscriptionCapture(client, language, state, click);
  if (state.startsWith('agent-guide-codex-desktop')) return codexDesktopCapture(client, language, state, click);
  if (agentEditors[state]) return agentEditor(client, language, click, agentEditors[state]);
  if (state === 'agent-guide-magpie-api') return magpieApi(client, language, click);

  const worker = workerPlans[state];
  await seedQoderModels(client, language, worker);
  if (state === 'agent-guide-qoder-models') {
    await click(translate(language, '模型', 'Models'));
    await waitFor(client, `Boolean(document.querySelector('.models-feature .master-list')?.textContent.includes('DeepSeek-Flash') && document.querySelector('.models-feature .master-list')?.textContent.includes('Kimi-K3.1-Pro'))`);
    return;
  }
  await evaluate(client, `import('/interaction-audit-fixtures.mjs').then(m => m.openInteractionAudit('routing', ${JSON.stringify(language)}))`);
  if (worker) {
    await evaluate(client, `(() => {
      const more = document.querySelector('.plan-more-settings');
      if (!more.open) more.querySelector('summary').click();
    })()`);
    await waitFor(client, `[...document.querySelectorAll('.v3-executor[aria-pressed="true"]')].some(node => node.querySelector('strong')?.textContent === ${JSON.stringify(worker[1])})`);
    await evaluate(client, `document.querySelector('[data-route-group="executor"]').scrollIntoView({ block: 'start', behavior: 'instant' })`);
  } else {
    try {
      await waitFor(client, `Boolean(document.querySelector('.route-lane') && document.querySelector('.plan-editor')?.textContent.includes('Kimi-K3.1-Pro'))`);
    } catch (error) {
      const visible = await evaluate(client, 'document.querySelector(".routing-page")?.innerText.slice(0, 1600)');
      throw new Error(`Difficulty capture did not show both models: ${visible}`, { cause: error });
    }
    await evaluate(client, `document.querySelector('.route-lane').scrollIntoView({ block: 'center', behavior: 'instant' })`);
  }
}
