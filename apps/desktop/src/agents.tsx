import { SecretTransportWarning } from './host/ConsoleUi';
import { AgentDiscoveryFeedback } from './features/agents/AgentDiscoveryFeedback';
import { AgentSkillDetails } from './features/agents/AgentSkillDetails';
import { loadAgentDiscoveryTargets, saveAgentDiscoveryTargets } from './features/agents/discovery';
import { PreviousRouting } from './features/agents/PreviousRouting';
import { CodexAccessPanel } from './features/agents/CodexAccessPanel';
import { confirmedCodexLaunchCommand, type PendingCodexLaunchCopy } from './features/agents/codex-launch';
import { BrandIcon, Dialog, Disclosure, ProductPage, UiIcon, copyText } from './ui';
import { confirmDiscard, useDiscardGuard } from './ui/discard-guard';
import React, { useEffect, useRef, useState } from 'react';
import { invoke } from './host';
import { agentEditorSeed, withModelSelection, agentModelFormInvalid, editorFingerprint, EMPTY_EDITOR_VALUES, type AgentEditorValues } from './features/agents/editor-state';
import { AgentSettingsFeedback } from './features/agents/AgentSettingsFeedback';
import { AgentModelEditor, modelSelectionSummary } from './features/agents/AgentModelEditor';
import { HermesRootFields } from './features/agents/HermesRootFields';
import { agentBrand, agentDisplayName, agentEcosystem, agentPageEntries, agentSupportsCollaboration, agentSupportsManagement, agentSupportsModelRouting } from './features/agents/ecosystems';
import { agentHasNoModelConnection, agentModelStatus } from './features/agents/status';
import { collaborationCheckFailureMessage } from './features/agents/collaboration-check-feedback';
import { agentSettingsSpec, agentTokenSpec, prerequisiteCheck } from './features/agents/settings-request';
import type {
  Agent, AgentSnapshot, AgentFacet as Facet, AgentCheckScope as CheckScope,
  AgentCollaborationTriggerMode, AgentNativeTarget, ModelStatus, Preview, Outcome,
} from './features/agents/types';
import { AgentTasks, type AgentTask, type AgentTaskRead } from './features/AgentTasks';
import type { OperationReference } from './features/model-connections/types';
import { safeDiagnosticCode } from './error-code';
import {
  agentActionErrorMessage,
  agentDisableMessage,
  classifyAgentMutation,
} from './features/agents/mutation-feedback';

export type {
  Agent, AgentSnapshot, AgentFixedModel, AgentModelSurface, AgentReasoningSelection,
  AgentNativeReasoning, ModelStatus, CollaborationStatus,
} from './features/agents/types';

type Editor = { context: string; facet: Facet };
type ModelReadback = {
  settings: ModelStatus & { context_id: string };
  plans: AgentSnapshot['plans'];
  trusted_authority: boolean;
};

export function desktopErrorCode(error: unknown): string {
  return safeDiagnosticCode(error, 'AGENT_UNAVAILABLE');
}

