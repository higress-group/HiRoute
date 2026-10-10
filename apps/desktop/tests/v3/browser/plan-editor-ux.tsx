// @ts-expect-error Existing JavaScript browser scenarios have no TypeScript declaration.
import { runRoutingWorkerScenarios } from './routing-worker-scenarios.mjs';
import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from '@tauri-apps/api/mocks';
import type { Draft, Editor } from '../../../src/plan-editor';
import { RoutingPage } from '../../../src/product/RoutingPage';
import { PresentationRoot } from '../../../src/ui';
import { manualModelError } from '../../../src/features/model-connections/form-support';
import { blankModel } from '../../../src/features/model-connections/state';
import { dailyPlan, fixtureTrace, mockProductInvoke, readyDesktop, resetFixtureTrace } from './product-fixtures';
import '../../../src/occami/styles.css';

const workerChecks = new URLSearchParams(location.search).get('checks') === 'workers';
const availabilityChecks = new URLSearchParams(location.search).get('checks') === 'availability';
let availabilityFailure = false;
let availabilityEmpty = false;
let unavailableRequested = false;
const plan = structuredClone(dailyPlan);
if (!workerChecks) { plan.desired.delegation_enabled = false; plan.desired.work = undefined; }
const submitted: { action: string; editor: Editor }[] = [];
const savedSnapshot = structuredClone(readyDesktop);
savedSnapshot.catalog = { ...savedSnapshot.catalog, plans: [plan], drafts: [] };
resetFixtureTrace();
(window as typeof window & { __HIRouteFixtureTrace?: typeof fixtureTrace }).__HIRouteFixtureTrace = fixtureTrace;
mockIPC((command, payload) => {
  const input = payload as { input?: { action: string; editor: Editor; draft_id: string; plan_id: string; expected_head_revision: number; expected_draft_revision: number | null } };
  if (command === 'desktop_snapshot') {
    fixtureTrace.commands.push(command);
    return structuredClone(savedSnapshot);
  }
  if (command === 'plan_editor_options' && availabilityChecks) {
    unavailableRequested = (payload as { input?: { include_unavailable?: boolean } }).input?.include_unavailable === true;
    if (availabilityFailure) throw { source: 'backend', envelope: { error: { code: 'DAEMON_UNAVAILABLE', private: 'provider-secret-debug' } } };
    const current = mockProductInvoke(command, payload as Record<string, unknown>, 'ready') as Record<string, unknown>;
    return { ...current,
      ...(availabilityEmpty ? { candidates: [] } : {}),
      unavailable_candidates: availabilityEmpty ? [] : [
        { binding_id: 'binding/invalid-retained', display_name: '错误模型', reason: 'invalid_model_id' },
        { binding_id: 'binding/disabled-retained', display_name: '停用模型', reason: 'source_not_ready' },
      ],
      ...(availabilityEmpty ? {} : { unavailable_candidate_count: 257 }),
    };
  }
  if (command === 'preview_plan_editor') {
    fixtureTrace.commands.push(command);
    const request = input.input!;
    submitted.push(structuredClone(request));
    if (request.action === 'save_draft') {
      const draft: Draft = { draft_id: request.draft_id, plan_id: request.plan_id, base_head_revision: request.expected_head_revision, revision: (request.expected_draft_revision ?? 0) + 1, editor: structuredClone(request.editor) };
      savedSnapshot.catalog.drafts = [draft];
    }
    return { state: 'succeeded', operation: { operation_id: `operation/ux-${submitted.length}`, state: 'succeeded', sequence: 1, cancellable: false, safe_error_code: null } };
  }
  return mockProductInvoke(command, payload as Record<string, unknown>, 'ready');
});

