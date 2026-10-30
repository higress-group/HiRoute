import type { Options } from '../plan-editor';
import { TrialQuickStart } from '../features/trial/TrialQuickStart';
import { NewModels } from '../features/trial/NewModels';
import { useTrustSuggestions } from '../features/trust/use-trust-suggestions';
import { NativeConnectionRecovery } from './NativeConnectionRecovery';
import { startupFailureText } from '../storage-recovery';
import { randomId } from '../random-id.ts';
import { hostCapabilities } from '../host/index.ts';
import { HostNotices } from '../host/ConsoleUi';
import type { DecisionIntent, OpenDecisionConnection } from '../features/decision-services/presentation';
import type { DecisionService } from '../features/decision-services/types';
import { DecisionServicesPage } from '../features/decision-services/DecisionServicesPage';
import { requestEditorReplacement } from '../ui/discard-guard';
import { useEffect, useRef, useState } from 'react';
import { invoke } from '../host/index.ts';
import { Agents } from '../agents';
import type { AgentTask, AgentTaskRead } from '../features/AgentTasks';
import { Sessions } from '../features/Sessions';
import {
  cancelWorkerTask,
  confirmWorkerTaskResidual,
  readWorkerTask,
  readWorkerTaskResultPage,
  WorkerTaskPager,
  type WorkerTaskLocator,
} from '../features/worker-task-client';
import {
  Home,
  HomeNavigation,
  deriveHomeServicePresentation,
  deriveHomeServiceStatus,
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
import { takeSubscriptionProxyReturn } from '../features/subscription-proxy-return';
import { planErrorMessage } from '../plan-editor-errors';
import {
  acceptObservedOperation,
  currentPendingHint,
  isTerminalOperation,
  OPERATION_RESULT_UNVERIFIED,
  operationFeedback,
  pendingFeedbackIdentity,
  representedByTrial,
  shouldObservePendingOperation,
  type OperationPresentation,
} from './operation-feedback';

type Page = 'home' | 'models' | 'routing' | 'agents' | 'sessions' | 'settings';

type ModelIntent = {
  key: string;
  sourceId?: string | null;
  startAdding?: boolean;
  startScanning?: boolean;
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

function failureHelp(code: string, language: 'zh' | 'en', operation?: DesktopOperation | null): string {
  if (code === 'QODER_MODEL_BUDGET_CONFLICT' || code === 'AGENT_MODEL_BUDGET_CONFLICT') return planErrorMessage(operation ?? code, language);
  const messages: Record<string, [string, string]> = {
    CONFIRMATION_EXPIRED: ['确认已过期，请重新预览。输入已保留。', 'Confirmation expired. Preview again; your input is retained.'],
    CONFIRMATION_STALE: ['确认已失效，请重新预览。', 'This confirmation is no longer valid. Preview again.'],
    REVISION_CONFLICT: ['方案已经变化，请读取当前状态后重新预览。', 'The plan changed. Refresh its current state and preview again.'],
    CHANGE_PREVIEW_STALE: ['预览已过期，请重新预览当前方案。', 'The preview is stale. Preview the current plan again.'],
    LATEST_EDIT_NOT_APPLIED: ['已恢复原操作，最新编辑尚未应用。请先核对原操作结果。', 'The original operation was restored. Check its result before applying the latest edit.'],
    OPERATION_IN_PROGRESS: ['这次保存仍在进行，完成后即可再次保存。', 'This save is still running. Save again after it finishes.'],
    TRUSTED_AUTHORITY_UNAVAILABLE: ['本机可信连接不可用，请恢复本机连接后重试。', 'The trusted local connection is unavailable. Restore the local connection and retry.'],
    IDEMPOTENCY_KEY_REUSED: ['上一次保存仍在处理中。请先查看其结果，最新编辑尚未应用。', 'The previous save is still being processed. Check its result first; the latest edit was not applied.'],
  };
  return messages[code]?.[language === 'zh' ? 0 : 1]
    ?? (language === 'zh' ? '操作暂未完成，请稍后重试。' : 'The action could not complete. Try again shortly.');
}

export function DesktopApp() {
  const preferences = usePresentationPreferences();
  const { language } = preferences;
  const home = useDesktopHome();
  const [trialIntent, setTrialIntent] = useState<{ modelId?: string; bindingId?: string } | null>(null);
  const [trialModelsOpen, setTrialModelsOpen] = useState(false);
  const trialOpen = Boolean(trialIntent) && !trialModelsOpen;
  const [page, setPage] = useState<Page>('home');
  const [modelTab, setModelTab] = useState<'general' | 'decisions'>('general');
  // Managing an API connection from a decision draft keeps that draft mounted behind the model page.
  const [decisionDetour, setDecisionDetour] = useState(false), [sourceRefreshKey, setSourceRefreshKey] = useState(0);
  const [decisionIntent, setDecisionIntent] = useState<DecisionIntent | null>(null);
  const decisionReturn = useRef<((service: DecisionService) => void) | null>(null);
  const [returningToRoute, setReturningToRoute] = useState(false);
  useEffect(() => { window.scrollTo({ top: 0, left: 0, behavior: 'instant' }); }, [page]);
  const [visited, setVisited] = useState<Set<Page>>(() => new Set(['home']));
  const [refreshing, setRefreshing] = useState(false);
  const [refreshVersion, setRefreshVersion] = useState(0);
  const [operation, setOperation] = useState<DesktopOperation | null>(null);
  const [trialOperationIds, setTrialOperationIds] = useState<string[]>([]);
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

  const serviceStatus = deriveHomeServiceStatus(home.reads.service);
  const servicePresentation = deriveHomeServicePresentation(home.reads.service);
  const serviceOperational = serviceStatus.operational;
  const mutationAllowed = serviceOperational && Boolean(home.desktopSnapshot?.trusted_authority && home.desktopSnapshot.service.mutation_available);

  useEffect(() => {
    if (!mutationAllowed) return;
    if (!takeSubscriptionProxyReturn()) return;
    setModelIntent({ key: 'models/proxy-return', startScanning: true });
    setVisited(current => new Set([...current, 'models']));
    setPage('models');
  }, [mutationAllowed]);

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

  useEffect(() => {
    const refresh = () => { home.invalidateService(true); void home.refreshDesktop(); };
    window.addEventListener('hiroute-native-connection-unavailable', refresh);
    return () => window.removeEventListener('hiroute-native-connection-unavailable', refresh);
  }, [home.invalidateService, home.refreshDesktop]);
  const startupFailure = home.startup?.state === 'failed' ? home.startup : null;
  const startupMessage = startupFailureText(startupFailure?.code, startupFailure?.storage_recovery, language, 'macos');
  const upgradePhase = home.startup?.state === 'starting' ? home.startup.upgrade_phase : undefined;
  const upgradePhaseText = upgradePhase ? ({
    source_check: language === 'zh' ? '正在校验存储格式' : 'Checking storage format',
    backup: language === 'zh' ? '正在保存升级前的完整备份' : 'Saving the complete pre-upgrade backup',
    conversion: language === 'zh' ? '正在升级本机数据' : 'Upgrading local data',
    validation: language === 'zh' ? '正在校验升级结果' : 'Verifying upgraded data',
    service_recovery: language === 'zh' ? '正在恢复本机服务' : 'Restoring the local service',
  })[upgradePhase] : undefined;
  const serviceLabel = {
    connection_lost: language === 'zh' ? '本机连接需要恢复' : 'Local connection needs recovery',
    read_only: text.serviceReadOnly,
    unavailable: text.serviceUnavailable,
    ready: text.serviceReady,
    unknown: text.serviceUnknown,
  }[servicePresentation.state];
  const trustSuggestions = useTrustSuggestions(observing && serviceOperational && Boolean(home.desktopSnapshot), `${refreshVersion}:${home.desktopSnapshot?.catalog.plans.map(p => `${p.agent_plan_id}:${p.head.head_revision}`).join(',') ?? ''}`, page === 'home');
  const navigationBaseLabel = servicePresentation.state === 'ready'
      ? language === 'zh' ? '仅在本机运行' : 'Running locally'
      : serviceLabel;
  const navigationLabel = servicePresentation.checkFailed
    ? language === 'zh' ? '状态读取失败' : 'Status check failed'
    : servicePresentation.refreshing
      ? `${navigationBaseLabel} · ${language === 'zh' ? '正在更新' : 'Updating'}`
      : navigationBaseLabel;
  const settingsServiceLabel = servicePresentation.state === 'ready'
    ? `${language === 'zh' ? '可用' : 'Available'}${servicePresentation.refreshing ? language === 'zh' ? ' · 正在更新' : ' · Updating' : ''}`
    : navigationLabel;

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
      if (!document.hidden) void (page === 'home' && !trialOpen ? home.refreshAll() : home.refreshDesktop());
    };
    window.addEventListener('focus', refreshOnFocus);
    return () => window.removeEventListener('focus', refreshOnFocus);
  }, [home.refreshAll, home.refreshDesktop, page, trialOpen]);

  useEffect(() => {
    if (!trialOpen && ['home', 'routing', 'models'].includes(page)) void home.refreshInvalidatedCatalog();
  }, [page, trialOpen, home.catalogInvalidation, home.refreshInvalidatedCatalog]);

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
            // This guarded lookup has reconciled the snapshot's discovery hint
            // with the current native operation. Do not treat that same hint as
            // another submission while the terminal Home readback is in flight.
            supersededPendingKey.current = home.desktopSnapshot?.pending?.idempotency_key
              ?? supersededPendingKey.current;
            setOperation(current => acceptObservedOperation(current, value));
            setOperationRecoveryPending(false);
          } else {
            noteUnconfirmed(OPERATION_RESULT_UNVERIFIED);
          }
          if (value && isTerminalOperation(value.state)) {
            home.invalidateCatalog();
            await home.refreshDesktop();
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
    home.refreshDesktop,
    home.invalidateCatalog,
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

  // The trial wizard is a full page: entering it resolves the current page's editor, so while it is
  // open its inline connection is the only model editor. Leaving follows that editor's guard too.
  async function openTrial(intent: { modelId?: string; bindingId?: string }) {
    const scope = editorScope(page);
    if (scope && !await requestEditorReplacement(scope)) return;
    setTrialModelsOpen(false); setTrialIntent(intent);
  }
  async function leaveTrial() {
    if (!trialIntent) return true;
    if (!await requestEditorReplacement('models') || !await requestEditorReplacement('routing')) return false;
    setTrialModelsOpen(false); setTrialIntent(null);
    return true;
  }

  async function navigate(id: string) {
    if (!navigation.some(item => item.id === id)) return;
    setNotice('');
    if (!await leaveTrial()) return;
    await openPageSafely(id as Page);
  }

  function openPage(next: Page) {
    if (next === 'home' && page !== 'home') void Promise.allSettled([home.refreshActivity(), home.refreshValue()]);
    setVisited(current => current.has(next) ? current : new Set([...current, next]));
    setPage(next);
    if (next !== 'models') { decisionReturn.current = null; setReturningToRoute(false); setDecisionDetour(false); }
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
    if (decisionDetour && next !== 'models' && !await requestEditorReplacement('decisions')) return false;
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
    if (next === 'decisions' && decisionDetour) { setDecisionDetour(false); setSourceRefreshKey(key => key + 1); }
  }

  async function manageDecisionSource(sourceId: string) {
    if (!await requestEditorReplacement('models')) return;
    setModelIntent({ key: `models/${randomId()}`, sourceId });
    setDecisionDetour(true);
    setModelTab('general');
  }

  const addDecisionForRoute: OpenDecisionConnection = async (kind, select) => {
    if (!await requestEditorReplacement('decisions')) return;
    decisionReturn.current = select;
    setReturningToRoute(true);
    setDecisionIntent({ key: randomId(), kind });
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
    if (!await allowPageChange('models', true)) return false;
    setNotice('');
    if (modelTab === 'decisions' && !await requestEditorReplacement('models')) return false;
    setModelTab('general');
    setModelIntent({ key: `models/${randomId()}`, ...intent });
    openPage('models');
    return true;
  }

  async function addTrialModel() {
    if (await showModels({ startAdding: true })) setTrialModelsOpen(true);
  }

  async function returnToTrial() {
    if (!await requestEditorReplacement('models')) return;
    setTrialModelsOpen(false);
  }

  async function showRouting(editor?: RoutingEditorIntent | null, routingNotice?: string) {
    if (!await allowPageChange('routing', true)) return;
    setNotice('');
    setRoutingIntent({ key: `routing/${randomId()}`, editor, notice: routingNotice });
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
    setAgentIntent({ key: `agents/${randomId()}`, tab: 'tasks', taskId });
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

  async function reviewTrustAction(planId: string, branchId: string) {
    if (trustSuggestions.status !== 'ready') return;
    const report = trustSuggestions.plans.find(p => p.plan_id === planId);
    const plan = home.desktopSnapshot?.catalog.plans.find(p => p.agent_plan_id === planId);
    const branch = report?.branches.find(b => b.branch_id === branchId);
    if (!plan || !report || !branch) return;
    if (report.active_revision !== plan.agent_plan_revision) {
      await showRouting({ key: `trust/${randomId()}`, plan, initialView: 'performance' }); return;
    }
    const suggestion = branch.suggestion;
    if (suggestion.kind === 'return_to_previous_routing' && report.previous_routing?.length) {
      setAgentIntent({ key: `agents/${randomId()}`, agentId: report.previous_routing[0].agent });
      await openPageSafely('agents'); return;
    }
    if (suggestion.kind === 'explore_division') {
      await showRouting({ key: `division/${randomId()}`, plan, initialView: 'performance', initialDivisionBranch: branchId }); return;
    }
    if ('threshold_millis' in suggestion) {
      let authoring;
      try {
        const options = await invoke<Options>('plan_editor_options', { input: { authoring: { plan_id: planId }, include_unavailable: true } });
        if (!options.authoring || options.authoring.expected_head_revision !== plan.head.head_revision) throw Error('CHANGE_PREVIEW_STALE');
        authoring = options.authoring;
      } catch { await showRouting({ key: `trust/${randomId()}`, plan, initialView: 'performance' }); return; }
      const editor = structuredClone(authoring.editor);
      if (editor.mode === 'smart_saving' && branchId === 'smart_saving') editor.smart.judgment.degree.simple_threshold_millis = suggestion.threshold_millis;
      else {
        const selected = editor.branch_routing?.branches.find(b => b.id === branchId);
        if (!selected || !editor.branch_routing) return;
        selected.judgment = structuredClone(selected.judgment ?? editor.branch_routing.judgment);
        selected.judgment.degree.simple_threshold_millis = suggestion.threshold_millis;
      }
      await showRouting({ key: `trust/${randomId()}`, plan, initialContent: editor }); return;
    }
    await showRouting({ key: `trust/${randomId()}`, plan });
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
      setAgentIntent({ key: `agents/${randomId()}` });
      openPage('agents');
      return;
    }
    if (action.kind === 'create-plan') {
      showRouting({ key: randomId() });
      return;
    }
    if (action.kind === 'configure-agent') {
      if (!await allowPageChange('agents', true)) return;
      setNotice('');
      setAgentIntent({
        key: `agents/${randomId()}`,
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
      if (plan) showRouting({ key: plan.agent_plan_id, plan, initialView: action.performance ? 'performance' : 'configuration' });
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
      setSessionIntent({ key: `sessions/${randomId()}`, sessionId: action.sessionId });
      openPage('sessions');
      return;
    }
    if (action.kind === 'view-all-sessions') {
      if (!await allowPageChange('sessions')) return;
      setNotice('');
      setSessionIntent({ key: `sessions/${randomId()}`, sessionId: null });
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
      setAgentIntent({ key: `agents/${randomId()}`, tab: 'tasks' });
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
  const trialRepresentsFeedback = representedByTrial(feedbackId, operationView.phase, presentedOperationError, operationRecoveryPending, trialOperationIds);
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
    <div className={`app-window host-${hostCapabilities().host}`}>
      <header className="titlebar" data-tauri-drag-region="deep" onMouseDown={event => {
        if (!hostCapabilities().titlebar || event.button !== 0 || event.detail !== 2) return;
        event.preventDefault();
        event.stopPropagation();
        void invoke('perform_titlebar_double_click');
      }} onMouseUp={event => {
        if (event.button === 0 && event.detail === 2) event.stopPropagation();
      }}>
        <div className="window-controls" data-tauri-drag-region aria-hidden="true" />
        <div className="titlebar-main" data-tauri-drag-region>
          <div className="window-title"><span>{trialOpen ? (language === 'zh' ? '试用新模型' : 'Try a new model') : text.pages[page]}</span></div>
        </div>
      </header>
      <HomeNavigation
        language={language}
        items={navigation}
        current={page}
        routingAttention={trustSuggestions.plans.length > 0}
        serviceLabel={navigationLabel}
        serviceReady={servicePresentation.state === 'ready'}
        serviceCheckFailed={servicePresentation.checkFailed}
        onRetryService={() => void home.refreshDesktop()}
        onNavigate={id => void navigate(id)}
        onOpenSettings={() => void (async () => { if (await leaveTrial()) await openPageSafely('settings'); })()}
      />
      <main className="main">
        <HostNotices language={language} />
        {notice && <div className="toast-stack" aria-live="polite"><div className="toast" role="status"><UiIcon name="info" /><span>{notice}</span></div></div>}
        {upgradePhaseText && <div className="callout" role="status" aria-live="polite"><UiIcon name="info" /><div><strong>{upgradePhaseText}</strong><p>{language === 'zh' ? '请等待升级完成；请保留原数据与完整安装包。' : 'Wait for the upgrade to finish. Keep the original data and complete installation package.'}</p></div></div>}
        <NativeConnectionRecovery status={home.desktopSnapshot?.native_connection} language={language} serviceResponsive={home.reads.service.status === 'ready'} onRefresh={home.refreshDesktop} onRecovered={async () => { home.invalidateService(); await home.refreshAll(); setRefreshVersion(value => value + 1); }} />
        {startupFailure && <div className="callout bad startup-recovery" role="alert" data-error-code={startupFailure.code}>
          <UiIcon name="warning" />
          <div>
            <strong>{language === 'zh' ? '本机服务启动失败' : 'Local service failed to start'}</strong>
            <p>{startupMessage.reason}</p>
            <p>{startupMessage.instructions}</p>
            {startupMessage.backupBatch && <div className="recovery-location">
              <p><strong>{language === 'zh' ? '已校验的原批次：' : 'Verified original batch: '}</strong><code>{startupMessage.backupBatch}</code></p>
              <p><strong>{language === 'zh' ? '恢复目录：' : 'Recovery directory: '}</strong><code>{startupMessage.backupDirectory}</code></p>
              <p>{startupMessage.locationHint}</p>
            </div>}
            {startupFailure.recovery_available && <button className="btn" type="button" onClick={() => {
              void invoke('open_startup_recovery_directory').catch(() => setNotice(language === 'zh' ? '无法安全打开恢复目录，请检查目录权限后重试。' : 'The recovery directory could not be opened safely. Check its permissions and try again.'));
            }}>{startupFailure.backup_directory ? (language === 'zh' ? '查看保留的恢复材料与说明' : 'Inspect preserved recovery material and guide') : (language === 'zh' ? '打开数据目录' : 'Open data directory')}</button>}
            <button className="btn" type="button" onClick={() => {
              void invoke('open_diagnostic_directory').catch(() => setNotice(language === 'zh' ? '无法安全打开诊断目录，请保留数据并联系支持。' : 'The diagnostic directory could not be opened safely. Keep your data and contact support.'));
            }}>{language === 'zh' ? '打开诊断目录' : 'Open diagnostics'}</button>
          </div>
        </div>}

        {trialIntent && <div className="trial-page-host" hidden={trialModelsOpen}><TrialQuickStart active={!trialModelsOpen} onOpenModels={() => void addTrialModel()} onOpenPerformance={plan => { setTrialIntent(null); void showRouting({ key: `trial/${randomId()}`, plan, initialView: 'performance' }); }} onOpenSessions={sessionId => { void (async () => { if (!await allowPageChange('sessions') || !await leaveTrial()) return; setSessionIntent({ key: `sessions/${randomId()}`, sessionId }); openPage('sessions'); })(); }} initialModelId={trialIntent.modelId} initialBindingId={trialIntent.bindingId} language={language} mutable={mutationAllowed} snapshot={home.desktopSnapshot} operation={operation} onRepresentedOperations={setTrialOperationIds} onOperation={value => acceptOperation(value, { kind: 'routing' })} onRefresh={async () => { setObserving(true); home.invalidateCatalog(); await home.refreshDesktop(); setRefreshVersion(value => value + 1); }} onClose={() => setTrialIntent(null)} onOpenAgent={id => { setTrialIntent(null); setAgentIntent({ key: `agents/${randomId()}`, agentId: id }); openPage('agents'); }} /></div>}
        <div hidden={trialOpen || page !== 'home'}><Home
            language={language}
            reads={home.reads}
            operation={homeOperation}
            trustSuggestions={trustSuggestions}
            onTrustAction={(plan, branch) => void reviewTrustAction(plan, branch)}
            newModels={<NewModels language={language} mutable={mutationAllowed} active={!trialOpen && page === 'home'} plans={home.desktopSnapshot?.catalog.plans ?? []} refreshVersion={refreshVersion} onTry={modelId => void openTrial({ modelId })} onOpenPlan={planId => { const plan = home.desktopSnapshot?.catalog.plans.find(item => item.agent_plan_id === planId); if (plan) void showRouting({ key: `models/${randomId()}`, plan, initialView: 'performance' }); }} />}
            hasTasks={taskRead.status === 'ready' && taskRead.tasks.length > 0}
            onAction={handleHomeAction}
          /></div>
        {trialIntent && trialModelsOpen && page === 'models' && <div className="trial-model-return" role="status"><span>{language === 'zh' ? '试用设置已保留，添加模型后可返回继续。' : 'Your trial setup is retained. Return after adding a model.'}</span><button type="button" className="btn" onClick={() => void returnToTrial()}>{language === 'zh' ? '返回试用' : 'Back to trial'}</button></div>}
        {visited.has('models') && <div hidden={trialOpen || page !== 'models' || modelTab !== 'general'}><ModelManagementPage
            key={modelIntent.key}
            language={language}
            active={!trialOpen && page === 'models' && modelTab === 'general'}
            tabs={<>{modelTabs}{decisionDetour && <div className="decision-detour" role="note"><button type="button" className="btn" onClick={() => void switchModelTab('decisions')}><UiIcon name="arrowLeft" />{language === 'zh' ? '返回决策模型 — 草稿已保留' : 'Back to decision model — draft retained'}</button></div>}</>}
            trustedAuthority={mutationAllowed}
            refreshVersion={refreshVersion}
            initialSourceId={modelIntent.sourceId}
            startAdding={modelIntent.startAdding}
            startScanning={modelIntent.startScanning}
            notice={modelIntent.notice}
            onOperation={value => acceptOperation(value, { kind: 'model-connection' })}
            onRecoveryRefresh={() => void home.refreshAll()}
            onChanged={() => void Promise.allSettled([home.refreshCompute(), home.refreshDesktop()])}
            plans={home.desktopSnapshot?.catalog.plans}
            agents={home.agentDirectory ?? undefined}
            onOpenPlan={planId => {
              const plan = home.desktopSnapshot?.catalog.plans.find(item => item.agent_plan_id === planId);
              if (plan) void showRouting({ key: plan.agent_plan_id, plan });
            }}
            onTryModel={(modelId, bindingId) => void openTrial({ modelId, bindingId })}
            onCreatePlan={bindingId => void showRouting({ key: `routing/${randomId()}`, initialBindingId: bindingId })}
          /></div>}
        {visited.has('models') && <div hidden={trialOpen || page !== 'models' || modelTab !== 'decisions'}><DecisionServicesPage
          language={language} active={!trialOpen && page === 'models' && modelTab === 'decisions'}
          mutable={mutationAllowed}
          tabs={modelTabs} intent={decisionIntent}
          plans={home.desktopSnapshot?.catalog.plans} drafts={home.desktopSnapshot?.catalog.drafts}
          onChanged={() => void home.refreshDesktop()}
          onOpenRoute={(plan, draft) => void showRouting({ key: draft?.draft_id ?? plan!.agent_plan_id, plan, draft })}
          onReturn={returningToRoute ? () => void openPageSafely('routing') : undefined}
          onUse={returningToRoute ? useDecisionInRoute : undefined}
          sourceRefreshKey={sourceRefreshKey} onManageSource={sourceId => void manageDecisionSource(sourceId)}
        /></div>}
        {visited.has('routing') && <div hidden={trialOpen || page !== 'routing'}><RoutingPage
            key={routingIntent.key}
            language={language}
            active={!trialOpen && page === 'routing'}
            refreshVersion={refreshVersion}
            snapshot={home.desktopSnapshot}
            mutationAllowed={mutationAllowed}
            operation={operation}
            loading={home.reads.plans.status === 'loading'}
            busy={refreshing}
            onOpenServices={addDecisionForRoute}
            initialEditor={routingIntent.editor}
            suggestionPlanIds={trustSuggestions.plans.map(p => p.plan_id)}
            notice={routingIntent.notice}
            onRefresh={async () => { setObserving(true); await home.refreshDesktop(); setRefreshVersion(v => v + 1); }}
            onOpenAgent={agentId => { void (async () => { if (!await allowPageChange('agents')) return; setAgentIntent({ key: `agents/${randomId()}`, agentId }); openPage('agents'); })(); }}
            onOpenSession={(sessionId, requestId) => { void (async () => { if (!await allowPageChange('sessions')) return; setSessionIntent({ key: `sessions/${randomId()}`, sessionId, requestId, returnToQuality: true }); openPage('sessions'); })(); }}
            onOpenSessions={sessionId => { void (async () => { if (!await allowPageChange('sessions')) return; setSessionIntent({ key: `sessions/${randomId()}`, sessionId }); openPage('sessions'); })(); }}
            onOperation={value => acceptOperation(value, { kind: 'routing' })}
          /></div>}
        {visited.has('agents') && <div hidden={trialOpen || page !== 'agents'}><Agents
            key={agentIntent.key}
            language={language}
            active={!trialOpen && page === 'agents'}
            operation={operation}
            initialAgentId={agentIntent.agentId}
            initialFacet={agentIntent.facet}
            initialTab={agentIntent.tab}
            initialTaskId={agentIntent.taskId}
            taskRead={taskRead}
            refreshVersion={refreshVersion}
            mutationAllowed={mutationAllowed}
            onMutation={() => {
              setObserving(true);
              home.invalidateCatalog();
              void home.refreshDesktop();
            }}
            onOperation={acceptOperation}
            onUnverifiedOperation={acceptUnverifiedOperation}
            onTaskConfirmResidual={confirmWorkerTaskResidual}
            onTaskCancel={async (task: AgentTask, onAccepted) => cancelWorkerTask(task, onAccepted)}
            onOpenTasks={() => void loadTaskHistory(true)}
            onLoadMoreTasks={() => loadTaskHistory(false)}
            onReadTask={hydrateTask}
            onLoadTaskResult={readWorkerTaskResultPage}
            onCreatePlan={() => void showRouting({ key: `routing/${randomId()}` })}
            onOpenTaskSession={sessionId => {
              void (async () => {
                if (!await allowPageChange('sessions')) return;
                setSessionIntent({ key: `sessions/${randomId()}`, sessionId });
                openPage('sessions');
              })();
            }}
          /></div>}
        {visited.has('sessions') && <div hidden={trialOpen || page !== 'sessions'}><Sessions
            key={sessionIntent.key}
            active={!trialOpen && page === 'sessions'}
            initialSession={sessionIntent.sessionId ?? null}
            initialRequest={sessionIntent.requestId ?? null}
            onReturnToQuality={sessionIntent.returnToQuality ? () => void openPageSafely('routing') : undefined}
            refreshVersion={refreshVersion}
            language={language}
            onOpenAgents={() => void openPageSafely('agents')}
          /></div>}
        <div hidden={trialOpen || page !== 'settings'}><SettingsPage
          active={!trialOpen && page === 'settings'}
          language={language}
          languagePreference={preferences.languagePreference}
          theme={preferences.theme}
          serviceLabel={settingsServiceLabel}
          serviceReady={servicePresentation.state === 'ready'}
          onServiceChanged={home.refreshDesktop}
          onLanguageChange={preferences.setLanguage}
          onThemeChange={preferences.setTheme}
          onOpenSessions={() => {
            void (async () => {
              if (!await allowPageChange('sessions')) return;
              setSessionIntent({ key: `sessions/${randomId()}`, sessionId: null });
              openPage('sessions');
            })();
          }}
        /></div>

        {feedbackId && feedbackId !== dismissedOperationId && !trialRepresentsFeedback && <article className={`operation-status surface ${operationView.phase}`} aria-live={operationView.phase === 'failed' ? 'assertive' : 'polite'} data-operation-state={operation?.state ?? 'unknown'} data-operation-phase={operationView.phase} data-observation-error-code={presentedOperationError || undefined}>
          <div className="operation-status-head">
            <strong>{['pending', 'unverified'].includes(operationView.phase) && <span className="oc-spinner" aria-hidden="true" />}{operationView.title}</strong>
            <div className="operation-status-actions">
              {operationView.phase === 'unverified' && <button className="btn btn-quiet" type="button" onClick={retryOperationObservation}>{language === 'zh' ? '重新查询' : 'Check again'}</button>}
              <button className="icon-btn" type="button" aria-label={text.close} onClick={dismissOperation}><UiIcon name="close" /></button>
            </div>
          </div>
          <p>{operationView.detail}</p>
          {operation?.safe_error_code && <div className="callout bad" role="alert" data-error-code={operation.safe_error_code}><UiIcon name="warning" /><span>{failureHelp(operation.safe_error_code, language, operation)}</span></div>}
        </article>}
      </main>
    </div>
    <WebConfirmationHost />
  </PresentationRoot>;
}
