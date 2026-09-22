import assert from 'node:assert/strict';
import test from 'node:test';
import {
  cancelWorkerTask,
  readWorkerTask,
  WorkerTaskPager,
  workerTaskDisplayTitle,
  workerTaskState,
} from '../src/features/worker-task-client.ts';

const presentation = planId => ({ unnamedTask: 'Task', unavailableBrief: 'Unavailable', routeName: planId });

function view(taskId, overrides = {}) {
  return {
    task_id: taskId,
    title: `Title ${taskId}`,
    brief: `Brief ${taskId}`,
    content_availability: 'available',
    plan_id: 'plan/one',
    plan_revision: 3,
    latest_admission_sequence: '8',
    latest_run_id: `run/${taskId}`,
    run: {
      task_id: taskId,
      run_id: `run/${taskId}`,
      state: 'running',
      state_revision: 4,
      cleanup: 'pending',
      result_available: false,
      admission_sequence: '8',
      accepted_at_ms: 200,
      accepted_time_state: 'recorded',
      executor: { harness: 'codex_cli', display_name: 'Codex CLI', basis: 'frozen_plan' },
    },
    resumable_until_ms: null,
    session_ids: [],
    session_links_complete: true,
    created_at_ms: 100,
    ...overrides,
  };
}

function envelope(data, nextActions = []) {
  return { status: 'succeeded', data, next_actions: nextActions, error: null };
}

test('worker task states remain distinct in the Desktop projection', () => {
  assert.deepEqual(
    ['accepted', 'preparing', 'running', 'cancelling', 'succeeded', 'failed', 'cancelled', 'unknown'].map(workerTaskState),
    ['queued', 'queued', 'running', 'cancelling', 'complete', 'failed', 'cancelled', 'unknown'],
  );
});

test('one instance pager preserves the opaque cursor and never sends an agent id', async () => {
  const calls = [];
  const replies = [
    envelope({ schema: 'worker_task_list.v1', tasks: [view('one')], next_cursor: 'opaque/page-2' }),
    envelope({ schema: 'worker_task_list.v1', tasks: [view('one'), view('two')], next_cursor: null }),
  ];
  const pager = new WorkerTaskPager(presentation, async (command, args) => {
    calls.push({ command, input: args.input });
    return replies.shift();
  });

  const first = await pager.next(12);
  const second = await pager.next(12);

  assert.deepEqual(first.tasks.map(task => task.taskId), ['one']);
  assert.deepEqual(second.tasks.map(task => task.taskId), ['two']);
  assert.equal(second.hasMore, false);
  assert.deepEqual(calls, [
    { command: 'worker_task_list', input: { cursor: null, limit: 12 } },
    { command: 'worker_task_list', input: { cursor: 'opaque/page-2', limit: 12 } },
  ]);
  assert.equal('agent_id' in calls[0].input, false);
});

test('a failed page leaves the cursor unchanged for retry', async () => {
  const cursors = [];
  let attempt = 0;
  const pager = new WorkerTaskPager(presentation, async (_command, args) => {
    cursors.push(args.input.cursor);
    attempt++;
    if (attempt === 1) throw new Error('temporary');
    return envelope({ schema: 'worker_task_list.v1', tasks: [], next_cursor: null });
  });

  await assert.rejects(pager.next());
  await pager.next();
  assert.deepEqual(cursors, [null, null]);
});

test('concurrent page reads share one native request', async () => {
  let calls = 0;
  let release;
  const waiting = new Promise(resolve => { release = resolve; });
  const pager = new WorkerTaskPager(presentation, async () => {
    calls++;
    await waiting;
    return envelope({ schema: 'worker_task_list.v1', tasks: [view('one')], next_cursor: null });
  });
  const first = pager.next();
  const second = pager.next();
  release();
  assert.deepEqual(await first, await second);
  assert.equal(calls, 1);
});

test('task detail uses the backend title and frozen executor without origin inference', async () => {
  const calls = [];
  const task = view('detail', {
    title: 'Backend supplied title',
    run: {
      ...view('detail').run,
      state: 'succeeded',
      result_available: true,
      executor: { harness: 'claude_code', display_name: 'Claude Code 4', basis: 'frozen_plan' },
    },
  });
  const projected = await readWorkerTask({ taskId: 'detail', runId: 'run/detail' }, presentation, async (command, args) => {
    calls.push({ command, input: args.input });
    if (command === 'worker_task_status') return envelope({ schema: 'worker_task_status.v1', task });
    return envelope({ schema: 'worker_task_result.v1', run: task.run, text: 'done', next_offset: null, incomplete: false });
  });

  assert.equal(projected.title, 'Backend supplied title');
  assert.equal(projected.executor, 'Claude Code 4');
  assert.equal(projected.result, 'done');
  assert.deepEqual(calls[0], {
    command: 'worker_task_status',
    input: { task_id: 'detail', run_id: 'run/detail', submission_key: null, submission_operation: null },
  });
  assert.equal('agent_id' in calls[0].input, false);
});

