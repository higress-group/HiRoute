import assert from 'node:assert/strict';
import test from 'node:test';
import { checkFailureCode, modelAvailabilityMessage, modelObservationMessage, safeConnectionErrorCode } from '../src/features/model-connections/copy.ts';
import { saveFailureDefinitelyPreAdmission } from '../src/features/subscriptions/model.ts';

test('nested native failures retain specific copy but classify pre-admission by machine code', () => {
  const failure = { source: 'backend', envelope: { error: { code: 'REVISION_CONFLICT', message_key: 'compute.registered_catalog_changed' } } };
  assert.equal(safeConnectionErrorCode(failure), 'compute.registered_catalog_changed');
  assert.equal(safeConnectionErrorCode(failure, true), 'REVISION_CONFLICT');
  assert.equal(saveFailureDefinitelyPreAdmission(safeConnectionErrorCode(failure, true)), true);
  assert.equal(saveFailureDefinitelyPreAdmission({ source: 'backend', envelope: { error: { code: 'RESOURCE_NOT_FOUND' }, operation: null } }), true);
  assert.equal(saveFailureDefinitelyPreAdmission({ source: 'backend', envelope: { error: { code: 'RESOURCE_NOT_FOUND' }, operation: { operation_id: 'operation/1' } } }), false);
  assert.equal(saveFailureDefinitelyPreAdmission({ source: 'transport', failure: { code: 'DEADLINE' } }), false);
  assert.equal(saveFailureDefinitelyPreAdmission(safeConnectionErrorCode({ failure: { code: 'DEADLINE' } }, true)), false);
  assert.equal(safeConnectionErrorCode({ error: { message_key: 'private raw credential text' } }), 'MODEL_CONNECTION_FAILED');
});

test('model availability reasons stay product-facing', () => {
  assert.equal(modelAvailabilityMessage('model_connections.capability_required', true), '需要补充模型能力信息。');
  assert.equal(modelAvailabilityMessage('model_connections.connection_check_required', false), 'Read and verify the connection again.');
  assert.doesNotMatch(modelAvailabilityMessage('internal.new_reason', true), /internal|model_connections/);
});

test('failed tool probe reports credential rejection before tool compatibility', () => {
  assert.equal(checkFailureCode({ authentication: 'rejected', inference: 'failed', issues: [{ code: 'INFERENCE_FAILED', message_key: 'x', retryable: true }] }), 'MODEL_CONNECTION_AUTHENTICATION_REJECTED');
  assert.equal(checkFailureCode({ authentication: 'verified', inference: 'failed', issues: [{ code: 'TOOL_CALL_NOT_VERIFIED', message_key: 'x', retryable: true }] }), 'MODEL_TOOL_CALL_NOT_VERIFIED');
  assert.equal(checkFailureCode({ authentication: 'unknown', inference: 'failed', issues: [{ code: 'INFERENCE_FAILED', message_key: 'x', retryable: true }] }), 'MODEL_INFERENCE_FAILED');
  assert.equal(checkFailureCode({ authentication: 'verified', inference: 'not_run' }), null);
});

test('observation labels separate catalog membership from this check', () => {
  const model = { upstream_model_id: 'observed' };
  const complete = { directory: 'available', reachability: 'reachable', observed_model_ids: ['observed'] };
  assert.equal(modelObservationMessage(complete, model, false), 'Returned by this check');
  assert.equal(modelObservationMessage(complete, { upstream_model_id: 'other' }, false), 'Not returned by this check');
  assert.equal(modelObservationMessage(complete, { upstream_model_id: 'other' }, true), '本次检查未返回');
  const partial = { directory: 'partial', reachability: 'reachable', observed_model_ids: [] };
  assert.equal(modelObservationMessage(partial, model, false), 'Not confirmed by this check');
  const failed = { directory: 'unavailable', reachability: 'transport_failed', observed_model_ids: [] };
  assert.equal(modelObservationMessage(failed, model, false), 'Not confirmed by this check');
  const empty = { directory: 'empty', reachability: 'reachable', observed_model_ids: [] };
  assert.equal(modelObservationMessage(empty, model, false), 'Not returned by this check');
});
