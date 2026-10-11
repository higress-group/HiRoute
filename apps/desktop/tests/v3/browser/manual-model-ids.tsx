import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ModelConnectionForm } from '../../../src/features/model-connections/ModelConnectionForm';
import { blankModel } from '../../../src/features/model-connections/state';
import { newModelConnectionDraft } from '../../../src/product/ModelManagementPage';
import { PresentationRoot } from '../../../src/ui';
import type { ModelConnectionBackend, ModelConnectionCheckRequest, ModelConnectionDraft, ModelConnectionCheckView } from '../../../src/features/model-connections/types';
import '../../../src/occami/styles.css';

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
function button(text: string) {
  const found = [...document.querySelectorAll<HTMLButtonElement>('button')].find(item => item.textContent?.trim() === text);
  assert(found, `Missing button ${text}`);
  return found;
}
function input(index: number, value: string) {
  const element = document.querySelectorAll<HTMLInputElement>('#mc-manual-form input')[index];
  assert(element, `Missing input ${index}`);
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(element, value);
  element.dispatchEvent(new Event('input', { bubbles: true }));
}
const digest: `sha256:${string}` = `sha256:${'a'.repeat(64)}`;
const operation = { operation_id: 'operation/test', state: 'succeeded' as const, sequence: 1, cancellable: false };

async function mounted(existingId: string | null, observedId: string | null, run: (trace: { checks: ModelConnectionCheckRequest[]; protectedInputs: number; saves: number }) => Promise<void>) {
  const trace = { checks: [] as ModelConnectionCheckRequest[], protectedInputs: 0, saves: 0 };
  const draft: ModelConnectionDraft = { ...newModelConnectionDraft(), display_name: '本地模型服务',
    base_url: 'https://example.invalid/v1', authentication: { kind: 'none' },
    ...(existingId ? { existing_source_id: 'source/existing', expected_source_revision: 1, models: [blankModel(existingId, '原有名称')] } : {}),
  };
  const backend: ModelConnectionBackend = {
    async listConnectionOptions() { return { schema: 'fixture', catalog: { product_release: 'fixture', catalog_binding_id: 'fixture', release_sequence: 1, connector_registry_digest: digest, model_data_digest: digest, cross_reference_digest: digest }, options: [] }; },
    async registerProtectedInput() { trace.protectedInputs++; return { input_candidate: { candidate_ref: 'input/test', candidate_revision: 1 } }; },
    async releaseProtectedInput() {},
    async cancelModelConnectionCheck() {},
    async checkRegisteredModelConnection() { throw new Error('Unexpected registered connection'); },
    async checkModelConnection(request) {
      trace.checks.push(structuredClone(request));
      const models = request.draft.models.length ? request.draft.models : observedId ? [blankModel(observedId, '目录模型')] : [];
      return {
        candidate: { candidate: { candidate_ref: 'candidate/test', candidate_revision: 1 },
          correlation: { candidate_ref: 'candidate/test', edit_revision: request.draft.edit_revision, check_id: request.draft.check_id, input_digest: digest },
          producer: 'native', provenance: 'user_configured', display_name: request.draft.display_name,
          fact_state: 'complete', input_state: 'not_required',
          models: models.map((model, index) => ({ model_ref: `model/${index}`, upstream_model_id: model.upstream_model_id,
            display_name: model.display_name, membership: observedId ? 'observed' : 'user_declared', fact_basis: 'user_declared', selectable: true })),
        },
        target: { scheme: 'https', authority: 'example.invalid', port: 443, request_path: '/v1/chat/completions', upstream_protocol: 'chat_completions', protocol_profile_id: 'profile/test', protocol_profile_revision: 1 },
        inventory_path: '/v1/models', reachability: 'reachable', authentication: 'not_required', directory: 'available', protocol: 'selected', inference: 'not_run',
        checked_model_count: models.length, invalid_model_count: 0, pages_read: 1, checked_at_unix_ms: 1, input_digest: digest,
      } satisfies ModelConnectionCheckView;
    },
    async previewComputeSave(change) { return { spec: { schema_version: { major: 1, minor: 0 }, command_id: 'fixture-save', desired_state: change }, accept_digest: digest, expected_revisions: change.expected_revisions, changes: [], affected_plan_refs: [] }; },
    async applyComputeSave() { trace.saves++; return { result: { operation_id: operation.operation_id, accepted_digest: digest, state: 'succeeded' }, operation }; },
    async getComputeSaveResult() { return { disposition: 'saved', bindings: [], source_id: 'source/test' }; },
  };
  const root = createRoot(document.getElementById('form-root')!);
  root.render(<PresentationRoot language="zh" theme="light" textScale={1}><ModelConnectionForm
    language="zh" mutable initialDraft={draft} expectedRevisions={{ target: 0, dependencies: {} }} backend={backend}
    onBack={() => {}} onCancel={() => {}} onSaveAccepted={() => {}} onSaveResult={() => {}} onSaveUncertain={() => {}} restoreFocus={() => {}}
  /></PresentationRoot>);
  try { await until(() => Boolean(document.querySelector('#mc-connection-form')), 'connection form'); await run(trace); }
  finally { root.unmount(); await tick(); }
}

