import { useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ModelConnectionForm } from '../../../src/features/model-connections/ModelConnectionForm';
import { newModelConnectionDraft } from '../../../src/product/ModelManagementPage';
import { Models } from '../../../src/features/models/Models';
import type { ModelConnectionBackend } from '../../../src/features/model-connections/types';
import { mockProductInvoke, readyManagement } from './product-fixtures';
import { PresentationRoot } from '../../../src/ui/PresentationRoot';
import { requestEditorReplacement } from '../../../src/ui/discard-guard';
import { blankModel } from '../../../src/features/model-connections/state';
import '../../../src/occami/styles.css';

const params = new URLSearchParams(location.search);
const kind = params.get('kind') === 'free' ? 'free' : params.get('kind') === 'custom' ? 'custom' : 'known';
const trace = { checks: [] as unknown[], registered: [] as string[], released: [] as string[], saves: [] as unknown[], applies: [] as unknown[], accepted: [] as unknown[], uncertain: [] as unknown[], optionReads: 0 };
const pendingChecks: (() => void)[] = [];
const pendingApplies: (() => void)[] = [];
const pendingOptions: (() => void)[] = [];
let optionMode = params.get('options') ?? 'all';
let checkError = params.get('check_error');
Object.assign(window, { connectionFlowTrace: trace, finishConnectionFlowCheck: () => pendingChecks.shift()?.(),
  finishConnectionFlowApply: () => pendingApplies.shift()?.(), requestConnectionReplacement: () => requestEditorReplacement('models'),
  finishConnectionOptions: () => pendingOptions.shift()?.(), pendingConnectionOptions: () => pendingOptions.length,
  setConnectionOptionMode: (mode: string) => { optionMode = mode; }, setConnectionCheckError: (code: string | null) => { checkError = code; } });

