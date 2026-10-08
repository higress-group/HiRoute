import type { DecisionIntent, OpenDecisionConnection } from '../features/decision-services/presentation';
import type { DecisionService } from '../features/decision-services/types';
import { DecisionServicesPage } from '../features/decision-services/DecisionServicesPage';
import { requestEditorReplacement } from '../ui/discard-guard';
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Agents } from '../agents';
import type { AgentTask, AgentTaskRead } from '../features/AgentTasks';
import { Sessions } from '../features/Sessions';
import {
  cancelWorkerTask,
  readWorkerTask,
  readWorkerTaskResultPage,
  WorkerTaskPager,
  type WorkerTaskLocator,
} from '../features/worker-task-client';
import {
  Home,
  HomeNavigation,
  visibleData,
  type HomeAction,
  type HomeNavigationItem,
} from '../features/home';
import type { OperationReference } from '../features/model-connections/types';
import {
  PresentationRoot,
  UiIcon,
  WebConfirmationHost,
  syncNativeWindowTheme,
  usePresentationPreferences,
} from '../ui';
import {
  projectHomeOperation,
  type DesktopOperation,
} from './home-projections';
import { ModelManagementPage } from './ModelManagementPage';
import { RoutingPage, type RoutingEditorIntent } from './RoutingPage';
import { SettingsPage } from './SettingsPage';
import { useDesktopHome } from './use-desktop-home';
import { safeDiagnosticCode } from '../error-code';
import { planErrorMessage } from '../plan-editor-errors';
import {
  acceptObservedOperation,
  currentPendingHint,
  isTerminalOperation,
  OPERATION_RESULT_UNVERIFIED,
  operationFeedback,
  pendingFeedbackIdentity,
  shouldObservePendingOperation,
  type OperationPresentation,
} from './operation-feedback';

type Page = 'home' | 'models' | 'routing' | 'agents' | 'sessions' | 'settings';

type ModelIntent = {
  key: string;
  sourceId?: string | null;
  startAdding?: boolean;
  notice?: string;
};

type RoutingIntent = {
  key: string;
  editor?: RoutingEditorIntent | null;
  notice?: string;
};

type AgentIntent = {
  key: string;
  agentId?: string | null;
  facet?: 'model' | 'collaboration';
  tab?: 'configuration' | 'tasks';
  taskId?: string | null;
};

type SessionIntent = { key: string; sessionId?: string | null; requestId?: string | null; returnToQuality?: boolean };
const OBSERVATION_GRACE_MS = 3_000;

function failureCode(error: unknown): string {
  return safeDiagnosticCode(error, 'CLIENT_ERROR');
}

function failureHelp(code: string, language: 'zh' | 'en'): string {
  if (code === 'QODER_MODEL_BUDGET_CONFLICT') return planErrorMessage(code, language);
  const messages: Record<string, [string, string]> = {
    CONFIRMATION_EXPIRED: ['确认已过期，请重新预览。输入已保留。', 'Confirmation expired. Preview again; your input is retained.'],
    CONFIRMATION_STALE: ['确认已失效，请重新预览。', 'This confirmation is no longer valid. Preview again.'],
    REVISION_CONFLICT: ['方案已经变化，请读取当前状态后重新预览。', 'The plan changed. Refresh its current state and preview again.'],
    CHANGE_PREVIEW_STALE: ['预览已过期，请重新预览当前方案。', 'The preview is stale. Preview the current plan again.'],
    LATEST_EDIT_NOT_APPLIED: ['已恢复原操作，最新编辑尚未应用。请先核对原操作结果。', 'The original operation was restored. Check its result before applying the latest edit.'],
    OPERATION_IN_PROGRESS: ['这次保存仍在进行，完成后即可再次保存。', 'This save is still running. Save again after it finishes.'],
    TRUSTED_AUTHORITY_UNAVAILABLE: ['此连接仅支持查看，无法授权变更。', 'This connection supports viewing but cannot authorize a change.'],
    IDEMPOTENCY_KEY_REUSED: ['上一次保存仍在处理中。请先查看其结果，最新编辑尚未应用。', 'The previous save is still being processed. Check its result first; the latest edit was not applied.'],
  };
  return messages[code]?.[language === 'zh' ? 0 : 1]
    ?? (language === 'zh' ? '操作暂未完成，请稍后重试。' : 'The action could not complete. Try again shortly.');
}