const scenarios: [string, () => Promise<void>][] = [
  ['manual IDs reject new Unicode before check/save, preserve input and recover with ASCII', () => mounted(null, null, async trace => {
    button('手工维护模型').click(); await tick();
    input(0, '中文模型'); await tick(); input(1, '中文显示名称'); await tick();
    button('保存接入').click();
    await until(() => Boolean(document.querySelector('[data-error-code="MODEL_ID_ASCII_REQUIRED"]')), 'ASCII entry explanation');
    assert(document.querySelector('[role="alert"]')?.textContent?.includes('模型名称'), 'Error did not explain the name field');
    assert(trace.checks.length === 0 && trace.protectedInputs === 0 && trace.saves === 0, 'Rejected input reached the backend');
    assert(document.querySelector<HTMLInputElement>('#mc-manual-form input')?.value === '中文模型', 'Rejected input was discarded');
    button('返回').click(); await tick(); button('检查接入').click(); await tick();
    assert(trace.checks.length === 0, 'Returning to connection check bypassed manual validation');
    button('手工维护模型').click(); await tick(); input(0, 'vendor/model-v1'); await tick(); button('保存接入').click();
    await until(() => trace.saves === 1, 'corrected ASCII save');
    assert(trace.checks[0].draft.models[0].upstream_model_id === 'vendor/model-v1', 'ASCII correction changed');
    assert(trace.checks[0].draft.models[0].display_name === '中文显示名称', 'Localized display name changed');
  })],
  ['saved Unicode ID permits name edits but not a new non-ASCII replacement', () => mounted('旧模型\ufeff', null, async trace => {
    button('手工维护模型').click(); await tick(); input(0, '另一个模型'); await tick(); button('保存接入').click();
    await until(() => Boolean(document.querySelector('[data-error-code="MODEL_ID_ASCII_REQUIRED"]')), 'new replacement rejected');
    assert(trace.checks.length === 0 && trace.saves === 0, 'Changed Unicode ID inherited the saved exception');
    input(0, '旧模型\ufeff'); await tick(); input(1, '更新后的中文名称'); await tick(); button('保存接入').click();
    await until(() => trace.saves === 1, 'unchanged existing ID saved');
    assert(trace.checks[0].draft.models[0].upstream_model_id === '旧模型\ufeff', 'Existing ID normalized or changed');
    assert(trace.checks[0].draft.models[0].display_name === '更新后的中文名称', 'Name edit was blocked');
  })],
  ['directory-provided Unicode ID remains editable and saveable', () => mounted(null, '目录模型🧠', async trace => {
    button('检查接入').click();
    await until(() => Boolean(document.querySelector('.model-result-row')), 'observed catalog');
    document.querySelector<HTMLInputElement>('.model-result-row input[type="checkbox"]')!.click(); await tick();
    button('编辑所选模型 / 添加模型').click(); await tick(); input(1, '自定义目录名称'); await tick(); button('保存接入').click();
    await until(() => trace.saves === 1, 'observed Unicode save');
    assert(trace.checks[1].draft.models[0].upstream_model_id === '目录模型🧠', 'Observed ID rejected or rewritten');
    assert(trace.checks[1].draft.models[0].display_name === '自定义目录名称', 'Observed model name edit lost');
  })],
];

function Harness() {
  const [running, setRunning] = useState(false);
  const [results, setResults] = useState<{ name: string; state: string; error?: string }[]>([]);
  async function run() {
    setRunning(true); const next = [];
    for (const [name, scenario] of scenarios) {
      try { await scenario(); next.push({ name, state: 'green' }); }
      catch (error) { next.push({ name, state: 'red', error: error instanceof Error ? error.message : String(error) }); }
    }
    setResults(next); setRunning(false);
  }
  return <><p>Rendered production form with a recording backend fixture; not native/Tauri or live-provider acceptance.</p>
    <button data-run-tests disabled={running} onClick={() => void run()}>Run checks</button>
    <pre data-test-results>{JSON.stringify({ tests: results.length, passed: results.filter(row => row.state === 'green').length, failed: results.filter(row => row.state === 'red').length, results }, null, 2)}</pre></>;
}
createRoot(document.getElementById('root')!).render(<Harness />);
