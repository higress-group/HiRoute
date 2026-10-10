import assert from 'node:assert/strict';
import test from 'node:test';
import { subscriptionAttentionCopy, subscriptionFailureCopy } from '../src/features/models/subscription-copy.ts';

test('subscription failures use distinct recovery copy and never request an API key', () => {
  const updating = subscriptionAttentionCopy('subscription_updating', 'zh');
  const authentication = subscriptionAttentionCopy('authentication_required', 'zh');
  const notAllowed = subscriptionAttentionCopy('model_not_allowed', 'en');
  const runtime = subscriptionAttentionCopy('runtime_unavailable', 'en');

  assert.equal(updating.title, '正在更新订阅授权');
  assert.equal(authentication.title, '需要更新订阅登录');
  assert.match(notAllowed.detail, /retained/);
  assert.equal(runtime.title, 'Subscription service unavailable');
  assert.doesNotMatch(
    [updating, authentication, notAllowed, runtime].map(copy => `${copy.title} ${copy.detail}`).join(' '),
    /API key/i,
  );
});


test('subscription repair follows saved mode and separates local and service failures', () => {
  const managed = subscriptionAttentionCopy('authentication_required', 'zh', 'cpa_managed');
  const borrowed = subscriptionAttentionCopy('authentication_required', 'zh', 'native_borrowed');
  assert.match(managed.detail, /HiRoute.*独立登录/);
  assert.doesNotMatch(managed.detail, /原生客户端/);
  assert.match(borrowed.detail, /原生客户端/);
  assert.match(subscriptionFailureCopy('SUBSCRIPTION_NATIVE_ACCOUNT_MISSING', 'zh'), /无法识别.*账号/);
  assert.match(subscriptionFailureCopy('SUBSCRIPTION_NATIVE_STORE_UNSUPPORTED', 'en'), /credential store.*independent/);
  assert.match(subscriptionFailureCopy('SUBSCRIPTION_MANAGED_LOGIN_REQUIRED', 'en'), /HiRoute/);
  const runtime = subscriptionFailureCopy('SUBSCRIPTION_RUNTIME_UNAVAILABLE', 'zh');
  assert.match(runtime, /服务和网络/);
  assert.doesNotMatch(runtime, /重新登录|原生客户端/);
  assert.equal(subscriptionFailureCopy('UNRELATED_ERROR', 'en'), null);
});
