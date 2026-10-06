import { scenario, scenarioCatalog, runScenarios } from './scenario-selection.mjs';
import { agentTrustRequired } from './scenario-requirements.mjs';

const c = () => window.agentTrust;
const tick = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
const assert = (condition, message) => { if (!condition) throw new Error(message); };
const visible = element => Boolean(element && element.getClientRects().length > 0);
const text = () => document.body.innerText;
const calls = name => c().commands.filter(item => item.command === name);
const facet = kind => document.querySelector(`[data-agent-facet="${kind}"]`);
const executableCallout = root => (root ?? document).querySelector('[data-agent-executable-state]');
const dialog = () => document.querySelector('[role="dialog"]');
const submit = () => document.querySelector('[role="dialog"] button[type="submit"]');
async function until(predicate, label) {
  const deadline = Date.now() + 5000;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error(`Timed out: ${label}; commands: ${calls('preview_agent_settings').length} previews, ${calls('check_agent_authentication').length} checks; feedback: ${document.querySelector('.agent-feedback')?.textContent?.trim() ?? ''}`);
    await new Promise(resolve => setTimeout(resolve, 20));
  }
  await tick();
}
async function fresh(kind, singleRoute = false) {
  c().reset();
  c().agents = c()[kind]();
  if (singleRoute) c().agents.plans.plans = c().agents.plans.plans.filter(plan => plan.head.status === 'enabled').slice(0, 1);
  await tick();
  await until(() => [...document.querySelectorAll('.native-list .list-row')].some(row => row.textContent.includes('Claude')), 'agents loaded');
  document.querySelectorAll('.native-list .list-row').forEach(row => { if (row.textContent.includes('Claude')) row.click(); });
  await until(() => document.querySelector('[data-agent-id="agent_claude_default"] .detail-hero'), 'Claude detail');
}
async function freshCodex(kind, singleRoute = false) {
  c().reset();
  c().agents = c()[kind]();
  if (singleRoute) c().agents.plans.plans = c().agents.plans.plans.filter(plan => plan.head.status === 'enabled').slice(0, 1);
  await tick();
  await until(() => [...document.querySelectorAll('.native-list .list-row')].some(row => row.textContent.includes('Codex')), 'agents loaded');
  document.querySelectorAll('.native-list .list-row').forEach(row => { if (row.textContent.includes('Codex')) row.click(); });
  await until(() => document.querySelector('[data-agent-id="agent_codex_default"] .detail-hero'), 'Codex detail');
}
async function openEditor(kind = 'model', advanced = true) {
  const trigger = facet(kind);
  assert(trigger && !trigger.disabled, `Facet button unavailable: ${kind}`);
  trigger.click();
  await until(() => visible(dialog()), 'editor open');
  if (advanced) { const details = [...dialog().querySelectorAll('details')].find(node => node.querySelector('summary')?.textContent.includes('高级设置')); if (details) details.open = true; await tick(); }
}
async function save() {
  const control = submit();
  assert(control && !control.disabled, 'Save button unavailable');
  const operations = c().operations.length;
  control.click();
  await until(() => c().operations.length === operations + 1 && !dialog(), 'save submitted and editor closed');
  assert(c().operations.at(-1).operation.operation_id === 'operation/agent-settings-fixture', 'Save did not hand its Operation to the host');
}
function changeSelect(select, value) {
  assert(select instanceof HTMLSelectElement, `Select unavailable for ${value}`);
  Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set.call(select, value);
  select.dispatchEvent(new Event('change', { bubbles: true }));
}
const configureSpecs = () => calls('preview_agent_settings').map(item => item.payload.input.spec);