const assert: (condition: unknown, message: string) => asserts condition = (condition, message) => {
  if (!condition) throw new Error(message);
};
const tick = () => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
async function until(predicate: () => boolean, label: string) {
  const deadline = Date.now() + 5000;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error(`Timed out: ${label}`);
    await new Promise(resolve => setTimeout(resolve, 20));
  }
  await tick();
}
function element<T extends HTMLElement>(selector: string) {
  const found = document.querySelector<T>(selector);
  assert(found, `Missing ${selector}`);
  return found;
}
function button(label: string) {
  const found = [...document.querySelectorAll<HTMLButtonElement>('button')].find(item => item.textContent?.trim() === label);
  assert(found, `Missing button ${label}`);
  return found;
}
function setInput(selector: string, value: string) {
  const input = element<HTMLInputElement>(selector);
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}
function chooseWindow(value: 'auto' | 'custom') {
  const select = element<HTMLSelectElement>('.plan-context-mode');
  select.value = value;
  select.dispatchEvent(new Event('change', { bubbles: true }));
}
const selectClassifier = async (kind: 'local' | 'rest') => {
  element<HTMLButtonElement>(`.classifier-choice:nth-child(${kind === 'local' ? 1 : 2})`).click();
  await tick();
};

const checks: [string, () => Promise<void>][] = [
  ['Built-in rules hide external connection controls and protocol', async () => {
    await until(() => Boolean(document.querySelector('[data-codex-capability-state]')), 'editor options');
    assert(!document.querySelector('.classifier-endpoint-field'), 'Built-in rules exposed the endpoint');
    assert(![...document.querySelectorAll('button')].some(item => item.textContent === '查看接入协议'), 'Built-in rules exposed the protocol');
    assert(!element<HTMLDetailsElement>('.plan-more-settings').open, 'New optional settings were expanded');
    assert(fixtureTrace.commands.every(command => !command.startsWith('worker_')), 'Hidden delegation queried an executor');
  }],
  ['Switching classifiers preserves configuration but clears unsaved authentication plaintext', async () => {
    await selectClassifier('rest');
    assert(button('查看接入协议'), 'Custom service lost the protocol entry');
    setInput('.classifier-endpoint-field', 'https://classifier.example/v1/decisions');
    setInput('.classifier-timeout-field', '3900');
    element<HTMLDetailsElement>('.classifier-auth').querySelector('summary')!.click();
    await tick();
    element<HTMLButtonElement>('.classifier-auth .option-row:nth-child(2)').click();
    await tick();
    setInput('.classifier-header-field', 'X-Classifier');
    setInput('.classifier-secret-ref-field', 'classifier/review');
    setInput('.classifier-secret-create input', 'synthetic-placeholder');
    await tick();
    await selectClassifier('local');
    assert(!document.querySelector('.classifier-endpoint-field'), 'Inactive REST controls remained visible');
    await selectClassifier('rest');
    assert(element<HTMLInputElement>('.classifier-endpoint-field').value === 'https://classifier.example/v1/decisions', 'Endpoint was reset');
    assert(element<HTMLInputElement>('.classifier-timeout-field').value === '3900', 'Timeout was reset');
    assert(element<HTMLInputElement>('.classifier-header-field').value === 'X-Classifier', 'Header name was reset');
    assert(element<HTMLInputElement>('.classifier-secret-ref-field').value === 'classifier/review', 'Secret reference was reset');
    assert(element<HTMLInputElement>('.classifier-secret-create input').value === '', 'Unsaved authentication plaintext was cached');
  }],
  ['Draft save rebuilds the real page and retains inactive inputs without submitting them', async () => {
    await selectClassifier('local');
    chooseWindow('custom');
    await tick();
    setInput('.plan-context-window-field', '32000');
    await tick();
    chooseWindow('auto');
    setInput('.plan-identity-fields input', 'Review-local-rules');
    await tick();
    const before = submitted.length;
    const previousEditor = element('.plan-editor');
    button('保存草稿').click();
    await until(() => submitted.length === before + 1 && !previousEditor.isConnected && !element<HTMLFieldSetElement>('.detail-fieldset').disabled, 'persisted draft rebuilt the editor');
    const classifier = submitted.at(-1)!.editor.smart.classifier;
    assert(JSON.stringify(classifier) === '{"kind":"local_rules"}', 'Inactive REST fields leaked into the submitted draft');
    assert(submitted.at(-1)!.editor.limits.context_window_tokens === undefined, 'Inactive window leaked into the draft');
    await selectClassifier('rest');
    assert(element<HTMLInputElement>('.classifier-endpoint-field').value === 'https://classifier.example/v1/decisions', 'Draft save reset the endpoint');
    assert(element<HTMLInputElement>('.classifier-timeout-field').value === '3900', 'Draft save reset the timeout');
    assert(element<HTMLInputElement>('.classifier-secret-ref-field').value === 'classifier/review', 'Draft save reset the authentication reference');
    assert(element<HTMLInputElement>('.classifier-secret-create input').value === '', 'Draft save cached authentication plaintext');
    await selectClassifier('local');
    await until(() => Boolean(document.querySelector('[data-codex-capability-state]')), 'options after draft reconstruction');
  }],
  ['Custom windows retain input and reject empty or out-of-bounds values before publication', async () => {
    chooseWindow('custom');
    await tick();
    assert(element<HTMLInputElement>('.plan-context-window-field').value === '32000', 'Draft save reset the custom window');
    chooseWindow('auto');
    await tick();
    assert(!document.querySelector('.plan-context-window-field'), 'Automatic mode exposed a custom input');
    chooseWindow('custom');
    await tick();
    assert(element<HTMLInputElement>('.plan-context-window-field').value === '32000', 'Custom window was reset');
    for (const value of ['64001', '']) {
      setInput('.plan-context-window-field', value);
      await tick();
      const before = submitted.length;
      button('发布更改').click();
      await tick();
      assert(submitted.length === before, 'Invalid custom window was submitted');
      assert(element<HTMLSelectElement>('.plan-context-mode').value === 'custom', 'Empty input silently selected automatic mode');
      assert(document.activeElement === element('.plan-context-window-field'), 'Invalid window was not focused');
      assert(element('[data-plan-context-window] [role="alert"]').textContent, 'Invalid window has no inline error');
    }
    setInput('.plan-context-window-field', '32000');
    await tick();
    chooseWindow('auto');
    await tick();
  }],
  ['Publication unfolds executor errors and collapsed installations stop discovery', async () => {
    const details = element<HTMLDetailsElement>('.plan-more-settings');
    details.querySelector('summary')!.click();
    await tick();
    element<HTMLButtonElement>('[data-route-group="executor"] .option-row').click();
    await tick();
    details.querySelector('summary')!.click();
    await tick();
    button('发布更改').click();
    await tick();
    assert(details.open, 'Executor error remained hidden');
    assert(document.activeElement === document.querySelector('.v3-executors button'), 'Executor error did not focus the selection');
    element<HTMLButtonElement>('.v3-executors button').click();
    await until(() => fixtureTrace.commands.includes('worker_dependencies_discover'), 'visible worker discovery');
    details.querySelector('summary')!.click();
    await tick();
    const before = fixtureTrace.commands.filter(command => command.startsWith('worker_')).length;
    await new Promise(resolve => setTimeout(resolve, 250));
    assert(fixtureTrace.commands.filter(command => command.startsWith('worker_')).length === before, 'Collapsed executor started another query');
  }],
];