export function DesktopApp() {
  const preferences = usePresentationPreferences();
  const { language } = preferences;
  const home = useDesktopHome();
  const [page, setPage] = useState<Page>('home');
  const [modelTab, setModelTab] = useState<'general' | 'decisions'>('general');
  const [decisionIntent, setDecisionIntent] = useState<DecisionIntent | null>(null);
  const decisionReturn = useRef<((service: DecisionService) => void) | null>(null);
  const [returningToRoute, setReturningToRoute] = useState(false);
  useEffect(() => { window.scrollTo({ top: 0, left: 0, behavior: 'instant' }); }, [page]);
  const [visited, setVisited] = useState<Set<Page>>(() => new Set(['home']));
  const [refreshing, setRefreshing] = useState(false);
  const [refreshVersion, setRefreshVersion] = useState(0);
  const [operation, setOperation] = useState<DesktopOperation | null>(null);
  const [operationPresentation, setOperationPresentation] = useState<OperationPresentation>({ kind: 'background' });
  const [observing, setObserving] = useState(true);
  const [operationRecoveryPending, setOperationRecoveryPending] = useState(false);
  const [pollingRevision, setPollingRevision] = useState(0);
  const [operationError, setOperationError] = useState('');
  const [dismissedOperationId, setDismissedOperationId] = useState<string | null>(null);
  const [notice, setNotice] = useState('');
  const [modelIntent, setModelIntent] = useState<ModelIntent>({ key: 'models/default' });
  const [routingIntent, setRoutingIntent] = useState<RoutingIntent>({ key: 'routing/default' });
  const [agentIntent, setAgentIntent] = useState<AgentIntent>({ key: 'agents/default' });
  const [taskRead, setTaskRead] = useState<AgentTaskRead>({ status: 'unavailable' });
  const taskPager = useRef<WorkerTaskPager | null>(null);
  const taskReadGeneration = useRef(0);
  const [sessionIntent, setSessionIntent] = useState<SessionIntent>({ key: 'sessions/default' });
  const pollingGeneration = useRef(0);
  const supersededPendingKey = useRef<string | null>(null);

  const text = language === 'zh'
    ? {
        pages: { home: '首页', models: '模型', routing: '智能路由', agents: 'Agent', sessions: '会话', settings: '设置' },
        settings: '设置', refresh: '刷新实际状态', close: '关闭',
        serviceReady: '本机服务可用', serviceReadOnly: '只读连接', serviceUnavailable: '本机服务不可用', serviceUnknown: '状态待核实',
        tasksUnavailable: '任务记录暂时无法读取，请确认本机服务后重试。',
        targetMissing: '原目标已不在当前实际快照中，请刷新后重试。',
        operationMissing: '当前没有与该标识匹配的可观察操作。',
        nativeThemeError: '内容主题已切换，但原生窗口外观暂时无法同步。',
      }
    : {
        pages: { home: 'Home', models: 'Models', routing: 'Smart routing', agents: 'Agent', sessions: 'Sessions', settings: 'Settings' },
        settings: 'Settings', refresh: 'Refresh actual state', close: 'Close',
        serviceReady: 'Local service ready', serviceReadOnly: 'Read-only connection', serviceUnavailable: 'Local service unavailable', serviceUnknown: 'Status unverified',
        tasksUnavailable: 'Task history is temporarily unavailable. Check the local service and try again.',
        targetMissing: 'The original target is no longer present in the current snapshot. Refresh and try again.',
        operationMissing: 'No observable operation matches that identifier.',
        nativeThemeError: 'The content theme changed, but the native window appearance could not be synchronized.',
      };

  const navigation: HomeNavigationItem[] = [
    { id: 'home', label: text.pages.home, icon: 'home' },
    { id: 'models', label: text.pages.models, icon: 'models' },
    { id: 'routing', label: text.pages.routing, icon: 'route' },
    { id: 'agents', label: text.pages.agents, icon: 'agent' },
    { id: 'sessions', label: text.pages.sessions, icon: 'sessions' },
  ];

  const service = visibleData(home.reads.service);
  const startupFailure = home.startup?.state === 'failed' ? home.startup : null;
  const upgradePhase = home.startup?.state === 'starting' ? home.startup.upgrade_phase : undefined;
  const upgradePhaseText = upgradePhase ? ({
    source_check: language === 'zh' ? '正在校验存储格式' : 'Checking storage format',
    backup: language === 'zh' ? '正在保存升级前的完整备份' : 'Saving the complete pre-upgrade backup',
    conversion: language === 'zh' ? '正在升级本机数据' : 'Upgrading local data',
    validation: language === 'zh' ? '正在校验升级结果' : 'Verifying upgraded data',
    service_recovery: language === 'zh' ? '正在恢复本机服务' : 'Restoring the local service',
  })[upgradePhase] : undefined;
  const serviceLabel = service?.daemon === 'read_only'
    ? text.serviceReadOnly
    : service?.daemon === 'unavailable'
      ? text.serviceUnavailable
      : service?.daemon === 'running' && ['ready', 'empty', 'no_new_calls'].includes(service.gateway)
        ? text.serviceReady
        : text.serviceUnknown;
  const serviceReady = service?.daemon === 'running' && service.gateway === 'ready';
  const serviceOperational = service?.daemon === 'running'
    && Boolean(service.recoveryReady)
    && ['ready', 'empty', 'no_new_calls'].includes(service.gateway);
  const navigationLabel = serviceOperational
      ? language === 'zh' ? '仅在本机运行' : 'Running locally'
      : serviceLabel;
  const settingsServiceLabel = serviceOperational
    ? language === 'zh' ? '可用' : 'Available'
    : serviceLabel;

  useEffect(() => {
    document.documentElement.lang = language === 'zh' ? 'zh-CN' : 'en';
  }, [language]);

  useEffect(() => {
    let current = true;
    void syncNativeWindowTheme(preferences.theme, preferences.resolvedTheme).catch(() => {
      if (current) setNotice(text.nativeThemeError);
    });
    return () => { current = false; };
  }, [preferences.theme, preferences.resolvedTheme, text.nativeThemeError]);

  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(''), 4200);
    return () => window.clearTimeout(timer);
  }, [notice]);

  async function refreshAll() {
    window.dispatchEvent(new Event('hiroute-content-invalidated'));
    setRefreshing(true);
    setOperationError('');
    try {
      await home.refreshAll();
      setRefreshVersion(v => v + 1);
    } finally {
      setRefreshing(false);
    }
  }

  useEffect(() => {
    const refreshOnFocus = () => {
      if (!document.hidden) void home.refreshAll();
    };
    window.addEventListener('focus', refreshOnFocus);
    return () => window.removeEventListener('focus', refreshOnFocus);
  }, [home.refreshAll]);

  useEffect(() => {
    const generation = ++pollingGeneration.current;
    if (!observing || (!home.desktopSnapshot?.pending && !operation && !operationRecoveryPending)) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let delay = 500;
    let unconfirmedSince: number | null = null;
    function noteUnconfirmed(code: string) {
      const now = performance.now();
      unconfirmedSince ??= now;
      setOperationError(now - unconfirmedSince >= OBSERVATION_GRACE_MS ? code : '');
    }
    async function poll() {
      if (pollingGeneration.current !== generation) return;
      if (document.hidden) {
        timer = setTimeout(poll, 2000);
        return;
      }
      try {
        const value = await invoke<DesktopOperation | null>('observe_operation');
        if (pollingGeneration.current !== generation) return;
        if (operation && value && operation.operation_id !== value.operation_id) {
          noteUnconfirmed('OPERATION_IDENTITY_MISMATCH');
        } else {
          if (value) {
            unconfirmedSince = null;
            setOperationError('');
            setOperation(current => acceptObservedOperation(current, value));
            setOperationRecoveryPending(false);
          } else {
            noteUnconfirmed(OPERATION_RESULT_UNVERIFIED);
          }
          if (value && isTerminalOperation(value.state)) {
            await home.refreshAll();
            return;
          }
        }
        delay = Math.min(2000, delay + 250);
      } catch (error) {
        if (pollingGeneration.current === generation) {
          noteUnconfirmed(failureCode(error));
          delay = Math.min(10000, delay * 2);
        }
      }
      if (pollingGeneration.current === generation) timer = setTimeout(poll, delay);
    }
    if (!operation || !isTerminalOperation(operation.state)) timer = setTimeout(poll, delay);
    return () => {
      pollingGeneration.current++;
      if (timer) clearTimeout(timer);
    };
  }, [
    Boolean(home.desktopSnapshot?.pending),
    home.desktopSnapshot?.pending?.operation_id,
    home.desktopSnapshot?.pending?.idempotency_key,
    home.refreshAll,
    observing,
    operation?.operation_id,
    operation?.state,
    operationRecoveryPending,
    pollingRevision,
  ]);

  useEffect(() => {
    if (!shouldObservePendingOperation(operation, home.desktopSnapshot?.pending ?? null, supersededPendingKey.current)) return;
    pollingGeneration.current++;
    setPollingRevision(current => current + 1);
    setOperation(null);
    setOperationRecoveryPending(true);
    setOperationPresentation({ kind: 'background' });
    setDismissedOperationId(null);
    setObserving(true);
    setOperationError('');
  }, [home.desktopSnapshot?.pending?.idempotency_key, home.desktopSnapshot?.pending?.operation_id, operation]);

  async function navigate(id: string) {
    if (!navigation.some(item => item.id === id)) return;
    setNotice('');
    await openPageSafely(id as Page);
  }

  function openPage(next: Page) {
    if (next === 'home' && page !== 'home') void home.refreshActivity();
    setVisited(current => current.has(next) ? current : new Set([...current, next]));
    setPage(next);
    if (next !== 'models') { decisionReturn.current = null; setReturningToRoute(false); }
  }

  function editorScope(value: Page): 'decisions' | 'models' | 'routing' | 'agents' | null {
    return value === 'models' ? modelTab === 'decisions' ? 'decisions' : 'models' : value === 'routing' || value === 'agents' ? value : null;
  }

  async function allowPageChange(next: Page, replaceTarget = false): Promise<boolean> {
    const currentScope = editorScope(page);
    if (currentScope && (page !== next || replaceTarget) && !await requestEditorReplacement(currentScope)) return false;
    // Adding a connection suspends the route editor. Other navigation must still
    // resolve that retained draft before replacing or abandoning it.
    if (decisionReturn.current && next !== 'routing' && (next !== 'models' || replaceTarget)
      && !await requestEditorReplacement('routing')) return false;
    const targetScope = editorScope(next);
    if (replaceTarget && targetScope && targetScope !== currentScope && !await requestEditorReplacement(targetScope)) return false;
    return true;
  }

  async function openPageSafely(next: Page, replaceTarget = false): Promise<boolean> {
    if (!await allowPageChange(next, replaceTarget)) return false;
    openPage(next);
    return true;
  }

  async function switchModelTab(next: 'general' | 'decisions') {
    if (next === modelTab || !await requestEditorReplacement(modelTab === 'general' ? 'models' : 'decisions')) return;
    setModelTab(next);
  }

  const addDecisionForRoute: OpenDecisionConnection = async (kind, select) => {
    if (!await requestEditorReplacement('decisions')) return;
    decisionReturn.current = select;
    setReturningToRoute(true);
    setDecisionIntent({ key: crypto.randomUUID(), kind });
    setModelTab('decisions');
    openPage('models');
  };

  function useDecisionInRoute(service: DecisionService) {
    decisionReturn.current?.(structuredClone(service));
    openPage('routing');
  }

  const modelTabs = <nav className="model-category-tabs" aria-label={language === 'zh' ? '模型类型' : 'Model categories'}>
    <button type="button" aria-pressed={modelTab === 'general'} onClick={() => void switchModelTab('general')}>{language === 'zh' ? '通用模型' : 'General models'}</button>
    <button type="button" aria-pressed={modelTab === 'decisions'} onClick={() => void switchModelTab('decisions')}>{language === 'zh' ? '决策模型' : 'Decision models'}</button>
  </nav>;

  function acceptOperation(
    reference: OperationReference | DesktopOperation | null,
    presentation: OperationPresentation = { kind: 'background' },
  ) {
    if (!reference) return;
    supersededPendingKey.current = home.desktopSnapshot?.pending?.idempotency_key ?? null;
    pollingGeneration.current++;
    setPollingRevision(current => current + 1);
    setOperation({ ...reference, safe_error_code: 'safe_error_code' in reference ? reference.safe_error_code : null });
    setOperationRecoveryPending(false);
    setOperationPresentation(presentation);
    setDismissedOperationId(null);
    setObserving(true);
    setOperationError('');
  }

  function acceptUnverifiedOperation(
    presentation: OperationPresentation = { kind: 'background' },
  ) {
    supersededPendingKey.current = home.desktopSnapshot?.pending?.idempotency_key ?? null;
    pollingGeneration.current++;
    setPollingRevision(current => current + 1);
    setOperation(null);
    setOperationRecoveryPending(true);
    setOperationPresentation(presentation);
    setDismissedOperationId(null);
    setObserving(true);
    setOperationError('');
  }

  function dismissOperation() {
    if (feedbackId) setDismissedOperationId(feedbackId);
    setOperationError('');
  }

  function retryOperationObservation() {
    pollingGeneration.current++;
    setPollingRevision(current => current + 1);
    setObserving(true);
    setOperationError('');
  }

  async function showModels(intent: Omit<ModelIntent, 'key'> = {}) {
    if (!await allowPageChange('models', true)) return;
    setNotice('');
    if (modelTab === 'decisions' && !await requestEditorReplacement('models')) return;
    setModelTab('general');
    setModelIntent({ key: `models/${crypto.randomUUID()}`, ...intent });
    openPage('models');
  }

  async function showRouting(editor?: RoutingEditorIntent | null, routingNotice?: string) {
    if (!await allowPageChange('routing', true)) return;
    setNotice('');
    setRoutingIntent({ key: `routing/${crypto.randomUUID()}`, editor, notice: routingNotice });
    openPage('routing');
  }

  function taskPresentation(planId: string) {
    const plan = home.desktopSnapshot?.catalog.plans.find(item => item.agent_plan_id === planId);
    const routeName = plan?.desired.display_name ?? (language === 'zh' ? '原 Agent 路由' : 'Original Agent route');
    return {
      unnamedTask: language === 'zh' ? '任务' : 'Task',
      unavailableBrief: language === 'zh' ? '当前服务未返回这项任务的正文。' : 'The service did not return this task’s content.',
      routeName,
    };
  }

  async function loadTaskHistory(reset = true) {
    if (reset || !taskPager.current) {
      taskReadGeneration.current += 1;
      taskPager.current = new WorkerTaskPager(taskPresentation);
      setTaskRead({ status: 'loading' });
    }
    const generation = taskReadGeneration.current;
    const pager = taskPager.current;
    if (!pager) throw new Error('TASK_LIST_UNAVAILABLE');
    try {
      const page = await pager.next(50);
      if (generation !== taskReadGeneration.current || pager !== taskPager.current) {
        throw new Error('TASK_LIST_READ_STALE');
      }
      if (reset) {
        setTaskRead({
          status: 'ready',
          tasks: page.tasks,
          hasMore: page.hasMore,
        });
      }
      return { tasks: page.tasks, hasMore: page.hasMore };
    } catch (error) {
      if (reset && generation === taskReadGeneration.current && pager === taskPager.current) {
        setTaskRead({
          status: 'error',
          message: failureHelp(failureCode(error), language),
        });
      }
      throw error;
    }
  }

  useEffect(() => {
    if (!serviceOperational) return;
    void loadTaskHistory(true).catch(() => {});
  }, [serviceOperational, refreshVersion]);

  async function hydrateTask(task: AgentTask) {
    return readWorkerTask(
      { taskId: task.taskId },
      taskPresentation,
      undefined,
      task,
    );
  }

  async function openKnownTask(taskId: string, runId?: string) {
    if (!await allowPageChange('agents', true)) return;
    setNotice('');
    const generation = ++taskReadGeneration.current;
    taskPager.current = null;
    setTaskRead({ status: 'loading' });
    setAgentIntent({ key: `agents/${crypto.randomUUID()}`, tab: 'tasks', taskId });
    openPage('agents');
    const locator: WorkerTaskLocator = { taskId, runId };
    try {
      const task = await readWorkerTask(locator, taskPresentation);
      if (generation === taskReadGeneration.current) setTaskRead({ status: 'ready', tasks: [task] });
    } catch (error) {
      if (generation !== taskReadGeneration.current) return;
      setTaskRead({
        status: 'error',
        message: failureHelp(failureCode(error), language),
      });
    }
  }

  async function handleHomeAction(action: HomeAction) {
    if (action.kind === 'retry-read') {
      void home.refreshDomain(action.domain);
      return;
    }
    if (action.kind === 'connect-models') {
      showModels({ startAdding: true });
      return;
    }
    if (action.kind === 'view-models') {
      showModels();
      return;
    }
    if (action.kind === 'view-routing') {
      showRouting();
      return;
    }
    if (action.kind === 'view-agents') {
      if (!await allowPageChange('agents', true)) return;
      setAgentIntent({ key: `agents/${crypto.randomUUID()}` });
      openPage('agents');
      return;
    }
    if (action.kind === 'create-plan') {
      showRouting({ key: crypto.randomUUID() });
      return;
    }
    if (action.kind === 'configure-agent') {
      if (!await allowPageChange('agents', true)) return;
      setNotice('');
      setAgentIntent({
        key: `agents/${crypto.randomUUID()}`,
        agentId: action.agentId,
        facet: action.facet ?? 'model',
      });
      openPage('agents');
      return;
    }
    if (action.kind === 'open-source') {
      showModels({ sourceId: action.sourceId });
      return;
    }
    if (action.kind === 'open-plan') {
      const plan = home.desktopSnapshot?.catalog.plans.find(item => item.agent_plan_id === action.planId);
      if (plan) showRouting({ key: plan.agent_plan_id, plan });
      else setNotice(text.targetMissing);
      return;
    }
    if (action.kind === 'open-draft') {
      const draft = home.desktopSnapshot?.catalog.drafts.find(item => item.draft_id === action.draftId);
      const plan = draft?.plan_id
        ? home.desktopSnapshot?.catalog.plans.find(item => item.agent_plan_id === draft.plan_id)
        : undefined;
      if (draft) showRouting({ key: draft.draft_id, draft, plan });
      else setNotice(text.targetMissing);
      return;
    }
    if (action.kind === 'open-session') {
      if (!await allowPageChange('sessions')) return;
      setNotice('');
      setSessionIntent({ key: `sessions/${crypto.randomUUID()}`, sessionId: action.sessionId });
      openPage('sessions');
      return;
    }
    if (action.kind === 'view-all-sessions') {
      if (!await allowPageChange('sessions')) return;
      setNotice('');
      setSessionIntent({ key: `sessions/${crypto.randomUUID()}`, sessionId: null });
      openPage('sessions');
      return;
    }
    if (action.kind === 'open-task') {
      void openKnownTask(action.taskId, action.runId);
      return;
    }
    if (action.kind === 'view-all-tasks') {
      if (!await allowPageChange('agents', true)) return;
      setNotice('');
      setAgentIntent({ key: `agents/${crypto.randomUUID()}`, tab: 'tasks' });
      openPage('agents');
      void loadTaskHistory(true);
      return;
    }
    const currentId = operation?.operation_id ?? home.desktopSnapshot?.pending?.operation_id;
    setNotice(currentId === action.operationId ? '' : text.operationMissing);
  }

  const homeOperation = projectHomeOperation(operation);
  const snapshotPending = home.desktopSnapshot?.pending;
  const pendingRecovery = currentPendingHint(snapshotPending, supersededPendingKey.current);
  const pendingId = pendingFeedbackIdentity(
    operation,
    pendingRecovery?.operation_id,
  );
  const feedbackId = pendingId ?? (operationRecoveryPending || pendingRecovery ? 'pending/unknown' : null);
  const presentedOperationError = operationError;
  const operationView = operationFeedback(operation, presentedOperationError, language, operationPresentation);
  useEffect(() => {
    if (!feedbackId || operationView.phase !== 'succeeded') return;
    const timeout = window.setTimeout(() => setDismissedOperationId(feedbackId), 4500);
    return () => window.clearTimeout(timeout);
  }, [operationView.phase, feedbackId]);
  return <PresentationRoot
    language={language}
    theme={preferences.resolvedTheme}
    textScale={preferences.textScale}
  >
    <div className="app-window">
      <header className="titlebar" data-tauri-drag-region="deep" onMouseDown={event => {
        if (event.button !== 0 || event.detail !== 2) return;
        event.preventDefault();
        event.stopPropagation();
        void invoke('perform_titlebar_double_click');
      }} onMouseUp={event => {
        if (event.button === 0 && event.detail === 2) event.stopPropagation();
      }}>
        <div className="window-controls" data-tauri-drag-region aria-hidden="true" />
        <div className="titlebar-main" data-tauri-drag-region>
          <div className="window-title"><span>{text.pages[page]}</span></div>
        </div>
      </header>
      <HomeNavigation
        language={language}
        items={navigation}
        current={page}
        serviceLabel={navigationLabel}
        serviceReady={serviceOperational}
        onNavigate={id => void navigate(id)}
        onOpenSettings={() => void openPageSafely('settings')}
      />
      <main className="main">
        {notice && <div className="toast-stack" aria-live="polite"><div className="toast" role="status"><UiIcon name="info" /><span>{notice}</span></div></div>}
        {upgradePhaseText && <div className="callout" role="status" aria-live="polite"><UiIcon name="info" /><div><strong>{upgradePhaseText}</strong><p>{language === 'zh' ? '请等待升级完成；请保留原数据与完整安装包。' : 'Wait for the upgrade to finish. Keep the original data and complete installation package.'}</p></div></div>}
        {startupFailure && <div className="callout bad" role="alert" data-error-code={startupFailure.code}>
          <UiIcon name="warning" />
          <div>
            <strong>{language === 'zh' ? '本机服务启动失败' : 'Local service failed to start'}</strong>
            <p>{startupFailure.code === 'DAEMON_UPGRADE_SOURCE_UNSUPPORTED'
                ? (language === 'zh' ? '此数据格式尚无经过验证的升级与恢复路径。请保留原数据并使用对应旧版本，勿清空目录重试。' : 'This data format has no verified upgrade and recovery path. Keep the data and use its matching previous version.')
              : startupFailure.backup_directory
              ? (language === 'zh' ? '请保留当前数据目录，按升级备份中的《恢复说明.md》恢复整组数据，并覆盖安装说明指定的旧版本。恢复会舍弃备份之后的新数据。' : 'Keep the current data directory. Follow the recovery guide in the upgrade backup to restore the complete data set and install the specified previous version. Changes made after the backup will be lost.')
              : (language === 'zh' ? `启动阶段失败（${startupFailure.code ?? 'STARTUP_FAILED'}）。请保留现有数据，打开恢复目录查看日志并备份。` : `Startup failed (${startupFailure.code ?? 'STARTUP_FAILED'}). Keep the existing data; open the recovery directory to inspect logs and back it up.`)}</p>
            {startupFailure.backup_directory && <p>{language === 'zh' ? '升级备份：' : 'Upgrade backup: '}{startupFailure.backup_directory}</p>}
            {startupFailure.recovery_available && <button className="btn" type="button" onClick={() => {
              void invoke('open_startup_recovery_directory').catch(() => setNotice(language === 'zh' ? '无法安全打开恢复目录，请检查目录权限后重试。' : 'The recovery directory could not be opened safely. Check its permissions and try again.'));
            }}>{startupFailure.backup_directory ? (language === 'zh' ? '打开升级备份与恢复说明' : 'Open upgrade backup and recovery guide') : (language === 'zh' ? '打开恢复目录' : 'Open recovery directory')}</button>}
          </div>
        </div>}

        <div hidden={page !== 'home'}><Home
            language={language}
            reads={home.reads}
            operation={homeOperation}
            hasTasks={taskRead.status === 'ready' && taskRead.tasks.length > 0}
            onAction={handleHomeAction}
          /></div>
        {visited.has('models') && <div hidden={page !== 'models' || modelTab !== 'general'}><ModelManagementPage
            key={modelIntent.key}
            language={language}
            active={page === 'models' && modelTab === 'general'}
            tabs={modelTabs}
            trustedAuthority={Boolean(home.desktopSnapshot?.trusted_authority && home.desktopSnapshot.service.mutation_available)}
            refreshVersion={refreshVersion}
            initialSourceId={modelIntent.sourceId}
            startAdding={modelIntent.startAdding}
            notice={modelIntent.notice}
            onOperation={value => acceptOperation(value, { kind: 'model-connection' })}
            onRecoveryRefresh={() => void home.refreshAll()}
            onChanged={() => void Promise.allSettled([home.refreshCompute(), home.refreshDesktop()])}
            plans={home.desktopSnapshot?.catalog.plans}
            agents={home.agentSnapshot?.agents}
            onOpenPlan={planId => {
              const plan = home.desktopSnapshot?.catalog.plans.find(item => item.agent_plan_id === planId);
              if (plan) void showRouting({ key: plan.agent_plan_id, plan });
            }}
            onCreatePlan={bindingId => void showRouting({ key: `routing/${crypto.randomUUID()}`, initialBindingId: bindingId })}
          /></div>}
        {visited.has('models') && <div hidden={page !== 'models' || modelTab !== 'decisions'}><DecisionServicesPage
          language={language} active={page === 'models' && modelTab === 'decisions'}
          mutable={!!home.desktopSnapshot?.trusted_authority && !!home.desktopSnapshot?.service.mutation_available}
          tabs={modelTabs} intent={decisionIntent}
          plans={home.desktopSnapshot?.catalog.plans} drafts={home.desktopSnapshot?.catalog.drafts}
          onChanged={() => void home.refreshDesktop()}
          onOpenRoute={(plan, draft) => void showRouting({ key: draft?.draft_id ?? plan!.agent_plan_id, plan, draft })}
          onReturn={returningToRoute ? () => void openPageSafely('routing') : undefined}
          onUse={returningToRoute ? useDecisionInRoute : undefined}
        /></div>}
        {visited.has('routing') && <div hidden={page !== 'routing'}><RoutingPage
            key={routingIntent.key}
            language={language}
            active={page === 'routing'}
            refreshVersion={refreshVersion}
            snapshot={home.desktopSnapshot}
            agentSnapshot={home.agentSnapshot}
            operation={operation}
            loading={home.reads.plans.status === 'loading'}
            busy={refreshing}
            onOpenServices={addDecisionForRoute}
            initialEditor={routingIntent.editor}
            notice={routingIntent.notice}
            onRefresh={async () => { setObserving(true); await home.refreshDesktop(); setRefreshVersion(v => v + 1); }}
            onOpenAgent={agentId => { void (async () => { if (!await allowPageChange('agents')) return; setAgentIntent({ key: `agents/${crypto.randomUUID()}`, agentId }); openPage('agents'); })(); }}
            onOpenSession={(sessionId, requestId) => { void (async () => { if (!await allowPageChange('sessions')) return; setSessionIntent({ key: `sessions/${crypto.randomUUID()}`, sessionId, requestId, returnToQuality: true }); openPage('sessions'); })(); }}
            onOperation={value => acceptOperation(value, { kind: 'routing' })}
          /></div>}
        {visited.has('agents') && <div hidden={page !== 'agents'}><Agents
            key={agentIntent.key}
            language={language}
            active={page === 'agents'}
            operation={operation}
            initialAgentId={agentIntent.agentId}
            initialFacet={agentIntent.facet}
            initialTab={agentIntent.tab}
            initialTaskId={agentIntent.taskId}
            taskRead={taskRead}
            refreshVersion={refreshVersion}
            mutationAllowed={Boolean(home.desktopSnapshot?.trusted_authority && home.desktopSnapshot.service.mutation_available)}
            onMutation={() => {
              setObserving(true);
              void Promise.allSettled([home.refreshAgents(), home.refreshDesktop()]);
            }}
            onOperation={acceptOperation}
            onUnverifiedOperation={acceptUnverifiedOperation}
            onTaskCancel={async (task: AgentTask, onAccepted) => cancelWorkerTask(task, onAccepted)}
            onOpenTasks={() => void loadTaskHistory(true)}
            onLoadMoreTasks={() => loadTaskHistory(false)}
            onReadTask={hydrateTask}
            onLoadTaskResult={readWorkerTaskResultPage}
            onCreatePlan={() => void showRouting({ key: `routing/${crypto.randomUUID()}` })}
            onOpenTaskSession={sessionId => {
              void (async () => {
                if (!await allowPageChange('sessions')) return;
                setSessionIntent({ key: `sessions/${crypto.randomUUID()}`, sessionId });
                openPage('sessions');
              })();
            }}
          /></div>}
        {visited.has('sessions') && <div hidden={page !== 'sessions'}><Sessions
            key={sessionIntent.key}
            active={page === 'sessions'}
            initialSession={sessionIntent.sessionId ?? null}
            initialRequest={sessionIntent.requestId ?? null}
            onReturnToQuality={sessionIntent.returnToQuality ? () => void openPageSafely('routing') : undefined}
            refreshVersion={refreshVersion}
            language={language}
            onOpenAgents={() => void openPageSafely('agents')}
          /></div>}
        <div hidden={page !== 'settings'}><SettingsPage
          active={page === 'settings'}
          language={language}
          languagePreference={preferences.languagePreference}
          theme={preferences.theme}
          textScale={preferences.textScale}
          serviceLabel={settingsServiceLabel}
          serviceReady={serviceOperational}
          onLanguageChange={preferences.setLanguage}
          onThemeChange={preferences.setTheme}
          onTextScaleChange={preferences.setTextScale}
          onOpenSessions={() => {
            void (async () => {
              if (!await allowPageChange('sessions')) return;
              setSessionIntent({ key: `sessions/${crypto.randomUUID()}`, sessionId: null });
              openPage('sessions');
            })();
          }}
        /></div>

        {feedbackId && feedbackId !== dismissedOperationId && <article className={`operation-status surface ${operationView.phase}`} aria-live={operationView.phase === 'failed' ? 'assertive' : 'polite'} data-operation-state={operation?.state ?? 'unknown'} data-operation-phase={operationView.phase} data-observation-error-code={presentedOperationError || undefined}>
          <div className="operation-status-head">
            <strong>{['pending', 'unverified'].includes(operationView.phase) && <span className="oc-spinner" aria-hidden="true" />}{operationView.title}</strong>
            <div className="operation-status-actions">
              {operationView.phase === 'unverified' && <button className="btn btn-quiet" type="button" onClick={retryOperationObservation}>{language === 'zh' ? '重新查询' : 'Check again'}</button>}
              <button className="icon-btn" type="button" aria-label={text.close} onClick={dismissOperation}><UiIcon name="close" /></button>
            </div>
          </div>
          <p>{operationView.detail}</p>
          {operation?.safe_error_code && <div className="callout bad" role="alert" data-error-code={operation.safe_error_code}><UiIcon name="warning" /><span>{failureHelp(operation.safe_error_code, language)}</span></div>}
        </article>}
      </main>
    </div>
    <WebConfirmationHost />
  </PresentationRoot>;
}
