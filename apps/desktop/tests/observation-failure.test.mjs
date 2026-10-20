import assert from 'node:assert/strict';
import test from 'node:test';
import { observationBusyRetryMs } from '../src/features/observation-failure.ts';

test('native backend envelopes preserve a bounded quiet busy retry', () => {
  const error = { source: 'backend', envelope: { error: { code: 'OBSERVATION_UNAVAILABLE', details_schema: 'hiroute.observation.read-failure/v1', retryable: true, observation_read: { reason: 'reader_busy', retry_after_ms: 100 } } } };
  assert.equal(observationBusyRetryMs(error), 100);
  error.envelope.error.observation_read.retry_after_ms = 10000;
  assert.equal(observationBusyRetryMs(error), 1000);
  assert.equal(observationBusyRetryMs(new Error('OBSERVATION_QUERY_BUSY')), 100);
  assert.equal(observationBusyRetryMs({ source: 'backend', envelope: { error: { code: 'SCOPE_TOO_LARGE', observation_read: { reason: 'timeout' } } } }), null);
  assert.equal(observationBusyRetryMs({ envelope: { error: { code: 'CAPABILITY_DENIED' } } }), null);
});
