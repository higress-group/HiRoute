import { BrandIcon, Dialog, Disclosure, ProductPage, UiIcon } from './ui';
import { confirmDiscard, useDiscardGuard } from './ui/discard-guard';
import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  agentEditorSeed,
  codexDefaultChoiceValid,
  editorFingerprint,
  type AgentClaudePresetMappings,
  type AgentCollaborationTriggerMode,
  type AgentDefaultChoice,
  type AgentEditorValues,
  type CodexNativeModelMode,
} from './agent-editor-state';
import type { Plan } from './plan-editor';
import { AgentTasks, type AgentTask, type AgentTaskRead } from './features/AgentTasks';
import type { OperationReference } from './features/model-connections/types';
import { safeDiagnosticCode } from './error-code';
import {
  agentActionErrorMessage,
  classifyAgentMutation,
  type AgentMutationOutcome,
} from './agent-mutation-feedback';

export type CollaborationStatus = {
  state: string;
  restore_point_ref: string | null;
  current_selection?: { trigger_mode: AgentCollaborationTriggerMode } | null;
};
export type AgentModelSurface = 'codex_cli' | 'codex_desktop' | 'claude_cli';
export type AgentReasoningSelection =
  | { kind: 'profile'; profile: string }
  | { kind: 'toggle'; enabled: boolean }
  | { kind: 'budget'; tokens: number };
export type AgentNativeReasoning =
  | { kind: 'fixed'; profile: string }
  | { kind: 'toggle'; parameter: string }
  | { kind: 'discrete'; parameter: string; profiles: string[] }
  | { kind: 'budget'; parameter: string; minimum_tokens: number; maximum_tokens: number; step_tokens: number };
export type AgentFixedModel = {
  client_model_id: string;
  candidate: { binding_id: string; reasoning?: AgentReasoningSelection };
};
type AgentModelSourceCoverage = {
  binding_id: string;
  source_label: string;
  account_scope_ref: string;
  account_scope_digest: string;
  state: 'ready' | 'credential_required' | 'authorization_required' | 'disabled' | 'model_unconfirmed';
  reasoning: AgentNativeReasoning;
};
type AgentNativeModel = {
  client_model_id: string;
  display_name: string;
  source_options?: AgentModelSourceCoverage[];
};
type AgentNativeModelCatalog = {
  metadata_source: 'user_configured' | 'target_cache' | 'target_bundled';
  native_default_model: string;
  models: AgentNativeModel[];
};
type CodexModelSelection = {
  mode: 'codex_default';
  native_model_mode: CodexNativeModelMode;
  fixed_models: AgentFixedModel[];
  allowed_plan_ids: string[];
  default_selection: AgentDefaultChoice;
};
type ClaudeModelSelection = {
  mode: 'claude_launcher';
  surfaces: ['claude_cli'];
  fixed_models: AgentFixedModel[];
  preset_mappings: AgentClaudePresetMappings;
};
type AgentModelSelection = CodexModelSelection | ClaudeModelSelection;
type AgentSurfaceResult = {
  surface: AgentModelSurface;
  applied_revision: number;
  state: 'not_verified' | 'passed' | 'failed';
  reason_code: string | null;
};
type AgentLiveCheckTarget = {
  context_id: string;
  surface: AgentModelSurface;
  expected_applied_revision: number;
  client_model_ids: string[];
};
export type ModelStatus = {
  state: string;
  model_verified: boolean;
  applied_revision?: number | null;
  surface_results?: AgentSurfaceResult[];
  live_check_targets?: AgentLiveCheckTarget[];
  current_selection?: AgentModelSelection | null;
  protected_native_model_ids?: string[];
  restore_point_ref: string | null;
  collaboration?: CollaborationStatus | null;
};
export type Agent = {
  agent_id: string;
  version: string;
  context_id: string | null;
  configuration_state: string;
  available_surfaces?: AgentModelSurface[];
  native_model_catalog?: AgentNativeModelCatalog | null;
  settings: ModelStatus | null;
  status_error: string | null;
};
export type AgentSnapshot = {
  agents: Agent[];
  plans: { plans: Plan[] };
  trusted_authority: boolean;
};
type Preview = {
  applicable: boolean;
  unproven_native_model_ids?: string[];
  blockers: { reason: string; capabilities?: { capability: string; reason: string }[]; model_ids?: string[] }[];
};
type Outcome = {
  preview: Preview;
  mutation: AgentMutationOutcome | null;
};
type Facet = 'model' | 'collaboration';
type CheckScope = 'native_authentication' | 'collaboration';
type Editor = { context: string; facet: Facet };

const EMPTY_EDITOR_VALUES: AgentEditorValues = {
  fixedModels: [],
  nativeModelMode: 'hiroute_only',
  allowedPlanIds: [],
  defaultChoice: { kind: 'preserve_native' },
  claudePresets: {
    opus: { kind: 'preserve_native' },
    sonnet: { kind: 'preserve_native' },
    haiku: { kind: 'preserve_native' },
  },
  triggerMode: 'explicit',
};
export function desktopErrorCode(error: unknown): string {
  return safeDiagnosticCode(error, 'AGENT_UNAVAILABLE');
}

function prerequisiteCheck(preview: Preview, facet: Facet, agent: Agent): CheckScope | null {
  if (preview.applicable) return null;
  const missing = new Set(preview.blockers.flatMap(block => block.capabilities?.map(item => item.capability) ?? []));
  // Native ingress authentication can gate either facet: collaboration still
  // enters through the selected Agent even though it ultimately dispatches a
  // task. Treat the capability as the authority instead of inferring the check
  // from whichever settings form happened to expose the blocker.
  if (missing.has('ingress_authentication')) return 'native_authentication';
  if (facet === 'collaboration' && (missing.has('skill_loading') || missing.has('trusted_cli_execution'))) return 'collaboration';
  return null;
}

