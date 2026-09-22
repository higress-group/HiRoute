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
    if (Date.now() > deadline) throw new Error(`Timed out: ${label}`);
    await pause(20);
  }
  await tick();
}

function setInput(input, value) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

const scenarios = [
  ['Codex capability summary separates candidate narrowing from fixed limits', async () => {
    await until(() => document.querySelector('[data-codex-capability-state="available"]'), 'Codex capability summary');
    const summary = document.querySelector('[data-codex-capability-summary]');
    assert(summary.textContent.includes('上下文 64K') && summary.textContent.includes('输入 文本'), 'The shared Codex capability summary is incomplete');
    assert(document.querySelector('.codex-capability-summary').textContent.includes('当前编辑内容') && document.querySelector('.codex-capability-summary').textContent.includes('Codex Responses 入口'), 'The capability preview claims published or upstream protocol support');
    const context = document.querySelector('[data-codex-capability-limit="context_window"]');
    const image = document.querySelector('[data-codex-capability-limit="image_input"]');
    assert(context?.textContent.includes('Qwen3-Coder-Plus'), 'The context narrowing candidate is not identified');
    assert(image?.textContent.includes('Qwen3-Coder-Plus'), 'The image narrowing candidate is not identified');
    assert(!document.querySelector('[data-codex-capability-limit="messages_protocol_boundary"]'), 'A native Responses candidate was labeled as Messages-limited');
    const fixed = document.querySelector('[data-codex-fixed-limit="parallel_tool_calls_disabled"]');
    assert(fixed?.textContent.includes('不由任何候选模型造成'), 'The fixed serial-tool limit is attributed to a candidate');
    assert(!document.querySelector('[data-codex-capability-unavailable]'), 'Complete metadata was rendered as unavailable');
  }],
  ['configured delegation discovers only while its configuration is visible', async () => {
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
  }],
  ['Codex CLI and Claude Code keep independent installation surfaces', async () => {
    trace().commands.length = 0;
    containingButton('Codex CLI').click();
    await until(() => document.body.innerText.includes('Codex CLI 本机安装'), 'Codex installation surface');
    await until(() => calls('worker_dependencies_discover') === 1, 'Codex discovery');
    containingButton('Claude Code').click();
    await until(() => document.body.innerText.includes('Claude Code 本机安装'), 'Claude installation surface');
    await until(() => calls('worker_dependencies_discover') === 2, 'Claude discovery');
    assert(document.querySelector('.v3-executor.selected')?.textContent.includes('Claude Code'), 'The selected executor did not return to Claude Code');
  }],
  // Selection uses one visible save click; native prepare/confirm stays an internal binding.
  ['a configured installation can be explicitly replaced through advanced paths', async () => {
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
  }],
  ['editing a replacement before saving commits only the latest path', async () => {
    trace().commands.length = 0;
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
  }],
  ['an unobserved route save does not lock the editor or the route list', async () => {
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
  }],
];

export async function runRoutingWorkerScenarios(start = 0, end = scenarios.length) {
  const results = [];
  for (const [name, run] of scenarios.slice(start, end)) {
    try { await run(); results.push({ name, state: 'green' }); }
    catch (error) { results.push({ name, state: 'red', error: error.message }); }
  }
  return { evidence: 'React components + mock IPC only; not native/Tauri and not the real daemon', tests: results.length, passed: results.filter(item => item.state === 'green').length, failed: results.filter(item => item.state === 'red').length, results };
}
