import assert from 'node:assert/strict';
import test from 'node:test';
import { collaborationCheckFailureMessage } from '../src/features/agents/collaboration-check-feedback.ts';

const schema = 'hiroute.agent-collaboration-check-failure/v1';
const reasons = [
  'login_required', 'installed_skill_missing', 'installed_skill_changed', 'installed_skill_invalid',
  'native_context_unavailable', 'native_context_changed', 'dependency_unavailable',
  'check_timed_out', 'verification_failed',
];
const sensitive = 'private-stage /private/native/settings token=secret-login-token';
const failure = reason => ({
  source: 'backend',
  envelope: {
    error: { details_schema: schema, message_key: sensitive },
    data: { schema, reason, stage: sensitive, path: sensitive, secret: sensitive },
  },
});

test('login failure directs the user to sign in normally with the selected client and retry', () => {
  assert.match(collaborationCheckFailureMessage(failure('login_required'), 'zh'), /正常打开已选客户端完成登录.*重试/);
  assert.match(collaborationCheckFailureMessage(failure('login_required'), 'en'), /selected client.*Open it normally, sign in, then retry/);
});

test('missing, changed and invalid installed Skills require target review and explicit safe recovery', () => {
  for (const reason of ['installed_skill_missing', 'installed_skill_changed', 'installed_skill_invalid']) {
    const zh = collaborationCheckFailureMessage(failure(reason), 'zh');
    const en = collaborationCheckFailureMessage(failure(reason), 'en');
    assert.match(zh, /检查协作目标.*安全恢复或重新启用.*重试/);
    assert.match(zh, /不会自动覆盖/);
    assert.match(en, /Review the collaboration target.*safely restore or re-enable.*retry/);
    assert.match(en, /will not be overwritten automatically/);
  }
});

test('unavailable or changed context and dependencies guide the user to select a usable target again', () => {
  for (const reason of ['native_context_unavailable', 'native_context_changed', 'dependency_unavailable']) {
    assert.match(collaborationCheckFailureMessage(failure(reason), 'zh'), /重新.*选择.*重试/);
    assert.match(collaborationCheckFailureMessage(failure(reason), 'en'), /[Ss]elect.*again.*retry/);
  }
});

test('timeout and failed verification remain unsuccessful and offer a retry', () => {
  assert.match(collaborationCheckFailureMessage(failure('check_timed_out'), 'zh'), /超时.*尚未确认协作可用.*重试/);
  assert.match(collaborationCheckFailureMessage(failure('check_timed_out'), 'en'), /timed out.*has not been verified.*retry/);
  assert.match(collaborationCheckFailureMessage(failure('verification_failed'), 'zh'), /验证未通过.*重试/);
  assert.match(collaborationCheckFailureMessage(failure('verification_failed'), 'en'), /verification failed.*retry/);
});

test('only a matching typed backend failure can produce collaboration feedback', () => {
  const wrongDetails = failure('login_required');
  wrongDetails.envelope.error.details_schema = 'hiroute.other-failure/v1';
  const wrongData = failure('login_required');
  wrongData.envelope.data.schema = 'hiroute.agent-collaboration-check-failure/v2';
  const noDetails = failure('login_required');
  delete noDetails.envelope.error.details_schema;
  const noData = failure('login_required');
  delete noData.envelope.data;
  for (const error of [
    null, undefined, sensitive, new Error(sensitive), [],
    wrongDetails, wrongData, noDetails, noData,
    { ...failure('login_required'), source: 'transport' },
    { schema, reason: 'login_required' },
    failure('future_reason'), failure(sensitive), failure('toString'), failure('__proto__'),
    failure({ reason: 'login_required', sensitive }), failure(['login_required']),
  ]) {
    for (const language of ['zh', 'en']) {
      assert.equal(collaborationCheckFailureMessage(error, language), null);
    }
  }
});

test('known failures never expose raw native details or claim a successful delegation', () => {
  for (const reason of reasons) {
    const error = failure(reason);
    const before = structuredClone(error);
    for (const language of ['zh', 'en']) {
      const message = collaborationCheckFailureMessage(error, language);
      assert.equal(typeof message, 'string');
      assert.doesNotMatch(message, /private-stage|\/private\/native|secret-login-token|token=/);
      assert.doesNotMatch(message, /Qoder|Codex|Claude|验证通过|完成委派|verification succeeded|delegation completed/);
    }
    assert.deepEqual(error, before);
  }
});