export function Agents({
  language,
  onMutation,
  onOperation,
  operation = null,
  onUnverifiedOperation,
  initialAgentId = null,
  initialFacet = 'model',
  refreshVersion = 0,
  mutationAllowed = true,
  taskRead = { status: 'unavailable' },
  initialTab = 'configuration',
  initialTaskId = null,
  onTaskCancel,
  onTaskConfirmResidual,
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
  operation?: OperationReference | null;
  onUnverifiedOperation?: (presentation: { kind: 'agent-settings'; target: string }) => void;
  initialAgentId?: string | null;
  initialFacet?: Facet;
  refreshVersion?: number;
  mutationAllowed?: boolean;
  taskRead?: AgentTaskRead;
  initialTab?: 'configuration' | 'tasks';
  initialTaskId?: string | null;
  onTaskConfirmResidual?: (task: AgentTask) => Promise<AgentTask>;
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
  const hermesTargets = useRef<AgentNativeTarget[]>(loadAgentDiscoveryTargets());
  const [scanning, setScanning] = useState(false);
  const [hermesRootOpen, setHermesRootOpen] = useState(false);
  const [hermesRoot, setHermesRoot] = useState('');
  const agentKey = (agent: Agent) => agent.agent_id === 'agent_hermes_default'
    ? `${agent.agent_id}:${agent.native_target?.config_root ?? agent.context_id ?? ''}` : agent.agent_id;
  const [loadError, setLoadError] = useState('');
  const [actionError, setActionErrorCode] = useState('');
  const [actionErrorHint, setActionErrorHint] = useState<string | null>(null);
  function setActionError(code: string) {
    setActionErrorCode(code);
    setActionErrorHint(null);
  }
  function setActionFailure(error: unknown) {
    setActionErrorCode(desktopErrorCode(error));
    setActionErrorHint(collaborationCheckFailureMessage(error, language));
  }
  const [notice, setNotice] = useState('');
  const [tokenEditing, setTokenEditing] = useState(false);
  const [tokenDraft, setTokenDraft] = useState('');
  const [selectedAgent, setSelectedAgent] = useState<string | null>(initialAgentId);
  const selectedAgentRef = useRef(selectedAgent);
  selectedAgentRef.current = selectedAgent;
  const [codexMode, setCodexMode] = useState<'profile' | 'root'>('profile');
  const [showAgentList, setShowAgentList] = useState(false);
  const [tab, setTab] = useState<'configuration' | 'tasks'>(initialTab);
  const agentName = (agent: Agent) => agentDisplayName(agent.agent_id, language);
  const [editor, setEditor] = useState<Editor | null>(null);
  const [editorValues, setEditorValues] = useState<AgentEditorValues>(EMPTY_EDITOR_VALUES);
  const [restoreNativeModel, setRestoreNativeModel] = useState('');
  const [selectionKnown, setSelectionKnown] = useState(false);
  const [baselineCodexMode, setBaselineCodexMode] = useState<'profile' | 'root'>('profile');
  const [pendingLaunchCopy, setPendingLaunchCopy] = useState<PendingCodexLaunchCopy | null>(null);
  const [pendingDisable, setPendingDisable] = useState<string | null>(null);
  const [baseline, setBaseline] = useState('');
  const fingerprint = editorFingerprint(editorValues);
  const dirty = !!editor && (fingerprint !== baseline || codexMode !== baselineCodexMode);
  useDiscardGuard('agents', dirty, language, confirmEditorReplacement);
  const queryGeneration = useRef(0);
  const [preview, setPreview] = useState<Preview | null>(null);
  const trigger = useRef<HTMLElement | null>(null);
  const firstField = useRef<HTMLElement | null>(null);
  const initialTargetHandled = useRef(false);

  async function refresh(target?: Agent) {
    const epoch = ++queryGeneration.current;
    setScanning(true);
    try {
      const result = await invoke<AgentSnapshot>('agent_snapshot', {
        nativeTargets: target?.context_id ? (target.native_target ? [target.native_target] : []) : hermesTargets.current,
        ...(target?.context_id ? { contextId: target.context_id } : {}),
      });
      if (epoch === queryGeneration.current) {
        if (target?.context_id) {
          const exact = result.agents.find(agent => agent.context_id === target.context_id
            && agentKey(agent) === agentKey(target));
          if (!exact) throw { code: 'AGENT_STATUS_UNAVAILABLE' };
          setSnapshot(current => ({ ...result, agents: (current?.agents ?? result.agents).map(agent =>
            agentKey(agent) === agentKey(target) ? exact
              : { ...agent, settings: null, status_error: 'AGENT_STATUS_UNVERIFIED' }) }));
        } else setSnapshot(result);
        setLoadError('');
        return target && result.trusted_authority ? result.agents.find(agent => agent.context_id === target.context_id && agentKey(agent) === agentKey(target)) : undefined;
      }
    } catch (caught) {
      if (epoch === queryGeneration.current) setLoadError(desktopErrorCode(caught));
    } finally { if (epoch === queryGeneration.current) setScanning(false); }
  }
  async function refreshModel(target: Agent) {
    // Codex access-mode metadata and Hermes roots still need their full scoped read.
    if (!target.context_id || target.codex_access || target.agent_id === 'agent_codex_default'
      || target.native_target || target.agent_id === 'agent_hermes_default') return refresh(target);
    const epoch = ++queryGeneration.current;
    setScanning(true);
    try {
      // The existing model projection verifies current file/grant/publication and service
      // fences. Discovery metadata is only a retained hint, never save confirmation.
      const result = await invoke<ModelReadback>('trial_agent_model', { contextId: target.context_id });
      if (epoch !== queryGeneration.current) return;
      if (result.settings.context_id !== target.context_id) throw { code: 'AGENT_STATUS_UNAVAILABLE' };
      setSnapshot(current => current && ({ ...current, plans: result.plans, trusted_authority: result.trusted_authority,
        agents: current.agents.map(agent => agentKey(agent) === agentKey(target)
          ? { ...agent, settings: result.settings, settings_scope: 'model', status_error: null }
          : { ...agent, settings: null, status_error: 'AGENT_STATUS_UNVERIFIED' }) }));
      setLoadError('');
    } catch (caught) {
      if (epoch === queryGeneration.current) setLoadError(desktopErrorCode(caught));
    } finally { if (epoch === queryGeneration.current) setScanning(false); }
  }
  async function discoverHermes() {
    if (busy || !hermesRoot.trim().startsWith('/')) return;
    setBusy(true); setActionError('');
    try {
      const target: AgentNativeTarget = { kind: 'hermes', config_root: hermesRoot.trim() };
      const targets = [...hermesTargets.current.filter(existing => existing.kind !== target.kind), target];
      const result = await invoke<AgentSnapshot>('agent_snapshot', { nativeTargets: targets });
      const candidate = [...result.agents].reverse().find(agent => agent.agent_id === 'agent_hermes_default' && agent.native_target);
      if (!candidate?.context_id || !candidate.native_target) throw new Error(candidate?.status_error ?? 'AGENT_INPUT_INVALID');
      hermesTargets.current = [...targets.filter(existing => existing.kind !== candidate.native_target!.kind), candidate.native_target];
      if (!saveAgentDiscoveryTargets(hermesTargets.current)) setNotice(text('目录已选择，但无法记住此选择；下次启动可能需要重新选择。', 'Directory selected, but this preference could not be saved. Select it again after restarting.'));
      setSnapshot(result); setSelectedAgent(agentKey(candidate)); setHermesRootOpen(false);
    } catch (error) { setActionFailure(error); }
    finally { setBusy(false); }
  }
  useEffect(() => {
    if (active) void refresh();
    return () => { queryGeneration.current++; };
  }, [active, refreshVersion]);
  useEffect(() => {
    if (!pendingLaunchCopy || operation?.operation_id !== pendingLaunchCopy.operationId) return;
    if (['failed', 'cancelled', 'compensated'].includes(operation.state)) { setPendingLaunchCopy(null); return; }
    if (operation.state !== 'succeeded') return;
    let cancelled = false;
    const target = pendingLaunchCopy;
    void (async () => {
      try {
        const current = await invoke<AgentSnapshot>('agent_snapshot', { nativeTargets: [], contextId: target.contextId });
        if (cancelled) return;
        const command = confirmedCodexLaunchCommand(target, operation, current.agents.find(agent => agent.agent_id === 'agent_codex_default'), navigator.platform);
        if (!command) { setPendingLaunchCopy(null); setNotice(text('保存已完成，请在启动区域重新读取并复制命令。', 'Save completed. Read and copy the command in the launch section.')); return; }
        await copyText(command);
        if (!cancelled) { setPendingLaunchCopy(null); setNotice(text('启动命令已复制，粘贴到终端即可使用。', 'Launch command copied. Paste it into a terminal to start.')); }
      } catch {
        if (!cancelled) {
          setPendingLaunchCopy(null);
          setNotice(text('保存已完成，自动复制失败。请使用“复制启动命令”或手动复制。', 'Save completed, but automatic copy failed. Use Copy launch command or copy it manually.'));
        }
      }
    })();
    return () => { cancelled = true; };
  }, [pendingLaunchCopy, operation]);
  useEffect(() => {
    if (pendingDisable && operation?.operation_id === pendingDisable) {
      setNotice(agentDisableMessage(operation.state, language));
      if (['succeeded', 'rolled_back', 'needs_attention'].includes(operation.state)) setPendingDisable(null);
    }
  }, [pendingDisable, operation, language]);
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
    if (!mutationAllowed || !snapshot?.trusted_authority || !agent.context_id || !agentEcosystem(agent.agent_id)
      || (facet === 'model' && (!agentSupportsModelRouting(agent.agent_id) || agentHasNoModelConnection(agent)))) return;
    if (dirty && !(await confirmDiscard(language))) return;
    if (facet !== 'model' && agent.settings_scope === 'model') {
      const current = await refresh(agent);
      if (!current || selectedAgentRef.current !== agentKey(agent)) return;
      agent = current;
    }
    trigger.current = source;
    setActionError('');
    setNotice('');
    setPreview(null);
    setTokenEditing(false);
    setTokenDraft('');
    setSelectedAgent(agentKey(agent));
    if (!agent.context_id) return;
    setEditor({ context: agent.context_id, facet });
    const { known, ...seed } = agentEditorSeed(agent, facet, enabledPlans.map(plan => plan.agent_plan_id));
    setSelectionKnown(known);
    setEditorValues(seed);
    const mode = agent.codex_access?.slot_occupied ? agent.codex_access.selected_mode : 'profile';
    setCodexMode(mode);
    setBaselineCodexMode(mode);
    setPendingLaunchCopy(null);
    setBaseline(editorFingerprint(seed));
    return true;
  }
  async function change(agent: Agent, facet: Facet, restore: boolean, source: HTMLElement | null) {
    if (busy || !mutationAllowed || !snapshot?.trusted_authority || !agent.context_id || !agentEcosystem(agent.agent_id) || (!restore && !selectionKnown)) return;
    setBusy(true);
    setBusyText(text('正在保存…', 'Saving…'));
    setActionError('');
    setNotice('');
    setPreview(null);
    let blockedByCheck: Preview | null = null;
    try {
      const input = { language, spec: agentSettingsSpec(agent, facet, restore, { values: editorValues, codexMode, restoreNativeModel }) };
      let result = await invoke<Outcome>('preview_agent_settings', { input });
      const checkScope = !restore && agentEcosystem(agent.agent_id) !== 'hermes' ? prerequisiteCheck(result.preview, facet) : null;
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
          if (restore && facet === 'model') {
            setNotice(agentDisableMessage(result.mutation.operation.state, language));
            if (!['succeeded', 'rolled_back', 'needs_attention'].includes(result.mutation.operation.state)) setPendingDisable(result.mutation.operation.operation_id);
          }
          if (!restore && facet === 'model' && agent.codex_access && activeCodexMode === 'profile' && !agentModelStatus(agent)?.current_selection) {
            setPendingLaunchCopy({ operationId: result.mutation.operation.operation_id, contextId: input.spec.context_id });
          }
          onOperation?.(result.mutation.operation, presentation);
        } else if (disposition === 'unverified') {
          onUnverifiedOperation?.(presentation);
        }
        if (disposition === 'submitted') {
          setBaseline(fingerprint);
          setEditor(null);
        }
      }
      // Complete this editor's exact readback before refreshing other pages.
      // Their independent discovery must not queue ahead of the result being saved.
      await (facet === 'model' ? refreshModel(agent) : refresh(agent));
      if (result.mutation) onMutation();
    } catch (caught) {
      setPreview(blockedByCheck);
      setActionFailure(caught);
      await (facet === 'model' ? refreshModel(agent) : refresh(agent));
      onMutation();
    } finally {
      setBusy(false);
      setBusyText('');
      requestAnimationFrame(() => { if (source?.isConnected) source.focus(); else trigger.current?.focus(); });
    }
  }
  async function changeToken(agent: Agent, regenerate: boolean, source: HTMLElement | null) {
    const selection = agentModelStatus(agent)?.current_selection;
    if (busy || !mutationAllowed || !snapshot?.trusted_authority || !agent.context_id || !agentEcosystem(agent.agent_id) || !selection || agentModelStatus(agent)?.state !== 'configured') return;
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
        spec: agentTokenSpec(agent, regenerate),
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
      }
      await refreshModel(agent);
      if (result.mutation) onMutation();
    } catch (caught) {
      setActionFailure(caught);
      await refreshModel(agent);
      onMutation();
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
            ? agent.agent_id === 'agent_qoder_default'
              ? agent.settings?.collaboration?.state === 'configured'
                ? text('用户协作技能验证通过；未执行委派任务。', 'Your installed collaboration skill passed verification; no delegated task was executed.')
                : text('协作能力检查通过，可以启用任务协作。', 'Collaboration capability check passed. You can enable task collaboration.')
              : text('任务委派技能检查通过；未调用上游模型。', 'Task delegation skill check passed; no upstream model was called.')
            : text('本机认证兼容性检查通过；尚未验证上游模型调用。', 'Local authentication compatibility passed; upstream model access is not verified.'),
      );
      setPreview(null);
      await refresh(agent);
    } catch (caught) {
      setActionFailure(caught);
    } finally {
      setBusy(false);
      setBusyText('');
      requestAnimationFrame(() => source.isConnected && source.focus());
    }
  }

  function setTriggerMode(triggerMode: AgentCollaborationTriggerMode) {
    setEditorValues(value => ({ ...value, triggerMode }));
    setPreview(null);
    setActionError('');
    setNotice('');
  }

  const plans = snapshot?.plans.plans ?? [];
  const enabledPlans = plans.filter(plan => plan.head.status === 'enabled');
  const agents = snapshot ? agentPageEntries(snapshot.agents) : [];
  const selected = agents.find(agent => agentKey(agent) === selectedAgent || agent.agent_id === selectedAgent)
    ?? (selectedAgent?.startsWith('agent_hermes_default:')
      ? agents.find(agent => agent.agent_id === 'agent_hermes_default' && !agent.context_id) : undefined)
    ?? (selectedAgent === null ? agents[0] : undefined);
  useEffect(() => {
    if (active && !busy && !scanning && !loadError && selected?.context_id
      && selected.status_error === 'AGENT_STATUS_UNVERIFIED') void refresh(selected);
  }, [active, busy, scanning, loadError, selected]);
  const skillsUnverified = selected?.settings_scope === 'model';
  const selectedManagement = selected?.settings?.management_skill;
  const selectedCollaboration = selected?.settings?.collaboration;
  const selectedModel = agentModelStatus(selected);
  const modelSelection = selectedModel?.current_selection;
  const taskSelection = selectedCollaboration?.current_selection;
  const needsRecovery = (state?: string | null) => state === 'drift' || state === 'needs_attention';
  const selectedHasDrift = needsRecovery(selectedModel?.state) || needsRecovery(selectedCollaboration?.state) || needsRecovery(selectedManagement?.state);
  const selectedPending = selectedModel?.state === 'pending' || selectedCollaboration?.state === 'pending' || selectedManagement?.state === 'pending';
  const authorityMutable = mutationAllowed && Boolean(snapshot?.trusted_authority);
  const mutable = authorityMutable && !scanning && !loadError
    && selected?.status_error !== 'AGENT_STATUS_UNVERIFIED';
  const qoderRecoveryOperation = selected?.agent_id === 'agent_qoder_default'
    && !selected.status_error && ['pending', 'needs_attention'].includes(selectedModel?.state ?? '')
    ? selectedModel?.operation_id : undefined;
  const hermesRecoveryOperation = selected?.agent_id === 'agent_hermes_default' && !selected.status_error
    ? [selectedModel, selectedManagement].find(facet => ['pending', 'needs_attention'].includes(facet?.state ?? ''))?.operation_id
    : undefined;
  const settingsRecoveryOperation = qoderRecoveryOperation ?? hermesRecoveryOperation
    ?? (!selected?.status_error ? [selectedModel, selectedManagement, selectedCollaboration]
      .find(facet => ['pending', 'needs_attention'].includes(facet?.state ?? ''))?.operation_id : undefined);
  async function retrySettingsOperation(operationId: string) {
    if (!selected?.context_id || busy || !snapshot?.trusted_authority) return;
    setBusy(true); setActionError('');
    try {
      await invoke('retry_agent_settings', { input: { schema: 'hiroute.agent-settings-retry/v1', context_id: selected.context_id, operation_id: operationId } });
      await refresh(selected);
    } catch (error) { setActionFailure(error); }
    finally { onMutation(); setBusy(false); }
  }

  const executableDiagnostic = (state: string) => {
    switch (state) {
      case 'not_found_in_scope':
        return {
          label: text('未发现入口', 'Entry not found'),
          title: text('当前配置范围内未找到 Agent 入口', 'No Agent entry was found in this configuration scope'),
          detail: text('请确认 Agent 已安装，或重新扫描当前环境。', 'Confirm that the Agent is installed, or scan the current environment again.'),
        };
      case 'executable_not_runnable':
        return {
          label: text('命令不可执行', 'Command not executable'),
          title: text('已定位 Agent，但它当前不可执行', 'The Agent was located but is not executable'),
          detail: text('请检查目标是否为普通可执行文件。安装目录的组写权限本身不会阻止接入。', 'Check that the target is a regular executable file. Group-writable installation directories do not block routing.'),
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
  const activeCodexMode = selected?.codex_access?.slot_occupied
    ? selected.codex_access.selected_mode : codexMode;
  const catalogModels = (selected?.native_model_catalog?.models ?? [])
    .map(model => ({ ...model, source_options: model.source_options ?? [] }));
  const nativeRestoreOptions = catalogModels.filter(model =>
    !plans.some(plan => plan.model_alias === model.client_model_id));
  const modelFormInvalid = agentModelFormInvalid(selected, editorValues, enabledPlans.map(plan => plan.agent_plan_id));
  const formInvalid = !selectionKnown || (editor?.facet === 'model' && modelFormInvalid);
  const creatingModelConnection = editor?.facet === 'model' && !modelSelection;
  const submitLabel = creatingModelConnection
    ? selected?.agent_id === 'agent_codex_default' && activeCodexMode === 'profile'
      ? text('启用并复制启动命令', 'Enable and copy launch command')
      : text('启用模型路由', 'Enable model routing')
    : editor?.facet === 'management_skill' ? text('启用 HiRoute 管理', 'Enable HiRoute management') : editor?.facet === 'collaboration' && !taskSelection
      ? text('启用任务路由', 'Enable task routing') : text('保存配置', 'Save settings');

  const brand = (agent: Agent) => agentBrand(agent.agent_id);
  const selectionSummary = modelSelectionSummary(modelSelection, plans, language);
  const blockedPreview = <AgentSettingsFeedback preview={preview} agent={selected} facet={editor?.facet}
    defaultChoice={editorValues.defaultChoice} language={language} disabled={busy || !mutable}
    onCheck={(scope, source) => { if (selected) void check(selected, scope, source); }} />;
  const executableStatus = selectedExecutableDiagnostic ? <div className="callout warn agent-feedback" data-agent-executable-state={selected?.configuration_state} role="status">
    <UiIcon name="warning" />
    <div>
      <strong>{selectedExecutableDiagnostic.title}</strong>
      <p>{selectedExecutableDiagnostic.detail}</p>
      <button className="btn" type="button" onClick={() => void refresh()}>{text('重新扫描', 'Scan again')}</button>
    </div>
  </div> : null;

  return <ProductPage title="Agent" subtitle={text('在你常用的 Agent 中使用智能路由', 'Use smart routing in your usual Agent')} flush className="agents-page">
    {hermesRootOpen && <Dialog open title={text('选择 Hermes 配置目录', 'Select Hermes configuration directory')} closeLabel={text('取消', 'Cancel')} closeDisabled={busy} onClose={() => setHermesRootOpen(false)} footer={<><button className="btn" type="button" disabled={busy} onClick={() => setHermesRootOpen(false)}>{text('取消', 'Cancel')}</button><button className="btn btn-primary" type="submit" form="hermes-root-selection" disabled={busy || !hermesRoot.trim().startsWith('/')}>{busy ? text('读取中…', 'Reading…') : text('读取配置', 'Read configuration')}</button></>}>
      <form id="hermes-root-selection" className="agent-form" onSubmit={event => { event.preventDefault(); void discoverHermes(); }}><HermesRootFields value={hermesRoot} onChange={setHermesRoot} disabled={busy} language={language} />{actionError && <p className="oc-inline-error" role="alert">{agentActionErrorMessage(actionError, language)}</p>}</form>
    </Dialog>}
    <div className="agents-workspace">
      <div className="agent-discovery-toolbar"><div className="agent-tabs segmented" role="tablist">
        <button className={`segment${tab === 'configuration' ? ' active' : ''}`} type="button" role="tab" aria-selected={tab === 'configuration'} onClick={() => { setTab('configuration'); setActionError(''); setNotice(''); }}>{text('我的 Agent', 'My Agents')}</button>
        <button className={`segment${tab === 'tasks' ? ' active' : ''}`} type="button" role="tab" aria-selected={tab === 'tasks'} onClick={() => { setTab('tasks'); setActionError(''); setNotice(''); onOpenTasks?.(); }}>{text('任务记录', 'Task history')}</button>
      </div><button className="btn btn-quiet" type="button" data-agent-rescan disabled={busy || scanning} onClick={() => void refresh()}>{scanning ? text('扫描中…', 'Scanning…') : text('重新扫描', 'Scan again')}</button></div>
      {!snapshot && !loadError && <div className="empty-state" role="status"><div><span className="oc-spinner" /><p>{text('正在读取本地 Agent…', 'Reading local Agents…')}</p></div></div>}
      {!snapshot && loadError && <div className="empty-state" data-error-code={loadError}><div><span className="empty-icon"><UiIcon name="agent" /></span><h2>{text('暂时无法读取 Agent', 'Agents are temporarily unavailable')}</h2><p>{text('请确认本机服务正在运行，然后重试。', 'Make sure the local service is running, then retry.')}</p><button className="btn btn-primary" type="button" onClick={() => void refresh()}>{text('重试', 'Retry')}</button></div></div>}
      {tab === 'tasks'
        ? <AgentTasks language={language} read={taskRead} initialTaskId={initialTaskId} onCancel={onTaskCancel} onConfirmResidual={onTaskConfirmResidual} onOpenSession={onOpenTaskSession} onBackToAgents={() => setTab('configuration')} onRefresh={onOpenTasks} onLoadMore={onLoadMoreTasks} onReadTask={onReadTask} onLoadTaskResult={onLoadTaskResult} active={active} />
        : snapshot && !agents.length
          ? <div className="empty-state"><div><span className="empty-icon"><UiIcon name="agent" /></span><h2>{text('没有发现可接入的 Agent', 'No compatible Agent found')}</h2><p>{text('安装或启动支持的 Agent 后再返回此页。', 'Install or start a supported Agent, then return here.')}</p><button className="btn" type="button" onClick={() => void refresh()}>{text('重新扫描', 'Scan again')}</button></div></div>
          : tab === 'configuration' && <div className={`split-view oc-agents${showAgentList ? '' : ' oc-agent-detail-open'}`}>
            <aside className="master-pane">
              <nav className="master-list native-list" aria-label={text('Agent 列表', 'Agent list')}>
                {agents.map(agent => {
                  const active = selected ? agentKey(selected) === agentKey(agent) : false;
                  const stateNeedsRecovery = needsRecovery(agentModelStatus(agent)?.state) || needsRecovery(agent.settings?.collaboration?.state);
                  const statePending = agentModelStatus(agent)?.state === 'pending' || agent.settings?.collaboration?.state === 'pending';
                  const executableState = executableDiagnostic(agent.configuration_state);
                  const stateLabel = agent.configuration_state === 'profile_not_selected'
                    ? text('未发现本机配置', 'No local configuration found')
                    : agent.status_error
                    ? text('状态待确认', 'Status unverified')
                    : executableState
                      ? executableState.label
                      : stateNeedsRecovery
                      ? text('接入需要处理', 'Connection needs attention')
                      : statePending
                        ? text('正在更新', 'Updating')
                        : agentModelStatus(agent)?.current_selection
                          ? text('路由已配置', 'Routing configured')
                          : agent.settings?.collaboration?.current_selection
                            ? text('任务路由已启用', 'Task routing enabled')
                          : text('已发现 · 尚未接入', 'Detected · not connected');
                  return <button className={`list-row${active ? ' active' : ''}`} key={agentKey(agent)} aria-current={active ? 'page' : undefined} onClick={async () => { if (dirty && !(await confirmDiscard(language))) return; setEditor(null); setPreview(null); setActionError(''); setNotice(''); setTokenEditing(false); setTokenDraft(''); setSelectedAgent(agentKey(agent)); setShowAgentList(false); }}>
                    <BrandIcon kind={brand(agent)} label={`${agentName(agent)} logo`} />
                    <span className="row-main"><span className="row-title">{agentName(agent)}</span>{agent.native_target && <span className="row-meta">{agent.native_target.config_root}</span>}<span className="row-meta">{stateLabel}</span></span>
                  </button>;
                })}
              </nav>
            </aside>
            <section className="detail-pane">
              <div className="oc-model-back"><button className="btn btn-quiet" type="button" onClick={() => setShowAgentList(true)}><UiIcon name="arrowLeft" />{text('返回 Agent 列表', 'Back to Agents')}</button></div>
              {selected && snapshot && <div className="detail-inner" data-agent-id={selected.agent_id}>
                <header className="detail-hero"><div className="detail-identity"><BrandIcon kind={brand(selected)} label={`${agentName(selected)} logo`} /><div><h2>{agentName(selected)}</h2><p>{text('选择你需要的路由能力', 'Choose the routing capabilities you need')}</p></div></div></header>
                {agentEcosystem(selected.agent_id) === 'hermes' && <section className="detail-section v3-agent-section" data-hermes-profile>
                  <div className="detail-section-head"><h3>{text('配置目录', 'Configuration directory')}</h3><button type="button" className="btn btn-quiet" data-hermes-select-root disabled={busy || scanning} onClick={() => { setHermesRoot(selected.native_target?.config_root ?? ''); setActionError(''); setHermesRootOpen(true); }}>{selected.native_target ? text('更改', 'Change') : text('手动选择目录', 'Select directory manually')}</button></div>
                  {selected.native_target
                    ? <p className="field-help">{selected.native_target.config_root}</p>
                    : <p>{text('选择本机 Hermes 实际使用的配置目录，即可配置模型路由、任务路由和 HiRoute 管理 Skill。独立 profile 请选对应目录。', 'Select the configuration directory used by Hermes on this computer to configure model routing, task routing and the HiRoute management Skill. For a separate profile, select its directory.')}</p>}
                </section>}
                {!authorityMutable && !selected.codex_access?.pending_operation && <div className="callout warn"><UiIcon name="warning" /><div><strong>{text('Agent 配置可以查看，暂时不能修改', 'Agent settings are viewable but cannot be changed')}</strong><p>{text('本机服务尚未就绪，连接恢复后即可保存配置。', 'The local service is not ready. Reconnect before saving.')}</p></div></div>}
                {selected.codex_access && <CodexAccessPanel access={selected.codex_access} language={language} retryDisabled={busy || Boolean(editor) || !snapshot.trusted_authority} onRetry={async () => {
                  const access = selected.codex_access;
                  if (access?.pending_operation) await retrySettingsOperation(access.pending_operation);
                }} />}
                <AgentDiscoveryFeedback agent={selected} language={language} scanning={scanning} onScan={() => void refresh()} />
                {executableStatus}
                {selectedHasDrift && <div className="callout warn"><UiIcon name="warning" /><div><strong>{text('接入配置已变化', 'Connection settings changed')}</strong><p>{text('HiRoute 不会覆盖已变化的设置。请在“连接详情与恢复”中恢复需要的路由配置。', 'HiRoute will not overwrite changed settings. Restore the required routing configuration under Connection details and recovery.')}</p></div></div>}
                {settingsRecoveryOperation && <div className="callout warn" role="alert" data-qoder-model-recovery={qoderRecoveryOperation ? true : undefined} data-hermes-settings-recovery={hermesRecoveryOperation ? true : undefined}>
                  <UiIcon name="warning" /><div>
                    <strong>{hermesRecoveryOperation ? text('Hermes 配置尚未完成', 'Hermes configuration is incomplete') : text('模型路由配置尚未完成', 'Model routing configuration is incomplete')}</strong>
                    <p>{hermesRecoveryOperation ? text('请先保存自己的编辑，恢复本次操作开始后发生的文件改动，再继续原操作。HiRoute 不会强制覆盖文件。', 'Save your edits and undo file changes made since this operation started, then resume it. HiRoute will not force an overwrite.') : text('请先保存自己的编辑，恢复本次操作开始后发生的配置文件改动，再继续原操作。HiRoute 不会强制覆盖文件；任务协作保持独立。', 'Save your edits and undo configuration changes made since this operation started, then resume it. HiRoute will not force an overwrite; task collaboration remains independent.')}</p>
                    <button className="btn" type="button" disabled={busy || Boolean(editor) || !snapshot.trusted_authority} onClick={() => void retrySettingsOperation(settingsRecoveryOperation)}>{text('重新检查并继续原操作', 'Recheck and resume operation')}</button>
                  </div>
                </div>}
                {selectedPending && !settingsRecoveryOperation && !selected.codex_access?.pending_operation && <div className="oc-status-row" role="status"><span className="oc-spinner" /><p>{text('路由配置正在应用，完成后会自动更新。', 'Routing settings are being applied and will update automatically.')}</p></div>}
                {agentSupportsModelRouting(selected.agent_id) && !agentHasNoModelConnection(selected) && <section className="detail-section v3-agent-section">
                  <div className="detail-section-head"><h3>{text('模型路由', 'Model routing')}</h3><button className="btn btn-quiet" data-agent-facet="model" disabled={busy || selectedPending || !selected.context_id || !agentEcosystem(selected.agent_id) || !mutable} onClick={event => open(selected, 'model', event.currentTarget)}>{modelSelection ? text('调整', 'Edit') : text('启用', 'Enable')}</button>{selectedModel?.restore_point_ref && <button className="btn btn-quiet" disabled={busy || selectedPending || !mutable} onClick={event => void change(selected, 'model', true, event.currentTarget)}>{text('停用', 'Disable')}</button>}</div>
                  <p>{modelSelection ? <>{text('当前选择：', 'Current selection: ')}<strong>{selectionSummary}</strong></> : selected.agent_id === 'agent_qoder_default'
                    ? text('添加可在 Qoder 中明确选择的 HiRoute 路由，保留原有模型和默认值。', 'Add HiRoute routes you can explicitly select in Qoder, while keeping native models and defaults.')
                    : text('为此 Agent 配置固定模型，或通过智能路由自动选择模型。', 'Configure a fixed model for this Agent, or use smart routing to choose a model automatically.')}</p>
                  {modelSelection?.mode === 'claude_launcher' && <p className="field-help">{text('直接启动 claude，使用 Opus、Sonnet、Haiku 原生预设选择已映射路由；其他计划不会自动出现在模型菜单中。若未显式选择当前模型，Claude Default 取决于账号；预设映射不证明 Default 可用，请通过真实调用验证。', 'Start claude normally and use its native Opus, Sonnet, or Haiku preset for mapped routes. Other plans do not automatically appear in the model picker. Without an explicit model selection, Claude Default depends on the account; preset mappings do not prove it works. Verify with a live call.')}</p>}
                  {modelSelection?.mode === 'qoder_additional' && <p className="field-help">{text('重新启动 Qoder 后，在模型选择中选用已添加的 HiRoute 路由。原有模型、登录和默认选择保持不变。', 'Restart Qoder and select an added HiRoute route in its model choices. Existing models, sign-in and defaults stay unchanged.')}</p>}
                </section>}
                {agentSupportsCollaboration(selected.agent_id) && <section className="detail-section v3-agent-section" data-agent-collaboration data-agent-skills-unverified={skillsUnverified || undefined}>
                  <div className="detail-section-head"><h3>{text('任务路由', 'Task routing')}</h3><button className="btn btn-quiet" data-agent-facet="collaboration" disabled={busy || !selected.context_id || !mutable} onClick={event => open(selected, 'collaboration', event.currentTarget)}>{skillsUnverified ? text('查看配置', 'View settings') : taskSelection ? text('调整', 'Edit') : text('启用', 'Enable')}</button></div>
                  {selected.agent_id === 'agent_qoder_default' && <div data-qoder-collaboration>
                    <p>{text('使用 Qoder 原有模型和登录配置，通过任务协作使用 HiRoute 计划。需要登录时，请在终端正常启动 Qoder CLI 并完成登录后重试。', 'Keep your Qoder models and sign-in configuration, and use HiRoute plans through task collaboration. If sign-in is needed, start Qoder CLI normally in a terminal, sign in, and retry.')}</p>
                  </div>}
                  <p>{skillsUnverified ? text('状态尚未读取，打开配置时会重新核实。', 'Status has not been read. Opening settings will verify it again.') : taskSelection?.trigger_mode === 'delegate_by_default'
                    ? text('由此 Agent 判断何时将任务交给执行 Agent；你仍可明确要求只回答、不执行。', 'This Agent decides when to delegate work to an execution Agent; you can still explicitly ask it to answer without execution.')
                    : taskSelection
                      ? text('仅在你明确要求执行或委派任务时，将任务交给执行 Agent。', 'Delegate work to an execution Agent only when you explicitly ask for execution or delegation.')
                      : text('让此 Agent 将任务交给合适的执行 Agent，并获取结果。', 'Let this Agent delegate work to a suitable execution Agent and retrieve the results.')}</p>
                  <AgentSkillDetails resource={selectedCollaboration?.resource} language={language} />
                  <p className="field-help">{text('启用后，可在对话中说：“使用 HiRoute，将当前任务委派给【路由名称】执行。”路由名称见“智能路由”，也可让 Agent 选择合适的路由。', 'After enabling, ask in conversation: “Use HiRoute to delegate this task to [route name].” Find route names under Smart routing, or ask the Agent to choose a suitable route.')}</p>
                </section>}
                {agentSupportsManagement(selected.agent_id) && <section className="detail-section v3-agent-section" data-agent-management data-hermes-management={agentEcosystem(selected.agent_id) === 'hermes' ? true : undefined}>
                  <div className="detail-section-head"><h3>{text('HiRoute 管理', 'HiRoute management')}</h3><button className="btn btn-quiet" role={skillsUnverified ? undefined : 'switch'} aria-checked={skillsUnverified ? undefined : selectedManagement?.state === 'configured'} data-agent-facet="management_skill" disabled={busy || !selected.context_id || !mutable || selectedManagement?.state === 'pending'} onClick={event => { if (selectedManagement?.state === 'configured' && selectedManagement.restore_point_ref) void change(selected, 'management_skill', true, event.currentTarget); else open(selected, 'management_skill', event.currentTarget); }}>{skillsUnverified ? text('查看配置', 'View settings') : selectedManagement?.state === 'configured' ? text('停用', 'Disable') : text('启用', 'Enable')}</button></div>
                  <p>{text('准备管理 Skill，让此 Agent 通过本机 CLI 管理模型来源和智能路由。可独立启用或恢复，无需先连接模型。', 'Prepare the management Skill so this Agent can manage model sources and smart routes through the local CLI. Enable or restore it independently, without model routing.')}</p>
                  {selectedManagement?.restore_point_ref && selectedManagement.state !== 'configured' && <button className="btn" disabled={busy || !mutable} onClick={event => void change(selected, 'management_skill', true, event.currentTarget)}>{text('恢复管理 Skill', 'Restore management Skill')}</button>}
                  <p>{skillsUnverified ? text('状态尚未读取，打开配置时会重新核实。', 'Status has not been read. Opening settings will verify it again.') : selectedManagement?.state === 'configured' ? text('当前配置已启用。', 'Enabled for this configuration.') : selectedManagement?.state === 'restored' ? text('当前配置已停用；共享或外部安装的 Skill 文件可能仍然可用。', 'Disabled for this configuration; a shared or externally installed Skill may remain available.') : text('当前配置未启用。', 'Not enabled for this configuration.')}</p>
                  {selectedManagement?.state === 'configured' && <button className="btn btn-quiet" type="button" disabled={busy || !mutable} onClick={event => open(selected, 'management_skill', event.currentTarget)}>{text('重新准备 Skill', 'Prepare Skill again')}</button>}
                  <AgentSkillDetails resource={selectedManagement?.resource} language={language} management />
                </section>}
                {selectedModel?.native_adapter && <p className="field-help" data-hermes-resource-state>{selectedModel.native_adapter.resources_installed
                  ? text('适配插件已准备；当前 Hermes 进程的加载状态尚未确认。请重新加载插件或开启新会话。', 'Adapter files are prepared; loading in the current Hermes process is unobserved. Reload plugins or start a new session.')
                  : text('适配资源需要修复；请检查自己的文件编辑后重新保存配置。', 'Adapter resources need repair. Review your file edits and save the settings again.')}</p>}
                {Boolean(selectedModel?.native_adapter?.known_channel_models?.length) && <p className="field-help" data-hermes-known-overrides>{text('已配置的通道模型覆盖', 'Configured channel model overrides')} · {selectedModel?.native_adapter?.known_channel_models?.join(', ')}. {text('需要改用 HiRoute 的通道，请通过 Hermes 的 /model 调整选择。', 'Use Hermes /model in a channel that should switch to HiRoute.')}</p>}
                <Disclosure className="native-details" label={text('连接详情与恢复', 'Connection details and recovery')} language={language}><dl>{selected.version && <div><dt>{text('版本', 'Version')}</dt><dd>{selected.version}</dd></div>}{selectedModel?.state === 'configured' && modelSelection && <div><dt>{text('本机接入令牌', 'Local access token')}</dt><dd><input className="input" type="password" value="••••••••••••••••" readOnly aria-label={text('当前令牌已隐藏', 'Current token hidden')} /></dd></div>}</dl>{selectedModel?.restore_point_ref && selected.agent_id === 'agent_codex_default' && selected.codex_access?.selected_mode !== 'profile' && <label className="field-label">{text('恢复时选择原生模型（可选）', 'Native model on restore (optional)')}<input list="codex-native-restore-models" value={restoreNativeModel} onChange={event => { setRestoreNativeModel(event.target.value); setPreview(null); }} placeholder={text('保留原设置', 'Keep original setting')} disabled={busy || !mutable} /><datalist id="codex-native-restore-models">{nativeRestoreOptions.map(model => <option key={model.client_model_id} value={model.client_model_id}>{model.display_name}</option>)}</datalist></label>}{selectedModel?.state === 'configured' && modelSelection && <div className="actions" data-agent-token-controls><button className="btn" disabled={busy || !mutable} onClick={() => { setTokenEditing(true); setTokenDraft(''); setActionError(''); }}>{text('修改令牌', 'Change token')}</button><button className="btn" disabled={busy || !mutable} onClick={event => void changeToken(selected, true, event.currentTarget)}>{text('重新生成', 'Regenerate')}</button></div>}{tokenEditing && selectedModel?.state === 'configured' && modelSelection && <form className="agent-token-form" onSubmit={event => { event.preventDefault(); void changeToken(selected, false, (event.nativeEvent as SubmitEvent).submitter as HTMLElement | null); }}><SecretTransportWarning language={language} /><label className="field"><span className="field-label">{text('新令牌', 'New token')}</span><input className="input" type="password" autoComplete="new-password" value={tokenDraft} onChange={event => setTokenDraft(event.target.value)} minLength={16} maxLength={128} pattern="[A-Za-z0-9._~\\-]{16,128}" required disabled={busy} /><span className="field-help">{text('16–128 位英文字母、数字及 . _ ~ -；保存后原令牌立即失效，无需重新接入。', 'Use 16–128 letters, numbers, or . _ ~ -. The old token stops working after save; reconnecting is unnecessary.')}</span></label><div className="actions"><button className="btn btn-primary" type="submit" disabled={busy || !mutable}>{text('保存令牌', 'Save token')}</button><button className="btn" type="button" disabled={busy} onClick={() => { setTokenEditing(false); setTokenDraft(''); }}>{text('取消', 'Cancel')}</button></div></form>}<div className="actions">{selectedCollaboration?.restore_point_ref && <button className="btn" disabled={busy || !mutable} onClick={event => void change(selected, 'collaboration', true, event.currentTarget)}>{text('停用任务路由', 'Disable task routing')}</button>}</div></Disclosure>
                {!editor && modelSelection && <PreviousRouting key={selected.agent_id} agent={selected.agent_id} language={language} disabled={busy || selectedPending || !mutable}
                  onDraft={async (selection, context, source) => {
                    if (context !== selected.context_id || !(await open(selected, 'model', source))) return;
                    setEditorValues(value => withModelSelection(value, selection));
                    setSelectionKnown(true);
                    setNotice(text('已载入之前的映射草稿，请审阅后保存。', 'Earlier mappings loaded as a draft. Review before saving.'));
                  }}
                  onRestore={selectedModel?.restore_point_ref ? source => void change(selected, 'model', true, source) : undefined} />}
                {!editor && loadError && <div className="callout warn agent-feedback agent-inline-feedback" role="alert" data-error-code={loadError}><UiIcon name="warning" /><div><strong>{text('状态刷新失败', 'Status refresh failed')}</strong><p>{text('当前状态尚未核实，请重试后再修改。', 'The current state is unverified. Retry before making changes.')}</p><button className="btn" type="button" disabled={scanning} onClick={() => void refresh(selected)}>{text('重试', 'Retry')}</button></div></div>}
                {!editor && blockedPreview}
                {!editor && notice && <div className="toast-stack" aria-live="polite"><div className="toast" role="status"><UiIcon name="check" /><span>{notice}</span></div></div>}
                {!editor && actionError && <div className="callout bad agent-feedback agent-inline-feedback" role="alert" data-error-code={actionError}><UiIcon name="warning" /><span>{actionErrorHint ?? agentActionErrorMessage(actionError, language)}</span></div>}
                {!selected.context_id && <div className="callout"><UiIcon name="info" /><span>{text('此 Agent 的设置入口尚不可用。', 'Settings for this Agent are not available yet.')}</span></div>}
                {editor?.context === selected.context_id && selected.context_id && <Dialog open title={`${agentName(selected)} · ${editor.facet === 'model' ? text('模型路由', 'Model routing') : editor.facet === 'management_skill' ? text('HiRoute 管理', 'HiRoute management') : text('任务路由', 'Task routing')}`} closeLabel={text('关闭配置', 'Close settings')} closeDisabled={busy} onClose={() => { if (!busy) close(); }} footer={<><button className="btn" type="button" disabled={busy} onClick={close}>{text('取消', 'Cancel')}</button><button className="btn btn-primary" type="submit" form="hr-agent-settings" disabled={busy || !mutable || Boolean(formInvalid)}>{busy ? busyText || text('正在保存…', 'Saving…') : submitLabel}</button></>}>
                  <form id="hr-agent-settings" className="agent-form" ref={node => { firstField.current = node?.querySelector<HTMLElement>('input:not([disabled]), select:not([disabled]), button:not([disabled])') ?? null; }} onSubmit={event => { event.preventDefault(); void change(selected, editor.facet, false, (event.nativeEvent as SubmitEvent).submitter as HTMLElement | null); }}>
                    <fieldset disabled={busy || !mutable}>
                      {editor.facet === 'model' && <p className="field-help" data-agent-service-responsibility>{text('使用路由时请保持 HiRoute 运行。启用不会自动设置开机启动。', 'Keep HiRoute running when using routing. Enabling does not set up automatic startup.')}</p>}
                      {editor.facet === 'model' && <AgentModelEditor agent={selected} values={editorValues} plans={plans} language={language} disabled={busy || !mutable} invalid={modelFormInvalid} codexMode={activeCodexMode}
                        onCodexMode={mode => { setCodexMode(mode); setPreview(null); setActionError(''); }}
                        onChange={setEditorValues}
                        onEdited={(clearNotice = true) => { setPreview(null); setActionError(''); if (clearNotice) setNotice(''); }}
                        onCreatePlan={onCreatePlan ? createPlanFromEditor : undefined} />}
                      {editor.facet === 'management_skill' && <div className="additional-model-editor"><p>{text('准备后，此 Agent 可读取 HiRoute 管理 Skill 并通过本机 CLI 配置 HiRoute。模型路由和任务路由保持当前设置；此操作不调用模型。', 'This Agent can read the management Skill and configure HiRoute through the local CLI. Model and task routing keep their current settings; this operation does not call a model.')}</p><AgentSkillDetails resource={selectedManagement?.resource} language={language} management /></div>}
                      {editor.facet === 'collaboration' && <fieldset className="worker-choices"><legend className="field-label">{text('委派时机', 'When to delegate')}</legend><label className="check-row"><input type="radio" name="agent-collaboration-trigger" value="explicit" checked={editorValues.triggerMode === 'explicit'} onChange={() => setTriggerMode('explicit')} /><div><strong>{text('仅在明确要求时', 'Only when explicitly requested')}</strong><span>{text('只有当你要求执行、委派或交给 Worker 时，Agent 才会启动任务。', 'The Agent starts a task only when you ask it to execute, delegate, or hand work to a Worker.')}</span></div></label><label className="check-row"><input type="radio" name="agent-collaboration-trigger" value="delegate_by_default" checked={editorValues.triggerMode === 'delegate_by_default'} onChange={() => setTriggerMode('delegate_by_default')} /><div><strong>{text('默认由 Agent 判断', 'Let the Agent decide by default')}</strong><span>{text('Agent 可以主动委派适合执行的任务；明确要求只回答时不会启动任务。', 'The Agent may proactively delegate suitable work, but will not start a task when you explicitly ask for an answer only.')}</span></div></label><p className="field-help">{text('可委派的任务路由在“智能路由”中管理。', 'Manage routes available for task delegation under Smart routing.')}</p></fieldset>}
                      {!selectionKnown && <div className="callout warn" role="alert"><UiIcon name="warning" /><span>{text('当前配置状态无法核实，不能覆盖保存。请刷新或处理配置变化。', 'The current setting cannot be verified. Refresh or resolve configuration changes before saving.')}</span></div>}
                    </fieldset>
                  </form>
                  {blockedPreview}
                  {executableStatus}
                  {notice && <div className="callout agent-feedback" role="status"><UiIcon name="info" /><span>{notice}</span></div>}
                  {actionError && <div className="callout bad agent-feedback" role="alert" data-error-code={actionError}><UiIcon name="warning" /><span>{actionErrorHint ?? agentActionErrorMessage(actionError, language)}</span></div>}
                </Dialog>}
              </div>}
            </section>
          </div>}
    </div>
  </ProductPage>;
}
