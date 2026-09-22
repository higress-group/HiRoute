import type { AgentTask } from './AgentTasks';

type WorkerRun = {
  task_id: string;
  run_id: string;
  state: 'accepted' | 'preparing' | 'running' | 'cancelling' | 'succeeded' | 'failed' | 'cancelled' | 'unknown';
  state_revision: number;
  cleanup: 'pending' | 'complete' | 'unknown' | 'residual_acknowledged';
  result_available: boolean;
  admission_sequence: string | number;
  accepted_at_ms: number | null;
  accepted_time_state: 'recorded' | 'legacy_unavailable';
  executor: {
    harness: 'codex_cli' | 'claude_code' | null;
    display_name: string | null;
    basis: 'frozen_plan' | 'unavailable';
  };
};

type WorkerTaskView = {
  task_id: string;
  title: string | null;
  brief: string | null;
  content_availability: 'available' | 'unavailable' | 'indeterminate';
  plan_id: string;
  plan_revision: number;
  latest_admission_sequence: string | number;
  latest_run_id: string;
  run: WorkerRun;
  resumable_until_ms: number | null;
  session_ids: string[];
  session_links_complete: boolean;
  created_at_ms: number;
};

type WorkerTaskStatus = { schema: string; task: WorkerTaskView };
type WorkerTaskList = { schema: string; tasks: WorkerTaskView[]; next_cursor: string | null };
type WorkerTaskResult = {
  schema: string;
  run: WorkerRun;
  text: string | null;
  next_offset: number | null;
  incomplete: boolean;
};
type WorkerCancel = { schema: string; operation_id: string; run: WorkerRun };
type WorkerWait = { schema: string; run: WorkerRun; changed: boolean; timed_out: boolean };

export type WorkerNextAction = {
  command_id: string;
  input: Record<string, unknown>;
  reason_code: string;
};

type WorkerEnvelope<T> = {
  status: 'succeeded' | 'accepted' | 'usage_error' | 'conflict' | 'denied' | 'not_found' | 'unavailable' | 'action_required' | 'needs_attention' | 'internal_error';
  data?: T | null;
  next_actions: WorkerNextAction[];
  error?: { code: string; message_key: string } | null;
};

export type WorkerTaskLocator = {
  taskId: string;
  runId?: string | null;
};

export type WorkerTaskPresentation = {
  unnamedTask: string;
  unavailableBrief: string;
  routeName: string;
};

export type WorkerTaskListRead = {
  tasks: AgentTask[];
  hasMore: boolean;
};

export type WorkerInvoke = <T>(command: string, args: { input: unknown }) => Promise<T>;

async function nativeInvoke<T>(command: string, args: { input: unknown }): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(command, args);
}

function envelopeData<T>(envelope: WorkerEnvelope<T>): T {
  if (envelope.error || !['succeeded', 'accepted'].includes(envelope.status) || envelope.data == null) {
    const failure = new Error(envelope.error?.message_key ?? 'WORKER_RESPONSE_DATA_MISSING');
    Object.assign(failure, { code: envelope.error?.code ?? 'RESPONSE_DATA_MISSING', envelope });
    throw failure;
  }
  return envelope.data;
}

export function workerTaskState(state: WorkerRun['state']): AgentTask['status'] {
  if (state === 'succeeded') return 'complete';
  if (state === 'failed') return 'failed';
  if (state === 'cancelled') return 'cancelled';
  if (state === 'cancelling') return 'cancelling';
  if (state === 'accepted' || state === 'preparing') return 'queued';
  if (state === 'unknown') return 'unknown';
  return 'running';
}

export function workerTaskDisplayTitle(title: string, limit = 60): string {
  const scalars = Array.from(title);
  return scalars.length <= limit ? title : `${scalars.slice(0, limit).join('')}…`;
}

