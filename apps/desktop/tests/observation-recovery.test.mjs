import assert from 'node:assert/strict';
import { getEventListeners } from 'node:events';
import test from 'node:test';
import { invoke, setHostTransport } from '../src/host/index.ts';
import { CONSOLE_EXPIRED_EVENT, CONSOLE_UNAVAILABLE_EVENT } from '../src/host/console.ts';
import { observationReadOnce } from '../src/features/observation-client.ts';
import { readObservation, readObservationPages } from '../src/features/observation-recovery.ts';

const flush = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
};
const busy = (hint = 100) => ({ source: 'backend', envelope: { error: {
  code: 'OBSERVATION_UNAVAILABLE', retryable: true, details_schema: 'hiroute.observation.read-failure/v1',
  observation_read: { reason: 'reader_busy', retry_after_ms: hint, action: 'retry' },
} } });

function host(t, read) {
  const visibility = Object.assign(new EventTarget(), { hidden: false });
  globalThis.document = visibility;
  setHostTransport({
    capabilities: () => ({ host: 'console' }),
    call: (command, payload) => {
      assert.equal(command, 'observation_read');
      assert.equal(payload.request.read_failure_details, true);
      return read(payload.request.intent);
    },
  });
  t.after(() => { setHostTransport(undefined); delete globalThis.document; });
  return visibility;
}

function clock(t) {
  t.mock.timers.enable({ apis: ['setTimeout', 'Date'], now: 0 });
  t.mock.method(performance, 'now', () => Date.now());
  return async ms => { t.mock.timers.tick(ms); await flush(); };
}

test('busy attempts are paced and leave a slot free, then return the actual fourth result', async t => {
  const advance = clock(t);
  const times = [];
  const query = { cursor: null, from_ms: 17, to_ms: 19 };
  host(t, async intent => {
    if (intent.view === 'home_value') return 'usage';
    assert.deepEqual(intent.query, query);
    times.push(Date.now());
    if (times.length < 4) throw busy(times.length === 2 ? -5 : 100);
    return 'sessions';
  });
  const result = readObservation('sessions', query, { signal: new AbortController().signal });
  await flush();
  assert.equal(await observationReadOnce('home_value', {}), 'usage');
  await advance(399);
  assert.deepEqual(times, [0]);
  await advance(1);
  await advance(1000);
  await advance(2000);
  assert.equal(await result, 'sessions');
  assert.deepEqual(times, [0, 400, 1400, 3400]);
});

test('a stale continuation replays only the requested range and discards partial generations', async t => {
  const advance = clock(t);
  const cursors = [];
  const states = [];
  let rebuilds = 0;
  host(t, async ({ query }) => {
    assert.equal(query.keyword, 'fixed');
    assert.equal(query.from_ms, 11);
    assert.equal(query.to_ms, 12);
    cursors.push(query.cursor);
    if (query.cursor === 'old' || query.cursor === 'new1') throw { code: 'CHANGE_PREVIEW_STALE' };
    if (!query.cursor) return { values: [`generation-${rebuilds}`], next_cursor: `new${rebuilds}` };
    return { values: ['next'], next_cursor: 'not-requested' };
  });
  const result = readObservationPages('session_search', { cursor: 'old', keyword: 'fixed', from_ms: 11, to_ms: 12 }, 1, {
    signal: new AbortController().signal, onRebuild: () => { rebuilds++; }, onRecovery: state => states.push(state),
  });
  await flush();
  await advance(400);
  assert.deepEqual(cursors, ['old', null, 'new1']);
  await advance(999);
  assert.equal(cursors.length, 3);
  await advance(1);
  assert.deepEqual(cursors, ['old', null, 'new1', null, 'new2']);
  assert.deepEqual(await result, { rebuilt: true, pages: [{ values: ['generation-2'], next_cursor: 'new2' }, { values: ['next'], next_cursor: 'not-requested' }] });
  assert.deepEqual(states, ['rebuilding']);
});

test('long recovery is disclosed at ten seconds and sustained reads back off to thirty seconds', async t => {
  const advance = clock(t);
  const times = [], states = [];
  host(t, async () => { times.push(Date.now()); throw busy(NaN); });
  const controller = new AbortController();
  const result = assert.rejects(readObservation('sessions', {}, {
    signal: controller.signal, onRecovery: state => states.push([state, Date.now()]),
  }), { name: 'AbortError' });
  await flush();
  for (const ms of [400, 1000, 2000, 4000, 2600, 5400, 15000]) await advance(ms);
  assert.deepEqual(states, [['retrying', 0], ['slow', 10000]]);
  assert.deepEqual(times, [0, 400, 1400, 3400, 7400, 15400, 30400]);
  for (let i = 0; i < 9; i++) await advance(30000);
  assert.ok(times.slice(7).every((time, index) => time - times[index + 6] === 30000));
  controller.abort();
  await result;
  await advance(30000);
  assert.equal(times.length, 16);
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});

