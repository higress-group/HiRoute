import assert from 'node:assert/strict';
import test from 'node:test';
import { subscriptionAttentionCopy } from '../src/features/models/subscription-copy.ts';

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
