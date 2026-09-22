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

type SessionIntent = { key: string; sessionId?: string | null; requestId?: string | null };
const OBSERVATION_GRACE_MS = 3_000;

function failureCode(error: unknown): string {
  return safeDiagnosticCode(error, 'CLIENT_ERROR');
}

function failureHelp(code: string, language: 'zh' | 'en'): string {
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
  }

  function editorScope(value: Page): 'models' | 'routing' | 'agents' | null {
    return value === 'models' || value === 'routing' || value === 'agents' ? value : null;
  }

  async function allowPageChange(next: Page, replaceTarget = false): Promise<boolean> {
    const currentScope = editorScope(page);
    if (currentScope && (page !== next || replaceTarget) && !await requestEditorReplacement(currentScope)) return false;
    const targetScope = editorScope(next);
    if (replaceTarget && targetScope && targetScope !== currentScope && !await requestEditorReplacement(targetScope)) return false;
    return true;
  }

  async function openPageSafely(next: Page, replaceTarget = false): Promise<boolean> {
    if (!await allowPageChange(next, replaceTarget)) return false;
    openPage(next);
    return true;
  }

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
        {startupFailure && <div className="callout bad" role="alert" data-error-code={startupFailure.code}>
          <UiIcon name="warning" />
          <div>
            <strong>{language === 'zh' ? '本机服务启动失败' : 'Local service failed to start'}</strong>
            <p>{startupFailure.code === 'DAEMON_STORAGE_UNREADABLE'
              ? (language === 'zh' ? 'HiRoute 数据无法读取或已损坏。原数据未被清空或迁移；请先备份恢复目录，再同时移走其中的 storage 和 gateway.lkg 后重启。' : 'HiRoute data is unreadable or damaged. It was not cleared or migrated. Back up the recovery directory, then move both storage and gateway.lkg aside before restarting.')
              : (language === 'zh' ? `启动阶段失败（${startupFailure.code ?? 'STARTUP_FAILED'}）。原数据保持不变，可打开恢复目录查看日志并备份。` : `Startup failed (${startupFailure.code ?? 'STARTUP_FAILED'}). Existing data is unchanged; open the recovery directory to inspect logs and back it up.`)}</p>
            {startupFailure.recovery_available && <button className="btn" type="button" onClick={() => {
              void invoke('open_startup_recovery_directory').catch(() => setNotice(language === 'zh' ? '无法安全打开恢复目录，请检查目录权限后重试。' : 'The recovery directory could not be opened safely. Check its permissions and try again.'));
            }}>{language === 'zh' ? '打开恢复目录' : 'Open recovery directory'}</button>}
          </div>
        </div>}

        <div hidden={page !== 'home'}><Home
            language={language}
            reads={home.reads}
            operation={homeOperation}
            hasTasks={taskRead.status === 'ready' && taskRead.tasks.length > 0}
            onAction={handleHomeAction}
          /></div>
        {visited.has('models') && <div hidden={page !== 'models'}><ModelManagementPage
            key={modelIntent.key}
            language={language}
            active={page === 'models'}
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
        {visited.has('routing') && <div hidden={page !== 'routing'}><RoutingPage
            key={routingIntent.key}
            language={language}
            active={page === 'routing'}
            snapshot={home.desktopSnapshot}
            agentSnapshot={home.agentSnapshot}
            operation={operation}
            loading={home.reads.plans.status === 'loading'}
            busy={refreshing}
            initialEditor={routingIntent.editor}
            notice={routingIntent.notice}
            onRefresh={async () => { setObserving(true); await home.refreshDesktop(); setRefreshVersion(v => v + 1); }}
            onOpenAgent={agentId => { void (async () => { if (!await allowPageChange('agents')) return; setAgentIntent({ key: `agents/${crypto.randomUUID()}`, agentId }); openPage('agents'); })(); }}
            onOpenSession={(sessionId, requestId) => { void (async () => { if (!await allowPageChange('sessions')) return; setSessionIntent({ key: `sessions/${crypto.randomUUID()}`, sessionId, requestId }); openPage('sessions'); })(); }}
            onOperation={value => acceptOperation(value, { kind: 'routing' })}
          /></div>}
        {visited.has('agents') && <div hidden={page !== 'agents'}><Agents
            key={agentIntent.key}
            language={language}
            active={page === 'agents'}
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