function executorName(run: WorkerRun): string | null {
  if (run.executor.basis !== 'frozen_plan') return null;
  return run.executor.display_name
    ?? (run.executor.harness === 'codex_cli'
      ? 'Codex CLI'
      : run.executor.harness === 'claude_code' ? 'Claude Code' : null);
}

async function readResult(
  runId: string,
  available: boolean,
  offset: number | null = null,
  invokeWorker: WorkerInvoke = nativeInvoke,
): Promise<{ text: string | null; nextOffset: number | null; incomplete: boolean; nextActions: WorkerNextAction[] }> {
  if (!available) return { text: null, nextOffset: null, incomplete: false, nextActions: [] };
  const envelope = await invokeWorker<WorkerEnvelope<WorkerTaskResult>>('worker_task_result', {
    input: { run_id: runId, offset, max_bytes: 65_536 },
  });
  const value = envelopeData(envelope);
  if (value.incomplete && (value.next_offset == null || value.next_offset === offset)) {
    throw new Error('WORKER_RESULT_PAGE_INVALID');
  }
  return {
    text: value.text,
    nextOffset: value.next_offset,
    incomplete: value.incomplete,
    nextActions: envelope.next_actions,
  };
}

function projectTask(
  task: WorkerTaskView,
  presentationFor: (planId: string) => WorkerTaskPresentation,
  detailsLoaded = false,
  result?: { text: string | null; nextOffset: number | null; incomplete: boolean; nextActions: WorkerNextAction[] },
  nextActions: WorkerNextAction[] = [],
): AgentTask {
  const presentation = presentationFor(task.plan_id);
  const title = task.title?.trim() || `${presentation.unnamedTask} ${task.task_id}`;
  return {
    taskId: task.task_id,
    runId: task.run.run_id,
    title,
    displayTitle: workerTaskDisplayTitle(title),
    executor: executorName(task.run),
    executorBasis: task.run.executor.basis,
    routeName: presentation.routeName,
    createdAtMs: task.created_at_ms,
    acceptedAtMs: task.run.accepted_at_ms,
    acceptedTimeState: task.run.accepted_time_state,
    admissionSequence: task.run.admission_sequence,
    status: workerTaskState(task.run.state),
    cleanup: task.run.cleanup,
    brief: task.brief?.trim() || presentation.unavailableBrief,
    contentAvailability: task.content_availability,
    result: result?.text,
    resultIncomplete: result?.incomplete ?? false,
    resultNextOffset: result?.nextOffset ?? null,
    detailsLoaded,
    resultAvailable: task.run.result_available,
    sessionId: task.session_ids[0] ?? null,
    sessionIds: task.session_ids,
    sessionsComplete: task.session_links_complete,
    nextActions: result?.nextActions.length ? result.nextActions : nextActions,
    facts: [],
  };
}

/** One instance-scoped pager. The server owns ordering and the opaque continuation cursor. */
export class WorkerTaskPager {
  private cursor: string | null | undefined;
  private readonly seen = new Set<string>();
  private pending: Promise<WorkerTaskListRead> | null = null;
  private readonly presentationFor: (planId: string) => WorkerTaskPresentation;
  private readonly invokeWorker: WorkerInvoke;

  constructor(
    presentationFor: (planId: string) => WorkerTaskPresentation,
    invokeWorker: WorkerInvoke = nativeInvoke,
  ) {
    this.presentationFor = presentationFor;
    this.invokeWorker = invokeWorker;
  }

  async next(limit = 50): Promise<WorkerTaskListRead> {
    if (this.cursor === null) return { tasks: [], hasMore: false };
    if (this.pending) return this.pending;
    this.pending = this.readPage(limit);
    try {
      return await this.pending;
    } finally {
      this.pending = null;
    }
  }

