import { candidateUnavailableMessage, type UnavailableCandidate } from './plan-candidate-availability';
import type { OpenDecisionConnection } from './features/decision-services/presentation';
import { BranchRoutingEditor, FollowUpPreference } from './features/decision-services/BranchRoutingEditor';
import { branchRouting, branchSelections, classifierIssue, judgmentIssue, defaultJudgment, emptyService, routingIssue, type Judgment, type BranchRouting, type Classifier, type DecisionService } from './features/decision-services/types';
import { DecisionSelector } from './features/decision-services/DecisionSelector';
import { JudgmentFields, judgmentSummary } from './features/decision-services/JudgmentSettings';
import { AgentProtocolChoice, type ProtocolAdvice } from './features/AgentProtocolChoice';
import { contextWindowError, type ContextWindowBounds } from './plan-context-window';
import { DEFAULT_REQUEST_TIMEOUT_MS, requestTimeoutError } from './plan-request-timeout';
import { ProviderIcon } from './ui/ProviderIcon';
import { connectionName } from './ui/provider-identity';
import { planErrorCode, planErrorMessage } from './plan-editor-errors';
import { ReasoningDialog, type NativeReasoning } from './ui/ReasoningDialog';
import { ModelPicker } from './ui/ModelPicker';
import { BrandIcon, Disclosure, UiIcon } from './ui';
import { WorkerDependencies } from './features/WorkerDependencies';
import { PlanQuality, type PlanQualityModel } from './features/PlanQuality';
import { PlanRuntimeSettings } from './features/PlanRuntimeSettings';
import { confirmSaveDraftOrDiscard, useDiscardGuard } from './ui/discard-guard';
import { resolvePersistedEditor, type PersistedEditor, type PersistenceIdentity, type PlanOperation } from './plan-editor-persistence';
import React, { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
export type Selection = { binding_id: string; reasoning?: { kind: 'profile'; profile: string } | { kind: 'toggle'; enabled: boolean } | { kind: 'budget'; tokens: number } };
type Mode = 'fixed_model' | 'smart_saving' | 'free_first' | 'custom_branches';
type Work = { harness: 'codex_cli' | 'claude_code' | 'qoder_cli' | 'pi' | 'deepseek_harness'; protocol: 'responses' | 'messages' };
type SmartClassifier = Classifier;
// Inactive inputs belong to this editing session, never to the persisted plan.
export type PlanEditorMemory = { branchRouting?: BranchRouting; customWindowTokens?: number };
export type Smart = { economy: Selection[]; primary: Selection[]; judgment: Judgment; reselect_on_user_message: boolean; classifier: SmartClassifier; complex_keywords: string[] };
type Free = { candidates: Selection[]; primary: Selection[]; primary_fallback: boolean };
export type Editor = { schema: string; display_name: string; purpose: string; custom_alias?: string; mode: Mode; branch_routing?: BranchRouting | null; candidates: Selection[]; smart: Smart; free: Free; delegation_enabled: boolean; work?: Work; requirements: Record<string, unknown>; limits: { context_window_tokens?: number; maximum_attempts: number; request_timeout_ms: number; attempt_timeout_ms: number } };
export type Plan = { protocol_advice?: ProtocolAdvice; agent_plan_id: string; desired: { display_name: string; purpose: string; mode: Mode; strategy: { mode: string; candidates?: Selection[]; routing?: BranchRouting } & Partial<Smart & Free>; delegation_enabled: boolean; work?: Work; requirements: Record<string, unknown>; limits: Editor['limits'] }; head: { head_revision: number; status: string }; agent_plan_revision: number; model_alias: string; publication: { revision: number; digest: string }; execution: string };
export type Draft = { draft_id: string; plan_id?: string; base_head_revision?: number; revision: number; editor: Editor };
type Native = NativeReasoning;
type Candidate = { binding_id: string; model_configuration_id: string; display_name: string; reasoning: Native; billing_class: string; routable: boolean; ingress_protocols: string[]; native_ingress_protocols?: string[] };
type CodexCapabilityLimit = { kind: 'context_window' | 'image_input'; binding_ids: string[] };
type CodexCapabilityIssue = { kind: 'plan_compilation' | 'invalid_compiled_plan' | 'responses_protocol' | 'request_capabilities' | 'instruction_roles' | 'context_input' | 'context_output' | 'context_total' | 'reasoning_profile' | 'context_window'; binding_id?: string };
type CodexCapabilities = { state: 'available'; context_window: number; input_modalities: ('text' | 'image')[]; reasoning: 'route_configuration'; limitations: CodexCapabilityLimit[]; fixed_limits: ('parallel_tool_calls_disabled')[] }
  | { state: 'unavailable'; issues: CodexCapabilityIssue[] };
export type ClaudeCapabilities = { state: 'available'; context_window: number; plan_window: number } | { state: 'unavailable'; reason: string };
export type Options = { unavailable_candidate_count?: number; unavailable_candidates?: UnavailableCandidate[]; claude_capabilities?: ClaudeCapabilities | null; context_window: ContextWindowBounds | null; suggested_alias: string | null; candidates: Candidate[]; free_suggestions: { candidates: { selection: Selection }[]; unavailable: Record<string, string> } | null; codex_capabilities: CodexCapabilities | null };
type ValidationIssue = { selector?: string; message: string; group?: string; bindingId?: string; field?: 'alias' | 'context_window' | 'request_timeout' };
export type PlanEditorHandle = {
  saveDraft(): Promise<boolean>;
  publish(): Promise<boolean>;
  cancel(): void;
};

function workerProtocolAdvice(editor: Editor, options: Options | null): ProtocolAdvice | undefined {
  if (!options) return undefined;
  const selections = editor.branch_routing && editor.mode === 'custom_branches' ? branchSelections(editor.branch_routing) : editor.mode === 'fixed_model' ? editor.candidates
    : editor.mode === 'smart_saving' ? [...editor.smart.economy, ...editor.smart.primary]
      : [...editor.free.candidates, ...(editor.free.primary_fallback ? editor.free.primary : [])];
  const candidates = selections.map(s => options.candidates.find(c => c.binding_id === s.binding_id));
  if (!candidates.length || candidates.some(c => !c)) return undefined;
  const protocols = ['responses', 'messages'] as const;
  return { supported: protocols.filter(p => candidates.every(c => c!.ingress_protocols.includes(p))),
    native: protocols.filter(p => candidates.every(c => c!.native_ingress_protocols?.includes(p))) };
}

function defaultReasoning(native: Native | undefined, preferred: 'low' | 'high' = 'high'): Selection['reasoning'] {
  if (!native || native.kind === 'fixed' || native.kind === 'budget') return undefined;
  if (native.kind === 'toggle') return { kind: 'toggle', enabled: preferred === 'high' };
  const enabled = native.profiles.filter(profile => !['none', 'off', 'disabled'].includes(profile.toLocaleLowerCase()));
  const profiles = enabled.length ? enabled : native.profiles;
  const profile = preferred === 'low' ? profiles[0] : profiles.at(-1);
  return profile ? { kind: 'profile', profile } : undefined;
}
export function emptyEditor(): Editor { return { schema: 'hiroute.plan-editor/v2', display_name: '', purpose: '', mode: 'fixed_model', candidates: [], smart: { economy: [], primary: [], judgment: structuredClone(defaultJudgment), reselect_on_user_message: true, classifier: { kind: 'decision_service', service: emptyService() }, complex_keywords: [] }, free: { candidates: [], primary: [], primary_fallback: false }, delegation_enabled: false, requirements: {}, limits: { maximum_attempts: 6, request_timeout_ms: DEFAULT_REQUEST_TIMEOUT_MS, attempt_timeout_ms: 30000 } }; }
export function reopen(plan: Plan): Editor {
  const e = { ...emptyEditor(), display_name: plan.desired.display_name, purpose: plan.desired.purpose, custom_alias: plan.model_alias, mode: plan.desired.mode, delegation_enabled: plan.desired.delegation_enabled, work: plan.desired.work, requirements: plan.desired.requirements, limits: plan.desired.limits };
  const s = plan.desired.strategy;
  if (s.mode === 'branches' && s.routing) e.branch_routing = structuredClone(s.routing);
  else if (e.mode === 'fixed_model') e.candidates = s.candidates ?? [];
  else if (e.mode === 'smart_saving') e.smart = { economy: s.economy ?? [], primary: s.primary ?? [], judgment: structuredClone(s.judgment!), reselect_on_user_message: s.reselect_on_user_message as boolean, classifier: s.classifier as SmartClassifier, complex_keywords: s.complex_keywords ?? [] };
  else e.free = { candidates: s.candidates ?? [], primary: s.primary ?? [], primary_fallback: s.primary_fallback ?? false };
  return e;
}
function activePlanSelections(plan: Plan): Selection[] {
  const strategy = plan.desired.strategy;
  if (strategy.routing) return branchSelections(strategy.routing);
  if (plan.desired.mode === 'smart_saving') return [...(strategy.economy ?? []), ...(strategy.primary ?? [])];
  if (plan.desired.mode === 'free_first') return [...(strategy.candidates ?? []), ...(strategy.primary_fallback ? strategy.primary ?? [] : [])];
  return strategy.candidates ?? [];
}
function activePlanQualityModels(plan: Plan, candidates: Candidate[]): PlanQualityModel[] {
  const byBinding = new Map(candidates.map(candidate => [candidate.binding_id, candidate]));
  const strategy = plan.desired.strategy;
  const groups: { id: string; name?: string; floor?: number; group: 'regular' | 'primary'; selections: Selection[] }[] = strategy.routing
    ? strategy.routing.branches.flatMap(b => [
      { id: b.id, name: b.name, floor: (b.judgment ?? strategy.routing!.judgment).competence.floor_millis, group: 'regular' as const, selections: b.candidates },
      { id: b.id, name: b.name, floor: (b.judgment ?? strategy.routing!.judgment).competence.floor_millis, group: 'primary' as const, selections: b.primary_candidates },
    ]) : plan.desired.mode === 'smart_saving' ? [
      { id: 'smart_saving', floor: strategy.judgment?.competence.floor_millis, group: 'regular', selections: strategy.economy ?? [] },
      { id: 'smart_saving', floor: strategy.judgment?.competence.floor_millis, group: 'primary', selections: strategy.primary ?? [] },
    ] : [];
  const models: PlanQualityModel[] = [];
  for (const group of groups) {
    for (const [candidate_index, selection] of group.selections.entries()) {
      const candidate = byBinding.get(selection.binding_id);
      if (!candidate) continue;
      const reasoning = selection.reasoning;
      const reasoning_profile_id = reasoning?.kind === 'profile' ? reasoning.profile
        : reasoning?.kind === 'toggle' ? reasoning.enabled ? 'enabled' : 'disabled'
        : reasoning?.kind === 'budget' ? 'budget-' + reasoning.tokens
        : candidate.reasoning.kind === 'fixed' ? candidate.reasoning.profile : null;
      models.push({ plan_revision: plan.agent_plan_revision, branch_name: group.name, floor_millis: group.floor, group: group.group, candidate_index, model_configuration_id: candidate.model_configuration_id, display_name: candidate.display_name, branch_id: group.id, reasoning_profile_id });
    }
  }
  return models;
}
export const PlanEditor = forwardRef<PlanEditorHandle, { plan?: Plan; draft?: Draft; creating?: boolean; initialBindingId?: string; editingMemory?: PlanEditorMemory; language: 'zh' | 'en'; active?: boolean; refreshVersion?: number; usedBy?: { id: string; name: string; brand: 'codex' | 'claude-code' | 'qoder' | 'pi' | 'dsh' | 'agent'; isDefault: boolean }[]; onOpenAgent?: (agentId: string) => void; onOpenSession?: (sessionId: string, requestId: string) => void; onOpenServices?: OpenDecisionConnection; onDone: () => Promise<void>; onClose: () => void; onDirty?: (dirty: boolean) => void; onEdit?: () => void; onBusyChange?: (busy: boolean) => void; onOperation?: (operation: PlanOperation | null) => void; onPersisted?: (editor: PersistedEditor<Plan, Draft> | null, action: 'save_draft' | 'publish', identity: PersistenceIdentity, operation: PlanOperation | null) => void }>(function PlanEditor({ plan, draft, creating = false, initialBindingId, editingMemory, language, active = true, refreshVersion = 0, usedBy = [], onOpenAgent, onOpenSession, onOpenServices, onDone, onClose, onDirty, onEdit, onBusyChange, onOperation, onPersisted }, ref) {
  const en = language === 'en', text = (zh: string, eng: string) => en ? eng : zh;
  const [editor, setEditor] = useState<Editor>(() => structuredClone(draft?.editor ?? (plan ? reopen(plan) : emptyEditor())));
  const [draftId] = useState(() => draft?.draft_id ?? 'draft/' + crypto.randomUUID());
  const [base, setBase] = useState(() => ({ expected_head_revision: draft ? draft.base_head_revision ?? null : plan?.head.head_revision ?? null, expected_draft_revision: draft?.revision ?? null }));
  const [options, setOptions] = useState<Options | null>(null), [busy, setBusy] = useState(false), [error, setError] = useState('');
  const [baseline, setBaseline] = useState(() => JSON.stringify(editor));
  const [notice, setNotice] = useState('');
  const [view, setView] = useState<'configuration' | 'performance'>('configuration');
  const [services, setServices] = useState<DecisionService[]>([]);
  useEffect(() => { if (active) void invoke<{ services: DecisionService[] }>('decision_services').then(value => setServices(value.services)).catch(() => {}); }, [active]);
  const [invalidFields, setInvalidFields] = useState(false);
  const [validationIssue, setValidationIssue] = useState<ValidationIssue | null>(null);
  const [diagnostic, setDiagnostic] = useState('');
  const memory = useRef(editingMemory ?? {}).current;
  const [moreSettingsOpen, setMoreSettingsOpen] = useState(editor.delegation_enabled);
  const [optionsError, setOptionsError] = useState('');
  const [optionsErrorCode, setOptionsErrorCode] = useState('');
  const [optionsRetry, setOptionsRetry] = useState(0);
  const [effort, setEffort] = useState<{ name: string; native: Native; value: Selection['reasoning']; apply: (value: Selection['reasoning']) => void } | null>(null);
  const [picker, setPicker] = useState<{ title: string; values: Selection[]; replace: (values: Selection[]) => void; free: boolean; preferred?: 'low' | 'high' } | null>(null);
  const unavailable = options?.unavailable_candidates ?? [];
  const candidateIssue = (id: string) => unavailable.find(item => item.binding_id === id);
  const [sources, setSources] = useState<Record<string, string>>({});
  const [sourceOptions, setSourceOptions] = useState<Record<string, string | null>>({});
  const dirty = JSON.stringify(editor) !== baseline;
  const timeoutValidationError = requestTimeoutError(editor.limits.request_timeout_ms, editor.limits.attempt_timeout_ms, language);
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
        const value = await invoke<Options>('plan_editor_options', { input: { display_name: editor.display_name, requirements: editor.requirements, editor, include_unavailable: true } });
        if (current) { setOptions(value); setOptionsError(''); setOptionsErrorCode(''); }
      } catch (cause) {
        if (current) {
          setOptions(null);
          setOptionsError(planErrorMessage(cause, language));
          setOptionsErrorCode(planErrorCode(cause));
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
  function focusField(selector: string) {
    const element = document.querySelector<HTMLElement>(selector);
    let details = element?.closest('details');
    while (details) { details.open = true; details = details.parentElement?.closest('details'); }
    element?.focus();
  }
  function reportError(cause: unknown) { setError(planErrorMessage(cause, language)); setDiagnostic(planErrorCode(cause)); }
  function update(patch: Partial<Editor>) { onEdit?.(); setEditor(e => ({ ...e, ...patch })); setError(''); setNotice(''); setDiagnostic(''); setValidationIssue(null); }
  function validateRoute(): ValidationIssue | null {
    if (editor.custom_alias !== undefined && !/^[a-z0-9][a-z0-9-]*$/.test(editor.custom_alias)) {
      return { field: 'alias', message: text('接入模型名只能使用小写字母、数字和连字符。', 'The connection model name may contain lowercase letters, numbers, and hyphens only.') };
    }
    if (timeoutValidationError) return { field: 'request_timeout', message: timeoutValidationError };
    if (!options) return { message: text('模型信息仍在读取，请稍后再试。', 'Model information is still loading. Try again shortly.') };
    const windowError = contextWindowError(editor.limits.context_window_tokens, options.context_window, language);
    if (windowError) return { field: 'context_window', message: windowError };
    if (editor.branch_routing && editor.mode === 'custom_branches') { const issue = routingIssue(editor.branch_routing, language); if (issue) return issue; }
    if (editor.mode === 'smart_saving') {
      const issue = classifierIssue(editor.smart.classifier, language) ?? (editor.smart.classifier.kind === 'local_rules' ? null : judgmentIssue(editor.smart.judgment, 'smart', true, language));
      if (issue) return issue;
    }
    if (editor.delegation_enabled && !editor.work) {
      return { group: 'executor', message: text('开启任务委派后，请选择一个执行 Agent。', 'Choose an execution agent when task delegation is enabled.') };
    }
    const groups: { id: string; values: Selection[]; free?: boolean }[] = editor.branch_routing && editor.mode === 'custom_branches'
      ? editor.branch_routing.branches.flatMap(b => [{ id: b.id, values: b.candidates }, ...(b.primary_candidates.length ? [{ id: b.id + '-primary', values: b.primary_candidates }] : [])])
      : editor.mode === 'fixed_model'
      ? [{ id: 'fixed', values: editor.candidates }]
      : editor.mode === 'smart_saving'
          ? [{ id: 'economy', values: editor.smart.economy }, { id: 'primary', values: editor.smart.primary }]
          : [{ id: 'free', values: editor.free.candidates, free: true }, ...(editor.free.primary_fallback ? [{ id: 'free-primary', values: editor.free.primary }] : [])];
    for (const group of groups) {
      if (!group.values.length) return { group: group.id, message: text('请为每个启用的模型组合添加模型。', 'Add a model to each enabled group.') };
      for (const selection of group.values) {
        const candidate = options.candidates.find(item => item.binding_id === selection.binding_id);
        if (!candidate || !candidate.routable) return { group: group.id, bindingId: selection.binding_id, message: candidateIssue(selection.binding_id) ? candidateUnavailableMessage(candidateIssue(selection.binding_id)!.reason, language) : text('所选模型当前不可用于路由，请更换模型。', 'A selected model is not currently routable. Choose another model.') };
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
          if (issue.selector) focusField(issue.selector);
          else if (issue.field === 'alias') focusField('.plan-alias-field');
          else if (issue.field === 'context_window') focusField('.plan-context-window-field');
          else if (issue.field === 'request_timeout') focusField('.plan-request-timeout-field');
          else if (issue.group === 'executor') focusField('.v3-executors button');
          else if (issue.group === 'classifier') { focusField('[data-route-group=classifier] select');
          } else {
            focusField(issue.bindingId ? `[data-binding-id="${CSS.escape(issue.bindingId)}"] .effort-select` : `[data-branch-id="${issue.group}"] textarea, [data-route-group="${issue.group}"] .add-candidate`);
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
    if (editor.branch_routing && editor.mode === 'custom_branches') memory.branchRouting = structuredClone(editor.branch_routing);
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
    if (mode === 'custom_branches') {
      update({ mode, branch_routing: structuredClone(editor.branch_routing ?? memory.branchRouting ?? branchRouting(services.find(service => service.connection.kind === 'system_one'))) }); return;
    }
    if (mode === 'smart_saving') {
      const smart = { ...editor.smart };
      if (!smart.economy.length) smart.economy = ready.slice(0, 1).map(c => ({ binding_id: c.binding_id, reasoning: defaultReasoning(c.reasoning, 'low') }));
      if (!smart.primary.length) smart.primary = ready.slice(-1).map(c => ({ binding_id: c.binding_id, reasoning: defaultReasoning(c.reasoning, 'high') }));
      if (smart.classifier.kind === 'decision_service' && !smart.classifier.service.name) {
        const service = services.find(s => s.connection.kind === 'system_one');
        if (service) smart.classifier = { kind: 'decision_service', service: structuredClone(service) };
      }
      update({ mode, smart }); return;
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
      const issue = candidateIssue(s.binding_id);
      const displayName = c?.display_name ?? issue?.display_name ?? (options ? text('不可用模型', 'Unavailable model') : text('正在读取模型…', 'Loading model…'));
      return <div className={`candidate-row route-candidate${validationIssue?.bindingId === s.binding_id ? ' invalid' : ''}`} data-binding-id={s.binding_id} key={s.binding_id}>
        <span className="drag-handle" aria-hidden="true"><UiIcon name="grip" /></span>
        <span className="candidate-index">{index + 1}</span>
        <ProviderIcon optionId={sourceOptions[s.binding_id]} language={language} />
        <div className="candidate-main"><strong>{displayName}</strong><span>{billing(c?.billing_class)} · {sources[s.binding_id] ?? text('来源暂不可得', 'Source unavailable')}{c && !c.routable ? text(' · 当前不可用', ' · Unavailable') : ''}</span>{issue && <span className="oc-inline-error">{candidateUnavailableMessage(issue.reason, language)}</span>}</div>
        {reasoningControl(s, c, put)}
        <div className="candidate-controls"><button type="button" className="icon-btn" disabled={index === 0} aria-label={text('上移', 'Move up')} onClick={() => move(index, -1)}><UiIcon name="arrowUp" /></button><button type="button" className="icon-btn" disabled={index === values.length - 1} aria-label={text('下移', 'Move down')} onClick={() => move(index, 1)}><UiIcon name="arrowDown" /></button><button type="button" className="icon-btn" aria-label={text('删除', 'Remove')} onClick={() => replace(values.filter((_, i) => i !== index))}><UiIcon name="trash" /></button></div>
      </div>;
    })}</div>{validationIssue?.group === groupId && <p className="oc-inline-error route-lane-error">{validationIssue.message}</p>}<button type="button" className="add-candidate" disabled={!options} onClick={() => setPicker({ title, values, replace, free, preferred })}><UiIcon name="plus" />{text('添加模型', 'Add model')}</button></section>;
  }
  const callsStopped = plan?.head.status === 'disabled';
  const editorState = dirty ? text('有未发布更改', 'Unpublished changes') : base.expected_draft_revision !== null ? text('草稿已保存', 'Draft saved') : plan ? callsStopped ? text('已停用', 'Disabled') : text('已启用', 'Active') : text('未发布', 'Unpublished');
  const editorTone = dirty || callsStopped ? 'warn' : plan && base.expected_draft_revision === null ? 'good' : 'info';
  return <section className="plan-editor" aria-label={text('路由编辑器', 'Plan editor')}>{!creating && <header className="editor-header"><div className="editor-title"><div className="title-with-status"><h2>{editor.display_name || text('新建智能路由', 'New smart routing')}</h2><span className={`badge ${editorTone} no-dot`}>{editorState}</span>{callsStopped && (dirty || base.expected_draft_revision !== null) && <span className="badge warn no-dot">{text('已停用', 'Disabled')}</span>}</div></div><div className="editor-actions"><button className="btn" type="button" disabled={busy || (!dirty && Boolean(plan || draft))} onClick={() => void act('save_draft')}>{text('保存草稿', 'Save draft')}</button><button type="button" className="btn btn-primary" disabled={busy || (!dirty && !!plan && base.expected_draft_revision === null)} onClick={() => void act('publish')}><UiIcon name="upload" />{plan ? text('发布更改', 'Publish changes') : text('启用', 'Enable')}</button></div></header>}
    {notice && <div className="callout" role="status"><UiIcon name="info" /><span>{notice}</span></div>}{error && <div role="alert" className="callout bad" data-error-code={diagnostic}><UiIcon name="warning" /><span>{error}</span></div>}
    {plan && <div className="segmented quality-view-tabs" role="tablist" aria-label={text('智能路由视图', 'Smart routing views')}>
      <button className={'segment' + (view === 'configuration' ? ' active' : '')} type="button" role="tab" aria-selected={view === 'configuration'} onClick={() => setView('configuration')}>{text('路由配置', 'Routing configuration')}</button>
      <button className={'segment' + (view === 'performance' ? ' active' : '')} type="button" role="tab" aria-selected={view === 'performance'} onClick={() => setView('performance')}>{text('模型表现', 'Model performance')}</button>
    </div>}
    <fieldset disabled={busy}><div hidden={!!plan && view !== 'configuration'}><section className="editor-section"><div className="editor-section-heading"><div><h3>{text('这份智能路由用来做什么', 'What this routing is for')}</h3><p>{text('Agent 根据用途选择适合任务的路由。', 'Your Agent uses this description to choose a route.')}</p></div></div><div className="plan-identity-fields"><label><span className="sr-only">{text('名称', 'Name')}</span><input className="input" autoFocus={creating} required placeholder={text('名称，例如：代码实现', 'Name, e.g. Code implementation')} aria-invalid={invalidFields && !editor.display_name.trim()} aria-describedby={invalidFields && !editor.display_name.trim() ? "route-name-error" : undefined} value={editor.display_name} maxLength={128} onChange={e => update({ display_name: e.target.value })} />{invalidFields && !editor.display_name.trim() && <span id="route-name-error" className="oc-inline-error">{text('请填写路由名称', 'Enter a route name')}</span>}</label><label><span className="sr-only">{text('使用场景', 'Purpose')}</span><input className="input" required placeholder={text('使用场景：适合做什么，期望交付什么', 'Purpose: tasks and expected results')} maxLength={512} aria-invalid={invalidFields && !editor.purpose.trim()} aria-describedby={invalidFields && !editor.purpose.trim() ? "route-purpose-error" : undefined} value={editor.purpose} onChange={e => update({ purpose: e.target.value })} />{invalidFields && !editor.purpose.trim() && <span id="route-purpose-error" className="oc-inline-error">{text('请填写使用场景', 'Enter a purpose')}</span>}</label>
    </div><div className="field plan-alias"><label><span className="field-label">{text('接入模型名', 'Connection model name')}</span><input className="input plan-alias-field" readOnly={!!plan} value={editor.custom_alias ?? options?.suggested_alias ?? ''} placeholder="hiroute-…" maxLength={64} aria-invalid={validationIssue?.field === 'alias'} onChange={e => update({ custom_alias: e.target.value })} /></label><span className="field-help">{text('在 Agent 中使用此名称调用这条智能路由。创建后保持稳定。', 'Use this name in an Agent to call the smart route. It remains stable after creation.')}</span>{validationIssue?.field === 'alias' && <span className="oc-inline-error">{validationIssue.message}</span>}{!plan && editor.custom_alias !== undefined && <button className="btn" type="button" onClick={() => update({ custom_alias: undefined })}>{text('恢复自动名称', 'Use automatic name')}</button>}</div></section>
    <section className="editor-section"><div className="editor-section-heading"><div><h3>{text('怎样使用模型', 'How to use models')}</h3></div></div>{optionsError && <div className="callout warn route-local-error" role="alert" data-error-code={optionsErrorCode}><UiIcon name="warning" /><span>{optionsError}</span><button className="btn" type="button" onClick={() => setOptionsRetry(value => value + 1)}>{text('重试', 'Retry')}</button></div>}{!optionsError && options && !options.candidates.some(candidate => candidate.routable) && <p className="field-help" role="status">{unavailable.length ? text('已接入的模型当前不可用于路由，请查看下方原因并在模型页面处理。', 'Connected models are currently unavailable for routing. Review the reasons below and update them on the Models page.') : text('还没有可用于路由的模型，请先在模型页面接入模型。', 'No routable models yet. Connect a model on the Models page.')}</p>}{!optionsError && unavailable.length > 0 && <Disclosure className="route-unavailable-models" language={language} label={text(`不可用模型（${unavailable.length}）`, `Unavailable models (${unavailable.length})`)}>{options?.unavailable_candidate_count && <p>{text(`共 ${options.unavailable_candidate_count} 个不可用模型，此处显示前 ${unavailable.length} 个。可在模型页面查看并处理全部接入。`, `${options.unavailable_candidate_count} models are unavailable; showing the first ${unavailable.length}. Review all connections on the Models page.`)}</p>}<ul>{unavailable.map(item => <li key={item.binding_id}><strong>{item.display_name}</strong> — {candidateUnavailableMessage(item.reason, language)}</li>)}</ul></Disclosure>}<div className="mode-switcher plan-mode-switcher">{(['fixed_model', 'smart_saving', 'custom_branches', 'free_first'] as Mode[]).map((m, i) => <button className={`mode-card${editor.mode === m ? ' active' : ''}`} type="button" aria-pressed={editor.mode === m} key={m} onClick={() => changeMode(m)}><strong>{text(['固定模型', '智能省钱', '自定义分支', '免费优先'][i], ['Fixed model', 'Smart saving', 'Custom branches', 'Free first'][i])}</strong><span>{text(['按固定顺序依次尝试候选模型', '简单任务用省钱组合，复杂任务用主力组合', '按自己的任务条件选择分支和模型', '先用免费模型，可选择主力兜底'][i], ['Try candidate models in a fixed order', 'Economy for simple tasks; primary for complex work', 'Choose branches and models by your task conditions', 'Use free models first, with optional primary fallback'][i])}</span></button>)}</div></section>
    {editor.mode === 'fixed_model' && <section className="editor-section"><div className="editor-section-heading"><div><h3>{text('固定模型顺序', 'Fixed model order')}</h3><p>{text('HiRoute 按此顺序尝试；不满足能力或暂时不可用的模型会被跳过。', 'HiRoute tries this order, skipping models that cannot satisfy the request or are temporarily unavailable.')}</p></div></div>{group(text('候选模型', 'Candidate models'), text('发布后保持此顺序', 'Keep this order after publication'), editor.candidates, candidates => update({ candidates }), { groupId: 'fixed' })}</section>}
    {editor.mode === 'smart_saving' && <>
      <DecisionSelector classifier={editor.smart.classifier} smart services={services} language={language} onChange={classifier => update({ smart: { ...editor.smart, classifier } })} onOpenServices={onOpenServices} error={validationIssue?.group === 'classifier' ? validationIssue.message : undefined} />
      <section className="editor-section">{group(text('省钱模型', 'Economy models'), text('处理简单任务，按顺序尝试。', 'For simple tasks, tried in this order.'), editor.smart.economy, economy => update({ smart: { ...editor.smart, economy } }), { groupId: 'economy', preferred: 'low' })}</section>
      <section className="editor-section">{group(text('主力模型', 'Primary models'), text('任务复杂或上一阶段不胜任时使用，按顺序尝试。', 'For complex tasks or low competence, tried in this order.'), editor.smart.primary, primary => update({ smart: { ...editor.smart, primary } }), { groupId: 'primary', preferred: 'high' })}</section>
      {editor.smart.classifier.kind !== 'local_rules' && <section className="editor-section"><Disclosure label={text(`判断设置 · ${judgmentSummary(editor.smart.judgment, true, language)}`, `Judgment settings · ${judgmentSummary(editor.smart.judgment, true, language)}`)} language={language}><JudgmentFields value={editor.smart.judgment} onChange={judgment => update({ smart: { ...editor.smart, judgment } })} id="smart" language={language} /></Disclosure></section>}
      <section className="editor-section"><Disclosure label={text('追问设置', 'Follow-up settings')} language={language}>
        <FollowUpPreference value={editor.smart.reselect_on_user_message} onChange={reselect_on_user_message => update({ smart: { ...editor.smart, reselect_on_user_message } })} language={language} />
      </Disclosure></section>
    </>}
    {editor.branch_routing && editor.mode === 'custom_branches' && <BranchRoutingEditor routing={editor.branch_routing} services={services} language={language} onChange={branch_routing => update({ branch_routing })} onOpenServices={onOpenServices} group={group} errorGroup={validationIssue?.group} error={validationIssue?.message} />}
    {editor.mode === 'free_first' && <section className="editor-section"><div className="editor-section-heading"><div><h3>{text('免费候选', 'Free candidates')}</h3><p>{text('当前候选与顺序在发布后保持固定。', 'Candidates and order stay fixed after publication.')}</p></div><button className="btn" type="button" onClick={() => void useAllFree()}>{text('使用当前全部可用免费模型', 'Use all available free models')}</button></div><div className="free-lane">{group(text('免费模型', 'Free models'), text('固定顺序', 'Fixed order'), editor.free.candidates, candidates => update({ free: { ...editor.free, candidates } }), { free: true, preferred: 'low', groupId: 'free' })}</div><div className="editor-section-heading fallback-heading"><div><h3>{text('免费模型都不可用时', 'When all free models are unavailable')}</h3></div></div><div className="option-panel"><button type="button" className="option-row" aria-pressed={!editor.free.primary_fallback} onClick={() => update({ free: { ...editor.free, primary_fallback: false } })}><div><strong>{text('停止并提示，只使用免费模型', 'Stop and report; free models only')}</strong><span>{text('绝不会进入订阅或付费模型', 'Never use subscription or paid models')}</span></div><span className={`badge ${!editor.free.primary_fallback ? 'info' : ''} no-dot`}>{!editor.free.primary_fallback ? text('已选择', 'Selected') : text('选择', 'Choose')}</span></button><button type="button" className="option-row" aria-pressed={editor.free.primary_fallback} onClick={() => update({ free: { ...editor.free, primary_fallback: true } })}><div><strong>{text('继续使用主力模型', 'Continue with primary models')}</strong><span>{text('只有免费池全部不可用时才进入主力组合', 'Use primary only when the free pool is unavailable')}</span></div><span className={`badge ${editor.free.primary_fallback ? 'info' : ''} no-dot`}>{editor.free.primary_fallback ? text('已选择', 'Selected') : text('选择', 'Choose')}</span></button></div>{editor.free.primary_fallback && <div className="free-lane">{group(text('主力模型', 'Primary models'), text('免费池的兜底', 'Fallback for free pool'), editor.free.primary, primary => update({ free: { ...editor.free, primary } }), { preferred: 'high', primary: true, groupId: 'free-primary' })}</div>}</section>}
    <PlanRuntimeSettings limits={editor.limits} options={options} language={language} editingMemory={memory} onChange={limits => update({ limits })} />
    <section className="editor-section">
      <Disclosure className="native-details plan-more-settings" label={text('更多设置', 'More settings')} language={language} defaultOpen={moreSettingsOpen} onOpenChange={setMoreSettingsOpen}>
    <section className="editor-subsection" data-route-group="executor"><div className="editor-section-heading"><div><h3>{text('任务委派', 'Task delegation')}</h3><p>{text('关闭时只使用模型路由，不显示或检测本机执行环境。', 'When off, this plan only routes models and does not show or detect a local execution environment.')}</p></div></div><div className="option-panel"><button type="button" className="option-row" aria-pressed={editor.delegation_enabled} onClick={() => update({ delegation_enabled: !editor.delegation_enabled })}><div><strong>{text('允许委派任务给执行 Agent', 'Allow delegation to an execution agent')}</strong><span>{text('开启后需要为这份计划选择一个执行 Agent；安装缺失不会阻止保存。', 'When enabled, choose one execution agent for this plan. Missing installation does not block saving.')}</span></div><span className={`switch${editor.delegation_enabled ? ' on' : ''}`} aria-hidden="true" /></button></div>
    {editor.delegation_enabled && <div className="field"><label className="field-label">{text('执行任务的 Agent', 'Task execution agent')}</label><p className="field-help">{text('每份计划选择一个执行 Agent，不做自动回退。', 'Choose one execution agent per plan; there is no automatic fallback.')}</p><div className="v3-executors">{([
      ['codex_cli', 'Codex CLI', text('使用 Codex CLI 执行委派任务', 'Use Codex CLI for delegated tasks')],
      ['claude_code', 'Claude Code', text('使用 Claude Code 执行委派任务', 'Use Claude Code for delegated tasks')],
      ['deepseek_harness', 'DeepSeek Harness', text('原生 ACP 任务委派', 'Native ACP task delegation')],
      ['pi', 'Pi', text('使用 Pi 执行委派任务', 'Use Pi for delegated tasks')],
      ['qoder_cli', 'Qoder CLI', text('使用 Qoder CLI 执行委派任务', 'Use Qoder CLI for delegated tasks')],
    ] as const).map(([value, title, description]) => <button className={`v3-executor${editor.work?.harness === value ? ' selected' : ''}`} type="button" key={value} aria-pressed={editor.work?.harness === value} onClick={() => { if (editor.work?.harness !== value) update({ work: { harness: value, protocol: value === 'claude_code' ? 'messages' : 'responses' } }); }}><BrandIcon kind={value === 'codex_cli' ? 'codex' : value === 'claude_code' ? 'claude-code' : value === 'pi' ? 'pi' : value === 'deepseek_harness' ? 'dsh' : 'qoder'} label={`${title} logo`} /><div><strong>{title}</strong><span>{description}</span></div>{editor.work?.harness === value && <UiIcon name="check" />}</button>)}</div>{validationIssue?.group === 'executor' && <p className="oc-inline-error route-lane-error">{validationIssue.message}</p>}
      {editor.work && (editor.work.harness === 'pi' || editor.work.harness === 'deepseek_harness' || editor.work.harness === 'qoder_cli') && <AgentProtocolChoice name={editor.display_name || text('任务路由', 'Task route')} language={language} value={editor.work.protocol} advice={workerProtocolAdvice(editor, options)} onChange={protocol => update({ work: { ...editor.work!, protocol } })} />}
{editor.work && <WorkerDependencies key={editor.work.harness} harness={editor.work.harness} language={language} active={active && moreSettingsOpen} onOperation={onOperation} />}
    </div>}</section>
      </Disclosure>
    </section>

    {plan && <section className="editor-section"><div className="editor-section-heading"><div><h3>{text('在哪里使用', 'Where it is used')}</h3><p>{text('在 Agent 页面调整模型路由。', 'Adjust model routing from the Agent page.')}</p></div></div>{usedBy.length ? <div className="agent-card-list">{usedBy.map(agent => <button className="agent-card" type="button" key={agent.id} onClick={() => onOpenAgent?.(agent.id)} disabled={!onOpenAgent}><BrandIcon kind={agent.brand} label={`${agent.name} logo`} /><div className="agent-main"><strong>{agent.name}</strong><span>{agent.isDefault ? text('主 Agent 默认使用', 'Default for main agent') : text('用于模型路由', 'Used for model routing')}</span></div><UiIcon name="chevronRight" /></button>)}</div> : <span className="muted plan-unused">{text('尚未被任何 Agent 用作模型路由', 'Not used by any agent for model routing yet')}</span>}</section>}

    {(base.expected_draft_revision !== null || plan) && <div className="editor-actions editor-footer">{base.expected_draft_revision !== null && <button className="btn" type="button" onClick={() => void act('discard_draft')}>{text('丢弃草稿', 'Discard draft')}</button>}{plan && <Disclosure className="native-details" label={text('更多操作', 'More actions')} language={language}><button className="btn" type="button" onClick={() => void act(plan.head.status === 'disabled' ? 'enable' : 'disable')}>{plan.head.status === 'disabled' ? text('恢复调用', 'Enable') : text('停用路由', 'Disable')}</button><button className="btn btn-danger" type="button" onClick={() => void act('delete')}>{text('删除路由', 'Delete')}</button></Disclosure>}</div>}
    </div>
    {plan && <section className="editor-section quality-performance-panel" hidden={view !== 'performance'} role="tabpanel"><div className="editor-section-heading"><div><h3>{text('模型表现', 'Model performance')}</h3><p>{text('观察已发布路由中各模型的阶段胜任度，再查看具体执行阶段与证据。', 'Compare stage competence for models in the published route, then inspect execution stages and evidence.')}</p></div><span className="badge no-dot">{text('生效版本 r' + plan.agent_plan_revision, 'Active r' + plan.agent_plan_revision)}</span></div><PlanQuality planId={plan.agent_plan_id} planRevision={plan.agent_plan_revision} currentModels={activePlanQualityModels(plan, options?.candidates ?? [])} language={language} active={active && view === 'performance'} refreshVersion={refreshVersion} onOpenEvidence={onOpenSession} /></section>}
    </fieldset>
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
      showUnavailable
      value={picker.values.map(value => value.binding_id)}
      items={[...(options?.candidates ?? []).map(candidate => ({
        id: candidate.binding_id,
        optionId: sourceOptions[candidate.binding_id],
        name: candidate.display_name,
        source: `${billing(candidate.billing_class)} · ${sources[candidate.binding_id] ?? text('来源暂不可得', 'Source unavailable')}`,
        unavailable: !candidate.routable
          ? candidateUnavailableMessage(candidateIssue(candidate.binding_id)?.reason ?? 'invalid_configuration', language)
          : picker.free && candidate.billing_class !== 'free'
            ? text('不符合免费组条件', 'Not eligible for the free group')
            : editor.delegation_enabled && editor.work && !candidate.ingress_protocols.includes(editor.work.protocol)
              ? text('不支持所选执行 Agent', 'Incompatible with selected execution agent')
              : undefined,
      })), ...unavailable.filter(item => !options?.candidates.some(candidate => candidate.binding_id === item.binding_id)).map(item => ({
        id: item.binding_id, name: item.display_name,
        optionId: sourceOptions[item.binding_id],
        source: sources[item.binding_id] ?? text('来源暂不可得', 'Source unavailable'),
        unavailable: candidateUnavailableMessage(item.reason, language),
      }))]}
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
