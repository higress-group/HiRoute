import { useEffect, useRef, useState } from 'react';
import { Dialog, UiIcon } from '../ui';
import type { WorkerNextAction } from './worker-task-client';

export type AgentTaskFact = {
  model: string;
  reasoning?: string | null;
  tokens?: string | null;
};

export type AgentTask = {
  taskId: string;
  runId: string;
  title: string;
  displayTitle?: string;
  executor?: string | null;
  executorBasis?: 'frozen_plan' | 'unavailable';
  routeName: string;
  createdAtMs?: number | null;
  acceptedAtMs?: number | null;
  acceptedTimeState?: 'recorded' | 'legacy_unavailable';
  admissionSequence?: string | number;
  status: 'queued' | 'running' | 'cancelling' | 'complete' | 'failed' | 'cancelled' | 'unknown';
  cleanup?: 'pending' | 'complete' | 'unknown' | 'residual_acknowledged';
  brief: string;
  contentAvailability?: 'available' | 'unavailable' | 'indeterminate';
  result?: string | null;
  resultAvailable?: boolean;
  resultIncomplete?: boolean;
  resultNextOffset?: number | null;
  detailsLoaded?: boolean;
  sessionId?: string | null;
  sessionIds?: string[];
  sessionsComplete?: boolean;
  nextActions?: WorkerNextAction[];
  facts: AgentTaskFact[];
};

export type AgentTaskRead =
  | { status: 'loading' }
  | { status: 'unavailable'; message?: string }
  | { status: 'error'; message?: string }
  | { status: 'ready'; tasks: AgentTask[]; hasMore?: boolean };