export function Agents({
  language,
  onMutation,
  onOperation,
  onUnverifiedOperation,
  initialAgentId = null,
  initialFacet = 'model',
  refreshVersion = 0,
  mutationAllowed = true,
  taskRead = { status: 'unavailable' },
  initialTab = 'configuration',
  initialTaskId = null,
  onTaskCancel,
  onOpenTaskSession,
  onOpenTasks,
  onLoadMoreTasks,
  onReadTask,
  onLoadTaskResult,
  onCreatePlan,
  active = true,
}: {
  language: 'zh' | 'en';
  onMutation: () => void;
  onOperation?: (operation: OperationReference, presentation: { kind: 'agent-settings'; target: string }) => void;
  onUnverifiedOperation?: (presentation: { kind: 'agent-settings'; target: string }) => void;
  initialAgentId?: string | null;
  initialFacet?: Facet;
  refreshVersion?: number;
  mutationAllowed?: boolean;
  taskRead?: AgentTaskRead;
  initialTab?: 'configuration' | 'tasks';
  initialTaskId?: string | null;
  onTaskCancel?: (task: AgentTask, onAccepted?: (status: AgentTask['status']) => void) => Promise<AgentTask['status']>;
  onOpenTaskSession?: (sessionId: string) => void;
  onOpenTasks?: () => void;
  onLoadMoreTasks?: () => Promise<{ tasks: AgentTask[]; hasMore: boolean }>;
  onReadTask?: (task: AgentTask) => Promise<AgentTask>;
  onLoadTaskResult?: (task: AgentTask) => Promise<AgentTask>;
  onCreatePlan?: () => void;
  active?: boolean;
}) {
  const zh = language === 'zh';
  const text = (cn: string, en: string) => (zh ? cn : en);
  const [snapshot, setSnapshot] = useState<AgentSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [busyText, setBusyText] = useState('');
  const [loadError, setLoadError] = useState('');
  const [actionError, setActionError] = useState('');
  const [notice, setNotice] = useState('');
  const [tokenEditing, setTokenEditing] = useState(false);
  const [tokenDraft, setTokenDraft] = useState('');
  const [selectedAgent, setSelectedAgent] = useState<string | null>(initialAgentId);
  const [showAgentList, setShowAgentList] = useState(false);
  const [tab, setTab] = useState<'configuration' | 'tasks'>(initialTab);
  const agentName = (agent: Agent) => agent.agent_id === 'agent_codex_default'
    ? 'Codex'
    : agent.agent_id === 'agent_claude_default'
      ? 'Claude Code'
      : agent.agent_id.toLowerCase().includes('cursor')
        ? 'Cursor'
        : text('本机 Agent', 'Local Agent');
  const [editor, setEditor] = useState<Editor | null>(null);
  const [editorValues, setEditorValues] = useState<AgentEditorValues>(EMPTY_EDITOR_VALUES);
  const [restoreNativeModel, setRestoreNativeModel] = useState('');
  const [selectionKnown, setSelectionKnown] = useState(false);
  const [facetEnabled, setFacetEnabled] = useState(false);
  const [baselineEnabled, setBaselineEnabled] = useState(false);
  const [baseline, setBaseline] = useState('');
  const fingerprint = editorFingerprint(editorValues);
  const dirty = !!editor && (fingerprint !== baseline || facetEnabled !== baselineEnabled);
  useDiscardGuard('agents', dirty, language, confirmEditorReplacement);
  const queryGeneration = useRef(0);
  const [preview, setPreview] = useState<Preview | null>(null);
  const trigger = useRef<HTMLElement | null>(null);
  const firstField = useRef<HTMLElement | null>(null);
  const initialTargetHandled = useRef(false);

  async function refresh() {
    const epoch = ++queryGeneration.current;
    try {
      const result = await invoke<AgentSnapshot>('agent_snapshot');
      if (epoch === queryGeneration.current) {
        setSnapshot(result);
        setLoadError('');
      }
    } catch (caught) {
      if (epoch === queryGeneration.current) setLoadError(desktopErrorCode(caught));
    }
  }
  useEffect(() => {
    void refresh();
  }, [refreshVersion]);
  useEffect(() => {
    if (editor) firstField.current?.focus();
  }, [editor]);
  useEffect(() => {
    if (!notice) return;
    const timeout = window.setTimeout(() => setNotice(''), 4500);
    return () => window.clearTimeout(timeout);
  }, [notice]);
  useEffect(() => {
    if (!snapshot || !initialAgentId || initialTargetHandled.current) return;
    const agent = snapshot.agents.find(item => item.agent_id === initialAgentId);
    const source = document.querySelector<HTMLElement>(
      `[data-agent-id="${CSS.escape(initialAgentId)}"] [data-agent-facet="${initialFacet}"]`,
    );
    initialTargetHandled.current = true;
    source?.scrollIntoView({ block: 'center' });
    if (agent && source && agent.context_id && snapshot.trusted_authority && mutationAllowed) open(agent, initialFacet, source);
    else source?.focus();
  }, [initialAgentId, initialFacet, mutationAllowed, snapshot]);

  function discardEditor() {
    setEditor(null);
    setPreview(null);
    setActionError('');
    setNotice('');
    requestAnimationFrame(() => trigger.current?.focus());
  }
  async function confirmEditorReplacement() {
    if (dirty && !(await confirmDiscard(language))) return false;
    discardEditor();
    return true;
  }
  function close() {
    discardEditor();
  }
  function createPlanFromEditor() {
    discardEditor();
    requestAnimationFrame(() => onCreatePlan?.());
  }
  async function open(agent: Agent, facet: Facet, source: HTMLElement) {
    if (!mutationAllowed || !snapshot?.trusted_authority || !agent.context_id) return;
    if (dirty && !(await confirmDiscard(language))) return;
    trigger.current = source;
    setActionError('');
    setNotice('');
    setPreview(null);
    setTokenEditing(false);
    setTokenDraft('');
    setSelectedAgent(agent.agent_id);
    setEditor({ context: agent.context_id, facet });
    const { known, ...seed } = agentEditorSeed(agent, facet);
    setSelectionKnown(known);
    setEditorValues(seed);
    const enabled = facet === 'model' ? Boolean(agent.settings?.current_selection) : Boolean(agent.settings?.collaboration?.current_selection);
    setFacetEnabled(enabled);
    setBaselineEnabled(enabled);
    setBaseline(editorFingerprint(seed));
  }
  function spec(agent: Agent, facet: Facet, restore: boolean) {
    if (!agent.context_id) throw new Error('AGENT_INPUT_INVALID');
    const keep = { intent: 'keep' };
    if (facet === 'model') {
      const settings = agent.agent_id === 'agent_codex_default'
        ? {
            mode: 'codex_default',
            native_model_mode: editorValues.nativeModelMode,
            fixed_models: editorValues.fixedModels,
            allowed_plan_ids: editorValues.allowedPlanIds,
            default_selection: editorValues.defaultChoice,
          }
        : {
            mode: 'claude_launcher',
            surfaces: ['claude_cli'],
            fixed_models: editorValues.fixedModels,
            preset_mappings: editorValues.claudePresets,
          };
      return {
        schema_version: { major: 2, minor: 0 },
        context_id: agent.context_id,
        restore_native_model: restore && agent.agent_id === 'agent_codex_default' && restoreNativeModel
          ? restoreNativeModel : undefined,
        model: restore
          ? { intent: 'restore', restore_point_ref: agent.settings?.restore_point_ref }
          : { intent: 'configure', settings },
        collaboration: keep,
      };
    }

    return {
      schema_version: { major: 2, minor: 0 },
      context_id: agent.context_id,
      model: keep,
      collaboration: restore
        ? {
            intent: 'restore',
            restore_point_ref: agent.settings?.collaboration?.restore_point_ref,
          }
        : {
            intent: 'configure',
            settings: {
              trigger_mode: editorValues.triggerMode,
            },
          },
    };
  }
  async function change(agent: Agent, facet: Facet, restore: boolean, source: HTMLElement | null) {
    if (busy || !mutationAllowed || !snapshot?.trusted_authority || !agent.context_id || (!restore && !selectionKnown)) return;
    setBusy(true);
    setBusyText(text('正在保存…', 'Saving…'));
    setActionError('');
    setNotice('');
    setPreview(null);
    let blockedByCheck: Preview | null = null;
    try {
      const input = { language, spec: spec(agent, facet, restore) };
      let result = await invoke<Outcome>('preview_agent_settings', { input });
      const checkScope = !restore ? prerequisiteCheck(result.preview, facet, agent) : null;
      if (checkScope) {
        blockedByCheck = result.preview;
        setBusyText(text('正在检查并保存…', 'Checking and saving…'));
        const checked = await invoke<boolean>('check_agent_authentication', {
          input: { agent_id: agent.agent_id, language, scope: checkScope },
        });
        if (!checked) {
          setPreview(blockedByCheck);
          setNotice(text('已取消检查，未提交变更。', 'Check cancelled. No changes were submitted.'));
          return;
        }
        setBusyText(text('正在保存…', 'Saving…'));
        blockedByCheck = null;
        result = await invoke<Outcome>('preview_agent_settings', { input });
      }
      setPreview(result.preview);
      if (result.mutation) {
        const disposition = classifyAgentMutation(result.mutation);
        const presentation = {
          kind: 'agent-settings' as const,
          target: `${agentName(agent)} ${text('配置', 'settings')}`,
        };
        setNotice(disposition === 'cancelled'
          ? text('已取消，未提交变更。', 'Cancelled before submission.')
          : '');
        if (result.mutation.operation) {
          onOperation?.(result.mutation.operation, presentation);
        } else if (disposition === 'unverified') {
          onUnverifiedOperation?.(presentation);
        }
        if (disposition === 'submitted') {
          setBaseline(fingerprint);
          setBaselineEnabled(facetEnabled);
          setEditor(null);
        }
        onMutation();
      }
      await refresh();
    } catch (caught) {
      setPreview(blockedByCheck);
      setActionError(desktopErrorCode(caught));
      onMutation();
      await refresh();
    } finally {
      setBusy(false);
      setBusyText('');
      requestAnimationFrame(() => { if (source?.isConnected) source.focus(); else trigger.current?.focus(); });
    }
  }
  async function changeToken(agent: Agent, regenerate: boolean, source: HTMLElement | null) {
    const selection = agent.settings?.current_selection;
    if (busy || !mutationAllowed || !snapshot?.trusted_authority || !agent.context_id || !selection || agent.settings?.state !== 'configured') return;
    const custom = regenerate ? undefined : tokenDraft;
    if (custom !== undefined && !/^[A-Za-z0-9._~-]{16,128}$/.test(custom)) {
      setActionError('AGENT_TOKEN_INVALID');
      return;
    }
    setBusy(true);
    setBusyText(text('正在更新令牌…', 'Updating token…'));
    setActionError('');
    setNotice('');
    try {
      const input = {
        language,
        spec: {
          schema_version: { major: 2, minor: 0 },
          context_id: agent.context_id,
          model: { intent: 'configure', settings: selection },
          collaboration: { intent: 'keep' },
          access_token: { intent: regenerate ? 'regenerate' : 'keep' },
        },
        custom_token: custom,
      };
      const result = await invoke<Outcome>('preview_agent_settings', { input });
      setPreview(result.preview);
      if (result.mutation) {
        const disposition = classifyAgentMutation(result.mutation);
        const presentation = { kind: 'agent-settings' as const, target: `${agentName(agent)} ${text('接入令牌', 'access token')}` };
        if (result.mutation.operation) onOperation?.(result.mutation.operation, presentation);
        else if (disposition === 'unverified') onUnverifiedOperation?.(presentation);
        if (disposition === 'submitted') {
          setTokenEditing(false);
          setTokenDraft('');
          setNotice(text('令牌变更已提交，完成后新请求会使用新令牌。', 'Token change submitted. New requests will use it when the operation completes.'));
        }
        onMutation();
      }
      await refresh();
    } catch (caught) {
      setActionError(desktopErrorCode(caught));
      await refresh();
    } finally {
      setBusy(false);
      setBusyText('');
      requestAnimationFrame(() => { if (source?.isConnected) source.focus(); });
    }
  }
  async function check(agent: Agent, scope: CheckScope, source: HTMLElement) {
    if (!mutationAllowed || !snapshot?.trusted_authority) return;
    setBusy(true);
    setBusyText(text('正在检查…', 'Checking…'));
    setActionError('');
    setNotice('');
    try {
      const checked = await invoke<boolean>('check_agent_authentication', {
        input: { agent_id: agent.agent_id, language, scope },
      });
      setNotice(
        !checked
          ? text('已取消检查。', 'Check cancelled.')
          : scope === 'collaboration'
            ? text('任务委派技能检查通过；未调用上游模型。', 'Task delegation skill check passed; no upstream model was called.')
            : text('本机认证兼容性检查通过；尚未验证上游模型调用。', 'Local authentication compatibility passed; upstream model access is not verified.'),
      );
      setPreview(null);
      await refresh();
    } catch (caught) {
      setActionError(desktopErrorCode(caught));
    } finally {
      setBusy(false);
      setBusyText('');
      requestAnimationFrame(() => source.isConnected && source.focus());
    }
  }

  function toggleAllowedPlan(planId: string) {
    setEditorValues(value => {
      const selected = value.allowedPlanIds.includes(planId);
      const allowedPlanIds = selected
        ? value.allowedPlanIds.filter(id => id !== planId)
        : [...value.allowedPlanIds, planId];
      const defaultChoice = selected
        && value.defaultChoice.kind === 'plan'
        && value.defaultChoice.plan_id === planId
        ? value.nativeModelMode === 'preserve_available'
          ? { kind: 'preserve_native' } as const
          : allowedPlanIds.length ? { kind: 'plan', plan_id: allowedPlanIds[0] } as const
            : value.fixedModels.length ? { kind: 'fixed_model', client_model_id: value.fixedModels[0].client_model_id } as const
              : { kind: 'preserve_native' } as const
        : !selected && value.nativeModelMode === 'hiroute_only' && value.defaultChoice.kind === 'preserve_native'
          ? { kind: 'plan', plan_id: planId } as const
          : value.defaultChoice;
      return { ...value, allowedPlanIds, defaultChoice };
    });
    setPreview(null);
    setActionError('');
    setNotice('');
  }

  function defaultFixedReasoning(reasoning: AgentNativeReasoning): AgentReasoningSelection | undefined {
    if (reasoning.kind === 'fixed') return undefined;
    if (reasoning.kind === 'toggle') return { kind: 'toggle', enabled: true };
    if (reasoning.kind === 'budget') return { kind: 'budget', tokens: reasoning.maximum_tokens };
    const enabled = reasoning.profiles.filter(profile => !['none', 'off', 'disabled'].includes(profile.toLocaleLowerCase()));
    const profile = enabled.at(-1) ?? reasoning.profiles.at(-1);
    return profile ? { kind: 'profile', profile } : undefined;
  }

  function selectFixedSource(model: AgentNativeModel, bindingId: string) {
    setEditorValues(value => {
      const fixedModels = value.fixedModels.filter(item => item.client_model_id !== model.client_model_id);
      const source = model.source_options?.find(option => option.binding_id === bindingId && option.state === 'ready');
      if (source) {
        fixedModels.push({
          client_model_id: model.client_model_id,
          candidate: {
            binding_id: source.binding_id,
            reasoning: defaultFixedReasoning(source.reasoning),
          },
        });
      }
      const defaultChoice = !source
        && value.defaultChoice.kind === 'fixed_model'
        && value.defaultChoice.client_model_id === model.client_model_id
        ? value.nativeModelMode === 'hiroute_only' && value.allowedPlanIds.length
          ? { kind: 'plan', plan_id: value.allowedPlanIds[0] } as const
          : { kind: 'preserve_native' } as const
        : source && value.nativeModelMode === 'hiroute_only' && value.defaultChoice.kind === 'preserve_native'
          ? { kind: 'fixed_model', client_model_id: model.client_model_id } as const
        : value.defaultChoice;
      return { ...value, fixedModels, defaultChoice };
    });
    setPreview(null);
    setActionError('');
    setNotice('');
  }

  function setFixedReasoning(clientModelId: string, reasoning: AgentReasoningSelection | undefined) {
    setEditorValues(value => ({
      ...value,
      fixedModels: value.fixedModels.map(model => model.client_model_id === clientModelId
        ? { ...model, candidate: { ...model.candidate, reasoning } }
        : model),
    }));
    setPreview(null);
    setActionError('');
    setNotice('');
  }

  function defaultChoiceValue(choice: AgentDefaultChoice): string {
    if (choice.kind === 'preserve_native') return 'native';
    return `${choice.kind === 'plan' ? 'plan' : 'fixed'}:${choice.kind === 'plan' ? choice.plan_id : choice.client_model_id}`;
  }

  function selectDefaultChoice(value: string) {
    const defaultChoice: AgentDefaultChoice = value === 'native'
      ? { kind: 'preserve_native' }
      : value.startsWith('plan:')
        ? { kind: 'plan', plan_id: value.slice(5) }
        : { kind: 'fixed_model', client_model_id: value.slice(6) };
    setEditorValues(current => ({ ...current, defaultChoice }));
    setPreview(null);
    setActionError('');
    setNotice('');
  }

  function selectNativeModelMode(mode: CodexNativeModelMode) {
    setEditorValues(value => {
      if (mode === value.nativeModelMode) return value;
      const fixedModels = mode === 'hiroute_only'
        ? value.fixedModels.filter(model => !protectedNativeModelIds.has(model.client_model_id))
        : value.fixedModels;
      const defaultChoice = mode === 'hiroute_only' && value.defaultChoice.kind === 'preserve_native'
        ? value.allowedPlanIds.length ? { kind: 'plan', plan_id: value.allowedPlanIds[0] } as const
          : fixedModels.length ? { kind: 'fixed_model', client_model_id: fixedModels[0].client_model_id } as const
            : value.defaultChoice
        : value.defaultChoice;
      return { ...value, nativeModelMode: mode, fixedModels, defaultChoice };
    });
    setPreview(null);
    setActionError('');
  }

  function selectClaudePreset(preset: keyof AgentClaudePresetMappings, planId: string) {
    setEditorValues(value => ({
      ...value,
      claudePresets: {
        ...value.claudePresets,
        [preset]: planId ? { kind: 'plan', plan_id: planId } : { kind: 'preserve_native' },
      },
    }));
    setPreview(null);
    setActionError('');
    setNotice('');
  }

  function setTriggerMode(triggerMode: AgentCollaborationTriggerMode) {
    setEditorValues(value => ({ ...value, triggerMode }));
    setPreview(null);
    setActionError('');
    setNotice('');
  }

  const plans = snapshot?.plans.plans ?? [];
  const enabledPlans = plans.filter(plan => plan.head.status === 'enabled');
  const selected = snapshot?.agents.find(agent => agent.agent_id === (selectedAgent ?? snapshot.agents[0]?.agent_id));
  const selectedCollaboration = selected?.settings?.collaboration;
  const modelSelection = selected?.settings?.current_selection;
  const protectedNativeModelIds = new Set(selected?.settings?.protected_native_model_ids ?? []);
  const taskSelection = selectedCollaboration?.current_selection;
  const needsRecovery = (state?: string | null) => state === 'drift' || state === 'needs_attention';
  const selectedHasDrift = needsRecovery(selected?.settings?.state) || needsRecovery(selectedCollaboration?.state);
  const selectedPending = selected?.settings?.state === 'pending' || selectedCollaboration?.state === 'pending';
  const mutable = mutationAllowed && Boolean(snapshot?.trusted_authority);
  const executableDiagnostic = (state: string) => {
    switch (state) {
      case 'not_found_in_scope':
        return {
          label: text('未发现安装', 'Not installed'),
          title: text('当前配置范围内未找到 Agent 入口', 'No Agent entry was found in this configuration scope'),
          detail: text('请确认 Agent 已安装，或重新扫描当前环境。', 'Confirm that the Agent is installed, or scan the current environment again.'),
        };
      case 'executable_not_runnable':
        return {
          label: text('命令不可执行', 'Command not executable'),
          title: text('已定位 Agent，但它当前不可执行', 'The Agent was located but is not executable'),
          detail: text('请检查目标是否为普通可执行文件。安装目录的组写权限本身不会阻止接入。', 'Check that the target is a regular executable file. Group-writable installation directories do not block routing.'),
        };
      case 'executable_probe_timed_out':
        return {
          label: text('检查超时', 'Check timed out'),
          title: text('Agent 版本检查超时', 'The Agent version check timed out'),
          detail: text('探测进程已回收；可以重新扫描后再进行真实验证。', 'The probe process was cleaned up. Scan again before live verification.'),
        };
      case 'executable_probe_unavailable':
        return {
          label: text('命令检查失败', 'Command check failed'),
          title: text('Agent 命令暂时无法检查', 'The Agent command could not be checked'),
          detail: text('路径解析或启动失败；这不表示安装来源不受信任。', 'Path resolution or launch failed. This does not mean the installation source is untrusted.'),
        };
      default:
        return null;
    }
  };
  const selectedExecutableDiagnostic = selected
    ? executableDiagnostic(selected.configuration_state)
    : null;
  const disableRestoreAvailable = editor?.facet === 'model'
    ? Boolean(selected?.settings?.restore_point_ref)
    : Boolean(selected?.settings?.collaboration?.restore_point_ref);
  const selectedPlansEnabled = editorValues.allowedPlanIds.every(id => enabledPlans.some(plan => plan.agent_plan_id === id));
  const defaultChoice = editorValues.defaultChoice;
  const detectedCodexSurfaces = new Set<AgentModelSurface>((selected?.available_surfaces ?? [])
    .filter(surface => surface === 'codex_cli' || surface === 'codex_desktop'));
  const catalogModels = (selected?.native_model_catalog?.models ?? [])
    .map(model => ({ ...model, source_options: model.source_options ?? [] }));
  const nativeRestoreOptions = catalogModels.filter(model =>
    !plans.some(plan => plan.model_alias === model.client_model_id));
  const fixedModelRows = editorValues.fixedModels.map(fixed => catalogModels
    .find(model => model.client_model_id === fixed.client_model_id) ?? {
      client_model_id: fixed.client_model_id,
      display_name: fixed.client_model_id,
      source_options: [],
    });
  const selectedFixedSourcesValid = editorValues.fixedModels.every(fixed => {
    if (protectedNativeModelIds.has(fixed.client_model_id)) return true;
    const source = catalogModels
      .find(model => model.client_model_id === fixed.client_model_id)
      ?.source_options.find(option => option.binding_id === fixed.candidate.binding_id);
    if (!source || source.state !== 'ready') return false;
    const selection = fixed.candidate.reasoning;
    if (source.reasoning.kind === 'fixed') return selection === undefined;
    if (source.reasoning.kind === 'toggle') return selection?.kind === 'toggle';
    if (source.reasoning.kind === 'discrete') {
      return selection?.kind === 'profile' && source.reasoning.profiles.includes(selection.profile);
    }
    return selection?.kind === 'budget'
      && selection.tokens >= source.reasoning.minimum_tokens
      && selection.tokens <= source.reasoning.maximum_tokens
      && (selection.tokens - source.reasoning.minimum_tokens) % source.reasoning.step_tokens === 0;
  });
  const nativeDefault = selected?.native_model_catalog?.native_default_model;
  const codexDefaultValid = codexDefaultChoiceValid(
    defaultChoice,
    editorValues.nativeModelMode,
    nativeDefault,
    editorValues.fixedModels,
    editorValues.allowedPlanIds,
  );
  const claudePlanIds = Object.values(editorValues.claudePresets)
    .flatMap(choice => choice.kind === 'plan' ? [choice.plan_id] : []);
  const modelFormInvalid = selected?.agent_id === 'agent_codex_default'
    ? editorValues.allowedPlanIds.length + editorValues.fixedModels.length === 0
      || !selectedFixedSourcesValid
      || !selectedPlansEnabled
      || !codexDefaultValid
    : claudePlanIds.length + editorValues.fixedModels.length === 0
      || claudePlanIds.some(id => !enabledPlans.some(plan => plan.agent_plan_id === id));
  const formInvalid = facetEnabled
    ? !selectionKnown || (editor?.facet === 'model' && modelFormInvalid)
    : !disableRestoreAvailable;

  const brand = (agent: Agent) => agent.agent_id === 'agent_codex_default' ? 'codex' : agent.agent_id === 'agent_claude_default' ? 'claude-code' : 'agent';
  const planName = (id: string) => plans.find(plan => plan.agent_plan_id === id)?.desired.display_name;
  const planEnabled = (id: string) => enabledPlans.some(plan => plan.agent_plan_id === id);
  const surfaceName = (surface: AgentModelSurface) => surface === 'codex_desktop'
    ? 'Codex Desktop'
    : surface === 'codex_cli'
      ? 'Codex CLI'
      : 'Claude Code CLI';
  const sourceStateLabel = (state: AgentModelSourceCoverage['state']) => state === 'ready'
    ? text('来源与账号已覆盖', 'Source and account covered')
    : state === 'credential_required'
      ? text('需要凭据', 'Credential required')
      : state === 'authorization_required'
        ? text('需要账号授权', 'Account authorization required')
        : state === 'disabled'
          ? text('来源已停用', 'Source disabled')
          : text('模型目录匹配未证明', 'Model directory match unproven');
  const fixedReasoningControl = (
    model: AgentNativeModel,
    fixed: AgentFixedModel,
    source: AgentModelSourceCoverage,
  ) => {
    const reasoning = source.reasoning;
    const selection = fixed.candidate.reasoning;
    if (reasoning.kind === 'fixed') {
      return <span className="field-help">{text('原生思考设置：', 'Native reasoning: ')}{reasoning.profile}</span>;
    }
    if (reasoning.kind === 'discrete') {
      return <label className="field"><span className="field-label">{text('思考强度', 'Reasoning effort')}</span><select className="select" value={selection?.kind === 'profile' ? selection.profile : ''} onChange={event => setFixedReasoning(model.client_model_id, { kind: 'profile', profile: event.target.value })}>{reasoning.profiles.map(profile => <option key={profile} value={profile}>{profile}</option>)}</select></label>;
    }
    if (reasoning.kind === 'toggle') {
      return <label className="field"><span className="field-label">{text('思考', 'Reasoning')}</span><select className="select" value={selection?.kind === 'toggle' && selection.enabled ? 'on' : 'off'} onChange={event => setFixedReasoning(model.client_model_id, { kind: 'toggle', enabled: event.target.value === 'on' })}><option value="on">{text('开启', 'On')}</option><option value="off">{text('关闭', 'Off')}</option></select></label>;
    }
    return <label className="field"><span className="field-label">{text('思考预算', 'Reasoning budget')}</span><input className="input" type="number" min={reasoning.minimum_tokens} max={reasoning.maximum_tokens} step={reasoning.step_tokens} value={selection?.kind === 'budget' ? selection.tokens : ''} onChange={event => setFixedReasoning(model.client_model_id, { kind: 'budget', tokens: Number(event.target.value) })} /><span className="field-help">{reasoning.minimum_tokens}–{reasoning.maximum_tokens} tokens</span></label>;
  };
  const modelSelectionSummary = modelSelection?.mode === 'codex_default'
    ? modelSelection.default_selection.kind === 'plan'
      ? planName(modelSelection.default_selection.plan_id) ?? text('当前路由不可用', 'Current route unavailable')
      : modelSelection.default_selection.kind === 'fixed_model'
        ? modelSelection.default_selection.client_model_id
        : text('使用 Codex 当前默认模型名称', 'Use the current Codex default model name')
    : modelSelection
      ? text('Opus、Sonnet、Haiku 分别映射', 'Separate Opus, Sonnet, and Haiku mappings')
      : '';
  const blockedCheckScope = preview && editor && selected
    ? prerequisiteCheck(preview, editor.facet, selected)
    : null;
  const blockedPreview = preview && !preview.applicable ? <div className="callout warn agent-feedback" role="status">
    <UiIcon name="warning" />
    <div>
    <strong>{text('当前无法应用，未修改配置。', 'Cannot apply yet. Configuration was not changed.')}</strong>
    {preview.blockers.map((block, index) => {
      const capabilities = new Set(block.capabilities?.map(capability => capability.capability) ?? []);
      const message = capabilities.has('ingress_authentication')
        ? text('本机认证兼容性尚未确认。', 'Local authentication compatibility is unconfirmed.')
        : block.reason === 'native_model_coverage_unavailable'
          ? `${text('保留原 Codex 模型需要先接入原订阅账号或 API 来源，并证明同账号路由。也可改选“只使用已配置的 HiRoute 模型”。', 'To keep native Codex models, connect the original subscription account or API source and prove same-account routes, or select HiRoute-only.')}${(block.model_ids ?? []).length ? ` ${block.model_ids!.join(', ')}` : ''}`
        : block.reason === 'native_default_invalid'
          ? `${text('原 Codex 默认模型没有同账号可用路由；请接入对应来源，或改选“只使用已配置的 HiRoute 模型”：', 'The native Codex default has no proven same-account route. Connect its source or select HiRoute-only: ')}${(block.model_ids ?? []).join(', ')}`
        : block.reason === 'restore_native_model_invalid'
          ? text('恢复将留下无效的原生默认模型。请在“连接详情与恢复”中明确选择一个原生模型后重试。', 'Restoring would leave an invalid native default. Choose a native model under Connection details and recovery, then retry.')
        : block.reason === 'model_plan_unavailable' && selected?.agent_id === 'agent_codex_default' && defaultChoice.kind === 'preserve_native'
          ? text('当前默认模型名称或所选路由无法用于 Codex Responses。请在“默认选择”中改选已勾选且支持该入口的路由；若要保留原生模型，请先接入对应来源。配置未修改。', 'The current default model name or a selected route cannot be used by Codex Responses. Under Default selection, choose an enabled route that supports this ingress; to preserve a native model, connect its source first. Configuration was not changed.')
        : block.reason === 'model_plan_unavailable' && selected?.agent_id === 'agent_codex_default'
          ? text('所选路由、固定来源或当前默认模型无法用于 Codex Responses。请核对路由是否已启用并支持该入口协议，或改选可用路由；配置未修改。', 'A selected route, fixed source, or current default model cannot be used by Codex Responses. Check that the route is enabled and supports this ingress protocol, or choose an available route. Configuration was not changed.')
        : block.reason === 'skill_file_conflict'
          ? text('同名任务委派技能内容不同；原文件已保留。请先移走或明确处理该文件后再预览。', 'A task delegation skill with different content already exists. The original was preserved; move or explicitly resolve it before previewing again.')
        : capabilities.has('skill_loading') || capabilities.has('trusted_cli_execution')
          ? text('任务委派技能尚未确认。', 'Task delegation skill capability is unconfirmed.')
          : text('有前置条件尚未满足，请检查当前 Agent 状态。', 'A prerequisite is unmet. Check the current Agent state.');
      return <p key={index}>{message}</p>;
    })}
    {preview.unproven_native_model_ids?.length ? <p>{text('目录中尚未证明可由原账号调用的模型（不会开放给 Codex）：', 'Catalog names not proven callable on the original account (not exposed to Codex): ')}{preview.unproven_native_model_ids.join(', ')}</p> : null}
    {blockedCheckScope && selected && <button className="btn" type="button" disabled={busy || !mutable} onClick={event => void check(selected, blockedCheckScope, event.currentTarget)}>{blockedCheckScope === 'native_authentication' ? text('重新检查本机认证', 'Check local authentication again') : text('重新检查任务委派技能', 'Check task delegation skill again')}</button>}
    </div>
  </div> : null;
  const executableStatus = selectedExecutableDiagnostic ? <div className="callout warn agent-feedback" data-agent-executable-state={selected?.configuration_state} role="status">
    <UiIcon name="warning" />
    <div>
      <strong>{selectedExecutableDiagnostic.title}</strong>
      <p>{selectedExecutableDiagnostic.detail}</p>
      <button className="btn" type="button" onClick={() => void refresh()}>{text('重新扫描', 'Scan again')}</button>
    </div>
  </div> : null;

  return <ProductPage title="Agent" subtitle={text('在你常用的 Agent 中使用智能路由', 'Use smart routing in your usual Agent')} flush className="agents-page">
    <div className="agents-workspace">
      <div className="agent-tabs segmented" role="tablist">
        <button className={`segment${tab === 'configuration' ? ' active' : ''}`} type="button" role="tab" aria-selected={tab === 'configuration'} onClick={() => { setTab('configuration'); setActionError(''); setNotice(''); }}>{text('我的 Agent', 'My Agents')}</button>
        <button className={`segment${tab === 'tasks' ? ' active' : ''}`} type="button" role="tab" aria-selected={tab === 'tasks'} onClick={() => { setTab('tasks'); setActionError(''); setNotice(''); onOpenTasks?.(); }}>{text('任务记录', 'Task history')}</button>
      </div>
      {!snapshot && !loadError && <div className="empty-state" role="status"><div><span className="oc-spinner" /><p>{text('正在读取本地 Agent…', 'Reading local Agents…')}</p></div></div>}
      {!snapshot && loadError && <div className="empty-state" data-error-code={loadError}><div><span className="empty-icon"><UiIcon name="agent" /></span><h2>{text('暂时无法读取 Agent', 'Agents are temporarily unavailable')}</h2><p>{text('请确认本机服务正在运行，然后重试。', 'Make sure the local service is running, then retry.')}</p><button className="btn btn-primary" type="button" onClick={() => void refresh()}>{text('重试', 'Retry')}</button></div></div>}
      {tab === 'tasks'
        ? <AgentTasks language={language} read={taskRead} initialTaskId={initialTaskId} onCancel={onTaskCancel} onOpenSession={onOpenTaskSession} onBackToAgents={() => setTab('configuration')} onRefresh={onOpenTasks} onLoadMore={onLoadMoreTasks} onReadTask={onReadTask} onLoadTaskResult={onLoadTaskResult} active={active} />
        : snapshot && !snapshot.agents.length
          ? <div className="empty-state"><div><span className="empty-icon"><UiIcon name="agent" /></span><h2>{text('没有发现可接入的 Agent', 'No compatible Agent found')}</h2><p>{text('安装或启动支持的 Agent 后再返回此页。', 'Install or start a supported Agent, then return here.')}</p><button className="btn" type="button" onClick={() => void refresh()}>{text('重新扫描', 'Scan again')}</button></div></div>
          : tab === 'configuration' && <div className={`split-view oc-agents${showAgentList ? '' : ' oc-agent-detail-open'}`}>
            <aside className="master-pane">
              <nav className="master-list native-list" aria-label={text('Agent 列表', 'Agent list')}>
                {snapshot?.agents.map(agent => {
                  const active = (selectedAgent ?? snapshot.agents[0]?.agent_id) === agent.agent_id;
                  const stateNeedsRecovery = needsRecovery(agent.settings?.state) || needsRecovery(agent.settings?.collaboration?.state);
                  const statePending = agent.settings?.state === 'pending' || agent.settings?.collaboration?.state === 'pending';
                  const executableState = executableDiagnostic(agent.configuration_state);
                  const stateLabel = agent.status_error
                    ? text('状态待确认', 'Status unverified')
                    : executableState
                      ? executableState.label
                      : stateNeedsRecovery
                      ? text('接入需要处理', 'Connection needs attention')
                      : statePending
                        ? text('正在更新', 'Updating')
                        : agent.settings?.current_selection
                          ? agent.settings.model_verified
                            ? text('路由已验证', 'Routing verified')
                            : text('路由已配置 · 调用未验证', 'Routing configured · call unverified')
                          : agent.settings?.collaboration?.current_selection
                            ? text('委派技能已启用', 'Delegation skill enabled')
                          : text('已发现 · 尚未接入', 'Detected · not connected');
                  return <button className={`list-row${active ? ' active' : ''}`} key={agent.agent_id} aria-current={active ? 'page' : undefined} onClick={async () => { if (dirty && !(await confirmDiscard(language))) return; setEditor(null); setPreview(null); setActionError(''); setNotice(''); setTokenEditing(false); setTokenDraft(''); setSelectedAgent(agent.agent_id); setShowAgentList(false); }}>
                    <BrandIcon kind={brand(agent)} label={`${agentName(agent)} logo`} />
                    <span className="row-main"><span className="row-title">{agentName(agent)}</span><span className="row-meta">{stateLabel}</span></span>
                  </button>;
                })}
              </nav>
            </aside>
            <section className="detail-pane">
              <div className="oc-model-back"><button className="btn btn-quiet" type="button" onClick={() => setShowAgentList(true)}><UiIcon name="arrowLeft" />{text('返回 Agent 列表', 'Back to Agents')}</button></div>
              {selected && snapshot && <div className="detail-inner" data-agent-id={selected.agent_id}>
                <header className="detail-hero"><div className="detail-identity"><BrandIcon kind={brand(selected)} label={`${agentName(selected)} logo`} /><div><h2>{agentName(selected)}</h2><p>{text('选择你需要的路由能力', 'Choose the routing capabilities you need')}</p></div></div></header>
                {!mutable && <div className="callout warn"><UiIcon name="warning" /><div><strong>{text('Agent 配置可以查看，暂时不能修改', 'Agent settings are viewable but cannot be changed')}</strong><p>{text('本机服务尚未就绪，连接恢复后即可保存配置。', 'The local service is not ready. Reconnect before saving.')}</p></div></div>}
                {selected.status_error && <div className="callout warn" data-error-code={selected.status_error}><UiIcon name="warning" /><div><strong>{text('暂时无法核实 Agent 状态', 'Agent status is temporarily unavailable')}</strong><p>{text('当前仍显示已发现的 Agent；重新读取成功前不会把未知状态当作已接入。', 'The detected Agent remains visible. Unknown status is not treated as connected until it can be read again.')}</p><button className="btn" type="button" onClick={() => void refresh()}>{text('重新读取', 'Try again')}</button></div></div>}
                {executableStatus}
                {selectedHasDrift && <div className="callout warn"><UiIcon name="warning" /><div><strong>{text('接入配置已变化', 'Connection settings changed')}</strong><p>{text('HiRoute 不会覆盖已变化的设置。请在“连接详情与恢复”中恢复需要的路由配置。', 'HiRoute will not overwrite changed settings. Restore the required routing configuration under Connection details and recovery.')}</p></div></div>}
                {selectedPending && <div className="oc-status-row" role="status"><span className="oc-spinner" /><p>{text('路由配置正在应用，完成后会自动更新。', 'Routing settings are being applied and will update automatically.')}</p></div>}
                <section className="detail-section v3-agent-section">
                  <div className="detail-section-head"><h3>{text('模型路由', 'Model routing')}</h3><button className="btn btn-quiet" data-agent-facet="model" disabled={busy || !selected.context_id || !mutable} onClick={event => open(selected, 'model', event.currentTarget)}>{modelSelection ? text('调整', 'Edit') : text('启用', 'Enable')}</button></div>
                  <p>{modelSelection ? <>{text('当前选择：', 'Current selection: ')}<strong>{modelSelectionSummary}</strong></> : text('按客户端原生语义选择固定模型或智能路由；保存配置本身不会伪装成模型验证。', 'Choose fixed models or smart routes using the client’s native semantics. Saving configuration is not treated as model verification.')}</p>
                  {modelSelection && (selected.available_surfaces ?? []).length > 0 && <div className="agent-surface-results" aria-label={text('当前客户端验证结果', 'Current client verification results')}>
                    {selected.available_surfaces?.map(surface => {
                      const revision = selected.settings?.applied_revision;
                      const result = selected.settings?.surface_results?.find(item => item.surface === surface && item.applied_revision === revision);
                      const state = selected.settings?.state === 'configured' && !selected.status_error && revision != null
                        ? result?.state ?? 'not_verified'
                        : 'not_verified';
                      return <div className="agent-surface-result" key={surface} data-agent-surface={surface}>
                        <span>{surfaceName(surface)} · {revision == null ? text('修订待确认', 'Revision unknown') : `r${revision}`}</span>
                        <span className={`badge no-dot ${state === 'passed' ? 'good' : state === 'failed' ? 'bad' : 'warn'}`}>{state === 'passed'
                          ? text('已验证', 'Verified')
                          : state === 'failed'
                            ? text('验证未通过', 'Verification failed')
                            : text('尚未验证', 'Not verified')}</span>
                      </div>;
                    })}
                  </div>}
                  {modelSelection?.mode === 'claude_launcher' && <p className="field-help">{text('直接启动 claude，使用 Opus、Sonnet、Haiku 原生预设选择已映射路由；其他计划不会自动出现在模型菜单中。若未显式选择当前模型，Claude Default 取决于账号；预设映射不证明 Default 可用，请通过真实调用验证。', 'Start claude normally and use its native Opus, Sonnet, or Haiku preset for mapped routes. Other plans do not automatically appear in the model picker. Without an explicit model selection, Claude Default depends on the account; preset mappings do not prove it works. Verify with a live call.')}</p>}
                </section>
                <section className="detail-section v3-agent-section">
                  <div className="detail-section-head"><h3>{text('任务委派技能', 'Task delegation skill')}</h3><button className="btn btn-quiet" data-agent-facet="collaboration" disabled={busy || !selected.context_id || !mutable} onClick={event => open(selected, 'collaboration', event.currentTarget)}>{taskSelection ? text('调整', 'Edit') : text('启用', 'Enable')}</button></div>
                  <p>{taskSelection?.trigger_mode === 'delegate_by_default'
                    ? text('默认允许 Agent 判断何时委派；你仍可明确要求只回答、不执行。', 'The Agent may delegate by default; you can still explicitly ask it to answer without execution.')
                    : taskSelection
                      ? text('仅在你明确要求执行或委派任务时使用。', 'Used only when you explicitly ask the Agent to execute or delegate a task.')
                      : text('启用后，Agent 可以从当前可用路由中选择执行 Agent；不维护第二套路由名单。', 'When enabled, the Agent can choose from currently available routes; there is no second route allowlist.')}</p>
                </section>
                <Disclosure className="native-details" label={text('连接详情与恢复', 'Connection details and recovery')} language={language}><dl>{selected.version && <div><dt>{text('版本', 'Version')}</dt><dd>{selected.version}</dd></div>}{selected.settings?.state === 'configured' && modelSelection && <div><dt>{text('本机接入令牌', 'Local access token')}</dt><dd><input className="input" type="password" value="••••••••••••••••" readOnly aria-label={text('当前令牌已隐藏', 'Current token hidden')} /></dd></div>}</dl>{selected.settings?.restore_point_ref && selected.agent_id === 'agent_codex_default' && <label className="field-label">{text('恢复时选择原生模型（可选）', 'Native model on restore (optional)')}<input list="codex-native-restore-models" value={restoreNativeModel} onChange={event => { setRestoreNativeModel(event.target.value); setPreview(null); }} placeholder={text('保留原设置', 'Keep original setting')} disabled={busy || !mutable} /><datalist id="codex-native-restore-models">{nativeRestoreOptions.map(model => <option key={model.client_model_id} value={model.client_model_id}>{model.display_name}</option>)}</datalist></label>}{selected.settings?.state === 'configured' && modelSelection && <div className="actions" data-agent-token-controls><button className="btn" disabled={busy || !mutable} onClick={() => { setTokenEditing(true); setTokenDraft(''); setActionError(''); }}>{text('修改令牌', 'Change token')}</button><button className="btn" disabled={busy || !mutable} onClick={event => void changeToken(selected, true, event.currentTarget)}>{text('重新生成', 'Regenerate')}</button></div>}{tokenEditing && selected.settings?.state === 'configured' && modelSelection && <form className="agent-token-form" onSubmit={event => { event.preventDefault(); void changeToken(selected, false, (event.nativeEvent as SubmitEvent).submitter as HTMLElement | null); }}><label className="field"><span className="field-label">{text('新令牌', 'New token')}</span><input className="input" type="password" autoComplete="new-password" value={tokenDraft} onChange={event => setTokenDraft(event.target.value)} minLength={16} maxLength={128} pattern="[A-Za-z0-9._~\\-]{16,128}" required disabled={busy} /><span className="field-help">{text('16–128 位英文字母、数字及 . _ ~ -；保存后原令牌立即失效，无需重新接入。', 'Use 16–128 letters, numbers, or . _ ~ -. The old token stops working after save; reconnecting is unnecessary.')}</span></label><div className="actions"><button className="btn btn-primary" type="submit" disabled={busy || !mutable}>{text('保存令牌', 'Save token')}</button><button className="btn" type="button" disabled={busy} onClick={() => { setTokenEditing(false); setTokenDraft(''); }}>{text('取消', 'Cancel')}</button></div></form>}<div className="actions">{selected.settings?.restore_point_ref && <button className="btn" disabled={busy || !mutable} onClick={event => void change(selected, 'model', true, event.currentTarget)}>{text('恢复模型设置', 'Restore model settings')}</button>}{selectedCollaboration?.restore_point_ref && <button className="btn" disabled={busy || !mutable} onClick={event => void change(selected, 'collaboration', true, event.currentTarget)}>{text('停用任务委派技能', 'Disable task delegation skill')}</button>}</div></Disclosure>
                {!editor && loadError && <div className="callout warn agent-feedback agent-inline-feedback" role="alert" data-error-code={loadError}><UiIcon name="warning" /><div><strong>{text('状态刷新失败', 'Status refresh failed')}</strong><p>{text('当前仍显示上次读取的结果。', 'The last loaded result is still shown.')}</p><button className="btn" type="button" onClick={() => void refresh()}>{text('重试', 'Retry')}</button></div></div>}
                {!editor && blockedPreview}
                {!editor && notice && <div className="toast-stack" aria-live="polite"><div className="toast" role="status"><UiIcon name="check" /><span>{notice}</span></div></div>}
                {!editor && actionError && <div className="callout bad agent-feedback agent-inline-feedback" role="alert" data-error-code={actionError}><UiIcon name="warning" /><span>{agentActionErrorMessage(actionError, language)}</span></div>}
                {!selected.context_id && <div className="callout"><UiIcon name="info" /><span>{text('此 Agent 的设置入口尚不可用。', 'Settings for this Agent are not available yet.')}</span></div>}
                {editor?.context === selected.context_id && selected.context_id && <Dialog open title={`${agentName(selected)} · ${editor.facet === 'model' ? text('模型路由', 'Model routing') : text('任务委派技能', 'Task delegation skill')}`} closeLabel={text('关闭配置', 'Close settings')} closeDisabled={busy} onClose={() => { if (!busy) close(); }} footer={<><button className="btn" type="button" disabled={busy} onClick={close}>{text('取消', 'Cancel')}</button><button className="btn btn-primary" type="submit" form="hr-agent-settings" disabled={busy || !mutable || Boolean(formInvalid)}>{busy ? busyText || text('正在保存…', 'Saving…') : text('保存配置', 'Save settings')}</button></>}>
                  <form id="hr-agent-settings" className="agent-form" onSubmit={event => { event.preventDefault(); void change(selected, editor.facet, !facetEnabled, (event.nativeEvent as SubmitEvent).submitter as HTMLElement | null); }}>
                    <fieldset disabled={busy || !mutable}>
                      <div className="option-row agent-facet-toggle"><div><strong>{editor.facet === 'model' ? text('使用 HiRoute 模型路由', 'Use HiRoute model routing') : text('启用任务委派技能', 'Enable task delegation skill')}</strong><span>{editor.facet === 'model' ? text('关闭后使用原有模型接入', 'Use the original model connection when disabled') : text('允许当前 Agent 在需要时调用执行 Agent', 'Allow this Agent to invoke an execution Agent when needed')}</span></div><button ref={node => { firstField.current = node; }} className={`switch${facetEnabled ? ' on' : ''}`} type="button" role="switch" aria-checked={facetEnabled} aria-label={editor.facet === 'model' ? text('模型路由', 'Model routing') : text('任务委派技能', 'Task delegation skill')} onClick={() => { setFacetEnabled(value => !value); setPreview(null); setActionError(''); setNotice(''); }} /></div>
                      {facetEnabled && editor.facet === 'model' && <p className="field-help" data-agent-service-responsibility>{text('保存不会设置登录项或保证 HiRoute 服务以后持续在线；需要路由时请保持本机服务运行。', 'Saving does not configure a login item or guarantee that the HiRoute service stays online; keep the local service running when you need routing.')}</p>}
                      {facetEnabled && editor.facet === 'model' && selected.agent_id === 'agent_codex_default' && <div className="worker-choices">
                        <fieldset data-agent-native-mode><legend className="field-label">{text('Codex 可用模型', 'Models available in Codex')}</legend>
                          <label className="check-row"><input type="radio" name="codex-native-mode" value="hiroute_only" checked={editorValues.nativeModelMode === 'hiroute_only'} onChange={() => selectNativeModelMode('hiroute_only')} /><div><strong>{text('只使用已配置的 HiRoute 模型', 'Use configured HiRoute models only')}</strong><small>{text('无需接入原 Codex 账号；关闭时恢复原配置。', 'No original Codex account connection required; disabling restores the original configuration.')}</small></div></label>
                          <label className="check-row"><input type="radio" name="codex-native-mode" value="preserve_available" checked={editorValues.nativeModelMode === 'preserve_available'} onChange={() => selectNativeModelMode('preserve_available')} /><div><strong>{text('同时保留 Codex 原有模型', 'Also keep native Codex models')}</strong><small>{text('先在“模型”接入原订阅账号或 API 来源；只保留能证明同账号可调用的模型。缓存目录不是账号权限。', 'Connect the original subscription account or API source under Models first. Only proven same-account models are kept; the cache is not entitlement.')}</small></div></label>
                        </fieldset>
                        <div className="callout" data-codex-shared-scope><UiIcon name="info" /><div><strong>{text('一套共享配置', 'One shared configuration')}</strong><p>{text('保存一次即作用于共享此配置作用域与 CODEX_HOME 的 Codex CLI 和 Desktop。下面的发现结果是当前事实，不是配置范围开关。', 'One save applies to Codex CLI and Desktop when they share this configuration scope and CODEX_HOME. Detection below is a current fact, not a configuration-scope switch.')}</p>{(['codex_desktop', 'codex_cli'] as const).map(surface => {
                          const detected = detectedCodexSurfaces.has(surface);
                          return <span className="field-help" key={surface} data-agent-surface-fact={surface} data-agent-surface-detected={detected ? 'true' : 'false'}>{surfaceName(surface)} · {detected ? text('当前已发现可执行入口', 'currently detected as runnable') : text('当前未发现可执行入口；不影响保存', 'not currently detected as runnable; saving is unaffected')}</span>;
                        })}</div></div>
                        {fixedModelRows.length > 0 && <fieldset data-agent-fixed-models><legend className="field-label">{text('已配置的原生模型固定来源', 'Configured native model sources')}</legend>
                          {fixedModelRows.map(model => {
                            const fixed = editorValues.fixedModels.find(item => item.client_model_id === model.client_model_id);
                            const source = fixed ? model.source_options.find(option => option.binding_id === fixed.candidate.binding_id) : undefined;
                            return <div className="candidate-row" key={model.client_model_id} data-client-model-id={model.client_model_id}>
                              <div><strong>{model.display_name}</strong><small><code>{model.client_model_id}</code></small>{protectedNativeModelIds.has(model.client_model_id) ? <span className="field-help" data-protected-native-model>{text('原生模型绑定由 HiRoute 自动保留；调整路由时无需重新选择。停用模型路由后可重新配置原生来源。', 'HiRoute keeps this native model binding automatically when routes are edited. Disable model routing to change its native source.')}</span> : null}<label className="field"><span className="field-label">{text('固定来源与账号范围', 'Fixed source and account scope')}</span><select className="select" value={fixed?.candidate.binding_id ?? ''} onChange={event => selectFixedSource(model, event.target.value)} disabled={protectedNativeModelIds.has(model.client_model_id)}>{!protectedNativeModelIds.has(model.client_model_id) && <option value="">{text('不通过 HiRoute 固定此名称', 'Do not fix this name through HiRoute')}</option>}{fixed && !source && <option value={fixed.candidate.binding_id} disabled>{protectedNativeModelIds.has(model.client_model_id) ? text('原有绑定已保留，当前列表未显示来源', 'Binding retained; source is not in the current list') : text('原有来源当前不可用', 'Previous source unavailable')}</option>}{model.source_options.map(option => <option key={option.binding_id} value={option.binding_id} disabled={option.state !== 'ready'}>{option.source_label} · {option.account_scope_ref} · {sourceStateLabel(option.state)}</option>)}</select></label>{source && <span className="field-help">{sourceStateLabel(source.state)} · {text('账号范围摘要 ', 'Account scope digest ')}<code>{source.account_scope_digest.slice(0, 18)}…</code></span>}{fixed && source?.state === 'ready' && !protectedNativeModelIds.has(model.client_model_id) && fixedReasoningControl(model, fixed, source)}{!model.source_options.length && !protectedNativeModelIds.has(model.client_model_id) && <span className="field-help">{text('此原生名称尚无独立来源/账号绑定；元数据不会被当作可调用证明。', 'This native name has no independent source/account binding; metadata is not treated as callable proof.')}</span>}</div>
                            </div>;
                          })}
                        </fieldset>}
                        <fieldset><legend className="field-label">{text('允许的智能路由', 'Allowed smart routes')}</legend>
                          <p className="field-help" data-agent-plan-compatibility>{text('Codex 使用 Codex Responses 入口。上游来源名称不决定路由能否接入；保存时会按当前发布核对所选路由。', 'Codex uses the Codex Responses ingress. An upstream source name does not determine route compatibility; saving checks selected routes against the current publication.')}</p>
                          {enabledPlans.map(plan => <label className="check-row" data-agent-plan-id={plan.agent_plan_id} key={plan.agent_plan_id}><input type="checkbox" checked={editorValues.allowedPlanIds.includes(plan.agent_plan_id)} onChange={() => toggleAllowedPlan(plan.agent_plan_id)} /><div><strong>{plan.desired.display_name}</strong></div></label>)}
                          {!enabledPlans.length && <div className="callout"><UiIcon name="route" /><div><span>{text('先创建并启用一条智能路由。', 'Create and enable a smart route first.')}</span></div>{onCreatePlan && <button className="btn" type="button" onClick={createPlanFromEditor}>{text('创建路由', 'Create route')}</button>}</div>}
                          {editorValues.allowedPlanIds.some(id => !planEnabled(id)) && <span className="oc-inline-error">{text('已有路由已停用或不存在，请取消选择后再保存。', 'A previously allowed route is disabled or missing. Remove it before saving.')}</span>}
                        </fieldset>
                        <label className="field"><span className="field-label">{text('默认选择', 'Default selection')}</span><select className="select" data-agent-default value={defaultChoiceValue(editorValues.defaultChoice)} onChange={event => selectDefaultChoice(event.target.value)}>{editorValues.nativeModelMode === 'preserve_available' && <option value="native">{text('使用 Codex 当前默认模型名称', 'Use the current Codex default model name')}</option>}{editorValues.fixedModels.map(model => <option key={model.client_model_id} value={`fixed:${model.client_model_id}`}>{model.client_model_id}</option>)}{editorValues.allowedPlanIds.filter(planEnabled).map(id => <option key={id} value={`plan:${id}`}>{planName(id)}</option>)}</select><span className="field-help">{defaultChoice.kind === 'preserve_native'
                          ? text('保留当前默认模型名称；请求仍经过 HiRoute。该名称须对应已勾选的路由或已绑定的固定模型。', 'Keep the current default model name; requests still pass through HiRoute. The name must match an enabled route or a bound fixed model.')
                          : defaultChoice.kind === 'plan'
                            ? editorValues.nativeModelMode === 'preserve_available' ? text('Codex 将默认使用所选智能路由；已证明可用的原生模型名称会继续自动保留。', 'Codex will use the selected smart route by default; proven native model names remain available automatically.') : text('Codex 将默认使用所选智能路由。', 'Codex will use the selected smart route by default.')
                            : text('Codex 将默认使用所选固定模型来源。', 'Codex will use the selected fixed model source by default.')}</span>{defaultChoice.kind === 'preserve_native' && !codexDefaultValid && <span className="oc-inline-error">{text('当前无法读取 Codex 原生默认模型；请修复当前配置后重试。', 'The native Codex default model cannot be read. Repair the current configuration and try again.')}</span>}</label>
                        {modelFormInvalid && <span className="oc-inline-error">{text('至少选择一项固定模型或智能路由，并修正不可用项。', 'Choose at least one fixed model or smart route and resolve unavailable selections.')}</span>}
                      </div>}
                      {facetEnabled && editor.facet === 'model' && selected.agent_id === 'agent_claude_default' && <div className="worker-choices">
                        <p className="field-help">{text('普通 claude 入口会读取保存的三个原生预设映射；可重复选择同一路由。若未显式设置当前模型，账号 Default 仍未知；保存不会验证它，请选择已映射预设并做真实调用。', 'The normal claude entry reads the three saved native preset mappings, and routes may be reused. Without an explicit current model, the account Default remains unknown; saving does not verify it. Select a mapped preset and make a live call.')}</p>
                        {(['opus', 'sonnet', 'haiku'] as const).map(preset => {
                          const choice = editorValues.claudePresets[preset];
                          const selectedPlan = choice.kind === 'plan' ? choice.plan_id : '';
                          return <label className="field" key={preset}><span className="field-label">{preset[0].toUpperCase() + preset.slice(1)}</span><select className="select" value={selectedPlan} onChange={event => selectClaudePreset(preset, event.target.value)}><option value="">{text('保留原生值或缺省关系', 'Preserve native value or default relationship')}</option>{selectedPlan && !planEnabled(selectedPlan) && <option value={selectedPlan} disabled>{text('原有路由当前不可用', 'Previous route unavailable')}</option>}{enabledPlans.map(plan => <option key={plan.agent_plan_id} value={plan.agent_plan_id}>{plan.desired.display_name}</option>)}</select></label>;
                        })}
                        {modelFormInvalid && <span className="oc-inline-error">{text('至少将一个预设映射到可用智能路由，或保留已有固定模型。', 'Map at least one preset to an enabled smart route, or retain an existing fixed model.')}</span>}
                        {!enabledPlans.length && <div className="callout"><UiIcon name="route" /><div><span>{text('先创建并启用一条智能路由。', 'Create and enable a smart route first.')}</span></div>{onCreatePlan && <button className="btn" type="button" onClick={createPlanFromEditor}>{text('创建路由', 'Create route')}</button>}</div>}
                      </div>}
                      {facetEnabled && editor.facet === 'collaboration' && <fieldset className="worker-choices"><legend className="field-label">{text('委派时机', 'When to delegate')}</legend><label className="check-row"><input type="radio" name="agent-collaboration-trigger" value="explicit" checked={editorValues.triggerMode === 'explicit'} onChange={() => setTriggerMode('explicit')} /><div><strong>{text('仅在明确要求时', 'Only when explicitly requested')}</strong><span>{text('只有当你要求执行、委派或交给 Worker 时，Agent 才会启动任务。', 'The Agent starts a task only when you ask it to execute, delegate, or hand work to a Worker.')}</span></div></label><label className="check-row"><input type="radio" name="agent-collaboration-trigger" value="delegate_by_default" checked={editorValues.triggerMode === 'delegate_by_default'} onChange={() => setTriggerMode('delegate_by_default')} /><div><strong>{text('默认由 Agent 判断', 'Let the Agent decide by default')}</strong><span>{text('Agent 可以主动委派适合执行的任务；明确要求只回答时不会启动任务。', 'The Agent may proactively delegate suitable work, but will not start a task when you explicitly ask for an answer only.')}</span></div></label><p className="field-help">{text('可用执行路由由当前路由目录决定，这里不维护第二份名单。', 'Available execution routes come from the current route catalog; this setting does not maintain a second list.')}</p></fieldset>}
                      {!selectionKnown && <div className="callout warn" role="alert"><UiIcon name="warning" /><span>{text('当前配置状态无法核实，不能覆盖保存。请刷新或处理配置变化。', 'The current setting cannot be verified. Refresh or resolve configuration changes before saving.')}</span></div>}
                    </fieldset>
                  </form>
                  {blockedPreview}
                  {executableStatus}
                  {notice && <div className="callout agent-feedback" role="status"><UiIcon name="info" /><span>{notice}</span></div>}
                  {actionError && <div className="callout bad agent-feedback" role="alert" data-error-code={actionError}><UiIcon name="warning" /><span>{agentActionErrorMessage(actionError, language)}</span></div>}
                </Dialog>}
              </div>}
            </section>
          </div>}
    </div>
  </ProductPage>;
}
