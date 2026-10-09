import { CodexAccessPanel } from './features/agents/CodexAccessPanel';
import { confirmedCodexLaunchCommand, type PendingCodexLaunchCopy } from './features/agents/codex-launch';
import { BrandIcon, Dialog, Disclosure, ProductPage, UiIcon, copyText } from './ui';
import { confirmDiscard, useDiscardGuard } from './ui/discard-guard';
import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { agentEditorSeed, agentModelFormInvalid, editorFingerprint, EMPTY_EDITOR_VALUES, type AgentEditorValues } from './features/agents/editor-state';
import { AgentSettingsFeedback } from './features/agents/AgentSettingsFeedback';
import { AgentModelEditor, modelSelectionSummary } from './features/agents/AgentModelEditor';
import { agentBrand, agentDisplayName, agentEcosystem, agentSupportsModelRouting } from './features/agents/ecosystems';
import { agentHasNoModelConnection, agentModelStatus } from './features/agents/status';
import { collaborationCheckFailureMessage } from './features/agents/collaboration-check-feedback';
import { agentSettingsSpec, agentTokenSpec, prerequisiteCheck } from './features/agents/settings-request';
import type {
  Agent, AgentSnapshot, AgentFacet as Facet, AgentCheckScope as CheckScope,
  AgentCollaborationTriggerMode, Preview, Outcome,
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
    if (!pendingLaunchCopy || operation?.operation_id !== pendingLaunchCopy.operationId) return;
    if (['failed', 'cancelled', 'compensated'].includes(operation.state)) { setPendingLaunchCopy(null); return; }
    if (operation.state !== 'succeeded') return;
    let cancelled = false;
    const target = pendingLaunchCopy;
    void (async () => {
      try {
        const current = await invoke<AgentSnapshot>('agent_snapshot');
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
    trigger.current = source;
    setActionError('');
    setNotice('');
    setPreview(null);
    setTokenEditing(false);
    setTokenDraft('');
    setSelectedAgent(agent.agent_id);
    setEditor({ context: agent.context_id, facet });
    const { known, ...seed } = agentEditorSeed(agent, facet, enabledPlans.map(plan => plan.agent_plan_id));
    setSelectionKnown(known);
    setEditorValues(seed);
    const mode = agent.codex_access?.slot_occupied ? agent.codex_access.selected_mode : 'profile';
    setCodexMode(mode);
    setBaselineCodexMode(mode);
    setPendingLaunchCopy(null);
    setBaseline(editorFingerprint(seed));
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
      const checkScope = !restore ? prerequisiteCheck(result.preview, facet) : null;
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
        onMutation();
      }
      await refresh();
    } catch (caught) {
      setPreview(blockedByCheck);
      setActionFailure(caught);
      onMutation();
      await refresh();
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
        onMutation();
      }
      await refresh();
    } catch (caught) {
      setActionFailure(caught);
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
            ? agent.agent_id === 'agent_qoder_default'
              ? agent.settings?.collaboration?.state === 'configured'
                ? text('用户协作技能验证通过；未执行委派任务。', 'Your installed collaboration skill passed verification; no delegated task was executed.')
                : text('协作能力检查通过，可以启用任务协作。', 'Collaboration capability check passed. You can enable task collaboration.')
              : text('任务委派技能检查通过；未调用上游模型。', 'Task delegation skill check passed; no upstream model was called.')
            : text('本机认证兼容性检查通过；尚未验证上游模型调用。', 'Local authentication compatibility passed; upstream model access is not verified.'),
      );
      setPreview(null);
      await refresh();
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
  const selected = snapshot?.agents.find(agent => agent.agent_id === (selectedAgent ?? snapshot.agents[0]?.agent_id));
  const selectedCollaboration = selected?.settings?.collaboration;
  const selectedModel = agentModelStatus(selected);
  const modelSelection = selectedModel?.current_selection;
  const taskSelection = selectedCollaboration?.current_selection;
  const needsRecovery = (state?: string | null) => state === 'drift' || state === 'needs_attention';
  const selectedHasDrift = needsRecovery(selectedModel?.state) || needsRecovery(selectedCollaboration?.state);
  const selectedPending = selectedModel?.state === 'pending' || selectedCollaboration?.state === 'pending';
  const mutable = mutationAllowed && Boolean(snapshot?.trusted_authority);
  const qoderRecoveryOperation = selected?.agent_id === 'agent_qoder_default'
    && !selected.status_error && ['pending', 'needs_attention'].includes(selectedModel?.state ?? '')
    ? selectedModel?.operation_id : undefined;
  async function retrySettingsOperation(operationId: string) {
    if (!selected?.context_id || busy || !snapshot?.trusted_authority) return;
    setBusy(true); setActionError('');
    try {
      await invoke('retry_agent_settings', { input: { schema: 'hiroute.agent-settings-retry/v1', context_id: selected.context_id, operation_id: operationId } });
      await refresh();
    } catch (error) { setActionFailure(error); }
    finally { onMutation(); setBusy(false); }
  }

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
    : editor?.facet === 'collaboration' && !taskSelection
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
    <div className="agents-workspace">
      <div className="agent-tabs segmented" role="tablist">
        <button className={`segment${tab === 'configuration' ? ' active' : ''}`} type="button" role="tab" aria-selected={tab === 'configuration'} onClick={() => { setTab('configuration'); setActionError(''); setNotice(''); }}>{text('我的 Agent', 'My Agents')}</button>
        <button className={`segment${tab === 'tasks' ? ' active' : ''}`} type="button" role="tab" aria-selected={tab === 'tasks'} onClick={() => { setTab('tasks'); setActionError(''); setNotice(''); onOpenTasks?.(); }}>{text('任务记录', 'Task history')}</button>
      </div>
      {!snapshot && !loadError && <div className="empty-state" role="status"><div><span className="oc-spinner" /><p>{text('正在读取本地 Agent…', 'Reading local Agents…')}</p></div></div>}
      {!snapshot && loadError && <div className="empty-state" data-error-code={loadError}><div><span className="empty-icon"><UiIcon name="agent" /></span><h2>{text('暂时无法读取 Agent', 'Agents are temporarily unavailable')}</h2><p>{text('请确认本机服务正在运行，然后重试。', 'Make sure the local service is running, then retry.')}</p><button className="btn btn-primary" type="button" onClick={() => void refresh()}>{text('重试', 'Retry')}</button></div></div>}
      {tab === 'tasks'
        ? <AgentTasks language={language} read={taskRead} initialTaskId={initialTaskId} onCancel={onTaskCancel} onConfirmResidual={onTaskConfirmResidual} onOpenSession={onOpenTaskSession} onBackToAgents={() => setTab('configuration')} onRefresh={onOpenTasks} onLoadMore={onLoadMoreTasks} onReadTask={onReadTask} onLoadTaskResult={onLoadTaskResult} active={active} />
        : snapshot && !snapshot.agents.length
          ? <div className="empty-state"><div><span className="empty-icon"><UiIcon name="agent" /></span><h2>{text('没有发现可接入的 Agent', 'No compatible Agent found')}</h2><p>{text('安装或启动支持的 Agent 后再返回此页。', 'Install or start a supported Agent, then return here.')}</p><button className="btn" type="button" onClick={() => void refresh()}>{text('重新扫描', 'Scan again')}</button></div></div>
          : tab === 'configuration' && <div className={`split-view oc-agents${showAgentList ? '' : ' oc-agent-detail-open'}`}>
            <aside className="master-pane">
              <nav className="master-list native-list" aria-label={text('Agent 列表', 'Agent list')}>
                {snapshot?.agents.map(agent => {
                  const active = (selectedAgent ?? snapshot.agents[0]?.agent_id) === agent.agent_id;
                  const stateNeedsRecovery = needsRecovery(agentModelStatus(agent)?.state) || needsRecovery(agent.settings?.collaboration?.state);
                  const statePending = agentModelStatus(agent)?.state === 'pending' || agent.settings?.collaboration?.state === 'pending';
                  const executableState = executableDiagnostic(agent.configuration_state);
                  const stateLabel = agent.status_error
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
                {!mutable && !selected.codex_access?.pending_operation && <div className="callout warn"><UiIcon name="warning" /><div><strong>{text('Agent 配置可以查看，暂时不能修改', 'Agent settings are viewable but cannot be changed')}</strong><p>{text('本机服务尚未就绪，连接恢复后即可保存配置。', 'The local service is not ready. Reconnect before saving.')}</p></div></div>}
                {selected.codex_access && <CodexAccessPanel access={selected.codex_access} language={language} retryDisabled={busy || Boolean(editor) || !snapshot.trusted_authority} onRetry={async () => {
                  const access = selected.codex_access;
                  if (access?.pending_operation) await retrySettingsOperation(access.pending_operation);
                }} />}
                {selected.status_error && <div className="callout warn" data-error-code={selected.status_error}><UiIcon name="warning" /><div><strong>{text('暂时无法核实 Agent 状态', 'Agent status is temporarily unavailable')}</strong><p>{text('当前仍显示已发现的 Agent；重新读取成功前不会把未知状态当作已接入。', 'The detected Agent remains visible. Unknown status is not treated as connected until it can be read again.')}</p><button className="btn" type="button" onClick={() => void refresh()}>{text('重新读取', 'Try again')}</button></div></div>}
                {executableStatus}
                {selectedHasDrift && <div className="callout warn"><UiIcon name="warning" /><div><strong>{text('接入配置已变化', 'Connection settings changed')}</strong><p>{text('HiRoute 不会覆盖已变化的设置。请在“连接详情与恢复”中恢复需要的路由配置。', 'HiRoute will not overwrite changed settings. Restore the required routing configuration under Connection details and recovery.')}</p></div></div>}
                {qoderRecoveryOperation && <div className="callout warn" role="alert" data-qoder-model-recovery>
                  <UiIcon name="warning" /><div>
                    <strong>{text('模型路由配置尚未完成', 'Model routing configuration is incomplete')}</strong>
                    <p>{text('请先保存自己的编辑，恢复本次操作开始后发生的配置文件改动，再继续原操作。HiRoute 不会强制覆盖文件；任务协作保持独立。', 'Save your edits and undo configuration changes made since this operation started, then resume it. HiRoute will not force an overwrite; task collaboration remains independent.')}</p>
                    <button className="btn" type="button" disabled={busy || Boolean(editor) || !snapshot.trusted_authority} onClick={() => void retrySettingsOperation(qoderRecoveryOperation)}>{text('重新检查并继续原操作', 'Recheck and resume operation')}</button>
                  </div>
                </div>}
                {selectedPending && !qoderRecoveryOperation && !selected.codex_access?.pending_operation && <div className="oc-status-row" role="status"><span className="oc-spinner" /><p>{text('路由配置正在应用，完成后会自动更新。', 'Routing settings are being applied and will update automatically.')}</p></div>}
                {agentSupportsModelRouting(selected.agent_id) && !agentHasNoModelConnection(selected) && <section className="detail-section v3-agent-section">
                  <div className="detail-section-head"><h3>{text('模型路由', 'Model routing')}</h3><button className="btn btn-quiet" data-agent-facet="model" disabled={busy || selectedPending || !selected.context_id || !agentEcosystem(selected.agent_id) || !mutable} onClick={event => open(selected, 'model', event.currentTarget)}>{modelSelection ? text('调整', 'Edit') : text('启用', 'Enable')}</button>{selectedModel?.restore_point_ref && <button className="btn btn-quiet" disabled={busy || selectedPending || !mutable} onClick={event => void change(selected, 'model', true, event.currentTarget)}>{text('停用', 'Disable')}</button>}</div>
                  <p>{modelSelection ? <>{text('当前选择：', 'Current selection: ')}<strong>{selectionSummary}</strong></> : selected.agent_id === 'agent_qoder_default'
                    ? text('添加可在 Qoder 中明确选择的 HiRoute 路由，保留原有模型和默认值。', 'Add HiRoute routes you can explicitly select in Qoder, while keeping native models and defaults.')
                    : text('为此 Agent 配置固定模型，或通过智能路由自动选择模型。', 'Configure a fixed model for this Agent, or use smart routing to choose a model automatically.')}</p>
                  {modelSelection?.mode === 'claude_launcher' && <p className="field-help">{text('直接启动 claude，使用 Opus、Sonnet、Haiku 原生预设选择已映射路由；其他计划不会自动出现在模型菜单中。若未显式选择当前模型，Claude Default 取决于账号；预设映射不证明 Default 可用，请通过真实调用验证。', 'Start claude normally and use its native Opus, Sonnet, or Haiku preset for mapped routes. Other plans do not automatically appear in the model picker. Without an explicit model selection, Claude Default depends on the account; preset mappings do not prove it works. Verify with a live call.')}</p>}
                  {modelSelection?.mode === 'qoder_additional' && <p className="field-help">{text('重新启动 Qoder 后，在模型选择中选用已添加的 HiRoute 路由。原有模型、登录和默认选择保持不变。', 'Restart Qoder and select an added HiRoute route in its model choices. Existing models, sign-in and defaults stay unchanged.')}</p>}
                </section>}
                <section className="detail-section v3-agent-section" data-agent-collaboration>
                  <div className="detail-section-head"><h3>{text('任务路由', 'Task routing')}</h3><button className="btn btn-quiet" data-agent-facet="collaboration" disabled={busy || !selected.context_id || !mutable} onClick={event => open(selected, 'collaboration', event.currentTarget)}>{taskSelection ? text('调整', 'Edit') : text('启用', 'Enable')}</button></div>
                  {selected.agent_id === 'agent_qoder_default' && <div data-qoder-collaboration>
                    <p>{text('使用 Qoder 原有模型和登录配置，通过任务协作使用 HiRoute 计划。需要登录时，请在终端正常启动 Qoder CLI 并完成登录后重试。', 'Keep your Qoder models and sign-in configuration, and use HiRoute plans through task collaboration. If sign-in is needed, start Qoder CLI normally in a terminal, sign in, and retry.')}</p>
                  </div>}
                  <p>{taskSelection?.trigger_mode === 'delegate_by_default'
                    ? text('由此 Agent 判断何时将任务交给执行 Agent；你仍可明确要求只回答、不执行。', 'This Agent decides when to delegate work to an execution Agent; you can still explicitly ask it to answer without execution.')
                    : taskSelection
                      ? text('仅在你明确要求执行或委派任务时，将任务交给执行 Agent。', 'Delegate work to an execution Agent only when you explicitly ask for execution or delegation.')
                      : text('让此 Agent 将任务交给合适的执行 Agent，并获取结果。', 'Let this Agent delegate work to a suitable execution Agent and retrieve the results.')}</p>
                  <p className="field-help">{text('启用后，可在对话中说：“使用 HiRoute，将当前任务委派给【路由名称】执行。”路由名称见“智能路由”，也可让 Agent 选择合适的路由。', 'After enabling, ask in conversation: “Use HiRoute to delegate this task to [route name].” Find route names under Smart routing, or ask the Agent to choose a suitable route.')}</p>
                </section>
                <Disclosure className="native-details" label={text('连接详情与恢复', 'Connection details and recovery')} language={language}><dl>{selected.version && <div><dt>{text('版本', 'Version')}</dt><dd>{selected.version}</dd></div>}{selectedModel?.state === 'configured' && modelSelection && <div><dt>{text('本机接入令牌', 'Local access token')}</dt><dd><input className="input" type="password" value="••••••••••••••••" readOnly aria-label={text('当前令牌已隐藏', 'Current token hidden')} /></dd></div>}</dl>{selectedModel?.restore_point_ref && selected.agent_id === 'agent_codex_default' && selected.codex_access?.selected_mode !== 'profile' && <label className="field-label">{text('恢复时选择原生模型（可选）', 'Native model on restore (optional)')}<input list="codex-native-restore-models" value={restoreNativeModel} onChange={event => { setRestoreNativeModel(event.target.value); setPreview(null); }} placeholder={text('保留原设置', 'Keep original setting')} disabled={busy || !mutable} /><datalist id="codex-native-restore-models">{nativeRestoreOptions.map(model => <option key={model.client_model_id} value={model.client_model_id}>{model.display_name}</option>)}</datalist></label>}{selectedModel?.state === 'configured' && modelSelection && <div className="actions" data-agent-token-controls><button className="btn" disabled={busy || !mutable} onClick={() => { setTokenEditing(true); setTokenDraft(''); setActionError(''); }}>{text('修改令牌', 'Change token')}</button><button className="btn" disabled={busy || !mutable} onClick={event => void changeToken(selected, true, event.currentTarget)}>{text('重新生成', 'Regenerate')}</button></div>}{tokenEditing && selectedModel?.state === 'configured' && modelSelection && <form className="agent-token-form" onSubmit={event => { event.preventDefault(); void changeToken(selected, false, (event.nativeEvent as SubmitEvent).submitter as HTMLElement | null); }}><label className="field"><span className="field-label">{text('新令牌', 'New token')}</span><input className="input" type="password" autoComplete="new-password" value={tokenDraft} onChange={event => setTokenDraft(event.target.value)} minLength={16} maxLength={128} pattern="[A-Za-z0-9._~\\-]{16,128}" required disabled={busy} /><span className="field-help">{text('16–128 位英文字母、数字及 . _ ~ -；保存后原令牌立即失效，无需重新接入。', 'Use 16–128 letters, numbers, or . _ ~ -. The old token stops working after save; reconnecting is unnecessary.')}</span></label><div className="actions"><button className="btn btn-primary" type="submit" disabled={busy || !mutable}>{text('保存令牌', 'Save token')}</button><button className="btn" type="button" disabled={busy} onClick={() => { setTokenEditing(false); setTokenDraft(''); }}>{text('取消', 'Cancel')}</button></div></form>}<div className="actions">{selectedCollaboration?.restore_point_ref && <button className="btn" disabled={busy || !mutable} onClick={event => void change(selected, 'collaboration', true, event.currentTarget)}>{text('停用任务路由', 'Disable task routing')}</button>}</div></Disclosure>
                {!editor && loadError && <div className="callout warn agent-feedback agent-inline-feedback" role="alert" data-error-code={loadError}><UiIcon name="warning" /><div><strong>{text('状态刷新失败', 'Status refresh failed')}</strong><p>{text('当前仍显示上次读取的结果。', 'The last loaded result is still shown.')}</p><button className="btn" type="button" onClick={() => void refresh()}>{text('重试', 'Retry')}</button></div></div>}
                {!editor && blockedPreview}
                {!editor && notice && <div className="toast-stack" aria-live="polite"><div className="toast" role="status"><UiIcon name="check" /><span>{notice}</span></div></div>}
                {!editor && actionError && <div className="callout bad agent-feedback agent-inline-feedback" role="alert" data-error-code={actionError}><UiIcon name="warning" /><span>{actionErrorHint ?? agentActionErrorMessage(actionError, language)}</span></div>}
                {!selected.context_id && <div className="callout"><UiIcon name="info" /><span>{text('此 Agent 的设置入口尚不可用。', 'Settings for this Agent are not available yet.')}</span></div>}
                {editor?.context === selected.context_id && selected.context_id && <Dialog open title={`${agentName(selected)} · ${editor.facet === 'model' ? text('模型路由', 'Model routing') : text('任务路由', 'Task routing')}`} closeLabel={text('关闭配置', 'Close settings')} closeDisabled={busy} onClose={() => { if (!busy) close(); }} footer={<><button className="btn" type="button" disabled={busy} onClick={close}>{text('取消', 'Cancel')}</button><button className="btn btn-primary" type="submit" form="hr-agent-settings" disabled={busy || !mutable || Boolean(formInvalid)}>{busy ? busyText || text('正在保存…', 'Saving…') : submitLabel}</button></>}>
                  <form id="hr-agent-settings" className="agent-form" ref={node => { firstField.current = node?.querySelector<HTMLElement>('input:not([disabled]), select:not([disabled]), button:not([disabled])') ?? null; }} onSubmit={event => { event.preventDefault(); void change(selected, editor.facet, false, (event.nativeEvent as SubmitEvent).submitter as HTMLElement | null); }}>
                    <fieldset disabled={busy || !mutable}>
                      {editor.facet === 'model' && <p className="field-help" data-agent-service-responsibility>{text('使用路由时请保持 HiRoute 运行。启用不会自动设置开机启动。', 'Keep HiRoute running when using routing. Enabling does not set up automatic startup.')}</p>}
                      {editor.facet === 'model' && <AgentModelEditor agent={selected} values={editorValues} plans={plans} language={language} disabled={busy || !mutable} invalid={modelFormInvalid} codexMode={activeCodexMode}
                        onCodexMode={mode => { setCodexMode(mode); setPreview(null); setActionError(''); }}
                        onChange={setEditorValues}
                        onEdited={(clearNotice = true) => { setPreview(null); setActionError(''); if (clearNotice) setNotice(''); }}
                        onCreatePlan={onCreatePlan ? createPlanFromEditor : undefined} />}
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
