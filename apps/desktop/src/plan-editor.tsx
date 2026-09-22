import { ProviderIcon } from './ui/ProviderIcon';
import { connectionName } from './ui/provider-identity';
import { planErrorCode, planErrorMessage } from './plan-editor-errors';
import { ReasoningDialog, type NativeReasoning } from './ui/ReasoningDialog';
import { ModelPicker } from './ui/ModelPicker';
import { BrandIcon, Disclosure, UiIcon } from './ui';
import { WorkerDependencies } from './features/WorkerDependencies';
import { PlanQuality, type PlanQualityModel } from './features/PlanQuality';
import { ClassifierProtocolDialog } from './features/ClassifierProtocolDialog';
import { confirmSaveDraftOrDiscard, useDiscardGuard } from './ui/discard-guard';
import { resolvePersistedEditor, type PersistedEditor, type PersistenceIdentity, type PlanOperation } from './plan-editor-persistence';
import React, { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
export type Selection = { binding_id: string; reasoning?: { kind: 'profile'; profile: string } | { kind: 'toggle'; enabled: boolean } | { kind: 'budget'; tokens: number } };
type Mode = 'fixed_model' | 'smart_saving' | 'free_first';
type Work = { harness: 'codex_cli' | 'claude_code'; protocol: 'responses' | 'messages' };
type ClassifierAuthHeader = { name: string; value_secret_ref: string };
type SmartClassifier = { kind: 'local_rules' } | {
  kind: 'rest';
  endpoint: string;
  timeout_ms: number;
  auth_header?: ClassifierAuthHeader | null;
};
type Smart = { economy: Selection[]; primary: Selection[]; primary_fallback: boolean; classifier: SmartClassifier; complex_keywords: string[] };
type Free = { candidates: Selection[]; primary: Selection[]; primary_fallback: boolean };
export type Editor = { schema: string; display_name: string; purpose: string; custom_alias?: string; mode: Mode; candidates: Selection[]; smart: Smart; free: Free; delegation_enabled: boolean; work?: Work; requirements: Record<string, unknown>; limits: { maximum_attempts: number; request_timeout_ms: number; attempt_timeout_ms: number } };
export type Plan = { agent_plan_id: string; desired: { display_name: string; purpose: string; mode: Mode; strategy: { mode: string; candidates?: Selection[] } & Partial<Smart & Free>; delegation_enabled: boolean; work?: Work; requirements: Record<string, unknown>; limits: Editor['limits'] }; head: { head_revision: number; status: string }; agent_plan_revision: number; model_alias: string; publication: { revision: number; digest: string }; execution: string };
export type Draft = { draft_id: string; plan_id?: string; base_head_revision?: number; revision: number; editor: Editor };
type Native = NativeReasoning;
type Candidate = { binding_id: string; model_configuration_id: string; display_name: string; reasoning: Native; billing_class: string; routable: boolean; ingress_protocols: string[] };
type CodexCapabilityLimit = { kind: 'context_window' | 'image_input'; binding_ids: string[] };
type CodexCapabilityIssue = { kind: 'plan_compilation' | 'invalid_compiled_plan' | 'responses_protocol' | 'request_capabilities' | 'instruction_roles' | 'context_input' | 'context_output' | 'context_total' | 'reasoning_profile' | 'context_window'; binding_id?: string };
type CodexCapabilities = { state: 'available'; context_window: number; input_modalities: ('text' | 'image')[]; reasoning: 'route_configuration'; limitations: CodexCapabilityLimit[]; fixed_limits: ('parallel_tool_calls_disabled')[] }
  | { state: 'unavailable'; issues: CodexCapabilityIssue[] };
type Options = { suggested_alias: string | null; candidates: Candidate[]; free_suggestions: { candidates: { selection: Selection }[]; unavailable: Record<string, string> } | null; codex_capabilities: CodexCapabilities | null };
type ValidationIssue = { message: string; group?: string; bindingId?: string; field?: 'alias' };
type ClassifierDecisionTestResult = { outcome: 'passed' | 'failed'; branch_id?: string | null; duration_millis?: number | null; failure_code?: string | null };
const DEFAULT_CLASSIFIER_TIMEOUT_MS = 3000;
const MAX_CLASSIFIER_TIMEOUT_MS = 3_600_000;
export type PlanEditorHandle = {
  saveDraft(): Promise<boolean>;
  publish(): Promise<boolean>;
  cancel(): void;
};

function defaultReasoning(native: Native | undefined, preferred: 'low' | 'high' = 'high'): Selection['reasoning'] {
  if (!native || native.kind === 'fixed' || native.kind === 'budget') return undefined;
  if (native.kind === 'toggle') return { kind: 'toggle', enabled: preferred === 'high' };
  const enabled = native.profiles.filter(profile => !['none', 'off', 'disabled'].includes(profile.toLocaleLowerCase()));
  const profiles = enabled.length ? enabled : native.profiles;
  const profile = preferred === 'low' ? profiles[0] : profiles.at(-1);
  return profile ? { kind: 'profile', profile } : undefined;
}
function defaultRestClassifier(): Extract<SmartClassifier, { kind: 'rest' }> {
  return { kind: 'rest', endpoint: '', timeout_ms: DEFAULT_CLASSIFIER_TIMEOUT_MS };
}
function validClassifierEndpoint(value: string): boolean {
  try {
    const endpoint = new URL(value);
    return value.trim() === value && !endpoint.username && !endpoint.password && !endpoint.hash
      && (endpoint.protocol === 'http:' || endpoint.protocol === 'https:')
      && Boolean(endpoint.hostname);
  } catch {
    return false;
  }
}
function validClassifierAuthHeader(value: ClassifierAuthHeader | null | undefined): boolean {
  if (!value) return true;
  const name = value.name;
  const forbidden = new Set(['host', 'content-length', 'content-type', 'transfer-encoding', 'connection', 'te', 'trailer', 'upgrade']);
  return name.length > 0 && name.length <= 128
    && /^[!#$%&'*+.^_`|~A-Za-z0-9-]+$/.test(name)
    && !forbidden.has(name.toLocaleLowerCase())
    && /^[a-zA-Z0-9._\-/:]{1,256}$/.test(value.value_secret_ref);
}
function validClassifierTimeout(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 1 && value <= MAX_CLASSIFIER_TIMEOUT_MS;
}
export function emptyEditor(): Editor { return { schema: 'hiroute.plan-editor/v2', display_name: '', purpose: '', mode: 'fixed_model', candidates: [], smart: { economy: [], primary: [], primary_fallback: false, classifier: { kind: 'local_rules' }, complex_keywords: [] }, free: { candidates: [], primary: [], primary_fallback: false }, delegation_enabled: false, requirements: {}, limits: { maximum_attempts: 6, request_timeout_ms: 60000, attempt_timeout_ms: 30000 } }; }
export function reopen(plan: Plan): Editor {
  const e = { ...emptyEditor(), display_name: plan.desired.display_name, purpose: plan.desired.purpose, custom_alias: plan.model_alias, mode: plan.desired.mode, delegation_enabled: plan.desired.delegation_enabled, work: plan.desired.work, requirements: plan.desired.requirements, limits: plan.desired.limits };
  const s = plan.desired.strategy;
  if (e.mode === 'fixed_model') e.candidates = s.candidates ?? [];
  else if (e.mode === 'smart_saving') e.smart = { economy: s.economy ?? [], primary: s.primary ?? [], primary_fallback: s.primary_fallback ?? false, classifier: s.classifier as SmartClassifier, complex_keywords: s.complex_keywords ?? [] };
  else e.free = { candidates: s.candidates ?? [], primary: s.primary ?? [], primary_fallback: s.primary_fallback ?? false };
  return e;
}
function activePlanSelections(plan: Plan): Selection[] {
  const strategy = plan.desired.strategy;
  if (plan.desired.mode === 'smart_saving') return [...(strategy.economy ?? []), ...(strategy.primary ?? [])];
  if (plan.desired.mode === 'free_first') return [...(strategy.candidates ?? []), ...(strategy.primary_fallback ? strategy.primary ?? [] : [])];
  return strategy.candidates ?? [];
}
function activePlanQualityModels(plan: Plan, candidates: Candidate[]): PlanQualityModel[] {
  const byBinding = new Map(candidates.map(candidate => [candidate.binding_id, candidate]));
  const seen = new Set<string>();
  const models: PlanQualityModel[] = [];
  for (const selection of activePlanSelections(plan)) {
    const candidate = byBinding.get(selection.binding_id);
    if (!candidate || seen.has(candidate.model_configuration_id)) continue;
    seen.add(candidate.model_configuration_id);
    models.push({ model_configuration_id: candidate.model_configuration_id, display_name: candidate.display_name });
  }
  return models;
}
export const PlanEditor = forwardRef<PlanEditorHandle, { plan?: Plan; draft?: Draft; creating?: boolean; initialBindingId?: string; language: 'zh' | 'en'; active?: boolean; usedBy?: { id: string; name: string; brand: 'codex' | 'claude-code' | 'agent'; isDefault: boolean }[]; onOpenAgent?: (agentId: string) => void; onOpenSession?: (sessionId: string, requestId: string) => void; onDone: () => Promise<void>; onClose: () => void; onDirty?: (dirty: boolean) => void; onEdit?: () => void; onBusyChange?: (busy: boolean) => void; onOperation?: (operation: PlanOperation | null) => void; onPersisted?: (editor: PersistedEditor<Plan, Draft> | null, action: 'save_draft' | 'publish', identity: PersistenceIdentity, operation: PlanOperation | null) => void }>(function PlanEditor({ plan, draft, creating = false, initialBindingId, language, active = true, usedBy = [], onOpenAgent, onOpenSession, onDone, onClose, onDirty, onEdit, onBusyChange, onOperation, onPersisted }, ref) {
  const en = language === 'en', text = (zh: string, eng: string) => en ? eng : zh;
  const [editor, setEditor] = useState<Editor>(() => structuredClone(draft?.editor ?? (plan ? reopen(plan) : emptyEditor())));
  const [draftId] = useState(() => draft?.draft_id ?? 'draft/' + crypto.randomUUID());
  const [base, setBase] = useState(() => ({ expected_head_revision: draft ? draft.base_head_revision ?? null : plan?.head.head_revision ?? null, expected_draft_revision: draft?.revision ?? null }));
  const [options, setOptions] = useState<Options | null>(null), [busy, setBusy] = useState(false), [error, setError] = useState('');
  const [baseline, setBaseline] = useState(() => JSON.stringify(editor));
  const [notice, setNotice] = useState('');
  const [keywordInput, setKeywordInput] = useState('');
  const [invalidFields, setInvalidFields] = useState(false);
  const [validationIssue, setValidationIssue] = useState<ValidationIssue | null>(null);
  const [diagnostic, setDiagnostic] = useState('');
  const classifierSecretInput = useRef<HTMLInputElement>(null);
  const [classifierTest, setClassifierTest] = useState<ClassifierDecisionTestResult | null>(null);
  const [classifierProtocolOpen, setClassifierProtocolOpen] = useState(false);
  const [optionsError, setOptionsError] = useState('');
  const [optionsRetry, setOptionsRetry] = useState(0);
  const [effort, setEffort] = useState<{ name: string; native: Native; value: Selection['reasoning']; apply: (value: Selection['reasoning']) => void } | null>(null);
  const [picker, setPicker] = useState<{ title: string; values: Selection[]; replace: (values: Selection[]) => void; free: boolean; preferred?: 'low' | 'high' } | null>(null);
  const [sources, setSources] = useState<Record<string, string>>({});
  const [sourceOptions, setSourceOptions] = useState<Record<string, string | null>>({});
  const dirty = JSON.stringify(editor) !== baseline;
  const confirmLeave = () => confirmSaveDraftOrDiscard(language, () => act('save_draft'));
  const confirmReplacement = async () => {
    const accepted = await confirmLeave();
    if (accepted) onClose();
    return accepted;
  };
  useDiscardGuard('routing', dirty, language, confirmReplacement);
  useEffect(() => { onDirty?.(dirty); }, [dirty, onDirty]);
  useEffect(() => { onBusyChange?.(busy); }, [busy, onBusyChange]);
  useImperativeHandle(ref, () => ({
    saveDraft: () => act('save_draft'),
    publish: () => act('publish'),
    cancel: onClose,
  }));
  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(''), 4500);
    return () => window.clearTimeout(timer);
  }, [notice]);
  useEffect(() => {
    if (!active) return;
    let current = true;
    void invoke<{ sources: { display_name: string; display_template_id?: string | null; connection_identity?: { connection_option_id: string | null }; models: { binding_id: string }[] }[] }>('compute_management_snapshot').then(v => {
      if (!current) return;
      setSources(Object.fromEntries(v.sources.flatMap(source => source.models.map(m => [m.binding_id, connectionName(source.connection_identity?.connection_option_id, language, source.display_name)]))));
      setSourceOptions(Object.fromEntries(v.sources.flatMap(source => source.models.map(m => [m.binding_id, source.display_template_id ?? source.connection_identity?.connection_option_id ?? null]))));
    }).catch(() => {});
    return () => { current = false; };
  }, [active, language]);
  useEffect(() => {
    if (!active) return;
    let current = true;
    const timer = setTimeout(async () => {
      try {
        const value = await invoke<Options>('plan_editor_options', { input: { display_name: editor.display_name, requirements: editor.requirements, editor } });
        if (current) { setOptions(value); setOptionsError(''); }
      } catch {
        try {
          const management = await invoke<{ sources: { models: unknown[] }[] }>('compute_management_snapshot');
          if (!current) return;
          if (!management.sources.some(source => source.models.length > 0)) {
            setOptions({ suggested_alias: null, candidates: [], free_suggestions: null, codex_capabilities: null });
            setOptionsError('');
          } else {
            setOptionsError(text('暂时无法读取可用于路由的模型，当前编辑已保留。', 'Unable to read routable models. Your edits are preserved.'));
          }
        } catch {
          if (current) setOptionsError(text('暂时无法读取可用于路由的模型，当前编辑已保留。', 'Unable to read routable models. Your edits are preserved.'));
        }
      }
    }, 200);
    return () => { current = false; clearTimeout(timer); };
  }, [active, editor, language, optionsRetry]);
  useEffect(() => {
    if (!creating || !options || editor.mode !== 'fixed_model' || editor.candidates.length) return;
    const first = options.candidates.find(candidate => candidate.routable && (!initialBindingId || candidate.binding_id === initialBindingId));
    if (!first) return;
    update({ candidates: [{ binding_id: first.binding_id, reasoning: defaultReasoning(first.reasoning) }] });
  }, [creating, options, editor.mode, editor.candidates.length, initialBindingId]);
  function reportError(cause: unknown) { setError(planErrorMessage(cause, language)); setDiagnostic(planErrorCode(cause)); }
  function update(patch: Partial<Editor>) { onEdit?.(); setEditor(e => ({ ...e, ...patch })); setError(''); setNotice(''); setDiagnostic(''); setValidationIssue(null); setClassifierTest(null); }
  function validateRoute(): ValidationIssue | null {
    if (editor.custom_alias !== undefined && !/^[a-z0-9][a-z0-9-]*$/.test(editor.custom_alias)) {
      return { field: 'alias', message: text('接入模型名只能使用小写字母、数字和连字符。', 'The connection model name may contain lowercase letters, numbers, and hyphens only.') };
    }
    if (!options) return { message: text('模型信息仍在读取，请稍后再试。', 'Model information is still loading. Try again shortly.') };
    if (editor.mode === 'smart_saving' && editor.smart.classifier.kind === 'rest') {
      const classifier = editor.smart.classifier;
      if (!validClassifierEndpoint(classifier.endpoint) || !validClassifierTimeout(classifier.timeout_ms) || !validClassifierAuthHeader(classifier.auth_header)) {
        return { group: 'classifier', message: text('请填写有效的 HTTP/HTTPS REST 地址和 1–3600000 毫秒分类超时；如启用认证，还需填写可用的请求头名称和 Secret 引用。', 'Enter a valid HTTP/HTTPS REST endpoint and a classifier timeout from 1 to 3600000 ms. If authentication is enabled, provide a valid header name and Secret reference.') };
      }
    }
    if (editor.delegation_enabled && !editor.work) {
      return { group: 'executor', message: text('开启任务委派后，请选择一个执行 Agent。', 'Choose an execution agent when task delegation is enabled.') };
    }
    const groups: { id: string; values: Selection[]; free?: boolean }[] = editor.mode === 'fixed_model'
      ? [{ id: 'fixed', values: editor.candidates }]
      : editor.mode === 'smart_saving'
          ? [{ id: 'economy', values: editor.smart.economy }, { id: 'primary', values: editor.smart.primary }]
          : [{ id: 'free', values: editor.free.candidates, free: true }, ...(editor.free.primary_fallback ? [{ id: 'free-primary', values: editor.free.primary }] : [])];
    for (const group of groups) {
      if (!group.values.length) return { group: group.id, message: text('请为每个启用的模型组合添加模型。', 'Add a model to each enabled group.') };
      for (const selection of group.values) {
        const candidate = options.candidates.find(item => item.binding_id === selection.binding_id);
        if (!candidate || !candidate.routable) return { group: group.id, bindingId: selection.binding_id, message: text('所选模型当前不可用于路由，请更换模型。', 'A selected model is not currently routable. Choose another model.') };
        if (group.free && candidate.billing_class !== 'free') return { group: group.id, bindingId: selection.binding_id, message: text('免费组合只能使用明确免费的模型。', 'The free group accepts only verified free models.') };
        if (editor.delegation_enabled && editor.work && !candidate.ingress_protocols.includes(editor.work.protocol)) return { group: group.id, bindingId: selection.binding_id, message: text(`“${candidate.display_name}”不支持所选执行 Agent 的协议，请更换模型或执行 Agent。`, `“${candidate.display_name}” does not support the selected execution agent protocol. Choose another model or agent.`) };
        const native = candidate.reasoning;
        const reasoning = selection.reasoning;
        const valid = native.kind === 'fixed' ? reasoning === undefined
          : native.kind === 'toggle' ? reasoning?.kind === 'toggle'
            : native.kind === 'discrete' ? reasoning?.kind === 'profile' && native.profiles.includes(reasoning.profile)
              : reasoning?.kind === 'budget' && Number.isSafeInteger(reasoning.tokens) && reasoning.tokens >= native.minimum_tokens && reasoning.tokens <= native.maximum_tokens && (reasoning.tokens - native.minimum_tokens) % native.step_tokens === 0;
        if (!valid) return { group: group.id, bindingId: selection.binding_id, message: native.kind === 'budget'
          ? text(`请为“${candidate.display_name}”填写有效的思考预算。`, `Enter a valid reasoning budget for “${candidate.display_name}”.`)
          : text(`请为“${candidate.display_name}”选择思考强度。`, `Choose reasoning for “${candidate.display_name}”.`) };
      }
    }
    return null;
  }
  async function act(action: string): Promise<boolean> {
    if (busy) return false;
    if (action === 'publish' || action === 'save_draft') {
      setInvalidFields(true);
      if (!editor.display_name.trim() || !editor.purpose.trim()) {
        setError(action === 'publish'
          ? text('请填写名称和使用场景后再启用路由。', 'Enter a name and purpose before enabling this route.')
          : text('请填写名称和使用场景后再保存草稿。', 'Enter a name and purpose before saving the draft.'));
        requestAnimationFrame(() => document.querySelector<HTMLInputElement>('.plan-identity-fields [aria-invalid=true]')?.focus());
        return false;
      }
      const issue = action === 'publish' ? validateRoute() : null;
      if (issue) {
        setValidationIssue(issue);
        setError(issue.message);
        requestAnimationFrame(() => {
          if (issue.field === 'alias') {
            const input = document.querySelector<HTMLInputElement>('.plan-alias-field');
            const details = input?.closest('details');
            if (details) details.open = true;
            input?.focus();
          } else if (issue.group === 'executor') {
            document.querySelector<HTMLElement>('.v3-executors button')?.focus();
          } else if (issue.group === 'classifier') {
            const classifier = editor.smart.classifier;
            const selector = classifier.kind === 'rest'
              && validClassifierEndpoint(classifier.endpoint)
              && !validClassifierTimeout(classifier.timeout_ms)
              ? '.classifier-timeout-field'
              : '.classifier-endpoint-field';
            document.querySelector<HTMLInputElement>(selector)?.focus();
          } else {
            document.querySelector<HTMLElement>(issue.bindingId ? `[data-binding-id="${CSS.escape(issue.bindingId)}"] .effort-select` : `[data-route-group="${issue.group}"] .add-candidate`)?.focus();
          }
        });
        return false;
      }
    }
    setBusy(true); setError(''); setNotice(''); setDiagnostic('');
    try {
      const outcome = await invoke<{ state: string; operation: PlanOperation | null }>('preview_plan_editor', { input: { action, plan_id: plan?.agent_plan_id ?? draft?.plan_id ?? null, draft_id: draftId, ...base, editor, language } });
      onOperation?.(outcome.operation);
      let persisted: PersistedEditor<Plan, Draft> | null = null;
      if (action === 'save_draft' || action === 'publish') {
        const identity: PersistenceIdentity = {
          draftId,
          planId: plan?.agent_plan_id ?? draft?.plan_id,
          modelAlias: editor.custom_alias ?? options?.suggested_alias ?? undefined,
          ...(action === 'publish'
            ? { targetHeadRevision: (base.expected_head_revision ?? 0) + 1 }
            : { targetDraftRevision: (base.expected_draft_revision ?? 0) + 1 }),
        };
        onPersisted?.(null, action, identity, outcome.operation);
        await onDone();
        const snapshot = await invoke<{ catalog: { drafts: Draft[]; plans: Plan[] } }>('desktop_snapshot');
        persisted = resolvePersistedEditor(action, snapshot.catalog, identity, {
          operationId: outcome.operation?.operation_id ?? null,
          operation: outcome.operation,
        });
        if (persisted) setBaseline(JSON.stringify(editor));
        onPersisted?.(persisted, action, identity, outcome.operation);
      } else await onDone();
      if ((action === 'save_draft' || action === 'publish') ? Boolean(persisted) : outcome.operation?.state === 'succeeded') {
        if (action === 'save_draft' || action === 'publish') {
          if (action === 'save_draft') {
            const saved = persisted?.draft;
            if (saved) setBase({ expected_head_revision: saved.base_head_revision ?? null, expected_draft_revision: saved.revision });
            if (!onPersisted) setNotice(text('草稿已保存，可以继续编辑或发布。', 'Draft saved. Continue editing or publish.'));
          } else {
            const publishedPlan = persisted?.plan;
            setBase({ expected_head_revision: publishedPlan?.head.head_revision ?? null, expected_draft_revision: null });
            if (!onPersisted) setNotice(text('更改已发布，新请求将使用当前路由。', 'Changes published. New requests will use this routing.'));
          }
        } else onClose();
        return true;
      }
    } catch (e) { if (planErrorCode(e) === 'PLAN_UNCHANGED') setNotice(planErrorMessage(e, language)); else reportError(e); }
    finally { setBusy(false); }
    return false;
  }
  async function useAllFree() {
    setBusy(true); setError('');
    try {
      const native_selections = Object.fromEntries(editor.free.candidates.filter(s => s.reasoning).map(s => [s.binding_id, s.reasoning]));
      const response = await invoke<Options>('plan_editor_options', { input: { requirements: editor.requirements, native_selections, suggest_free: true } });
      const candidates = response.free_suggestions?.candidates.map(c => c.selection) ?? [];
      if (candidates.length) {
        update({ free: { ...editor.free, candidates } });
        setNotice(text(`已加入 ${candidates.length} 个当前可用的免费模型。`, `Added ${candidates.length} currently available free models.`));
      } else {
        setNotice(text('当前没有符合能力和价格条件的免费模型。', 'No free model currently satisfies the capability and pricing requirements.'));
      }
    } catch (e) { reportError(e); } finally { setBusy(false); }
  }
  const billing = (value?: string) => value === 'free' ? text('免费', 'Free') : value === 'subscription' ? text('订阅', 'Subscription') : value === 'paid' ? text('按量付费', 'Usage priced') : text('价格未记录', 'Price not recorded');
  function reasoningControl(selection: Selection, candidate: Candidate | undefined, apply: (reasoning: Selection['reasoning']) => void, labeled = false) {
    const native = candidate?.reasoning;
    const reasoningLabel = selection.reasoning?.kind === 'profile' ? selection.reasoning.profile
      : selection.reasoning?.kind === 'toggle' ? (selection.reasoning.enabled ? text('思考开启', 'Reasoning on') : text('思考关闭', 'Reasoning off'))
      : selection.reasoning?.kind === 'budget' ? `${selection.reasoning.tokens} tokens` : text('设置思考强度', 'Set reasoning');
    if (native?.kind === 'fixed') return <span className="effort-select">{text('供应商默认', 'Provider default')}</span>;
    if (!native) return <span className="effort-select">{text('思考设置未取得', 'Reasoning unavailable')}</span>;
    return <button type="button" className="effort-select" aria-label={`${candidate.display_name} · ${text('思考强度', 'Reasoning')}`} onClick={() => setEffort({ name: candidate.display_name, native, value: selection.reasoning, apply })}>{labeled ? `${text('思考强度', 'Reasoning')}: ${reasoningLabel}` : reasoningLabel}</button>;
  }
  function changeMode(mode: Mode) {
    if (mode === editor.mode) return;
    const ready = options?.candidates.filter(candidate => candidate.routable) ?? [];
    if (mode === 'fixed_model') {
      const current = editor.candidates[0] ?? editor.smart.primary[0] ?? editor.smart.economy[0] ?? editor.free.primary[0] ?? editor.free.candidates[0];
      const candidate = (current ? ready.find(item => item.binding_id === current.binding_id) : undefined) ?? ready[0];
      update({ mode, candidates: editor.candidates.length ? editor.candidates : candidate
        ? [current?.binding_id === candidate.binding_id
          ? current
          : { binding_id: candidate.binding_id, reasoning: defaultReasoning(candidate.reasoning) }]
        : [] });
      return;
    }
    if (mode === 'smart_saving') {
      const economy = editor.smart.economy.length ? editor.smart.economy : ready.filter(candidate => candidate.billing_class === 'free').slice(0, 1).map(candidate => ({ binding_id: candidate.binding_id, reasoning: defaultReasoning(candidate.reasoning, 'low') }));
      const primary = editor.smart.primary.length ? editor.smart.primary : ready.filter(candidate => candidate.billing_class !== 'free').slice(0, 1).map(candidate => ({ binding_id: candidate.binding_id, reasoning: defaultReasoning(candidate.reasoning, 'high') }));
      update({ mode, smart: { ...editor.smart, economy, primary } });
      return;
    }
    const candidates = editor.free.candidates.length ? editor.free.candidates : ready.filter(candidate => candidate.billing_class === 'free').map(candidate => ({ binding_id: candidate.binding_id, reasoning: defaultReasoning(candidate.reasoning, 'low') }));
    update({ mode, free: { ...editor.free, candidates } });
  }
  function group(title: string, subtitle: string, values: Selection[], replace: (s: Selection[]) => void, { free = false, preferred, primary = false, groupId = '' }: { free?: boolean; preferred?: 'low' | 'high'; primary?: boolean; groupId?: string } = {}) {
    const candidates = options?.candidates ?? [];
    const move = (index: number, direction: -1 | 1) => {
      const target = index + direction;
      if (target < 0 || target >= values.length) return;
      const copy = [...values];
      [copy[target], copy[index]] = [copy[index], copy[target]];
      replace(copy);
    };
    const initials = (name: string) => name.split(/[\s._-]+/).filter(Boolean).slice(0, 2).map(part => part[0]?.toUpperCase()).join('').slice(0, 2) || 'M';
    return <section className={`route-lane${primary ? ' primary' : ''}${validationIssue?.group === groupId ? ' invalid' : ''}`} data-route-group={groupId}><div className="lane-head"><div><strong>{title}</strong><span>{subtitle}</span></div><span className="badge no-dot">{values.length}</span></div><div className="candidate-list">{values.map((s, index) => {
      const c = candidates.find(c => c.binding_id === s.binding_id);
      const put = (reasoning: Selection['reasoning']) => replace(values.map((v, i) => i === index ? { ...v, reasoning } : v));
      const displayName = c?.display_name ?? (options ? text('不可用模型', 'Unavailable model') : text('正在读取模型…', 'Loading model…'));
      return <div className={`candidate-row route-candidate${validationIssue?.bindingId === s.binding_id ? ' invalid' : ''}`} data-binding-id={s.binding_id} key={s.binding_id}>
        <span className="drag-handle" aria-hidden="true"><UiIcon name="grip" /></span>
        <span className="candidate-index">{index + 1}</span>
        <ProviderIcon optionId={sourceOptions[s.binding_id]} language={language} />
        <div className="candidate-main"><strong>{displayName}</strong><span>{billing(c?.billing_class)} · {sources[s.binding_id] ?? text('来源暂不可得', 'Source unavailable')}{c && !c.routable ? text(' · 当前不可用', ' · Unavailable') : ''}</span></div>
        {reasoningControl(s, c, put)}
        <div className="candidate-controls"><button type="button" className="icon-btn" disabled={index === 0} aria-label={text('上移', 'Move up')} onClick={() => move(index, -1)}><UiIcon name="arrowUp" /></button><button type="button" className="icon-btn" disabled={index === values.length - 1} aria-label={text('下移', 'Move down')} onClick={() => move(index, 1)}><UiIcon name="arrowDown" /></button><button type="button" className="icon-btn" aria-label={text('删除', 'Remove')} onClick={() => replace(values.filter((_, i) => i !== index))}><UiIcon name="trash" /></button></div>
      </div>;
    })}</div>{validationIssue?.group === groupId && <p className="oc-inline-error route-lane-error">{validationIssue.message}</p>}<button type="button" className="add-candidate" disabled={!options} onClick={() => setPicker({ title, values, replace, free, preferred })}><UiIcon name="plus" />{text('添加模型', 'Add model')}</button></section>;
  }
  async function saveClassifierSecret(classifier: Extract<SmartClassifier, { kind: 'rest' }>) {
    const auth = classifier.auth_header;
    const input = classifierSecretInput.current;
    if (!auth || !validClassifierAuthHeader(auth) || !input?.value) {
      setError(text('请填写 Secret 引用和认证值。', 'Enter a Secret reference and authentication value.'));
      return;
    }
    setBusy(true); setError(''); setDiagnostic('');
    try {
      const request = invoke<{ state: string }>('save_classifier_header_secret', { input: { secret_id: auth.value_secret_ref, secret: input.value } });
      input.value = '';
      const result = await request;
      if (result.state !== 'succeeded') throw new Error('CLASSIFIER_SECRET_SAVE_FAILED');
      setNotice(text('认证 Secret 已安全保存。', 'Authentication Secret saved securely.'));
    } catch (cause) {
      if (input) input.value = '';
      reportError(cause);
    } finally { setBusy(false); }
  }
  async function testRestClassifier(classifier: Extract<SmartClassifier, { kind: 'rest' }>) {
    if (!validClassifierEndpoint(classifier.endpoint) || !validClassifierTimeout(classifier.timeout_ms) || !validClassifierAuthHeader(classifier.auth_header)) {
      setError(text('请先填写有效的分类服务地址、分类超时和认证配置。', 'Enter a valid classifier endpoint, timeout, and authentication configuration first.'));
      return;
    }
    setBusy(true); setError(''); setDiagnostic(''); setClassifierTest(null);
    try {
      const result = await invoke<ClassifierDecisionTestResult>('test_classifier_decision', { input: { classifier } });
      setClassifierTest(result);
    } catch (cause) { reportError(cause); } finally { setBusy(false); }
  }
  function classifierEditor() {
    const classifier = editor.smart.classifier;
    const replace = (next: SmartClassifier) => update({ smart: { ...editor.smart, classifier: next } });
    const patchRest = (patch: Partial<Extract<SmartClassifier, { kind: 'rest' }>>) => {
      if (classifier.kind === 'rest') replace({ ...classifier, ...patch });
    };
    return <>
      <div className="editor-section-heading"><div><h3>{text('如何判断任务复杂度', 'How should task complexity be determined?')}</h3><p>{text('自定义服务会增加一次有界外部调用；失败时固定回退内置规则。', 'A custom service adds one bounded external call; failures fall back to built-in rules.')}</p></div><button className="btn" type="button" onClick={() => setClassifierProtocolOpen(true)}>{text('查看接入协议', 'View protocol')}</button></div>
      <div className="option-panel classifier-choices">
        <button type="button" className="option-row classifier-choice" aria-pressed={classifier.kind === 'local_rules'} onClick={() => { if (classifier.kind !== 'local_rules') replace({ kind: 'local_rules' }); }}><div><strong>{text('内置规则', 'Built-in rules')}</strong><span>{text('默认，不访问外部分类服务', 'Default; no external classifier call')}</span></div><span className={`badge ${classifier.kind === 'local_rules' ? 'info' : ''} no-dot`}>{classifier.kind === 'local_rules' ? text('已选择', 'Selected') : text('选择', 'Choose')}</span></button>
        <button type="button" className="option-row classifier-choice" aria-pressed={classifier.kind === 'rest'} onClick={() => { if (classifier.kind !== 'rest') replace(defaultRestClassifier()); }}><div><strong>{text('自定义分类服务', 'Custom classifier service')}</strong><span>{text('连接你部署的分类服务，可基于 Jev、LLM 或其他自定义策略实现', 'Connect a classifier you deploy, powered by Jev, an LLM, or another custom strategy')}</span></div><span className={`badge ${classifier.kind === 'rest' ? 'info' : ''} no-dot`}>{classifier.kind === 'rest' ? text('已选择', 'Selected') : text('选择', 'Choose')}</span></button>
      </div>
      {classifier.kind === 'rest' && <div className="field" data-route-group="classifier">
        <div className="callout"><UiIcon name="info" /><span>{text('这是你信任的服务。HiRoute 会发送完整 latest_user 和简化的 Agent turn 历史；服务自行决定是否使用或按模型上下文裁剪。', 'This must be a service you trust. HiRoute sends the complete latest_user and simplified Agent-turn history; the service decides what to use and how to trim it.')}</span></div>
        <label><span className="field-label">{text('分类服务地址', 'Classifier endpoint')}</span><input className="input classifier-endpoint-field" required maxLength={2048} aria-invalid={validationIssue?.group === 'classifier'} value={classifier.endpoint} placeholder="https://classifier.example/v1/decisions" onChange={event => patchRest({ endpoint: event.target.value })} /></label>
        <label><span className="field-label">{text('分类超时（毫秒）', 'Classifier timeout (ms)')}</span><input className="input classifier-timeout-field" type="number" required min={1} max={MAX_CLASSIFIER_TIMEOUT_MS} step={1} aria-invalid={validationIssue?.group === 'classifier'} value={classifier.timeout_ms} onChange={event => patchRest({ timeout_ms: Number(event.target.value) })} /><span className="field-help">{text('覆盖历史准备、认证、连接和响应读取。外置服务自身的总超时应略小于此值；官方 Jev 默认 2800 ms。', 'Covers history preparation, authentication, connection, and response reading. Set the external service timeout slightly lower; the official Jev decider defaults to 2800 ms.')}</span></label>
        <div className="option-panel">
          <button type="button" className="option-row" aria-pressed={!classifier.auth_header} onClick={() => patchRest({ auth_header: null })}><div><strong>{text('无认证', 'No authentication')}</strong><span>{text('适合本机或可信内网服务', 'For local or trusted-network services')}</span></div></button>
          <button type="button" className="option-row" aria-pressed={Boolean(classifier.auth_header)} onClick={() => patchRest({ auth_header: classifier.auth_header ?? { name: 'Authorization', value_secret_ref: '' } })}><div><strong>{text('自定义认证头', 'Custom authentication header')}</strong><span>{text('值从 Secret 读取，不保存在计划中', 'The value comes from a Secret and is never stored in the plan')}</span></div></button>
        </div>
        {classifier.auth_header && <><div className="oc-field-grid"><label><span className="field-label">{text('请求头名称', 'Header name')}</span><input className="input" required maxLength={128} value={classifier.auth_header.name} placeholder="Authorization" onChange={event => patchRest({ auth_header: { ...classifier.auth_header!, name: event.target.value } })} /></label><label><span className="field-label">{text('Secret 引用', 'Secret reference')}</span><input className="input" required maxLength={256} value={classifier.auth_header.value_secret_ref} placeholder="classifier/main" onChange={event => patchRest({ auth_header: { ...classifier.auth_header!, value_secret_ref: event.target.value } })} /></label></div><div className="oc-field-grid classifier-secret-create"><label><span className="field-label">{text('新建 Secret 的认证值', 'Authentication value for a new Secret')}</span><input ref={classifierSecretInput} className="input" type="password" autoComplete="new-password" maxLength={32768} placeholder="Bearer …" /></label><div className="field-actions"><button className="btn" type="button" onClick={() => void saveClassifierSecret(classifier)}>{text('安全保存 Secret', 'Save Secret securely')}</button><span className="field-help">{text('只创建新引用；替换时请使用新的引用。认证值不会写入计划。', 'Creates a new reference only; use a new reference to replace it. The value is not stored in the plan.')}</span></div></div></>}
        <div className="field-help">{text('请求仅包含允许分支、完整当前用户输入、简化历史、历史完整性和可评分范围。认证、上下文裁剪和策略部署由服务管理。', 'The request contains only allowed branches, the complete current user input, simplified history, history completeness, and the assessable range. The service owns authentication, context trimming, and strategy deployment.')}</div>
        <div className="callout warn"><UiIcon name="warning" /><span>{text('测试会向上述服务发送一个要求选择省钱分支的固定合成问题，并可能产生服务费用；不会读取真实会话，保存和发布也不会自动测试。', 'Testing sends a fixed synthetic prompt that asks for the economy branch and may incur service charges. It does not read a real conversation, and saving or publishing never tests automatically.')}</span></div>
        <div className="field-actions"><button className="btn" type="button" onClick={() => void testRestClassifier(classifier)}>{text('测试决策', 'Test decision')}</button>{classifierTest && <span className={`badge ${classifierTest.outcome === 'passed' ? 'good' : 'bad'} no-dot`}>{classifierTest.outcome === 'passed' ? text(`已选择 ${classifierTest.branch_id} · ${classifierTest.duration_millis} ms`, `Selected ${classifierTest.branch_id} · ${classifierTest.duration_millis} ms`) : text(`测试失败：${classifierTest.failure_code ?? 'unknown'}`, `Test failed: ${classifierTest.failure_code ?? 'unknown'}`)}</span>}</div>
        {validationIssue?.group === 'classifier' && <p className="oc-inline-error">{validationIssue.message}</p>}
      </div>}
    </>;
  }
  const editorState = dirty ? text('有未发布更改', 'Unpublished changes') : base.expected_draft_revision !== null ? text('草稿已保存', 'Draft saved') : plan ? text('已启用', 'Active') : text('未发布', 'Unpublished');
  const editorTone = dirty ? 'warn' : plan && base.expected_draft_revision === null ? 'good' : 'info';
  const codexCapabilities = options?.codex_capabilities;
  const formatContext = (tokens: number) => tokens >= 1000 && tokens % 1000 === 0 ? `${tokens / 1000}K` : tokens.toLocaleString(en ? 'en' : 'zh-CN');
  const capabilityBindings = (ids: string[]) => ids.map(id => options?.candidates.find(candidate => candidate.binding_id === id)?.display_name ?? id).join(text('、', ', '));
  const capabilityIssue = (issue: CodexCapabilityIssue) => {
    const candidate = issue.binding_id ? capabilityBindings([issue.binding_id]) : text('当前路由', 'This route');
    const reason = issue.kind === 'responses_protocol' ? text('缺少 Codex Responses 协议', 'is missing the Codex Responses protocol')
      : issue.kind === 'request_capabilities' ? text('Codex Responses 请求路径的协议能力无法证明（不一定是模型元信息缺失）', 'cannot prove the required Codex Responses protocol semantics (not necessarily missing model metadata)')
      : issue.kind === 'instruction_roles' ? text('当前接口无法保留 Codex 常规请求中的独立 developer 指令或中途指令位置；若是 Messages 来源，请改用原生 Responses', 'cannot preserve Codex developer instructions or mid-conversation instruction positions; use native Responses for a Messages source')
      : issue.kind === 'context_input' ? text('缺少输入上下文上限', 'is missing its input context limit')
      : issue.kind === 'context_output' ? text('缺少输出上限', 'is missing its output limit')
      : issue.kind === 'context_total' ? text('缺少总上下文上限', 'is missing its total context limit')
      : issue.kind === 'reasoning_profile' ? text('缺少所选推理配置事实', 'is missing facts for the selected reasoning configuration')
      : issue.kind === 'context_window' ? text('无法得到有效上下文窗口', 'does not yield a valid context window')
      : issue.kind === 'plan_compilation' ? text('当前候选无法按发布规则编译', 'cannot be compiled under the publication rules')
      : text('编译后的路由事实无效', 'has invalid compiled routing facts');
    return en ? `${candidate} ${reason}.` : `${candidate}：${reason}。`;
  };
  return <section className="plan-editor" aria-label={text('路由编辑器', 'Plan editor')}>{!creating && <header className="editor-header"><div className="editor-title"><div className="title-with-status"><h2>{editor.display_name || text('新建智能路由', 'New smart routing')}</h2><span className={`badge ${editorTone} no-dot`}>{editorState}</span></div></div><div className="editor-actions"><button className="btn" type="button" disabled={busy || (!dirty && Boolean(plan || draft))} onClick={() => void act('save_draft')}>{text('保存草稿', 'Save draft')}</button><button type="button" className="btn btn-primary" disabled={busy || (!dirty && !!plan && base.expected_draft_revision === null)} onClick={() => void act('publish')}><UiIcon name="upload" />{plan ? text('发布更改', 'Publish changes') : text('启用', 'Enable')}</button></div></header>}
    {notice && <div className="callout" role="status"><UiIcon name="info" /><span>{notice}</span></div>}{error && <div role="alert" className="callout bad" data-error-code={diagnostic}><UiIcon name="warning" /><span>{error}</span></div>}
    <fieldset disabled={busy}><section className="editor-section"><div className="editor-section-heading"><div><h3>{text('这份智能路由用来做什么', 'What this routing is for')}</h3><p>{text('Agent 根据用途选择适合任务的路由。', 'Your Agent uses this description to choose a route.')}</p></div></div><div className="plan-identity-fields"><label><span className="sr-only">{text('名称', 'Name')}</span><input className="input" autoFocus={creating} required placeholder={text('名称，例如：代码实现', 'Name, e.g. Code implementation')} aria-invalid={invalidFields && !editor.display_name.trim()} aria-describedby={invalidFields && !editor.display_name.trim() ? "route-name-error" : undefined} value={editor.display_name} maxLength={128} onChange={e => update({ display_name: e.target.value })} />{invalidFields && !editor.display_name.trim() && <span id="route-name-error" className="oc-inline-error">{text('请填写路由名称', 'Enter a route name')}</span>}</label><label><span className="sr-only">{text('使用场景', 'Purpose')}</span><input className="input" required placeholder={text('使用场景：适合做什么，期望交付什么', 'Purpose: tasks and expected results')} maxLength={512} aria-invalid={invalidFields && !editor.purpose.trim()} aria-describedby={invalidFields && !editor.purpose.trim() ? "route-purpose-error" : undefined} value={editor.purpose} onChange={e => update({ purpose: e.target.value })} />{invalidFields && !editor.purpose.trim() && <span id="route-purpose-error" className="oc-inline-error">{text('请填写使用场景', 'Enter a purpose')}</span>}</label>
    </div><div className="field plan-alias"><label><span className="field-label">{text('接入模型名', 'Connection model name')}</span><input className="input plan-alias-field" readOnly={!!plan} value={editor.custom_alias ?? options?.suggested_alias ?? ''} placeholder="hiroute-…" maxLength={64} aria-invalid={validationIssue?.field === 'alias'} onChange={e => update({ custom_alias: e.target.value })} /></label><span className="field-help">{text('在 Agent 中使用此名称调用这条智能路由。创建后保持稳定。', 'Use this name in an Agent to call the smart route. It remains stable after creation.')}</span>{validationIssue?.field === 'alias' && <span className="oc-inline-error">{validationIssue.message}</span>}{!plan && editor.custom_alias !== undefined && <button className="btn" type="button" onClick={() => update({ custom_alias: undefined })}>{text('恢复自动名称', 'Use automatic name')}</button>}</div></section>
    <section className="editor-section"><div className="editor-section-heading"><div><h3>{text('怎样使用模型', 'How to use models')}</h3></div></div>{optionsError && <div className="callout warn route-local-error" role="alert"><UiIcon name="warning" /><span>{optionsError}</span><button className="btn" type="button" onClick={() => setOptionsRetry(value => value + 1)}>{text('重试', 'Retry')}</button></div>}<div className="mode-switcher plan-mode-switcher">{(['fixed_model', 'smart_saving', 'free_first'] as Mode[]).map((m, i) => <button className={`mode-card${editor.mode === m ? ' active' : ''}`} type="button" aria-pressed={editor.mode === m} key={m} onClick={() => changeMode(m)}><strong>{text(['固定模型', '智能省钱', '免费优先'][i], ['Fixed model', 'Smart saving', 'Free first'][i])}</strong><span>{text(['按固定顺序依次尝试候选模型', '简单任务用省钱组合，复杂任务用主力组合', '先用免费模型，可选择主力兜底'][i], ['Try candidate models in a fixed order', 'Economy for simple tasks; primary for complex work', 'Use free models first, with optional primary fallback'][i])}</span></button>)}</div></section>
    {editor.mode === 'fixed_model' && <section className="editor-section"><div className="editor-section-heading"><div><h3>{text('固定模型顺序', 'Fixed model order')}</h3><p>{text('HiRoute 按此顺序尝试；不满足能力或暂时不可用的模型会被跳过。', 'HiRoute tries this order, skipping models that cannot satisfy the request or are temporarily unavailable.')}</p></div></div>{group(text('候选模型', 'Candidate models'), text('发布后保持此顺序', 'Keep this order after publication'), editor.candidates, candidates => update({ candidates }), { groupId: 'fixed' })}</section>}
    {editor.mode === 'smart_saving' && <section className="editor-section">
      <div className="editor-section-heading"><div><h3>{text('简单任务与复杂任务', 'Simple and complex tasks')}</h3><p>{text('简单任务先用省钱组合；复杂任务直接使用主力组合。', 'Simple tasks use the economy group; complex tasks use the primary group.')}</p></div></div>
      <div className="mini-route">{group(text('简单任务', 'Simple tasks'), text('省钱组合', 'Economy group'), editor.smart.economy, economy => update({ smart: { ...editor.smart, economy } }), { preferred: 'low', groupId: 'economy' })}<span className="lane-connector"><UiIcon name="route" /></span>{group(text('复杂任务', 'Complex tasks'), text('主力组合', 'Primary group'), editor.smart.primary, primary => update({ smart: { ...editor.smart, primary } }), { preferred: 'high', primary: true, groupId: 'primary' })}</div>
      <div className="option-panel"><button type="button" className="option-row" aria-pressed={editor.smart.primary_fallback} onClick={() => update({ smart: { ...editor.smart, primary_fallback: !editor.smart.primary_fallback } })}><div><strong>{text('省钱组合不可用时继续主力组合', 'Use primary when economy is unavailable')}</strong><span>{text('复杂任务不会降级到省钱组合', 'Complex tasks never downgrade to economy')}</span></div><span className={`switch${editor.smart.primary_fallback ? ' on' : ''}`} aria-hidden="true" /></button></div>
      {classifierEditor()}
      <Disclosure className="native-details keywords" label={text('关键词规则', 'Keyword rules')} language={language}><div className="editor-section-heading"><div><h3>{text('强制使用主力组合的关键词', 'Keywords that force the primary group')}</h3><p>{text('默认规则会识别多目标、架构、调试、跨文件和较长任务；REST 服务失败时也使用这些规则。', 'Default rules recognize multi-goal, architecture, debugging, cross-file and longer tasks; these rules are also used when the REST service fails.')}</p></div></div><div className="keyword-box">{editor.smart.complex_keywords.filter(Boolean).map(word => <span className="keyword-chip" key={word}>{word}<button type="button" aria-label={`${text('移除关键词', 'Remove keyword')} ${word}`} onClick={() => update({ smart: { ...editor.smart, complex_keywords: editor.smart.complex_keywords.filter(value => value !== word) } })}><UiIcon name="close" /></button></span>)}</div><div className="keyword-add"><input className="input" value={keywordInput} maxLength={64} placeholder={text('输入一个关键词或短语', 'Enter a keyword or phrase')} onChange={event => setKeywordInput(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); const word = keywordInput.trim(); if (word && !editor.smart.complex_keywords.includes(word)) update({ smart: { ...editor.smart, complex_keywords: [...editor.smart.complex_keywords, word] } }); setKeywordInput(''); } }} /><button className="btn" type="button" onClick={() => { const word = keywordInput.trim(); if (word && !editor.smart.complex_keywords.includes(word)) update({ smart: { ...editor.smart, complex_keywords: [...editor.smart.complex_keywords, word] } }); setKeywordInput(''); }}>{text('添加', 'Add')}</button></div></Disclosure>
    </section>}
    {editor.mode === 'free_first' && <section className="editor-section"><div className="editor-section-heading"><div><h3>{text('免费候选', 'Free candidates')}</h3><p>{text('当前候选与顺序在发布后保持固定。', 'Candidates and order stay fixed after publication.')}</p></div><button className="btn" type="button" onClick={() => void useAllFree()}>{text('使用当前全部可用免费模型', 'Use all available free models')}</button></div><div className="free-lane">{group(text('免费模型', 'Free models'), text('固定顺序', 'Fixed order'), editor.free.candidates, candidates => update({ free: { ...editor.free, candidates } }), { free: true, preferred: 'low', groupId: 'free' })}</div><div className="editor-section-heading fallback-heading"><div><h3>{text('免费模型都不可用时', 'When all free models are unavailable')}</h3></div></div><div className="option-panel"><button type="button" className="option-row" aria-pressed={!editor.free.primary_fallback} onClick={() => update({ free: { ...editor.free, primary_fallback: false } })}><div><strong>{text('停止并提示，只使用免费模型', 'Stop and report; free models only')}</strong><span>{text('绝不会进入订阅或付费模型', 'Never use subscription or paid models')}</span></div><span className={`badge ${!editor.free.primary_fallback ? 'info' : ''} no-dot`}>{!editor.free.primary_fallback ? text('已选择', 'Selected') : text('选择', 'Choose')}</span></button><button type="button" className="option-row" aria-pressed={editor.free.primary_fallback} onClick={() => update({ free: { ...editor.free, primary_fallback: true } })}><div><strong>{text('继续使用主力模型', 'Continue with primary models')}</strong><span>{text('只有免费池全部不可用时才进入主力组合', 'Use primary only when the free pool is unavailable')}</span></div><span className={`badge ${editor.free.primary_fallback ? 'info' : ''} no-dot`}>{editor.free.primary_fallback ? text('已选择', 'Selected') : text('选择', 'Choose')}</span></button></div>{editor.free.primary_fallback && <div className="free-lane">{group(text('主力模型', 'Primary models'), text('免费池的兜底', 'Fallback for free pool'), editor.free.primary, primary => update({ free: { ...editor.free, primary } }), { preferred: 'high', primary: true, groupId: 'free-primary' })}</div>}</section>}
    {codexCapabilities && <section className="editor-section codex-capability-summary" data-codex-capability-state={codexCapabilities.state}><div className="editor-section-heading"><div><h3>{text('对 Codex 的能力预览', 'Codex capability preview')}</h3><p>{text('依据当前编辑内容及候选的 Codex Responses 入口能力计算。上游使用其他协议时仍可能通过 HiRoute 适配；发布和接入时会重新核对，预览不代表真实客户端验证。', 'Calculated from the current edit and candidate Codex Responses ingress capabilities. HiRoute may adapt another upstream protocol; publication and connection recheck compatibility, and this preview is not a live client verification.')}</p></div></div>{codexCapabilities.state === 'available' ? <div className="option-panel"><div className="option-row" data-codex-capability-summary><div><strong>{text(`上下文 ${formatContext(codexCapabilities.context_window)} · 输入 ${codexCapabilities.input_modalities.includes('image') ? '文本/图片' : '文本'}`, `Context ${formatContext(codexCapabilities.context_window)} · Input ${codexCapabilities.input_modalities.includes('image') ? 'text/images' : 'text'}`)}</strong><span>{text('推理强度由路由配置决定', 'Reasoning effort is determined by the route configuration')}</span></div></div>{codexCapabilities.limitations.map(limit => <div className="callout warn" role="status" data-codex-capability-limit={limit.kind} key={limit.kind}><UiIcon name="warning" /><div><strong>{limit.kind === 'context_window' ? text('候选收窄了共同上下文', 'A candidate narrows the shared context') : text('候选收窄了图片输入', 'A candidate narrows image input')}</strong><p>{limit.kind === 'context_window' ? text(`${capabilityBindings(limit.binding_ids)} 将共同上下文限制为 ${formatContext(codexCapabilities.context_window)}。`, `${capabilityBindings(limit.binding_ids)} limits the shared context to ${formatContext(codexCapabilities.context_window)}.`) : text(`${capabilityBindings(limit.binding_ids)} 不支持完整的 Codex 图片输入；最终目录只声明文本。`, `${capabilityBindings(limit.binding_ids)} does not support complete Codex image input, so the final catalog declares text only.`)}</p></div></div>)}{codexCapabilities.fixed_limits.includes('parallel_tool_calls_disabled') && <div className="callout" data-codex-fixed-limit="parallel_tool_calls_disabled"><UiIcon name="info" /><div><strong>{text('HiRoute 当前固定使用串行工具调用', 'HiRoute currently uses serial tool calls')}</strong><p>{text('这是当前实现限制，不由任何候选模型造成。', 'This is a fixed implementation limit, not a candidate-model limitation.')}</p></div></div>}</div> : <div className="callout bad" role="alert" data-codex-capability-unavailable><UiIcon name="warning" /><div><strong>{text('无法生成可靠的 Codex 客户端能力', 'Reliable Codex client capabilities are unavailable')}</strong>{codexCapabilities.issues.map((issue, index) => <p key={`${issue.kind}:${issue.binding_id ?? index}`}>{capabilityIssue(issue)}</p>)}</div></div>}</section>}
    <section className="editor-section" data-route-group="executor"><div className="editor-section-heading"><div><h3>{text('任务委派', 'Task delegation')}</h3><p>{text('关闭时只使用模型路由，不显示或检测本机执行环境。', 'When off, this plan only routes models and does not show or detect a local execution environment.')}</p></div></div><div className="option-panel"><button type="button" className="option-row" aria-pressed={editor.delegation_enabled} onClick={() => update({ delegation_enabled: !editor.delegation_enabled })}><div><strong>{text('允许委派任务给执行 Agent', 'Allow delegation to an execution agent')}</strong><span>{text('开启后需要为这份计划选择一个执行 Agent；安装缺失不会阻止保存。', 'When enabled, choose one execution agent for this plan. Missing installation does not block saving.')}</span></div><span className={`switch${editor.delegation_enabled ? ' on' : ''}`} aria-hidden="true" /></button></div>
    {editor.delegation_enabled && <div className="field"><label className="field-label">{text('执行任务的 Agent', 'Task execution agent')}</label><p className="field-help">{text('每份计划选择一个执行 Agent，不做自动回退。', 'Choose one execution agent per plan; there is no automatic fallback.')}</p><div className="v3-executors">{([
      ['codex_cli', 'Codex CLI', text('使用 Codex CLI 执行委派任务', 'Use Codex CLI for delegated tasks')],
      ['claude_code', 'Claude Code', text('使用 Claude Code 执行委派任务', 'Use Claude Code for delegated tasks')],
    ] as const).map(([value, title, description]) => <button className={`v3-executor${editor.work?.harness === value ? ' selected' : ''}`} type="button" key={value} aria-pressed={editor.work?.harness === value} onClick={() => update({ work: { harness: value, protocol: value === 'claude_code' ? 'messages' : 'responses' } })}><BrandIcon kind={value === 'codex_cli' ? 'codex' : 'claude-code'} label={`${title} logo`} /><div><strong>{title}</strong><span>{description}</span></div>{editor.work?.harness === value && <UiIcon name="check" />}</button>)}</div>{validationIssue?.group === 'executor' && <p className="oc-inline-error route-lane-error">{validationIssue.message}</p>}
      {editor.work && <WorkerDependencies key={editor.work.harness} harness={editor.work.harness} language={language} active={active} onOperation={onOperation} />}
    </div>}</section>

    {plan && <section className="editor-section"><div className="editor-section-heading"><div><h3>{text('在哪里使用', 'Where it is used')}</h3><p>{text('在 Agent 页面修改默认模型路由。', 'Change the default model route from the Agent page.')}</p></div></div>{usedBy.length ? <div className="agent-card-list">{usedBy.map(agent => <button className="agent-card" type="button" key={agent.id} onClick={() => onOpenAgent?.(agent.id)} disabled={!onOpenAgent}><BrandIcon kind={agent.brand} label={`${agent.name} logo`} /><div className="agent-main"><strong>{agent.name}</strong><span>{agent.isDefault ? text('主 Agent 默认使用', 'Default for main agent') : text('用于模型路由', 'Used for model routing')}</span></div><UiIcon name="chevronRight" /></button>)}</div> : <span className="muted plan-unused">{text('尚未被任何 Agent 用作模型路由', 'Not used by any agent for model routing yet')}</span>}</section>}

    {plan && <section className="editor-section"><div className="editor-section-heading"><div><h3>{text('运行表现', 'Runtime performance')}</h3><p>{text('默认比较当前生效版本所选模型在时间范围内的阶段胜任度；未评分与部分证据会明确标出。', 'Compare stage competence over time for models selected by the active revision. Unrated and partial evidence remain explicit.')}</p></div></div><PlanQuality planId={plan.agent_plan_id} planRevision={plan.agent_plan_revision} currentModels={activePlanQualityModels(plan, options?.candidates ?? [])} language={language} onOpenEvidence={onOpenSession} /></section>}

    {(base.expected_draft_revision !== null || plan) && <div className="editor-actions editor-footer">{base.expected_draft_revision !== null && <button className="btn" type="button" onClick={() => void act('discard_draft')}>{text('丢弃草稿', 'Discard draft')}</button>}{plan && <Disclosure className="native-details" label={text('更多操作', 'More actions')} language={language}><button className="btn" type="button" onClick={() => void act(plan.head.status === 'disabled' ? 'enable' : 'disable')}>{plan.head.status === 'disabled' ? text('恢复调用', 'Enable') : text('停用路由', 'Disable')}</button><button className="btn btn-danger" type="button" onClick={() => void act('delete')}>{text('删除路由', 'Delete')}</button></Disclosure>}</div>}
    </fieldset>
    <ClassifierProtocolDialog open={classifierProtocolOpen} language={language} onClose={() => setClassifierProtocolOpen(false)} />
    {effort && <ReasoningDialog
      name={effort.name}
      native={effort.native}
      value={effort.value}
      language={language}
      onClose={() => setEffort(null)}
      onApply={value => { effort.apply(value); setEffort(null); setValidationIssue(null); setError(''); }}
    />}
    {picker && <ModelPicker
      title={text('添加候选模型', 'Add a candidate')}
      description={editor.display_name}
      language={language}
      single={false}
      value={picker.values.map(value => value.binding_id)}
      items={(options?.candidates ?? []).map(candidate => ({
        id: candidate.binding_id,
        optionId: sourceOptions[candidate.binding_id],
        name: candidate.display_name,
        source: `${billing(candidate.billing_class)} · ${sources[candidate.binding_id] ?? text('来源暂不可得', 'Source unavailable')}`,
        unavailable: !candidate.routable
          ? text('当前模型不可用', 'Model unavailable')
          : picker.free && candidate.billing_class !== 'free'
            ? text('不符合免费组条件', 'Not eligible for the free group')
            : editor.delegation_enabled && editor.work && !candidate.ingress_protocols.includes(editor.work.protocol)
              ? text('不支持所选执行 Agent', 'Incompatible with selected execution agent')
              : undefined,
      }))}
      onClose={() => setPicker(null)}
      onApply={ids => {
        const selections = ids.map<Selection>(id => {
          const previous = picker.values.find(value => value.binding_id === id);
          if (previous) return previous;
          const candidate = options?.candidates.find(value => value.binding_id === id);
          return { binding_id: id, reasoning: defaultReasoning(candidate?.reasoning, picker.preferred) };
        });
        picker.replace(selections);
        setValidationIssue(null);
        setError('');
        setPicker({ ...picker, values: selections });
      }}
    />}
  </section>;
});