function Harness() {
  const [open, setOpen] = useState(true);
  const [savedName, setSavedName] = useState('');
  const appendTo = params.has('append') ? { ...readyManagement.sources[0], display_name: '已有 · 接入', state: 'disabled' as const } : undefined;
  const initial = useMemo(() => appendTo ? { ...newModelConnectionDraft('custom'),
    existing_source_id: appendTo.source_id, display_name: appendTo.display_name, base_url: 'https://api.example.test',
  } : { ...newModelConnectionDraft(kind), ...(params.has('initial_model') ? { models: [blankModel('existing-model')] } : {}) }, []);
  const [language, setLanguage] = useState<'zh' | 'en'>(params.get('lang') === 'en' ? 'en' : 'zh');
  Object.assign(window, { setConnectionLanguage: setLanguage });
  const backend = useMemo<ModelConnectionBackend>(() => {
    const invoke = async (command: string, payload?: Record<string, unknown>) => mockProductInvoke(command, payload, kind === 'custom' && !appendTo ? 'unknown_model' : 'ready') as any;
    const check = async (command: string, request: any) => {
      if (params.get('hold_check')) await new Promise<void>(resolve => pendingChecks.push(resolve));
      if (checkError) throw { source: 'backend', envelope: {
        error: { code: 'INVALID_ARGUMENTS', message_key: checkError },
      } };
      const result = await invoke(command, { request });
      const ref = request.input_candidate?.candidate_ref ?? 'candidate/no-auth';
      return { ...result, candidate: { ...result.candidate, candidate: { candidate_ref: ref, candidate_revision: 2 },
        correlation: { ...result.candidate.correlation, candidate_ref: ref } } };
    };
    return {
      listConnectionOptions: async () => {
        const mode = optionMode;
        trace.optionReads++;
        if (mode === 'fail') throw new Error('fixture options unavailable');
        const options = await invoke('compute_connection_options');
        if (params.has('prefill_model')) options.options = options.options.map((option: { connection_option_id: string }) => option.connection_option_id === 'anthropic.platform.global.v1'
          ? { ...option, known_models: { 'fixture-prefill-model': 'Fixture prefill model' } } : option);
        if (mode === 'hold-one' || mode === 'hold-fail') await new Promise<void>(resolve => pendingOptions.push(resolve));
        if (mode === 'hold-fail') throw new Error('fixture deferred read failure');
        if (mode === 'one-noauth') return { ...options, options: options.options.filter((option: { connection_option_id: string }) => option.connection_option_id === 'anthropic.platform.global.v1').map((option: { endpoints: object[] }) => ({ ...option, endpoints: option.endpoints.map(endpoint => ({ ...endpoint, authentication_semantics: { kind: 'none' } })) })) };
        const allowed = mode === 'one' || mode === 'hold-one' ? ['anthropic.platform.global.v1']
          : mode === 'same-brand' || mode === 'bailian-disabled' ? ['bailian.payg.cn.v1', 'bailian.coding-plan.cn.v1']
            : mode === 'one-bailian' ? ['bailian.payg.cn.v1']
              : mode === 'none' ? [] : null;
        if (mode === 'unavailable') return { ...options, options: options.options.filter((option: { connection_option_id: string }) => option.connection_option_id === 'anthropic.platform.global.v1').map((option: object) => ({ ...option, registered_check_available: false })) };
        if (mode === 'bailian-disabled') return { ...options, options: options.options.filter((option: { connection_option_id: string }) => allowed!.includes(option.connection_option_id)).map((option: { connection_option_id: string }) => option.connection_option_id === 'bailian.payg.cn.v1' ? { ...option, registered_check_available: false } : option) };
        return allowed ? { ...options, options: options.options.filter((option: { connection_option_id: string }) => allowed.includes(option.connection_option_id)) } : options;
      },
      registerProtectedInput: async secret => {
        // Test labels identify fixture credentials without putting them into a wire draft.
        trace.registered.push(secret === 'fixture-key-a' ? 'a' : secret === 'fixture-key-b' ? 'b' : 'other');
        return { input_candidate: { candidate_ref: `input/${trace.registered.length}`, candidate_revision: 1 } };
      },
      releaseProtectedInput: async input => { trace.released.push(input.candidate_ref); },
      checkModelConnection: async request => { trace.checks.push(request); setSavedName(request.draft.display_name); return check('check_model_connection', request); },
      checkRegisteredModelConnection: async request => { trace.checks.push(request); setSavedName(request.display_name); return check('check_registered_model_connection', request); },
      cancelModelConnectionCheck: async () => undefined,
      previewComputeSave: async change => { trace.saves.push(change); return invoke('preview_compute_save', { change }); },
      applyComputeSave: async request => {
        trace.applies.push(request);
        if (params.has('hold_apply')) await new Promise<void>(resolve => pendingApplies.push(resolve));
        if (params.get('save_mode') === 'uncertain') throw new Error('fixture uncertain response');
        return invoke('apply_compute_save', { request });
      },
      getComputeSaveResult: async operation => {
        const result = await invoke('get_compute_save_result', { operation });
        return params.get('save_mode') === 'pending' ? { ...result, disposition: 'pending' } : result;
      },
    };
  }, [language]);
  const management = useMemo(() => ({ ...readyManagement, sources: readyManagement.sources.map(source =>
    source.provenance === 'registered' ? { ...source, display_name: savedName || '团队模型服务' } : source) }), [savedName]);
  return <PresentationRoot language={language} theme={params.get('theme') === 'dark' ? 'dark' : 'light'} textScale={params.get('scale') === '2' ? 2 : 1}>
    <Models language={language} snapshot={management} mutable onRefresh={async () => undefined}
      onPrepareKeyInput={async () => ({ input_candidate: { candidate_ref: 'input/saved', candidate_revision: 1 }, key_id: 'credential/test-saved' })}
      onPreview={async () => { throw new Error('Credential editing is outside this fixture'); }}
      onApply={async () => 'saved'} onDiscardProtectedInputs={() => undefined} />
    {open && <ModelConnectionForm language={language} initialDraft={initial} expectedRevisions={readyManagement.revisions} mutable backend={backend}
      singleProductShortcut={params.has('shortcut')} allowCustom={!params.has('no-custom')}
      appendTo={appendTo} existingNames={params.has('occupied') ? ['百炼 · Token Plan', '百炼 · Token Plan · 2'] : []}
      onBack={() => setOpen(false)} onCancel={() => setOpen(false)} onSaveAccepted={operation => trace.accepted.push(operation)}
      onSaveResult={() => setOpen(false)} onSaveUncertain={input => { trace.uncertain.push(input); setOpen(false); }} restoreFocus={() => undefined} />}
  </PresentationRoot>;
}
createRoot(document.getElementById('root')!).render(<Harness />);
