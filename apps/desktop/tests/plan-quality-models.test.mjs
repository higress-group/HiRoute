import { test } from 'node:test';
import assert from 'node:assert/strict';
import { qualityExecutionModels } from '../src/features/plan-quality-models.ts';

const stage = (overrides = {}) => ({ segment_id: 'stage/a', session_id: 'session/a', attribution: 'single', execution_evidence_available: true, first_request_id: 'request/first', last_request_id: 'request/last', ...overrides });
test('historical model names require the exact session and request, not a nearby stage', async () => {
  const calls = [];
  const result = await qualityExecutionModels([stage()], async (session, request) => {
    calls.push([session, request]);
    return request === 'request/last'
      ? [{ session_id: 'session/other', request_id: request, final_native_model: 'wrong-model' }]
      : [{ session_id: session, request_id: request, final_native_model: 'historical-model' }];
  });
  assert.deepEqual(result, { 'stage/a': 'historical-model' });
  assert.deepEqual(calls, [['session/a', 'request/last'], ['session/a', 'request/first']]);
});
test('mixed, unknown and unavailable execution evidence cannot claim a single model', async () => {
  const result = await qualityExecutionModels([stage({ attribution: 'mixed' }), stage({ attribution: 'unknown' }), stage({ execution_evidence_available: false })], async () => { assert.fail('Unattributable stage must not read a model'); });
  assert.deepEqual(result, {});
});
test('missing or unreadable request evidence leaves a name unavailable without losing the stage', async () => {
  assert.deepEqual(await qualityExecutionModels([stage()], async () => { throw new Error('unavailable'); }), {});
  assert.deepEqual(await qualityExecutionModels([stage()], async () => []), {});
});
test('shared evidence is read once per page and successful last evidence avoids another query', async () => {
  let calls = 0;
  const result = await qualityExecutionModels([stage(), stage({ segment_id: 'stage/b' })], async (session, request) => {
    calls++;
    return [{ session_id: session, request_id: request, final_native_model: 'executed-model' }];
  });
  assert.deepEqual(result, { 'stage/a': 'executed-model', 'stage/b': 'executed-model' });
  assert.equal(calls, 1);
});