test('task list title truncation counts Unicode scalars', () => {
  const title = `${'界'.repeat(59)}😀more`;
  assert.equal(workerTaskDisplayTitle(title), `${'界'.repeat(59)}😀…`);
});

test('refresh discovers an external Continue and never keeps the previous result', async () => {
  const oldView = view('continued', { run: { ...view('continued').run, state: 'succeeded', result_available: true } });
  const old = await readWorkerTask({ taskId: 'continued' }, presentation, async command =>
    command === 'worker_task_status'
      ? envelope({ task: oldView })
      : envelope({ text: 'old result', next_offset: null, incomplete: false }));
  const next = view('continued', { latest_run_id: 'run/new', run: { ...view('continued').run, run_id: 'run/new' } });
  const refreshed = await readWorkerTask({ taskId: old.taskId }, presentation, async (command, args) => {
    assert.equal(command, 'worker_task_status');
    assert.equal(args.input.run_id, null);
    return envelope({ task: next });
  }, old);
  assert.equal(refreshed.runId, 'run/new');
  assert.equal(refreshed.status, 'running');
  assert.notEqual(refreshed.result, 'old result');
  assert.equal(refreshed.resultAvailable, false);
});

test('terminal refresh preserves loaded pages but refreshes cleanup and clears withdrawn content', async () => {
  const task = view('finished', { run: { ...view('finished').run, state: 'succeeded', result_available: true } });
  const previous = await readWorkerTask({ taskId: 'finished' }, presentation, async command =>
    command === 'worker_task_status' ? envelope({ task })
      : envelope({ text: 'page one', next_offset: 10, incomplete: true }));
  previous.result = 'page one plus page two';
  previous.resultNextOffset = 20;
  const refreshed = await readWorkerTask({ taskId: 'finished' }, presentation, async command => {
    assert.equal(command, 'worker_task_status');
    return envelope({ task: { ...task, run: { ...task.run, cleanup: 'complete' } } });
  }, previous);
  assert.equal(refreshed.result, previous.result);
  assert.equal(refreshed.resultNextOffset, 20);
  const withdrawn = await readWorkerTask({ taskId: 'finished' }, presentation, async command => {
    assert.equal(command, 'worker_task_status');
    return envelope({ task: { ...task, content_availability: 'unavailable', run: { ...task.run, result_available: false } } });
  }, refreshed);
  assert.equal(withdrawn.resultAvailable, false);
  assert.notEqual(withdrawn.result, previous.result);
});

test('same-name tasks remain separate and a hidden title falls back to task identity', async () => {
  const pager = new WorkerTaskPager(presentation, async () => envelope({
    schema: 'hiroute.delegation-list/v1',
    tasks: [
      view('first', { title: 'Repeated title' }),
      view('second', { title: 'Repeated title' }),
      view('hidden', { title: null, brief: null, content_availability: 'unavailable' }),
    ],
    next_cursor: null,
  }));
  const page = await pager.next();
  assert.deepEqual(page.tasks.map(task => task.taskId), ['first', 'second', 'hidden']);
  assert.equal(page.tasks[2].title, 'Task hidden');
});

test('an uncertain cancel retry reuses one idempotency key and waits on the exact run', async () => {
  const task = {
    taskId: 'task/cancel',
    runId: 'run/cancel/1',
    title: 'Cancel me',
    routeName: 'Plan',
    status: 'running',
    brief: '',
    facts: [],
  };
  const calls = [];
  const invokeWorker = async (command, args) => {
    calls.push({ command, input: args.input });
    if (command === 'worker_task_cancel' && calls.length === 1) throw new Error('reply lost');
    if (command === 'worker_task_cancel') return envelope({
      schema: 'hiroute.delegation-cancel/v1',
      operation_id: 'operation/cancel',
      run: { ...view('cancel').run, run_id: task.runId, state: 'cancelling', state_revision: 5 },
    });
    return envelope({
      schema: 'hiroute.delegation-wait/v1',
      run: { ...view('cancel').run, run_id: task.runId, state: 'cancelled', state_revision: 6 },
      changed: true,
      timed_out: false,
    });
  };

  assert.equal(await cancelWorkerTask(task, undefined, invokeWorker), 'cancelled');
  assert.equal(calls[0].input.idempotency_key, calls[1].input.idempotency_key);
  assert.deepEqual(calls[2], {
    command: 'worker_task_wait',
    input: { run_id: task.runId, after_revision: 5, wait_timeout_secs: 5 },
  });
});