const availabilityCases: [string, () => Promise<void>][] = [
  ['routing.options.id-safety: manual input follows the shared provider ID byte and character bounds', async () => {
    for (const id of ['内网模型', 'Vendor/Model@2026?revision#1', 'Model A', 'e\u0301', '\ufeff模型\ufeff', '\ufeff', '模型🧠', '🧠'.repeat(128), 'x'.repeat(512), '界'.repeat(170) + 'ab']) {
      assert(manualModelError(blankModel(id, '中文名称')) === '', `Safe opaque ID was rejected: ${JSON.stringify(id)}`);
    }
    for (const id of [' bad', 'bad\u00a0', 'bad\u3000', 'bad\tmodel', 'bad\0model', 'bad\u0085model', '\u0085bad', 'bad\u0085', 'bad\ud800model', '\udfffbad', 'x'.repeat(513), '界'.repeat(171), '🧠'.repeat(129)]) {
      assert(manualModelError(blankModel(id, '中文名称')) === 'MODEL_ID_INVALID', `Unsafe ID was accepted: ${JSON.stringify(id)}`);
    }
  }],
  ['routing.options.exclusions: rejected models remain visible with disabled selection and safe reasons', async () => {
    await until(() => Boolean(document.querySelector('.route-unavailable-models')), 'unavailable model explanations');
    assert(unavailableRequested, 'Route editor did not opt in to explanations');
    element<HTMLDetailsElement>('.route-unavailable-models').querySelector('summary')!.click();
    assert(element('.route-unavailable-models').textContent?.includes('共 257 个不可用模型'), 'Truncated explanations concealed their actual count');
    element<HTMLButtonElement>('.add-candidate').click();
    await until(() => Boolean(document.querySelector('.catalog-picker')), 'model picker');
    const rows = [...document.querySelectorAll<HTMLElement>('.catalog-picker .list-row')];
    const rejected = rows.filter(row => row.textContent?.includes('错误模型') || row.textContent?.includes('停用模型'));
    assert(rejected.length === 2, 'Unavailable candidate identities were silently omitted');
    const selected = document.querySelectorAll('.route-candidate').length;
    for (const row of rejected) {
      const add = row.querySelector<HTMLButtonElement>('button')!;
      assert(add.disabled, 'Unavailable candidate could be selected');
      assert(row.textContent?.includes('模型页面'), 'Unavailable candidate lost its actionable reason');
      add.click();
    }
    await tick();
    assert(document.querySelectorAll('.route-candidate').length === selected, 'Rejected selection changed the route');
    element<HTMLButtonElement>('button[aria-label="关闭候选模型"]').click();
  }],
  ['routing.options.service-error: visible retry preserves input and recovers after real error mapping', async () => {
    availabilityFailure = true;
    setInput('.plan-identity-fields input', '保留用户输入');
    await until(() => Boolean(document.querySelector('[data-error-code="DAEMON_UNAVAILABLE"]')), 'visible service failure');
    const alert = element('[data-error-code="DAEMON_UNAVAILABLE"]');
    assert(alert.textContent?.includes('本机服务'), 'Service error did not identify the unavailable service');
    assert(!alert.textContent?.includes('provider-secret-debug'), 'Backend error payload leaked into copy');
    assert(element<HTMLInputElement>('.plan-identity-fields input').value === '保留用户输入', 'Service error discarded user input');
    availabilityFailure = false;
    alert.querySelector<HTMLButtonElement>('button')!.click();
    await until(() => !document.querySelector('[data-error-code="DAEMON_UNAVAILABLE"]'), 'options retry recovery');
  }],
  ['routing.options.empty: no sources is readable and distinct from service failure', async () => {
    availabilityEmpty = true;
    setInput('.plan-identity-fields input', '无候选模型');
    await until(() => [...document.querySelectorAll('[role="status"]')].some(row => row.textContent?.includes('还没有可用于路由的模型')), 'empty source guidance');
    assert(!document.querySelector('.route-local-error'), 'Empty options were shown as a service error');
    assert(!document.querySelector('.route-unavailable-models'), 'Empty source list invented exclusions');
  }],
];

