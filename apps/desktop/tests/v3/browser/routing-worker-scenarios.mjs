import { scenario, scenarioCatalog, runScenarios } from './scenario-selection.mjs';
import { routingWorkerRequired } from './scenario-requirements.mjs';

const tick = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
const pause = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
const assert = (condition, message) => { if (!condition) throw new Error(message); };
const visible = element => Boolean(element && element.getClientRects().length > 0);
const buttons = () => [...document.querySelectorAll('button')].filter(visible);
const button = label => buttons().find(element => element.textContent.trim() === label);
const containingButton = label => buttons().find(element => element.textContent.includes(label));
const trace = () => window.__HIRouteFixtureTrace;
const calls = name => trace().commands.filter(command => command === name).length;

async function until(predicate, label) {
  const deadline = Date.now() + 5000;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error(`Timed out: ${label}; plan requests: ${calls('preview_plan_editor')}; fixture feedback: ${[...document.querySelectorAll('.plan-editor [role="alert"]')].map(node => node.textContent.trim()).join(' | ').slice(0, 400)}`);
    await pause(20);
  }
  await tick();
}

function setInput(input, value) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

const scenarios = [
  scenario('routing.capabilities.candidate-vs-fixed', ['routing-capabilities'], 'Codex capability summary separates candidate narrowing from fixed limits', async () => {
    await until(() => document.querySelector('[data-codex-capability-state="available"]'), 'Codex capability summary');
    const summary = document.querySelector('[data-codex-capability-summary]');
    assert(summary.textContent.includes('上下文 64K') && summary.textContent.includes('输入 文本'), 'The shared Codex capability summary is incomplete');
    const details = document.querySelector('.plan-capability-details');
    assert(details && !details.open, 'Lengthy capability explanations are not folded by default');
    assert(visible(summary), 'The capability summary was folded');
    details.querySelector('summary').click();
    await tick();
    assert(details.textContent.includes('当前编辑内容') && details.textContent.includes('Codex Responses 入口'), 'The capability preview claims published or upstream protocol support');
    const context = document.querySelector('[data-codex-capability-limit="context_window"]');
    const image = document.querySelector('[data-codex-capability-limit="image_input"]');
    assert(visible(context) && context?.textContent.includes('Qwen3-Coder-Plus'), 'The context narrowing candidate is not identified');
    assert(visible(image) && image?.textContent.includes('Qwen3-Coder-Plus'), 'The image narrowing candidate is not identified');
    assert(!document.querySelector('[data-codex-capability-limit="messages_protocol_boundary"]'), 'A native Responses candidate was labeled as Messages-limited');
    const fixed = document.querySelector('[data-codex-fixed-limit="parallel_tool_calls_disabled"]');
    assert(visible(fixed) && fixed?.textContent.includes('不由任何候选模型造成'), 'The fixed serial-tool limit is attributed to a candidate');
    assert(!document.querySelector('[data-codex-capability-unavailable]'), 'Complete metadata was rendered as unavailable');
  }),
  scenario('worker.discovery.visible-only', ['worker-installation'], 'configured delegation discovers only while its configuration is visible', async () => {
    await until(() => document.body.innerText.includes('Claude Code 本机安装'), 'initial worker installation');
    await until(() => calls('worker_dependencies_discover') === 1, 'initial dependency discovery');
    trace().commands.length = 0;
    containingButton('允许委派任务给执行 Agent').click();
    await tick();
    assert(!document.querySelector('.worker-dependencies'), 'Turning delegation off left installation controls visible');
    await pause(120);
    assert(calls('worker_executor_availability') === 0 && calls('worker_dependencies_discover') === 0, 'The hidden configuration started a dependency query');
    containingButton('允许委派任务给执行 Agent').click();
    await until(() => document.body.innerText.includes('Claude Code 本机安装'), 'restored worker installation');
    await until(() => calls('worker_dependencies_discover') === 1, 'discovery after explicit reopen');
  }),
  scenario('worker.installation.harness-isolation', ['worker-installation'], 'Codex CLI and Claude Code keep independent installation surfaces', async () => {
    trace().commands.length = 0;
    containingButton('Codex CLI').click();
    await until(() => document.body.innerText.includes('Codex CLI 本机安装'), 'Codex installation surface');
    await until(() => calls('worker_dependencies_discover') === 1, 'Codex discovery');
    containingButton('Claude Code').click();
    await until(() => document.body.innerText.includes('Claude Code 本机安装'), 'Claude installation surface');
    await until(() => calls('worker_dependencies_discover') === 2, 'Claude discovery');
    assert(document.querySelector('.v3-executor.selected')?.textContent.includes('Claude Code'), 'The selected executor did not return to Claude Code');
  }),
  // Selection uses one visible save click; native prepare/confirm stays an internal binding.
  scenario('worker.installation.replace', ['worker-replacement'], 'a configured installation can be explicitly replaced through advanced paths', async () => {
    trace().commands.length = 0;
    button('更换或高级设置').click();
    await until(() => document.querySelector('#worker-claude_code-adapter'), 'manual adapter path');
    const adapter = document.querySelector('#worker-claude_code-adapter');
    const replacement = '/opt/hiroute/claude-acp/adapter.js';
    setInput(adapter, replacement);
    await until(() => button('更换安装'), 'replacement save action');
    assert(calls('worker_dependencies_select_prepare') === 0 && calls('worker_dependencies_select_confirm') === 0,
      'Editing the replacement submitted it before the save click');
    button('更换安装').click();
    await until(() => document.body.innerText.includes('安装已配置；本实例中使用该执行 Agent 的计划会共享此选择。'), 'selection commit notice');
    assert(calls('worker_dependencies_select_prepare') === 1 && calls('worker_dependencies_select_confirm') === 1,
      'One explicit click did not prepare and commit exactly once');
    assert(adapter.value === replacement, 'The committed adapter path did not remain selected');
    assert(!document.querySelector('[role="alertdialog"]'), 'An obsolete second confirmation appeared');
  }),
  scenario('worker.installation.latest-edit', ['worker-replacement'], 'editing a replacement before saving commits only the latest path', async () => {
    trace().commands.length = 0;
    if (!document.querySelector('#worker-claude_code-cli')) {
      button('更换或高级设置').click();
      await until(() => document.querySelector('#worker-claude_code-cli'), 'manual CLI path');
    }
    const cli = document.querySelector('#worker-claude_code-cli');
    assert(cli, 'The manual CLI path is missing');
    setInput(cli, '/opt/hiroute/claude-next');
    await until(() => button('更换安装'), 'second replacement save action');
    setInput(cli, '/opt/hiroute/claude-final');
    await tick();
    assert(calls('worker_dependencies_select_prepare') === 0 && calls('worker_dependencies_select_confirm') === 0,
      'Unsaved path edits prepared or committed a replacement');
    button('更换安装').click();
    await until(() => document.body.innerText.includes('安装已配置；本实例中使用该执行 Agent 的计划会共享此选择。'), 'second selection commit notice');
    assert(calls('worker_dependencies_select_prepare') === 1 && calls('worker_dependencies_select_confirm') === 1,
      'The latest path was not saved by one explicit click');
    assert(cli.value === '/opt/hiroute/claude-final', 'An obsolete CLI path replaced the latest edit');
    assert(!document.querySelector('[role="alertdialog"]'), 'An obsolete second confirmation appeared');
  }),
  scenario('qoder.installation.single-cli', ['qoder', 'qoder-worker'], 'Qoder saves one CLI and switching execution Agents retains their independent installations', async () => {
    if (!document.querySelector('#worker-claude_code-cli')) {
      containingButton('Claude Code').click();
      await until(() => button('更换或高级设置'), 'Claude installation before switching');
      button('更换或高级设置').click(); await tick();
    }
    const previousClaude = document.querySelector('#worker-claude_code-cli').value;
    trace().commands.length = 0; trace().requests.length = 0;
    containingButton('Qoder CLI').click();
    await until(() => button('一键检测'), 'Qoder detection action');
    assert(calls('worker_dependencies_select_prepare') === 0, 'Choosing Qoder saved an installation implicitly');
    button('一键检测').click();
    await until(() => button('使用此安装'), 'single CLI recommendation');
    button('高级设置').click(); await tick();
    const cli = document.querySelector('#worker-qoder_cli-cli');
    assert(cli && !document.querySelector('#worker-qoder_cli-adapter') && !document.querySelector('#worker-qoder_cli-node'), 'Qoder did not render exactly its one CLI input');
    assert(!document.querySelector('.worker-dependencies').textContent.includes('Node.js'), 'Qoder advertised an unnecessary runtime');
    assert(document.querySelector('[data-qoder-installation-guide] a').href === 'https://docs.qoder.com/cli/installation', 'Official installation guidance is missing');
    setInput(cli, '/home/fixture/.local/bin/qodercli'); await tick();
    assert(!button('使用此安装').disabled, 'A standalone Qoder CLI required adapter placeholders');
    button('使用此安装').click();
    await until(() => document.querySelector('.worker-dependencies .badge.good'), 'Qoder selection persisted');
    const request = trace().requests.find(item => item.command === 'worker_dependencies_select_prepare').payload.input;
    assert(request.harness === 'qoder_cli' && request.cli_path === '/home/fixture/.local/bin/qodercli', 'The selected CLI did not cross the public boundary');
    assert(!('adapter_path' in request) && !('node_path' in request), 'Qoder request did not omit unsupported components');
    assert(calls('worker_dependencies_select_prepare') === 1 && calls('worker_dependencies_select_confirm') === 1, 'One click did not commit exactly once');
    containingButton('Claude Code').click();
    await until(() => button('更换或高级设置'), 'existing Claude installation');
    button('更换或高级设置').click(); await tick();
    assert(document.querySelector('#worker-claude_code-cli')?.value === previousClaude, 'Qoder replaced the previously saved Claude installation');
    containingButton('Qoder CLI').click();
    await until(() => button('更换或高级设置'), 'saved Qoder installation');
    button('更换或高级设置').click(); await tick();
    assert(document.querySelector('#worker-qoder_cli-cli')?.value === request.cli_path, 'Switching away lost the Qoder installation');
  }),
  scenario('routing.save.unobserved-editable', ['route-save'], 'an unobserved route save does not lock the editor or the route list', async () => {
    trace().commands.length = 0;
    const name = document.querySelector('.plan-identity-fields input');
    assert(name, 'The route editor is missing');
    setInput(name, 'Updated route');
    await until(() => !button('保存草稿')?.disabled, 'draft action');
    button('保存草稿').click();
    await until(() => calls('preview_plan_editor') === 1, 'route save submission');
    await until(() => !document.querySelector('.detail-fieldset')?.disabled, 'route editor released after submission');
    assert(!button('新建智能路由')?.disabled, 'Another route is blocked by the prior save');
    assert([...document.querySelectorAll('.master-list .list-row')].every(row => !row.disabled), 'The route list is blocked by the prior save');
    setInput(name, 'Newer local edit');
    await until(() => document.querySelector('.plan-identity-fields input')?.value === 'Newer local edit', 'newer input retained');
  }),
  scenario('routing.publish.qoder-budget-conflict', ['route-save', 'qoder'], 'a smaller Qoder model budget keeps route edits and explains model reconnection without disabling collaboration', async () => {
    trace().commands.length = 0;
    const contextMode = document.querySelector('.plan-context-mode');
    assert(contextMode && visible(contextMode), 'Context window mode is not reachable');
    contextMode.value = 'custom';
    contextMode.dispatchEvent(new Event('change', { bubbles: true }));
    await until(() => visible(document.querySelector('.plan-context-window-field')), 'custom context window');
    const window = document.querySelector('.plan-context-window-field');
    assert(window && visible(window), 'The user cannot edit the plan context window');
    setInput(window, '32000');
    await until(() => document.querySelector('[data-codex-capability-summary]')?.textContent.includes('32K'), 'model options for the edited context window');
    await until(() => !button('发布更改')?.disabled, 'plan publication action');
    trace().planPublicationError = 'QODER_MODEL_BUDGET_CONFLICT';
    try {
      button('发布更改').click();
      await until(() => document.querySelector('.plan-editor [role="alert"]')?.textContent.includes('停用 Qoder 的模型路由'), 'Qoder budget recovery guidance');
      const feedback = document.querySelector('.plan-editor [role="alert"]').textContent;
      assert(feedback.includes('再发布此计划并重新配置模型路由') && feedback.includes('任务协作无需停用'), 'Budget recovery confuses model reconnection with task collaboration');
      const publication = trace().requests.filter(item => item.command === 'preview_plan_editor').at(-1).payload.input;
      assert(publication.action === 'publish' && publication.editor.limits.context_window_tokens === 32000, 'The attempted publication did not carry the edited budget');
      assert(window.value === '32000' && !window.disabled && !button('发布更改').disabled, 'Rejected publication discarded or locked route edits');
      assert(calls('preview_agent_settings') === 0 && calls('check_agent_live') === 0, 'Publication failure implicitly changed an Agent connection or called a model');
    } finally { trace().planPublicationError = null; }
  }),
];

export const routingWorkerScenarioCatalog = scenarioCatalog(scenarios);

export async function runRoutingWorkerScenarios(selection = { requiredIds: routingWorkerRequired }) {
  return runScenarios(scenarios, selection);
}