test('queued cancellation frees a pending place and sent calls hold both real slots until they end', async t => {
  const started = [];
  const gates = [deferred(), deferred()];
  host(t, async ({ query }) => {
    started.push(query.id);
    if (query.id < 2) await gates[query.id].promise;
    return query.id;
  });
  const original = new AbortController();
  const a = assert.rejects(readObservation('content', { id: 0 }, { signal: original.signal }), { name: 'AbortError' });
  const b = observationReadOnce('content', { id: 1 });
  const queued = Array.from({ length: 32 }, (_, index) => {
    const controller = new AbortController();
    const result = assert.rejects(observationReadOnce('content', { id: index + 2 }, controller.signal), { name: 'AbortError' });
    return { controller, result };
  });
  await assert.rejects(observationReadOnce('content', { id: 34 }), { code: 'OBSERVATION_QUERY_BUSY' });
  original.abort();
  queued[12].controller.abort();
  await queued[12].result;
  const c = observationReadOnce('content', { id: 35 });
  for (const item of queued) item.controller.abort();
  await Promise.all(queued.map(item => item.result));
  assert.deepEqual(started, [0, 1]);
  gates[0].resolve();
  assert.equal(await c, 35);
  await a;
  gates[1].resolve();
  assert.equal(await b, 1);
  assert.deepEqual(started, [0, 1, 35]);
  assert.ok(queued.every(item => getEventListeners(item.controller.signal, 'abort').length === 0));
});

test('hidden waits do not send calls, and abort removes visibility listeners', async t => {
  const advance = clock(t);
  let attempts = 0;
  const visibility = host(t, async () => {
    if (++attempts === 1) throw busy();
    return 'loaded';
  });
  const controller = new AbortController();
  const result = readObservation('sessions', {}, { signal: controller.signal });
  await flush();
  visibility.hidden = true;
  visibility.dispatchEvent(new Event('visibilitychange'));
  await advance(30000);
  assert.equal(attempts, 1);
  assert.equal(getEventListeners(visibility, 'visibilitychange').length, 1);
  visibility.hidden = false;
  visibility.dispatchEvent(new Event('visibilitychange'));
  visibility.dispatchEvent(new Event('visibilitychange'));
  assert.equal(await result, 'loaded');
  assert.equal(attempts, 2);
  visibility.hidden = true;
  const waiting = assert.rejects(readObservation('content', {}, { signal: controller.signal }), { name: 'AbortError' });
  controller.abort();
  await waiting;
  assert.equal(getEventListeners(visibility, 'visibilitychange').length, 0);
  assert.equal(attempts, 2);
});

test('the real console transport leaves a recovering read on its page and still expires authorization', async t => {
  const advance = clock(t);
  const storage = new Map([['hiroute.console.session', 'synthetic-test-session']]);
  globalThis.document = Object.assign(new EventTarget(), { hidden: false });
  globalThis.window = Object.assign(new EventTarget(), { sessionStorage: {
    getItem: key => storage.get(key) ?? null,
    setItem: (key, value) => storage.set(key, value),
    removeItem: key => storage.delete(key),
  } });
  t.after(() => { delete globalThis.window; delete globalThis.document; });
  const events = [];
  for (const event of [CONSOLE_UNAVAILABLE_EVENT, CONSOLE_EXPIRED_EVENT]) {
    window.addEventListener(event, () => events.push(event));
  }
  const calls = [];
  let fail = true;
  let expired = false;
  t.mock.method(globalThis, 'fetch', async (path, request) => {
    calls.push(path);
    assert.equal(request.headers.get('Authorization'), 'Bearer synthetic-test-session');
    if (fail) throw new TypeError('temporary network error');
    return expired ? Response.json({ code: 'CONSOLE_AUTH_REQUIRED' }, { status: 401 }) : Response.json({ sessions: ['ready'] });
  });
  const read = () => readObservation('sessions', { cursor: null }, { signal: new AbortController().signal });
  const result = read();
  await flush();
  assert.deepEqual(events, []);
  fail = false;
  await advance(400);
  assert.deepEqual(await result, { sessions: ['ready'] });
  assert.deepEqual(events, []);
  fail = true;
  await assert.rejects(invoke('desktop_snapshot'), { code: 'CONSOLE_UNAVAILABLE' });
  assert.deepEqual(events, [CONSOLE_UNAVAILABLE_EVENT]);
  fail = false;
  expired = true;
  await assert.rejects(read(), { code: 'CONSOLE_AUTH_REQUIRED' });
  assert.deepEqual(events, [CONSOLE_UNAVAILABLE_EVENT, CONSOLE_EXPIRED_EVENT]);
  assert.equal(storage.has('hiroute.console.session'), false);
  assert.deepEqual(calls.map(path => path.split('/').at(-1)), ['observation_read', 'observation_read', 'desktop_snapshot', 'observation_read']);
});