function Harness() {
  const [snapshot, setSnapshot] = useState(() => structuredClone(savedSnapshot));
  const [running, setRunning] = useState(false);
  const [results, setResults] = useState<{ name: string; state: string; error?: string }[]>([]);
  async function run() {
    setRunning(true);
    if (workerChecks) {
      const ids = ['routing.capabilities.candidate-vs-fixed', 'worker.discovery.visible-only',
        'worker.installation.harness-isolation', 'worker.installation.replace', 'worker.installation.latest-edit'];
      const outcome = await runRoutingWorkerScenarios({ ids, requiredIds: ids });
      setResults(outcome.results);
      setRunning(false);
      return;
    }
    const next = [];
    for (const [name, check] of availabilityChecks ? availabilityCases : checks) {
      try { await check(); next.push({ name, state: 'green' }); }
      catch (error) { next.push({ name, state: 'red', error: error instanceof Error ? error.message : String(error) }); }
    }
    setResults(next);
    setRunning(false);
  }
  return <PresentationRoot language="zh" theme="light" textScale={1}>
    <aside style={{ padding: 16 }}>
      <p>React + mock IPC 回归检查；不代表原生 Desktop 或真实 daemon 验收。</p>
      <button className="btn" disabled={running || results.length > 0} onClick={() => void run()}>运行智能路由交互检查</button>
      <output aria-live="polite" data-ux-results>{results.length > 0 && JSON.stringify({ tests: results.length, passed: results.filter(result => result.state === 'green').length, results })}</output>
    </aside>
    <RoutingPage language="zh" snapshot={snapshot} initialEditor={{ key: plan.agent_plan_id, plan }} loading={false} busy={false} onRefresh={async () => setSnapshot(structuredClone(savedSnapshot))} onOperation={() => {}} />
  </PresentationRoot>;
}
createRoot(document.getElementById('root')!).render(<Harness />);