export function AgentTasks({
  language,
  read,
  initialTaskId = null,
  onCancel,
  onOpenSession,
  onBackToAgents,
  onRefresh,
  onLoadMore,
  onReadTask,
  onLoadTaskResult,
  active = true,
}: {
  language: 'zh' | 'en';
  read: AgentTaskRead;
  initialTaskId?: string | null;
  onCancel?: (task: AgentTask, onAccepted?: (status: AgentTask['status']) => void) => Promise<AgentTask['status']>;
  onOpenSession?: (sessionId: string) => void;
  onBackToAgents(): void;
  onRefresh?: () => void;
  onLoadMore?: () => Promise<{ tasks: AgentTask[]; hasMore: boolean }>;
  onReadTask?: (task: AgentTask) => Promise<AgentTask>;
  onLoadTaskResult?: (task: AgentTask) => Promise<AgentTask>;
  active?: boolean;
}) {
  const zh = language === 'zh';
  const text = (cn: string, en: string) => zh ? cn : en;
  const taskKey = (task: AgentTask) => task.taskId;
  const sameTask = (left: AgentTask, right: AgentTask) => taskKey(left) === taskKey(right);
  const sameRun = (left: AgentTask, right: AgentTask) => sameTask(left, right) && left.runId === right.runId;
  const [tasks, setTasks] = useState<AgentTask[]>(read.status === 'ready' ? read.tasks : []);
  const [selectedKey, setSelectedKey] = useState<string | null>(() => {
    const initial = read.status === 'ready' ? read.tasks.find(task => task.taskId === initialTaskId) : null;
    return initial ? taskKey(initial) : null;
  });
  const [confirming, setConfirming] = useState<AgentTask | null>(null);
  const [choosingSession, setChoosingSession] = useState<AgentTask | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [loadingMore, setLoadingMore] = useState(false);
  const [loadingDetails, setLoadingDetails] = useState<string | null>(null);
  const [loadingResult, setLoadingResult] = useState(false);
  const [hasMore, setHasMore] = useState(read.status === 'ready' && Boolean(read.hasMore));
  const cancelTrigger = useRef<HTMLButtonElement | null>(null);
  const detailGeneration = useRef(0);

  useEffect(() => {
    if (read.status !== 'ready') return;
    const next = read.tasks.find(task => task.taskId === initialTaskId) ?? read.tasks[0];
    setTasks(read.tasks);
    setHasMore(Boolean(read.hasMore));
    setSelectedKey(current => current && read.tasks.some(task => taskKey(task) === current)
      ? current
      : next ? taskKey(next) : null);
  }, [read, initialTaskId]);

  const selected = tasks.find(task => taskKey(task) === selectedKey) ?? tasks[0];
  useEffect(() => {
    if (active && read.status === 'ready' && selected && !selected.detailsLoaded && onReadTask) {
      void selectTask(selected);
    }
  }, [active, read.status, selected?.taskId, selected?.runId]);

  useEffect(() => {
    if (!active || !onReadTask || !selected?.detailsLoaded) return;
    let disposed = false;
    let timer: number | undefined;
    const poll = async (current: AgentTask) => {
      if (disposed) return;
      try {
        const hydrated = await onReadTask(current);
        if (disposed) return;
        setTasks(items => items.map(item => sameRun(item, current) ? hydrated : item));
        setError('');
        // A completed run does not mean the task cannot be continued elsewhere.
        timer = window.setTimeout(() => void poll(hydrated),
          ['queued', 'running', 'cancelling'].includes(hydrated.status) ? 1800 : 4500);
      } catch {
        if (disposed) return;
        setError(text('任务状态暂时无法刷新，将继续重试。', 'Task status could not be refreshed. Retrying.'));
        timer = window.setTimeout(() => void poll(current), 4500);
      }
    };
    timer = window.setTimeout(() => void poll(selected),
      ['queued', 'running', 'cancelling'].includes(selected.status) ? 1800 : 4500);
    return () => {
      disposed = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [active, onReadTask, selected]);
  const statusLabel = (status: AgentTask['status']) => ({
    running: text('执行中', 'Running'),
    queued: text('准备中', 'Preparing'),
    cancelling: text('取消中', 'Cancelling'),
    complete: text('完成', 'Complete'),
    failed: text('需要处理', 'Action required'),
    cancelled: text('已取消', 'Cancelled'),
    unknown: text('状态待确认', 'Status unknown'),
  })[status];
  const statusTone = (status: AgentTask['status']) => status === 'complete' ? 'good' : status === 'failed' ? 'bad' : status === 'running' || status === 'queued' ? 'info' : 'warn';

  function taskTime(task: AgentTask) {
    const recorded = task.acceptedAtMs ?? task.createdAtMs;
    if (!recorded) return text('时间未记录', 'Time not recorded');
    const date = new Date(recorded);
    const current = new Date();
    const day = (input: Date) => new Date(input.getFullYear(), input.getMonth(), input.getDate()).getTime();
    const delta = Math.round((day(current) - day(date)) / 86_400_000);
    if (delta === 0) return date.toLocaleTimeString(zh ? 'zh-CN' : 'en', { hour: '2-digit', minute: '2-digit', hour12: false });
    if (delta === 1) return text('昨天', 'Yesterday');
    return `${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
  }

  function sessions(task: AgentTask) {
    return task.sessionIds?.length ? task.sessionIds : task.sessionId ? [task.sessionId] : [];
  }

  function openSessions(task: AgentTask) {
    const ids = sessions(task);
    if (ids.length === 1) onOpenSession?.(ids[0]);
    else if (ids.length > 1) setChoosingSession(task);
  }

  const taskTitle = (task: AgentTask) => task.title || `${text('任务', 'Task')} · ${task.taskId}`;
  const taskBrief = (task: AgentTask) => task.brief || (task.contentAvailability === 'unavailable'
    ? text('这项任务的简述已不可用。', 'This task summary is unavailable.')
    : text('任务简述暂时无法读取。', 'The task summary is temporarily unavailable.'));
  const executionLabel = (task: AgentTask) => task.executor
    ? task.status === 'queued'
      ? text(`待由 ${task.executor} 执行`, `Waiting for ${task.executor}`)
      : text(`执行 Agent：${task.executor}`, `Executor: ${task.executor}`)
    : text('执行 Agent 未记录', 'Executor not recorded');

  async function selectTask(task: AgentTask) {
    setSelectedKey(taskKey(task));
    setError('');
    if (!onReadTask || task.detailsLoaded || loadingDetails === taskKey(task)) return;
    const epoch = ++detailGeneration.current;
    setLoadingDetails(taskKey(task));
    try {
      const hydrated = await onReadTask(task);
      if (epoch !== detailGeneration.current) return;
      setTasks(current => current.map(item => sameRun(item, task) ? hydrated : item));
    } catch {
      if (epoch === detailGeneration.current) setError(text('任务详情暂时无法读取，列表状态已保留。', 'Task details are temporarily unavailable. The list state is preserved.'));
    } finally {
      if (epoch === detailGeneration.current) setLoadingDetails(null);
    }
  }

  async function loadMore() {
    if (!onLoadMore || loadingMore) return;
    setLoadingMore(true);
    setError('');
    try {
      const next = await onLoadMore();
      setTasks(current => [...current, ...next.tasks.filter(task => !current.some(item => item.taskId === task.taskId))]);
      setHasMore(next.hasMore);
    } catch {
      setError(text('更多任务暂时无法读取，请稍后重试。', 'More tasks are temporarily unavailable. Try again later.'));
    } finally {
      setLoadingMore(false);
    }
  }

  async function loadMoreResult(task: AgentTask) {
    if (!onLoadTaskResult || loadingResult || !task.resultIncomplete) return;
    setLoadingResult(true);
    setError('');
    try {
      const hydrated = await onLoadTaskResult(task);
      setTasks(current => current.map(item => sameRun(item, task) ? hydrated : item));
    } catch {
      setError(text('更多执行结果暂时无法读取，请重试。', 'More result content is temporarily unavailable. Try again.'));
    } finally {
      setLoadingResult(false);
    }
  }

  async function confirmCancel() {
    if (!confirming || !onCancel) return;
    const taskToCancel = confirming;
    let accepted = false;
    setBusy(true);
    setError('');
    try {
      const status = await onCancel(taskToCancel, acceptedStatus => {
        accepted = true;
        setTasks(current => current.map(task => sameRun(task, taskToCancel) ? { ...task, status: acceptedStatus } : task));
        setConfirming(null);
      });
      setTasks(current => current.map(task => sameRun(task, taskToCancel) ? { ...task, status } : task));
      setConfirming(null);
    } catch {
      setError(accepted
        ? text('已请求取消，最终状态将在刷新后更新。', 'Cancellation was requested. Refresh to confirm the final state.')
        : text('取消请求未完成。任务状态没有改变，请重试。', 'Cancellation did not complete. The task state is unchanged; try again.'));
      if (accepted) setConfirming(null);
    } finally {
      setBusy(false);
      requestAnimationFrame(() => cancelTrigger.current?.focus());
    }
  }

  if (read.status === 'loading') return <div className="empty-state" role="status"><div><span className="oc-spinner" /><p>{text('正在读取任务记录…', 'Reading task history…')}</p></div></div>;
  if (read.status === 'unavailable' || read.status === 'error') return <div className="empty-state"><div><span className="empty-icon"><UiIcon name="tasks" /></span><h3>{text('任务记录暂时不可用', 'Task history is unavailable')}</h3><p>{read.message || text('当前本机服务还不能提供可核实的任务记录。', 'The local service cannot currently provide verified task history.')}</p>{onRefresh && <button className="btn btn-primary" type="button" onClick={onRefresh}>{text('重新读取', 'Try again')}</button>}</div></div>;
  if (!tasks.length) return <div className="empty-state"><div><span className="empty-icon"><UiIcon name="tasks" /></span><h3>{text('还没有任务记录', 'No task history yet')}</h3><p>{text('启用任务委派后，在常用 Agent 中提出任务。执行结果会显示在这里。', 'Enable task delegation and ask for a task in your usual Agent. Results will appear here.')}</p><button className="btn btn-primary" type="button" onClick={onBackToAgents}>{text('查看 Agent 配置', 'View Agent settings')}</button></div></div>;

  return <div className="split-view task-workspace">
    <aside className="master-pane"><div className="master-toolbar"><span>{text('最近的任务', 'Recent tasks')}</span></div><nav className="master-list native-list" aria-label={text('任务记录', 'Task history')}>{tasks.map(task => <button className={`list-row${selected && sameTask(selected, task) ? ' active' : ''}`} key={taskKey(task)} aria-current={selected && sameTask(selected, task) ? 'page' : undefined} onClick={() => void selectTask(task)}><span className="row-main"><span className="row-title">{task.displayTitle ?? taskTitle(task)}</span><span className="row-meta">{executionLabel(task)} · {taskTime(task)}</span><span className="row-meta">task_id: {task.taskId} · run_id: {task.runId}</span></span><span className={`badge ${statusTone(task.status)} no-dot`}>{statusLabel(task.status)}</span></button>)}{hasMore && <button className="btn task-load-more" type="button" disabled={loadingMore} onClick={() => void loadMore()}>{loadingMore ? text('正在读取…', 'Loading…') : text('查看更多', 'Load more')}</button>}</nav></aside>
    <section className="detail-pane">{selected && <div className="detail-inner task-detail">
      <div className="detail-hero"><div className="detail-identity"><div><h2>{taskTitle(selected)}</h2><p>{executionLabel(selected)} · {text('创建于', 'Created')} {taskTime(selected)}</p><p className="oc-meta">task_id: {selected.taskId} · run_id: {selected.runId}</p></div></div><span className={`badge ${statusTone(selected.status)}`}>{statusLabel(selected.status)}</span></div>
      <div className="task-actions">{['queued', 'running'].includes(selected.status) && onCancel && <button ref={cancelTrigger} className="btn btn-danger" type="button" onClick={() => { setError(''); setConfirming(selected); }}>{text('取消任务', 'Cancel task')}</button>}{sessions(selected).length > 0 && onOpenSession && <button className="btn" type="button" onClick={() => openSessions(selected)}><UiIcon name="sessions" />{sessions(selected).length > 1 ? text(`查看 ${sessions(selected).length} 个关联会话`, `View ${sessions(selected).length} related sessions`) : text('查看关联会话', 'View related session')}</button>}</div>
      {selected.sessionsComplete === false && <div className="callout warn"><UiIcon name="warning" /><span>{text('关联会话仍在整理，当前列表可能不完整。', 'Related sessions are still being prepared, so this list may be incomplete.')}</span></div>}
      <section className="detail-section"><h3>{text('任务简述', 'Task summary')}</h3><p className="task-text">{taskBrief(selected)}</p></section>
      {error && <div className="callout bad" role="alert"><UiIcon name="warning" /><span>{error}</span></div>}
      <section className="detail-section"><h3>{text('执行结果', 'Result')}</h3>{loadingDetails === taskKey(selected) ? <div className="oc-status-row"><span className="oc-spinner" /><p>{text('正在读取执行结果…', 'Reading the result…')}</p></div> : selected.status === 'cancelling' ? <div className="oc-status-row"><span className="oc-spinner" /><p>{text('已请求停止，正在等待执行结束。', 'Stop requested. Waiting for execution to end.')}</p></div> : ['queued', 'running'].includes(selected.status) ? <p className="oc-meta">{text('任务正在执行，结果将在完成后显示。', 'The task is running. The result will appear when it finishes.')}</p> : <><p className="task-text">{selected.result || (selected.status === 'cancelled' ? text('任务已取消，已经发生的文件变更不会自动撤销。', 'The task was cancelled. Existing file changes were not reverted.') : selected.status === 'unknown' ? text('当前结果尚无法核实，请稍后刷新。', 'The result cannot currently be verified. Refresh later.') : text('没有可用的结果文本。', 'No result text is available.'))}</p>{selected.resultIncomplete && <button className="btn" type="button" disabled={loadingResult} onClick={() => void loadMoreResult(selected)}>{loadingResult ? text('正在读取…', 'Loading…') : text('继续读取结果', 'Load more result')}</button>}</>}</section>
      {!['queued', 'running', 'cancelling'].includes(selected.status) && selected.cleanup !== 'complete' && <div className="callout warn"><UiIcon name="warning" /><span>{text(
        `任务已处于 ${statusLabel(selected.status)} 状态，但临时资源清理状态为“${selected.cleanup ?? 'unknown'}”。任务结果与资源回收分别确认。`,
        `The task is ${statusLabel(selected.status)}, while temporary-resource cleanup is “${selected.cleanup ?? 'unknown'}”. Task outcome and resource cleanup are tracked separately.`,
      )}</span></div>}
      <p className="oc-meta">{text('后续操作以当前任务状态和服务返回的可用动作为准。', 'Use the current task state and the actions returned by the service for follow-up work.')}</p>
    </div>}</section>

    <Dialog open={Boolean(confirming)} title={text('取消这个任务？', 'Cancel this task?')} description={confirming?.title} closeLabel={text('关闭取消确认', 'Close cancellation confirmation')} closeDisabled={busy} onClose={() => !busy && setConfirming(null)} footer={<><button className="btn" type="button" disabled={busy} onClick={() => setConfirming(null)}>{text('继续执行', 'Keep running')}</button><button className="btn btn-danger" type="button" disabled={busy} onClick={() => void confirmCancel()}>{busy ? text('正在取消…', 'Cancelling…') : text('取消任务', 'Cancel task')}</button></>}><p>{text('只停止这次执行；已经发生的文件变更不会自动撤销，其他任务不受影响。', 'Stop only this run. Existing file changes will not be reverted, and other tasks are unaffected.')}</p>{error && <div className="callout bad" role="alert"><UiIcon name="warning" /><span>{error}</span></div>}</Dialog>

    <Dialog open={Boolean(choosingSession)} title={text('选择关联会话', 'Choose a related session')} description={choosingSession?.title} closeLabel={text('关闭会话选择', 'Close session picker')} onClose={() => setChoosingSession(null)} footer={<button className="btn" type="button" onClick={() => setChoosingSession(null)}>{text('取消', 'Cancel')}</button>}>
      <div className="native-list v3-catalog">{choosingSession && sessions(choosingSession).map((sessionId, index) => <button className="list-row" type="button" key={sessionId} onClick={() => { setChoosingSession(null); onOpenSession?.(sessionId); }}><UiIcon name="sessions" /><span className="row-main"><span className="row-title">{text(`关联会话 ${index + 1}`, `Related session ${index + 1}`)}</span><span className="row-meta">{text('查看这次任务关联的会话内容', 'View the session linked to this task')}</span></span><UiIcon name="chevronRight" /></button>)}</div>
    </Dialog>
  </div>;
}
