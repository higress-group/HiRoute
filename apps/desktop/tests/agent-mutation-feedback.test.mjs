import assert from 'node:assert/strict';
import test from 'node:test';
import { agentActionErrorMessage, classifyAgentMutation } from '../src/agent-mutation-feedback.ts';

const operation = {
  operation_id: 'operation/agent-settings',
  state: 'accepted',
  sequence: 1,
  cancellable: true,
};

test('only an explicit pre-apply cancellation is presented as cancelled', () => {
  assert.equal(classifyAgentMutation({ state: 'cancelled_before_apply', operation: null }), 'cancelled');
});

test('a returned operation is submitted even when the native outcome is a recovery', () => {
  assert.equal(classifyAgentMutation({ state: 'original_operation_restored', operation }), 'submitted');
});

test('missing operation identity stays unverified instead of claiming no change', () => {
  assert.equal(classifyAgentMutation({ state: 'response_unknown', operation: null }), 'unverified');
  assert.equal(classifyAgentMutation({ state: 'unexpected_future_state', operation: null }), 'unverified');
});

test('a pre-admission Agent conflict tells the user no save occurred and offers a real retry', () => {
  for (const code of ['CHANGE_PREVIEW_STALE', 'application.error.revision_conflict']) {
    assert.match(agentActionErrorMessage(code, 'zh'), /本次未提交/);
    assert.match(agentActionErrorMessage(code, 'zh'), /再次保存/);
  }
  assert.match(agentActionErrorMessage('SERVICE_UNAVAILABLE', 'en'), /local service is unavailable/);
  assert.doesNotMatch(agentActionErrorMessage('SERVICE_UNAVAILABLE', 'en'), /enabled in Login Items/);
});
