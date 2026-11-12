import assert from 'node:assert/strict';
import test from 'node:test';
import { observationAccessLost, observationBusyRetryMs, observationCancelled, observationRecoveryAction } from '../src/features/observation-failure.ts';

test('native backend envelopes preserve a bounded quiet busy retry', () => {
  const error = { source: 'backend', envelope: { error: { code: 'OBSERVATION_UNAVAILABLE', details_schema: 'hiroute.observation.read-failure/v1', retryable: true, observation_read: { reason: 'reader_busy', retry_after_ms: 100 } } } };
  assert.equal(observationBusyRetryMs(error), 400);
  error.envelope.error.observation_read.retry_after_ms = 600;
  assert.equal(observationBusyRetryMs(error), 600);
  error.envelope.error.observation_read.retry_after_ms = 10000;
  assert.equal(observationBusyRetryMs(error), 1000);
  assert.equal(observationBusyRetryMs(new Error('OBSERVATION_QUERY_BUSY')), 400);
  assert.equal(observationBusyRetryMs({ source: 'backend', envelope: { error: { code: 'SCOPE_TOO_LARGE', observation_read: { reason: 'timeout' } } } }), null);
  assert.equal(observationBusyRetryMs({ envelope: { error: { code: 'CAPABILITY_DENIED' } } }), null);
});

test('automatic recovery uses machine codes, the typed details schema, and retryability', () => {
  const error = {
    code: 'OBSERVATION_UNAVAILABLE',
    details_schema: 'hiroute.observation.read-failure/v1',
    retryable: true,
    observation_read: { reason: 'reader_busy', retry_after_ms: 100, action: 'retry' },
  };
  for (const reason of ['reader_busy', 'timeout', 'storage_unavailable']) {
    const details = { ...error, observation_read: { ...error.observation_read, reason } };
    assert.deepEqual(observationRecoveryAction({ source: 'backend', envelope: { error: details } }), { kind: 'retry', hint: 100 });
    assert.deepEqual(observationRecoveryAction({ error: details }), { kind: 'retry', hint: 100 });
  }
  for (const failure of [
    { ...error, retryable: false },
    { ...error, details_schema: undefined },
    { ...error, observation_read: { reason: 'index_unavailable' } },
    { ...error, code: 'SCOPE_TOO_LARGE' },
    { code: 'REVISION_CONFLICT', message_key: 'CHANGE_PREVIEW_STALE' },
    { code: 'CAPABILITY_DENIED' },
    { code: 'INVALID_ARGUMENTS' },
    { message_key: 'OBSERVATION_QUERY_BUSY' },
    'OBSERVATION_QUERY_BUSY',
  ]) assert.equal(observationRecoveryAction(failure).kind, 'stop');
  for (const code of ['schema_incompatible', 'peer_rejected', 'frame_invalid', 'frame_too_large', 'response_mismatch']) {
    assert.equal(observationRecoveryAction({ source: 'transport', failure: { code } }).kind, 'stop');
  }
  for (const code of ['deadline', 'transport_unavailable']) {
    assert.equal(observationRecoveryAction({ source: 'transport', failure: { code, submission: 'may_have_reached_server' } }).kind, 'retry');
  }
  for (const code of ['READ_SNAPSHOT_CHANGED', 'TRUSTED_AUTHORITY_BUSY', 'CONSOLE_UNAVAILABLE', 'OBSERVATION_QUERY_BUSY']) {
    assert.equal(observationRecoveryAction({ source: 'native', code }).kind, 'retry');
  }
  assert.equal(observationRecoveryAction({ source: 'native', code: 'TRUSTED_AUTHORITY_UNAVAILABLE' }).kind, 'stop');
  assert.equal(observationAccessLost({ source: 'native', code: 'TRUSTED_AUTHORITY_UNAVAILABLE' }), true);
  assert.equal(observationRecoveryAction({ source: 'backend', envelope: { error: { code: 'CHANGE_PREVIEW_STALE', retryable: false } } }).kind, 'rebuild');
  assert.equal(observationCancelled({ source: 'transport', failure: { code: 'observation_stopped' } }), true);
});
