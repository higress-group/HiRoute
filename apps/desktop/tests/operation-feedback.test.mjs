import assert from 'node:assert/strict';
import test from 'node:test';
import {
  acceptObservedOperation,
  currentPendingHint,
  isTerminalOperation,
  operationFeedback,
  pendingFeedbackIdentity,
  representedByTrial,
  shouldObservePendingOperation,
} from '../src/product/operation-feedback.ts';

const operation = (operation_id, sequence, state = 'accepted') => ({
  operation_id,
  sequence,
  state,
  cancellable: true,
  safe_error_code: null,
});

test('a late observation cannot replace a newly accepted identity', () => {
  const current = operation('operation/new', 1);
  assert.equal(acceptObservedOperation(current, operation('operation/old', 99)), current);
});

test('one identity advances monotonically by sequence', () => {
  const current = operation('operation/a', 4, 'running');
  assert.equal(acceptObservedOperation(current, operation('operation/a', 3)), current);
  assert.deepEqual(acceptObservedOperation(current, operation('operation/a', 5, 'succeeded')), operation('operation/a', 5, 'succeeded'));
});

test('a newly pending identity replaces retained terminal feedback', () => {
  assert.equal(shouldObservePendingOperation(operation('old', 4, 'succeeded'), { operation_id: 'new', idempotency_key: 'intent/new' }), true);
  assert.equal(shouldObservePendingOperation(operation('old', 4, 'succeeded'), { operation_id: null, idempotency_key: 'intent/new' }), true);
  assert.equal(shouldObservePendingOperation(operation('old', 3, 'running'), { operation_id: 'new', idempotency_key: 'intent/new' }), false);
  assert.equal(shouldObservePendingOperation(operation('old', 3, 'running'), { operation_id: null, idempotency_key: 'intent/new' }), false);
  assert.equal(shouldObservePendingOperation(operation('same', 4, 'succeeded'), { operation_id: 'same', idempotency_key: 'intent/same' }), false);
});

test('an older native pending snapshot cannot replace a newly accepted terminal operation', () => {
  const current = operation('operation/new', 1, 'succeeded');
  const stale = { operation_id: 'operation/old', idempotency_key: 'intent/old' };
  assert.equal(shouldObservePendingOperation(current, stale, 'intent/old'), false);
  assert.equal(shouldObservePendingOperation(current, { ...stale, operation_id: null }, 'intent/old'), false);
  assert.equal(shouldObservePendingOperation(current, { operation_id: 'operation/next', idempotency_key: 'intent/next' }, 'intent/old'), true);
  assert.equal(currentPendingHint(stale, 'intent/old'), null);
  assert.equal(currentPendingHint({ operation_id: 'operation/next', idempotency_key: 'intent/next' }, 'intent/old')?.operation_id, 'operation/next');
});

test('a lookup without an actual operation does not create a user-facing record', () => {
  assert.equal(pendingFeedbackIdentity(null, null), null);
  assert.equal(pendingFeedbackIdentity(operation('known', 1), null), 'known');
  assert.equal(pendingFeedbackIdentity(null, 'operation/known'), 'operation/known');
  assert.equal(pendingFeedbackIdentity(null, undefined), null);
});

test('operation feedback distinguishes progress, terminal results and observation failure', () => {
  const presentation = { kind: 'agent-settings', target: 'Codex 配置' };
  assert.equal(operationFeedback(operation('a', 1), '', 'zh', presentation).phase, 'pending');
  assert.match(operationFeedback(operation('a', 2, 'succeeded'), '', 'zh', presentation).title, /已完成/);
  assert.equal(operationFeedback(operation('a', 2, 'rolled_back'), '', 'en', presentation).phase, 'failed');
  assert.equal(operationFeedback(operation('a', 2, 'needs_attention'), '', 'en', presentation).phase, 'failed');
  const unverified = operationFeedback(operation('a', 1, 'running'), 'SERVICE_UNAVAILABLE', 'zh', presentation);
  assert.equal(unverified.phase, 'unverified');
  assert.match(unverified.detail, /可继续编辑/);
  assert.equal(operationFeedback(null, 'OPERATION_RESULT_UNVERIFIED', 'en', presentation).phase, 'unverified');
  assert.match(operationFeedback(null, 'OPERATION_RESULT_UNVERIFIED', 'zh', presentation).detail, /尚未查到/);
  assert.equal(operationFeedback(null, '', 'zh', presentation).phase, 'pending');
  assert.match(operationFeedback(operation('a', 1), 'OPERATION_IDENTITY_MISMATCH', 'en', presentation).detail, /[Aa]nother operation/);
});

test('all save surfaces use one neutral status while an observation is uncertain', () => {
  for (const kind of ['agent-settings', 'model-connection', 'routing', 'background']) {
    for (const error of ['SERVICE_UNAVAILABLE', 'OPERATION_IDENTITY_MISMATCH', 'OPERATION_RESULT_UNVERIFIED']) {
      const zh = operationFeedback(operation('a', 1, 'running'), error, 'zh', { kind });
      const en = operationFeedback(operation('a', 1, 'running'), error, 'en', { kind });
      assert.equal(zh.phase, 'unverified');
      assert.equal(en.phase, 'unverified');
      assert.match(zh.title, /结果待确认$/);
      assert.match(en.title, /result pending confirmation$/);
      assert.doesNotMatch(`${zh.title}${zh.detail}`, /上一项后台变更|状态暂不可用|查询失败/);
      assert.doesNotMatch(`${en.title}${en.detail}`, /Previous background change|status unavailable|query failed/i);
    }
    assert.equal(operationFeedback(operation('a', 2, 'succeeded'), 'SERVICE_UNAVAILABLE', 'zh', { kind }).phase, 'succeeded');
  }
});

test('only actual terminal server states end progress', () => {
  assert.equal(isTerminalOperation('succeeded'), true);
  assert.equal(isTerminalOperation('rolled_back'), true);
  assert.equal(isTerminalOperation('needs_attention'), true);
  assert.equal(isTerminalOperation('running'), false);
  assert.equal(isTerminalOperation('unknown'), false);
});

test('trial feedback replaces only the same known pending or successful operation', () => {
  assert.equal(representedByTrial('known', 'pending', '', false, ['known']), true);
  assert.equal(representedByTrial('known', 'succeeded', '', false, ['known']), true);
  assert.equal(representedByTrial('other', 'pending', '', false, ['known']), false);
  assert.equal(representedByTrial(null, 'pending', '', false, ['known']), false);
  assert.equal(representedByTrial('known', 'failed', '', false, ['known']), false);
  assert.equal(representedByTrial('known', 'unverified', '', false, ['known']), false);
  assert.equal(representedByTrial('known', 'pending', 'AUTHORITY_LOST', false, ['known']), false);
  assert.equal(representedByTrial('known', 'pending', '', true, ['known']), false);
  assert.equal(representedByTrial('known', 'pending', '', false, []), false);
});