const scenarios = [
  scenario('codex.restore.conflict-active', ['agent-recovery'], 'Codex file conflicts preserve active access and allow other settings', async () => {
    await freshCodex('splitCodexStatus');
    const agent = c().agents.agents.find(agent => agent.agent_id === 'agent_codex_default');
    agent.codex_access.conflict_fields = ['model_providers.hiroute.name'];
    agent.codex_access.target_file = `/fixture/${'configuration'.repeat(16)}/codex/config.toml`;
    c().refresh(); await tick();
    await until(() => document.querySelector('[data-codex-restore-conflict]'), 'file conflict diagnosis');
    const alert = document.querySelector('[data-codex-restore-conflict]');
    assert(alert.textContent.includes('原连接仍有效') && alert.textContent.includes('model_providers.hiroute.name'), 'Conflict hides active access or fields');
    assert(!alert.textContent.includes('旧令牌已失效') && !alert.textContent.includes('新的配置保存会暂停'), 'Ordinary conflict claims revoked access or globally blocked settings');
    assert(!facet('model').disabled, 'Unrelated configuration action is blocked');
    await openEditor('model');
    const location = [...dialog().querySelectorAll('details')].find(node => node.querySelector('summary')?.textContent.includes('配置位置与会话历史'));
    assert(location, 'The real configuration location is not available');
    location.open = true; await tick();
    const path = [...location.querySelectorAll('code')].find(node => node.textContent === agent.codex_access.target_file);
    assert(path, 'The real configuration path was hidden or replaced with an internal identity');
    const bounds = location.getBoundingClientRect();
    assert(bounds.width > 200, 'The component fixture did not give configuration details a usable viewport');
    assert([...path.getClientRects()].every(rect => rect.left >= bounds.left && rect.right <= bounds.right), 'Long configuration paths overflow the configuration details');
  }),
  scenario('codex.enable.draft-cancel', ['agent-model-routing'], 'Codex first enable is a ready-to-submit draft and cancel does not apply', async () => {
    await freshCodex('cliOnly', true);
    await openEditor('model', false);
    assert(!dialog().querySelector('[role="switch"]'), 'Redundant enable switch remains');
    assert(dialog().querySelector('[data-agent-service-responsibility]')?.textContent.includes('启用不会自动设置开机启动'), 'Enabling routing promises automatic startup');
    const route = dialog().querySelector('[data-agent-plan-id] input');
    assert(route?.checked && !submit().disabled, 'Sole route is not ready to submit');
    assert(!dialog().querySelector('details').open, 'Advanced settings are open on first enable');
    assert(!dialog().querySelector('[data-agent-capability-preview]') && calls('plan_editor_options').length === 0, 'Enable still loads a read-only capacity preview');
    assert(dialog().textContent.includes('开启新会话'), 'Profile activation step is missing');
    const cancel = [...dialog().querySelectorAll('button')].find(button => button.textContent.trim() === '取消');
    cancel.click(); await tick();
    assert(!dialog() && calls('preview_agent_settings').length === 0 && c().operations.length === 0, 'Cancel applied a draft');
  }),
  scenario('claude.enable.shared-presets', ['agent-model-routing'], 'Claude first enable shares one route while retaining independent advanced presets', async () => {
    await fresh('notRunnable', true);
    await openEditor('model', false);
    assert(!dialog().querySelector('[role="switch"]'), 'Redundant enable switch remains');
    const route = dialog().querySelector('select[aria-label="Claude Code 路由"]');
    assert(route?.value && !submit().disabled, 'Sole Claude route is not ready to submit');
    assert(!dialog().querySelector('details').open, 'Advanced presets are open by default');
    assert(!dialog().querySelector('[data-agent-capability-preview]') && calls('plan_editor_options').length === 0, 'Enable still loads a read-only capacity preview');
    assert(dialog().textContent.includes('重新启动 Claude Code'), 'Claude activation step is missing');
    assert(dialog().textContent.includes('账号 Default 仍需真实调用验证'), 'Preset routing overstates account Default verification');
    assert(!text().includes('hiroute agent launch --agent claude-code'), 'Claude routing requires a special launcher');
    await save();
    const spec = configureSpecs().at(-1).model.settings;
    assert(Object.values(spec.preset_mappings).every(choice => choice.kind === 'plan' && choice.plan_id === route.value), 'Shared route did not cover all three presets');
    assert(!Object.hasOwn(spec, 'model'), 'Shortcut changed the native current model');
  }),
  scenario('codex.enable.confirmed-command', ['agent-model-routing'], 'Codex copies the confirmed profile command through native IPC when browser copying rejects', async () => {
    await freshCodex('cliOnly', true);
    const agent = c().agents.agents.find(agent => agent.agent_id === 'agent_codex_default');
    const command = `CODEX_HOME='/path with spaces' codex --profile hiroute`;
    agent.codex_access = {
      codex_home: '/path with spaces', slot_id: 'slot/copy',
      profile_context_id: 'context/copy/profile', root_context_id: agent.context_id,
      selected_mode: 'profile', slot_occupied: false, target_file: '/path with spaces/hiroute.config.toml',
      profile_name: 'hiroute', commands: { 'bash/zsh': command },
      pending_operation: null, access_revoked: false, conflict_fields: [],
    };
    c().refresh(); await tick();
    const previousClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    const previousCopy = Object.getOwnPropertyDescriptor(document, 'execCommand');
    const previousNative = Object.getOwnPropertyDescriptor(window, 'isTauri');
    const copied = [];
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { async writeText() { throw new Error('NotAllowedError'); } } });
    Object.defineProperty(document, 'execCommand', { configurable: true, value: () => { throw new Error('Browser clipboard is unavailable'); } });
    Object.defineProperty(window, 'isTauri', { configurable: true, value: true });
    try {
      c().handlers['plugin:clipboard-manager|write_text'] = payload => { copied.push(payload.text); };
      c().handlers.preview_agent_settings = payload => {
        agent.context_id = agent.codex_access.profile_context_id;
        agent.codex_access.slot_occupied = true;
        agent.settings.state = 'configured';
        agent.settings.current_selection = payload.input.spec.model.settings;
        return { preview: { applicable: true, blockers: [] }, mutation: { state: 'applied', operation: { operation_id: 'operation/agent-settings-fixture', state: 'succeeded', sequence: 1, cancellable: false } } };
      };
      await openEditor('model', false);
      assert(copied.length === 0, 'Opening the draft copied a command');
      await save();
      await until(() => text().includes('启动命令已复制'), 'copy success after confirmed save');
      assert(copied.length === 1 && copied[0] === command, 'Confirmed command was changed or copied twice');
      assert(!document.querySelector('textarea'), 'Temporary clipboard field remains in the UI');
    } finally {
      if (previousClipboard) Object.defineProperty(navigator, 'clipboard', previousClipboard); else delete navigator.clipboard;
      if (previousCopy) Object.defineProperty(document, 'execCommand', previousCopy); else delete document.execCommand;
      if (previousNative) Object.defineProperty(window, 'isTauri', previousNative); else delete window.isTauri;
    }
  }),
  scenario('codex.routing.fixed-binding', ['agent-model-routing'], 'Desktop-only Codex repairs an existing fixed-model binding without a surface selector', async () => {
    await freshCodex('desktopWithFixed');
    await openEditor('model');
    assert(!document.querySelector('[data-agent-surface-option]'), 'Legacy surface selector is still rendered');
    assert(visible(document.querySelector('[data-codex-shared-scope]')), 'Shared-scope explanation is missing');
    assert(document.querySelector('[data-agent-surface-fact="codex_desktop"]').getAttribute('data-agent-surface-detected') === 'true', 'Desktop discovery fact missing');
    assert(document.querySelector('[data-agent-surface-fact="codex_cli"]').getAttribute('data-agent-surface-detected') === 'false', 'Missing CLI was not reported as a fact');
    assert(submit().disabled, 'Unavailable existing source did not block save');
    assert(document.querySelectorAll('[data-agent-fixed-models] [data-client-model-id]').length === 1, 'Unconfigured catalog overrides appeared in the form');
    const fixed = document.querySelector('[data-client-model-id="gpt-5.6-sol"] select');
    changeSelect(fixed, 'binding/codex/gpt-5.6-sol');
    await tick();
    const effort = [...document.querySelectorAll('[data-client-model-id="gpt-5.6-sol"] select.select')].at(-1);
    assert(effort && effort.value === 'high', 'Fixed model did not carry explicit native effort');
    const defaultChoice = document.querySelector('[data-agent-default]');
    changeSelect(defaultChoice, 'fixed:gpt-5.6-sol');
    await tick();
    await save();
    const spec = configureSpecs().at(-1).model.settings;
    assert(!Object.hasOwn(spec, 'surfaces'), 'Desktop discovery leaked into the persisted configuration');
    assert(spec.fixed_models.length === 1 && spec.allowed_plan_ids.length === 0, 'Fixed-only selection was not preserved');
    assert(spec.fixed_models[0].candidate.reasoning.profile === 'high', 'Native effort was not submitted');
  }),
  scenario('codex.routing.plan-only', ['agent-model-routing'], 'CLI-only Codex saves one shared plan configuration without selecting Desktop', async () => {
    await freshCodex('cliOnly');
    await openEditor('model');
    assert(!document.querySelector('#hr-agent-settings [role="switch"]'), 'Enable requires a redundant switch');
    await tick();
    assert(!document.querySelector('[data-agent-surface-option]'), 'Legacy surface selector is still rendered');
    assert(document.querySelector('[data-agent-surface-fact="codex_cli"]').getAttribute('data-agent-surface-detected') === 'true', 'CLI discovery fact missing');
    assert(document.querySelector('[data-agent-surface-fact="codex_desktop"]').getAttribute('data-agent-surface-detected') === 'false', 'Missing Desktop was not reported as a fact');
    const plan = document.querySelector('[data-agent-plan-id]');
    if (!plan.querySelector('input').checked) plan.querySelector('input').click();
    await tick();
    const planId = plan.getAttribute('data-agent-plan-id');
    if (document.querySelector('[data-agent-default]')) changeSelect(document.querySelector('[data-agent-default]'), `plan:${planId}`);
    await tick();
    assert(!dialog().querySelector('[data-agent-fixed-models]'), 'Plan-only routing exposes unconfigured fixed overrides');
    await save();
    const spec = configureSpecs().at(-1).model.settings;
    assert(!Object.hasOwn(spec, 'surfaces'), 'CLI discovery leaked into the persisted configuration');
    assert(spec.fixed_models.length === 0 && spec.allowed_plan_ids.length === 1, 'Plan-only selection was not preserved');
  }),
  scenario('codex.routing.mixed-native-default', ['agent-model-routing'], 'Codex mixed selection repairs a fixed route and adds a Plan while preserving the covered native default', async () => {
    await freshCodex('desktopWithFixed');
    await openEditor('model');
    changeSelect(document.querySelector('[data-client-model-id="gpt-5.6-sol"] select'), 'binding/codex/gpt-5.6-sol');
    const plan = document.querySelector('[data-agent-plan-id]');
    if (!plan.querySelector('input').checked) plan.querySelector('input').click();
    await tick();
    assert(document.querySelector('[data-agent-default]').value === 'native', 'Native default was changed without user action');
    assert(dialog().textContent.includes('保留当前默认模型名称；请求仍经过 HiRoute'), 'Preserved native default routing is not explained');
    changeSelect(document.querySelector('[data-agent-default]'), `plan:${plan.getAttribute('data-agent-plan-id')}`);
    await tick();
    assert(dialog().textContent.includes('Codex 将默认使用所选智能路由'), 'Selected default route is not explained');
    changeSelect(document.querySelector('[data-agent-default]'), 'native');
    await tick();
    await save();
    const spec = configureSpecs().at(-1).model.settings;
    assert(spec.fixed_models.length === 1 && spec.allowed_plan_ids.length === 1, 'Mixed selection lost a route family');
    assert(spec.default_selection.kind === 'preserve_native', 'Covered native default was not preserved');
  }),
  scenario('agent.configuration.shared-interactions', ['agent-configuration', 'qoder'], 'Codex, Claude and Qoder share configuration actions without model verification steps', async () => {
    const snapshots = [c().splitCodexStatus(), c().registered(), c().qoderRouted()];
    for (const snapshot of snapshots) {
      const id = snapshot.agents.some(agent => agent.agent_id === 'agent_qoder_default')
        ? 'agent_qoder_default' : snapshot === snapshots[0] ? 'agent_codex_default' : 'agent_claude_default';
      for (const state of ['not_verified', 'passed', 'failed']) {
        c().reset();
        c().agents = structuredClone(snapshot);
        c().agents.agents = c().agents.agents.filter(agent => agent.agent_id === id);
        const agent = c().agents.agents[0];
        agent.settings.model_verified = state === 'passed';
        agent.settings.surface_results = agent.available_surfaces.map(surface => ({
          surface, applied_revision: agent.settings.applied_revision - 1, state, reason_code: null,
        }));
        await tick();
        await until(() => document.querySelector(`[data-agent-id="${id}"]`), `${id} configured detail`);
        assert(text().includes('路由已配置'), `${id} lost its configured status`);
        assert(!/尚未验证|调用未验证|路由已验证|验证未通过/.test(text()), `${id} exposed model verification as configuration status`);
        assert(!document.querySelector('[data-agent-surface]'), `${id} exposes revision-scoped verification results`);
        assert(![...document.querySelectorAll('button')].some(button => /验证模型调用|验证任务协作|检查协作能力/.test(button.textContent)), `${id} requires an extra verification step`);
        assert(facet('model')?.textContent === '调整' && !facet('model').disabled, `${id} cannot edit model routing`);
        assert(facet('collaboration')?.textContent === '调整' && !facet('collaboration').disabled, `${id} cannot edit task routing`);
        assert([...document.querySelectorAll('button')].some(button => button.textContent === '停用' && !button.disabled), `${id} lost model recovery`);
        assert(calls('check_agent_live').length === 0 && calls('check_agent_authentication').length === 0, `${id} probes on ordinary navigation`);
      }
    }
  }),
  scenario('codex.recovery.token-controls', ['agent-recovery'], 'Codex recovery keeps native model selection and local token controls together', async () => {
    await freshCodex('splitCodexStatus');
    const details = document.querySelector('.native-details');
    const summary = details.querySelector('summary');
    const actionLabel = () => summary.querySelector('.disclosure-action').textContent.trim();
    assert(!details.open && summary.getAttribute('aria-expanded') === 'false', 'Closed recovery disclosure is not announced as collapsed');
    assert(actionLabel() === '展开', 'Closed recovery disclosure does not offer Expand');
    summary.click();
    await tick();
    assert(details.open && summary.getAttribute('aria-expanded') === 'true', 'Open recovery disclosure is not announced as expanded');
    assert(actionLabel() === '收起', 'Open recovery disclosure does not offer Collapse');
    assert(visible(details.querySelector('input[list="codex-native-restore-models"]')), 'Native model restore selector is missing');
    const controls = details.querySelector('[data-agent-token-controls]');
    assert(visible(controls), 'Local token controls are missing beside native recovery');
    const edit = [...controls.querySelectorAll('button')].find(button => button.textContent.trim() === '修改令牌');
    assert(edit && !edit.disabled, 'Local token cannot be edited');
    edit.click();
    await tick();
    assert(visible(details.querySelector('.agent-token-form input[type="password"]')), 'Token edit form did not open');
    const cancel = [...details.querySelectorAll('.agent-token-form button')].find(button => button.textContent.trim() === '取消');
    cancel.click();
    await tick();
    assert(!details.querySelector('.agent-token-form'), 'Token edit form did not close');
    const restore = [...document.querySelectorAll('.detail-section-head button')].find(button => button.textContent.trim() === '停用');
    assert(restore && !restore.disabled, 'Model recovery became unavailable after token edit');
    summary.click();
    await tick();
    assert(!details.open && summary.getAttribute('aria-expanded') === 'false', 'Closing recovery disclosure did not restore its collapsed announcement');
    assert(actionLabel() === '展开' && !controls.checkVisibility(), 'Closing recovery disclosure did not restore Expand and hide its controls');
  }),
  scenario('codex.routing.preview-rejection', ['agent-model-routing'], 'a Codex plan rejected by Preview explains the Responses boundary without applying it', async () => {
    await freshCodex('cliOnly');
    c().handlers.preview_agent_settings = () => ({
      preview: { applicable: false, blockers: [{ reason: 'model_plan_unavailable' }] },
      mutation: null,
    });
    await openEditor('model');
    assert(!document.querySelector('#hr-agent-settings [role="switch"]'), 'Enable requires a redundant switch');
    await tick();
    const plan = document.querySelector('[data-agent-plan-id]');
    if (!plan.querySelector('input').checked) plan.querySelector('input').click();
    await tick();
    if (document.querySelector('[data-agent-default]')) changeSelect(document.querySelector('[data-agent-default]'), `plan:${plan.getAttribute('data-agent-plan-id')}`);
    await tick();
    assert(text().includes('Codex Responses'), 'The form did not explain the required ingress');
    submit().click();
    await until(() => calls('preview_agent_settings').length === 1, 'incompatible plan preview');
    assert(dialog()?.textContent.includes('Codex Responses') && dialog().textContent.includes('配置未修改'), 'The blocker did not explain why nothing was applied');
    assert(c().operations.length === 0 && c().mutations === 0, 'An incompatible plan reached the mutation path');
  }),
  scenario('agent.diagnostics.non-runnable', ['agent-diagnostics'], 'a non-runnable executable reports the real reason without becoming a trust gate', async () => {
    await fresh('notRunnable');
    const callout = executableCallout();
    assert(visible(callout), 'Executable diagnostic missing');
    assert(callout.getAttribute('data-agent-executable-state') === 'executable_not_runnable', 'Wrong executable diagnostic state');
    assert(callout.textContent.includes('不可执行'), 'Diagnostic does not name the executable failure');
    assert(!callout.textContent.includes('不受信任'), 'Diagnostic still makes an installation trust claim');
    assert(!facet('model').disabled && !facet('collaboration').disabled, 'Executable diagnostics still block configuration');
    await openEditor('model');
    assert(visible(dialog()), 'Editor did not open alongside the executable diagnostic');
    assert(calls('preview_agent_settings').length === 0, 'Opening the editor dispatched a mutation request');
  }),
  scenario('agent.diagnostics.rescan', ['agent-diagnostics'], 'rescan replaces a non-runnable diagnostic with the current installation state', async () => {
    await fresh('notRunnable');
    assert(visible(executableCallout()), 'Executable diagnostic missing');
    c().agents = c().registered();
    executableCallout().querySelector('button').click();
    await until(() => !executableCallout(), 'executable diagnostic cleared after rescan');
    assert(!facet('model').disabled, 'Configure entry changed after rescan');
  }),
  scenario('agent.diagnostics.late-failure', ['agent-diagnostics'], 'a late executable failure remains diagnostic while the shared configuration stays editable', async () => {
    await fresh('registered');
    await openEditor('model');
    assert(!submit().disabled && !executableCallout(dialog()), 'Registered editor unexpectedly restricted');
    const snapshots = calls('agent_snapshot').length;
    c().agents = c().notRunnable();
    c().refresh();
    await until(() => calls('agent_snapshot').length > snapshots, 'late state refetched');
    await until(() => executableCallout(dialog()), 'late executable diagnostic rendered');
    const before = calls('preview_agent_settings').length;
    await save();
    assert(calls('preview_agent_settings').length === before + 1, 'Executable diagnostic prevented a shared configuration save');
  }),
  scenario('agent.recovery.non-runnable', ['agent-recovery'], 'a non-runnable install keeps the managed recovery entry reachable', async () => {
    await fresh('notRunnableWithRestore');
    assert(visible(executableCallout()), 'Executable diagnostic missing');
    const details = document.querySelector('.native-details');
    details.querySelector('summary').click();
    await tick();
    const restore = [...document.querySelectorAll('.detail-section-head button')].find(item => item.textContent.trim() === '停用');
    assert(restore && !restore.disabled, 'Managed recovery entry blocked by an executable diagnostic');
    restore.click();
    await until(() => calls('preview_agent_settings').length === 1, 'recovery request dispatched');
    assert(configureSpecs()[0].model.intent === 'restore', 'Recovery entry dispatched a configure request');
    assert(!dialog(), 'Recovery reply opened a configuration editor');
  }),
  scenario('agent.configuration.independent-facets', ['agent-model-routing', 'agent-collaboration'], 'registered installs keep both facets configurable in one step', async () => {
    await fresh('registered');
    assert(text().includes('直接启动 claude，使用 Opus、Sonnet、Haiku 原生预设'), 'Claude preset routing does not explain the ordinary CLI entry');
    assert(text().includes('路由已配置'), 'Configured routing status missing');
    await openEditor('model');
    assert(!executableCallout(dialog()), 'Registered install wrongly reported an executable failure');
    await save();
    assert(configureSpecs().length === 1 && configureSpecs()[0].model.intent === 'configure', 'Model route did not save');
    await until(() => !dialog(), 'editor closed after save');
    await openEditor('collaboration');
    assert(!executableCallout(dialog()), 'Registered install wrongly reported an executable failure on the second facet');
    await save();
    const specs = configureSpecs();
    assert(specs.length === 2 && specs[1].collaboration.intent === 'configure', 'Task delegation skill did not save independently');
    assert(specs[1].collaboration.settings.trigger_mode === 'delegate_by_default', 'Task trigger mode was not preserved');
    assert(c().mutations === 2, 'Saves were not reported to the host');
  }),
  scenario('agent.configuration.prerequisite', ['agent-verification'], 'registered installs still pass the prerequisite check before saving', async () => {
    await fresh('registered');
    let previews = 0;
    c().handlers.preview_agent_settings = () => {
      previews += 1;
      return previews === 1
        ? { preview: { applicable: false, blockers: [{ reason: 'capability_unavailable', capabilities: [{ capability: 'ingress_authentication', reason: 'unverified' }] }] }, mutation: null }
        : { preview: { applicable: true, blockers: [] }, mutation: { state: 'applied', operation: { operation_id: 'operation/agent-settings-fixture', state: 'succeeded', sequence: 1, cancellable: false } } };
    };
    await openEditor('model');
    await save();
    assert(calls('check_agent_authentication').length === 1, 'Prerequisite check was skipped');
    assert(calls('preview_agent_settings').length === 2, 'Save did not re-preview after the prerequisite check');
    assert(!executableCallout() && !executableCallout(dialog()), 'A prerequisite gap was reported as an executable failure');
  }),
  scenario('claude.collaboration.check-before-save', ['claude-collaboration'], 'Claude collaboration checks Skill and trusted CLI before Preview and Apply', async () => {
    await fresh('registered');
    let previews = 0;
    let finishCheck;
    c().handlers.preview_agent_settings = () => {
      previews += 1;
      return previews === 1
        ? { preview: { applicable: false, blockers: [{ reason: 'capability_unavailable', capabilities: [
          { capability: 'skill_loading', reason: 'unverified' },
          { capability: 'trusted_cli_execution', reason: 'unverified' },
        ] }] }, mutation: null }
        : { preview: { applicable: true, blockers: [] }, mutation: { state: 'applied', operation: { state: 'succeeded' } } };
    };
    c().handlers.check_agent_authentication = () => new Promise(resolve => { finishCheck = resolve; });
    await openEditor('collaboration');
    submit().click();
    await until(() => calls('check_agent_authentication').length === 1, 'Claude check dispatched');
    assert(!visible(dialog().querySelector('.agent-feedback')), 'Unconfirmed preview flashed during the automatic check');
    assert(submit().disabled, 'Save remained enabled during the automatic check');
    finishCheck(true);
    await until(() => !dialog(), 'Claude collaboration saved after check');
    const checks = calls('check_agent_authentication');
    assert(checks.length === 1, 'Claude collaboration check was skipped');
    assert(checks[0].payload.input.agent_id === 'agent_claude_default' && checks[0].payload.input.scope === 'collaboration', 'Claude check did not use the collaboration scope');
    assert(calls('preview_agent_settings').length === 2, 'Claude save did not re-preview after the formal check');
    assert(!dialog(), 'Claude collaboration editor remained open after Apply');
  }),
  scenario('claude.collaboration.retry-preserves-edit', ['claude-collaboration'], 'Claude collaboration check failure offers a retry and preserves the edit', async () => {
    await fresh('registered');
    c().handlers.preview_agent_settings = () => ({ preview: { applicable: false, blockers: [{ reason: 'capability_unavailable', capabilities: [
      { capability: 'skill_loading', reason: 'unverified' },
    ] }] }, mutation: null });
    c().handlers.check_agent_authentication = () => { throw new Error('CHECK_UNAVAILABLE'); };
    await openEditor('collaboration');
    submit().click();
    await until(() => dialog()?.querySelector('[role="alert"]'), 'Claude check failure');
    const retry = [...dialog().querySelectorAll('button')].find(button => button.textContent.includes('重新检查任务委派技能'));
    assert(retry && !retry.disabled, 'Claude check has no recovery action');
    assert(configureSpecs().every(spec => spec.collaboration.intent === 'configure'), 'Claude edit changed during failed check');
    c().handlers.check_agent_authentication = () => true;
    retry.click();
    await until(() => text().includes('任务委派技能检查通过'), 'Claude collaboration check retried');
    assert(calls('check_agent_authentication').length === 2, 'Retry did not dispatch a new formal check');
  }),
  scenario('claude.collaboration.obsolete-blocker', ['claude-collaboration'], 'an unrelated Agent preview failure clears an obsolete prerequisite blocker', async () => {
    await fresh('registered');
    c().handlers.preview_agent_settings = () => ({ preview: { applicable: false, blockers: [{ reason: 'capability_unavailable', capabilities: [
      { capability: 'skill_loading', reason: 'unverified' },
    ] }] }, mutation: null });
    c().handlers.check_agent_authentication = () => false;
    await openEditor('collaboration');
    submit().click();
    await until(() => visible(dialog()?.querySelector('.agent-feedback')), 'prerequisite blocker');
    c().handlers.preview_agent_settings = () => { throw new Error('PREVIEW_UNAVAILABLE'); };
    submit().click();
    await until(() => dialog()?.querySelector('[role="alert"]'), 'new preview failure');
    assert(!dialog().querySelector('button')?.textContent.includes('重新检查任务委派技能'), 'Obsolete blocker remained');
    assert(!visible(dialog().querySelector('.callout.warn')), 'Obsolete blocker remained visible');
  }),
  scenario('agent.diagnostics.configuration-state', ['agent-diagnostics'], 'non-executable states keep their own diagnostics', async () => {
    await fresh('unregisteredEndpoint');
    assert(!executableCallout(), 'Configuration state was generalized to an executable failure');
    assert(!facet('model').disabled, 'Configure entry wrongly blocked for a configuration state');
    await openEditor('model');
    assert(visible(dialog()), 'Editor did not open for a configuration state');
    assert(calls('preview_agent_settings').length === 0, 'Opening the editor dispatched a mutation request');
  }),
  scenario('codex.routing.unproven-native-model', ['agent-model-routing'], 'Codex explains unproven cache names and can switch to HiRoute-only without native bindings', async () => {
    await freshCodex('protectedCodex');
    c().handlers.preview_agent_settings = ({ input }) => input.spec.model.settings.native_model_mode === 'preserve_available'
      ? { preview: { applicable: false, blockers: [{ reason: 'native_model_coverage_unavailable' }], unproven_native_model_ids: ['gpt-unproven'] }, mutation: null }
      : { preview: { applicable: true, blockers: [], unproven_native_model_ids: [] }, mutation: { state: 'applied', operation: { operation_id: 'operation/agent-settings-fixture', state: 'succeeded', sequence: 1, cancellable: false } } };
    await openEditor('model');
    assert(document.querySelector('[data-agent-native-mode] input[value="preserve_available"]:checked'), 'Saved preserve choice missing');
    submit().click();
    await until(() => dialog()?.textContent.includes('gpt-unproven'), 'unproven model guidance');
    assert(dialog().textContent.includes('只使用已配置的 HiRoute 模型'), 'HiRoute-only recovery path missing');
    document.querySelector('[data-agent-native-mode] input[value="hiroute_only"]').click();
    await tick();
    assert(!document.querySelector('[data-client-model-id="gpt-5.6-sol"]'), 'Protected native binding leaked into HiRoute-only selection');
    const plan = document.querySelector('[data-agent-plan-id]');
    if (!plan.querySelector('input').checked) plan.querySelector('input').click();
    await tick();
    assert(!document.querySelector('[data-agent-default]') || document.querySelector('[data-agent-default]').value.startsWith('plan:'), 'HiRoute-only default was not selected');
    await save();
    const spec = configureSpecs().at(-1).model.settings;
    assert(spec.native_model_mode === 'hiroute_only' && spec.fixed_models.length === 0, 'Native model leaked into HiRoute-only save');
  }),
  scenario('qoder.collaboration.enable-without-model', ['qoder', 'qoder-collaboration'], 'Qoder enables task collaboration without a model connection or published plan', async () => {
    c().reset(); c().agents = c().qoderFresh(); await tick();
    await until(() => document.querySelector('[data-agent-id="agent_qoder_default"]'), 'Qoder detail');
    assert(facet('model') && !document.querySelector('[data-agent-token-controls]'), 'Optional model routing requires a connection or token');
    assert(c().agents.plans.plans.length === 0 && !c().agents.agents[0].settings.current_selection, 'Collaboration fixture already has a route');
    const context = c().agents.agents[0].context_id;
    assert(!text().includes(context) && !text().includes('当前配置范围'), 'An opaque authority ID was presented as a readable configuration scope');
    let previews = 0;
    c().handlers.preview_agent_settings = () => ++previews === 1
      ? { preview: { applicable: false, blockers: [{ reason: 'capability_unavailable', capabilities: [{ capability: 'skill_loading', reason: 'unverified' }] }] }, mutation: null }
      : { preview: { applicable: true, blockers: [] }, mutation: { state: 'applied', operation: { operation_id: 'operation/agent-settings-fixture', state: 'succeeded', sequence: 1, cancellable: false } } };
    await openEditor('collaboration');
    document.querySelector('input[name="agent-collaboration-trigger"][value="delegate_by_default"]').click();
    await tick(); await save();
    const check = calls('check_agent_authentication').at(-1)?.payload.input;
    assert(check?.agent_id === 'agent_qoder_default' && check.scope === 'collaboration', 'Saving Qoder skipped the required collaboration check');
    assert(previews === 2, 'Qoder did not re-preview after its automatic check');
    const spec = configureSpecs().at(-1);
    assert(spec.model.intent === 'keep' && !spec.model.settings && !spec.access_token, 'Qoder save invented model configuration');
    assert(spec.context_id === context, 'Hiding the internal identity changed the authoritative save target');
    assert(spec.collaboration.settings.trigger_mode === 'delegate_by_default', 'The chosen trigger mode was lost');
    assert(calls('check_agent_live').length === 0, 'Task collaboration called model verification');
  }),
  scenario('qoder.collaboration.retry-and-restore', ['qoder', 'qoder-collaboration'], 'Qoder preserves edits through prerequisite failures and independently restores collaboration', async () => {
    c().reset(); c().agents = c().qoderConfigured(); await tick();
    await until(() => document.querySelector('[data-agent-id="agent_qoder_default"]'), 'configured Qoder');
    c().handlers.preview_agent_settings = () => ({ preview: { applicable: false, blockers: [{ reason: 'capability_unavailable', capabilities: [{ capability: 'skill_loading', reason: 'unverified' }] }] }, mutation: null });
    await openEditor('collaboration');
    let attempts = 0;
    c().handlers.check_agent_authentication = () => {
      if (++attempts <= 2) {
        const schema = 'hiroute.agent-collaboration-check-failure/v1';
        throw { source: 'backend', envelope: {
          error: { details_schema: schema, code: 'ACTION_REQUIRED' },
          data: { schema, reason: attempts === 1 ? 'login_required' : 'installed_skill_changed' },
        } };
      }
      return true;
    };
    submit().click();
    await until(() => text().includes('正常打开已选客户端完成登录'), 'normal sign-in recovery');
    assert(!text().includes('用户协作技能验证通过'), 'Failed verification was presented as success');
    assert(facet('collaboration').textContent === '调整' && dialog(), 'Failed check discarded the enabled collaboration or edit');
    const retry = () => [...dialog().querySelectorAll('button')].find(button => button.textContent.includes('重新检查任务委派技能'));
    assert(retry() && !retry().disabled, 'Qoder check failure has no shared retry action');
    retry().click();
    await until(() => text().includes('已安装的协作 Skill 内容发生变化'), 'changed skill recovery');
    assert(text().includes('不会自动覆盖'), 'Changed user skill was not protected');
    assert(c().operations.length === 0 && c().mutations === 1, 'A failed prerequisite applied or restored a user Skill');
    assert(configureSpecs().every(spec => spec.model.intent === 'keep' && spec.collaboration.intent === 'configure'), 'Retry changed the pending intent');
    retry().click();
    await until(() => text().includes('用户协作技能验证通过；未执行委派任务。'), 'installed user skill verification');
    assert(calls('check_agent_live').length === 0, 'Qoder invoked model verification');
    [...dialog().querySelectorAll('button')].find(button => button.textContent === '取消').click();
    await until(() => !dialog(), 'collaboration editor cancelled');
    c().handlers.preview_agent_settings = () => ({ preview: { applicable: true, blockers: [] }, mutation: { state: 'applied', operation: { operation_id: 'operation/agent-settings-fixture', state: 'succeeded', sequence: 1, cancellable: false } } });
    const details = document.querySelector('.native-details'); details.querySelector('summary').click(); await tick();
    assert(!details.querySelector('input[type="password"]'), 'Qoder recovery exposed a model token');
    [...details.querySelectorAll('button')].find(button => button.textContent === '停用任务路由').click();
    await until(() => configureSpecs().some(spec => spec.collaboration.intent === 'restore'), 'collaboration restore submitted');
    const spec = configureSpecs().at(-1);
    assert(spec.model.intent === 'keep' && spec.collaboration.restore_point_ref === 'task-restore/qoder', 'Restore changed a model facet or used the wrong Skill owner');
  }),
  scenario('qoder.routing.additional-plans', ['qoder', 'qoder-model-routing'], 'Qoder adds selected routes without replacing native models or applying a cancelled draft', async () => {
    c().reset(); c().agents = c().qoderFresh(); c().agents.plans = c().registered().plans; await tick();
    await until(() => facet('model'), 'Qoder optional model routing');
    await openEditor('model');
    const pick = () => [...dialog().querySelectorAll('[data-agent-plan-id] input')].slice(0, 2).forEach(input => input.click());
    pick(); await tick();
    assert(dialog().textContent.includes('当前默认模型保持不变') && dialog().textContent.includes('/model'), 'Additional routing does not explain native preservation and selection');
    assert(!dialog().querySelector('[data-agent-default], [data-client-model-id], [data-agent-fixed-models]'), 'Qoder borrowed native model, source or default controls');
    assert(dialog().querySelectorAll('[data-plan-protocol] select').length === 2, 'Each selected route needs its own protocol choice');
    [...dialog().querySelectorAll('button')].find(button => button.textContent === '取消').click(); await tick();
    assert(configureSpecs().length === 0 && !dialog(), 'Cancelling Qoder model draft changed configuration');
    await openEditor('model'); pick(); await tick();
    const selected = [...dialog().querySelectorAll('[data-agent-plan-id]')].filter(row => row.querySelector('input').checked).map(row => row.dataset.agentPlanId);
    changeSelect(dialog().querySelector(`[data-agent-plan-id="${selected[1]}"] [data-plan-protocol] select`), 'messages');
    await tick();
    await save();
    const spec = configureSpecs().at(-1);
    assert(JSON.stringify(spec.model.settings) === JSON.stringify({ mode: 'qoder_additional', allowed_plan_ids: selected,
      plan_protocols: { [selected[0]]: 'responses', [selected[1]]: 'messages' } }), 'Qoder changed the ecosystem, selected routes or their independent protocols');
    assert(spec.collaboration.intent === 'keep' && !spec.restore_native_model, 'Model save changed collaboration or a native default');
    assert(!c().commands.some(({ command }) => /subscription|discovered_model|model_catalog/.test(command)), 'Additional routing opened model discovery or import');
    assert(calls('check_agent_live').length === 0, 'Saving routes implicitly invoked a model');
  }),
  scenario('qoder.routing.adjust-and-restore', ['qoder', 'qoder-model-routing'], 'Qoder repairs selected routes, rotates their token and restores each facet independently', async () => {
    c().reset(); c().agents = c().qoderRouted();
    const unavailablePlan = c().agents.agents[0].settings.current_selection.allowed_plan_ids[1];
    c().agents.plans.plans = c().agents.plans.plans.filter(plan => plan.agent_plan_id !== unavailablePlan);
    await tick();
    await until(() => facet('model')?.textContent === '调整', 'configured Qoder model routing');
    const agent = c().agents.agents[0];
    let restoreBlocker = null;
    c().handlers.preview_agent_settings = ({ input: { spec } }) => {
      if (spec.model.intent === 'restore' && restoreBlocker) return {
        preview: { applicable: false, blockers: [{ reason: restoreBlocker }] }, mutation: null,
      };
      if (spec.model.intent === 'configure') agent.settings.current_selection = structuredClone(spec.model.settings);
      if (spec.model.intent === 'restore') {
        agent.settings.state = 'not_configured'; agent.settings.current_selection = null; agent.settings.restore_point_ref = null;
      }
      if (spec.collaboration.intent === 'restore') agent.settings.collaboration = { state: 'restored', current_selection: null, restore_point_ref: null };
      return { preview: { applicable: true, blockers: [] }, mutation: { state: 'applied', operation: { operation_id: 'operation/agent-settings-fixture', state: 'succeeded', sequence: 1, cancellable: false } } };
    };
    await openEditor('model');
    const retired = dialog().querySelector(`[data-agent-plan-id="${unavailablePlan}"] input`);
    assert(retired?.checked && submit().disabled, 'An unavailable saved route disappeared or was silently replaced');
    retired.click();
    const added = [...dialog().querySelectorAll('[data-agent-plan-id] input')].find(input => !input.checked); added.click();
    await tick(); await save();
    const saved = structuredClone(configureSpecs().at(-1).model.settings);
    const details = document.querySelector('.native-details'); details.querySelector('summary').click(); await tick();
    [...details.querySelectorAll('button')].find(button => button.textContent === '重新生成').click();
    await until(() => configureSpecs().some(spec => spec.access_token?.intent === 'regenerate'), 'Qoder token regeneration');
    assert(JSON.stringify(configureSpecs().at(-1).model.settings) === JSON.stringify(saved), 'Token rotation used another selection');
    await until(() => ![...document.querySelectorAll('.detail-section-head button')].find(button => button.textContent === '停用')?.disabled, 'model restore ready');
    const beforeRestore = c().operations.length;
    for (const [reason, guidance] of [['qoder_default_in_use', '通过 /model 切换到其他模型'], ['qoder_model_file_conflict', '不会覆盖冲突字段或无关配置']]) {
      restoreBlocker = reason;
      [...document.querySelectorAll('.detail-section-head button')].find(button => button.textContent === '停用').click();
      await until(() => document.querySelector('.agent-feedback')?.textContent.includes(guidance), `restore guidance: ${reason}`);
      assert(c().operations.length === beforeRestore && agent.settings.current_selection.mode === 'qoder_additional', 'A blocked restore changed the connection');
      assert(agent.settings.collaboration.state === 'configured', 'A model conflict disabled collaboration');
    }
    restoreBlocker = null;
    [...document.querySelectorAll('.detail-section-head button')].find(button => button.textContent === '停用').click();
    await until(() => c().operations.length === beforeRestore + 1 && agent.settings.state === 'not_configured', 'model restore request');
    assert(configureSpecs().at(-1).collaboration.intent === 'keep' && agent.settings.collaboration.state === 'configured', 'Model restore removed task collaboration');
    agent.settings = c().qoderRouted().agents[0].settings; c().refresh(); await tick();
    await until(() => facet('model')?.textContent === '调整' && !facet('model').disabled, 'model connection refreshed');
    const recovery = document.querySelector('.native-details'); if (!recovery.open) recovery.querySelector('summary').click(); await tick();
    [...recovery.querySelectorAll('button')].find(button => button.textContent === '停用任务路由').click();
    await until(() => configureSpecs().some(spec => spec.collaboration.intent === 'restore'), 'independent collaboration restore');
    assert(configureSpecs().at(-1).model.intent === 'keep' && agent.settings.current_selection.mode === 'qoder_additional', 'Collaboration restore removed model routes');
  }),
  ...['pi', 'dsh'].map(ecosystem => scenario(`${ecosystem}.routing.default-in-use`, ['agent-recovery'], `${ecosystem} preserves a route used by its native default and directs recovery to the selected client`, async () => {
    c().reset(); c().agents = c().additionalRouted(ecosystem); await tick();
    await until(() => facet('model')?.textContent === '调整' && !facet('model').disabled, 'additional model routing');
    const agent = c().agents.agents[0];
    const before = structuredClone(agent.settings);
    c().handlers.preview_agent_settings = () => ({
      preview: { applicable: false, blockers: [{ reason: 'additional_default_in_use' }] }, mutation: null,
    });
    [...document.querySelectorAll('.detail-section-head button')].find(button => button.textContent === '停用').click();
    await until(() => document.querySelector('.agent-feedback')?.textContent.includes('当前默认模型仍引用'), 'default reference recovery');
    const guidance = document.querySelector('.agent-feedback').textContent;
    assert(guidance.includes('所选客户端中切换到其他模型') && guidance.includes('HiRoute 不会替你更改默认模型'), 'Recovery did not direct the user to their selected client');
    assert(!guidance.includes('Pi 中') && !guidance.includes('/model'), 'Shared recovery assumed another ecosystem or a CLI-only selector');
    assert(c().operations.length === 0 && JSON.stringify(agent.settings) === JSON.stringify(before), 'Blocked removal changed model routes or task collaboration');
    assert(configureSpecs().at(-1).collaboration.intent === 'keep', 'Model recovery tried to restore task collaboration');
    assert(calls('check_agent_live').length === 0, 'Recovery called a model');
    const spec = configureSpecs().at(-1);
    assert(spec.context_id === agent.context_id && spec.model.intent === 'restore'
      && spec.model.restore_point_ref === before.restore_point_ref, 'Removal targeted another client or restore point');
  })),
  scenario('qoder.routing.resume-pending', ['qoder', 'qoder-model-routing'], 'Qoder resumes the original incomplete model operation without replacing task collaboration', async () => {
    c().reset(); c().agents = c().qoderRouted();
    const agent = c().agents.agents[0];
    const configured = structuredClone(agent.settings);
    agent.settings = { ...configured, state: 'pending', current_selection: null, operation_id: 'operation/qoder-pending', operation_state: 'activating' };
    c().agents.trusted_authority = false;
    await tick();
    const retry = () => document.querySelector('[data-qoder-model-recovery] button');
    await until(() => retry(), 'pending model recovery');
    assert(retry().disabled, 'Untrusted status authorized recovery');
    c().agents.trusted_authority = true; c().refresh();
    await until(() => retry() && !retry().disabled, 'trusted original-operation retry');
    c().handlers.retry_agent_settings = ({ input }) => {
      assert(input.schema === 'hiroute.agent-settings-retry/v1'
        && input.context_id === agent.context_id && input.operation_id === 'operation/qoder-pending', 'Retry changed the authorized operation or context');
      agent.settings = configured;
      return { operation_id: input.operation_id, state: 'succeeded' };
    };
    retry().click();
    await until(() => !retry() && facet('model') && !facet('model').disabled, 'authoritative recovered status');
    assert(c().commands.filter(item => item.command === 'retry_agent_settings').length === 1, 'Recovery was duplicated');
    assert(!c().commands.some(item => item.command === 'preview_agent_settings'), 'Recovery created a new settings operation');
    assert(agent.settings.collaboration.current_selection.trigger_mode === configured.collaboration.current_selection.trigger_mode, 'Recovery changed collaboration');
    assert(!agent.settings.model_verified, 'Recovery fabricated a model verification');
  }),

];

export const agentTrustScenarioCatalog = scenarioCatalog(scenarios);
export const agentTrustScenarioNames = agentTrustScenarioCatalog.map(item => item.name);

export async function runAgentTrustScenarios(selection = { requiredIds: agentTrustRequired }) {
  return runScenarios(scenarios, selection);
}