  private async readPage(limit: number): Promise<WorkerTaskListRead> {
    const envelope = await this.invokeWorker<WorkerEnvelope<WorkerTaskList>>('worker_task_list', {
      input: { cursor: this.cursor ?? null, limit },
    });
    const value = envelopeData(envelope);
    const tasks = value.tasks
      .filter(task => {
        if (this.seen.has(task.task_id)) return false;
        this.seen.add(task.task_id);
        return true;
      })
      .map(task => projectTask(task, this.presentationFor, false, undefined, envelope.next_actions));
    this.cursor = value.next_cursor;
    return { tasks, hasMore: this.cursor !== null };
  }
}

export async function readWorkerTask(
  locator: WorkerTaskLocator,
  presentationFor: (planId: string) => WorkerTaskPresentation,
  invokeWorker: WorkerInvoke = nativeInvoke,
  previous?: AgentTask,
): Promise<AgentTask> {
  const envelope = await invokeWorker<WorkerEnvelope<WorkerTaskStatus>>('worker_task_status', {
    input: {
      task_id: locator.taskId,
      run_id: locator.runId || null,
      submission_key: null,
      submission_operation: null,
    },
  });
  const task = envelopeData(envelope).task;
  // Completed result content is immutable. Preserve already loaded pages while
  // refreshing cleanup/latest-run facts; a Continue must fetch its own result.
  if (previous?.detailsLoaded && previous.taskId === task.task_id
    && previous.runId === task.run.run_id && previous.resultAvailable === task.run.result_available
    && previous.contentAvailability === task.content_availability
    && previous.status === workerTaskState(task.run.state)
    && ['complete', 'failed', 'cancelled'].includes(previous.status)) {
    return { ...projectTask(task, presentationFor, true, undefined, envelope.next_actions),
      result: previous.result, resultAvailable: previous.resultAvailable,
      resultIncomplete: previous.resultIncomplete, resultNextOffset: previous.resultNextOffset };
  }
  const result = await readResult(task.run.run_id, task.run.result_available, null, invokeWorker);
  return projectTask(task, presentationFor, true, result, envelope.next_actions);
}

export async function readWorkerTaskResultPage(
  task: AgentTask,
  invokeWorker: WorkerInvoke = nativeInvoke,
): Promise<AgentTask> {
  if (!task.runId || !task.resultIncomplete) return task;
  const page = await readResult(
    task.runId,
    task.resultAvailable === true,
    task.resultNextOffset ?? null,
    invokeWorker,
  );
  return {
    ...task,
    result: `${task.result ?? ''}${page.text ?? ''}` || null,
    resultIncomplete: page.incomplete,
    resultNextOffset: page.nextOffset,
    nextActions: page.nextActions,
  };
}

export async function cancelWorkerTask(
  task: AgentTask,
  onAccepted?: (status: AgentTask['status']) => void,
  invokeWorker: WorkerInvoke = nativeInvoke,
): Promise<AgentTask['status']> {
  const idempotencyKey = `desktop-cancel:${crypto.randomUUID()}`;
  const input = { run_id: task.runId, idempotency_key: idempotencyKey, reason: 'user_requested' };
  let envelope: WorkerEnvelope<WorkerCancel>;
  try {
    envelope = await invokeWorker<WorkerEnvelope<WorkerCancel>>('worker_task_cancel', { input });
  } catch {
    // A rejected invoke has no machine envelope, so acceptance is uncertain. Retry this
    // logical cancellation once with the exact same key; a backend envelope is definitive.
    envelope = await invokeWorker<WorkerEnvelope<WorkerCancel>>('worker_task_cancel', { input });
  }
  const value = envelopeData(envelope);
  const acceptedStatus = workerTaskState(value.run.state);
  onAccepted?.(acceptedStatus);
  if (value.run.state !== 'cancelling') return acceptedStatus;
  const waitedEnvelope = await invokeWorker<WorkerEnvelope<WorkerWait>>('worker_task_wait', {
    input: { run_id: task.runId, after_revision: value.run.state_revision, wait_timeout_secs: 5 },
  });
  return workerTaskState(envelopeData(waitedEnvelope).run.state);
}
