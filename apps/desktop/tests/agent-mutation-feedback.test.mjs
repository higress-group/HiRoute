import assert from 'node:assert/strict';
import test from 'node:test';
import { agentActionErrorMessage, agentSettingsFailureMessage, agentDisableMessage, classifyAgentMutation } from '../src/agent-mutation-feedback.ts';

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

test('disable feedback reports success, verified rollback and uncertain recovery separately', () => {
  assert.match(agentDisableMessage('succeeded', 'zh'), /连接已停用/);
  assert.match(agentDisableMessage('rolled_back', 'zh'), /原连接仍有效/);
  assert.match(agentDisableMessage('rolled_back', 'en'), /rolled back/);
  assert.match(agentDisableMessage('needs_attention', 'zh'), /回滚尚未确认/);
  assert.doesNotMatch(agentDisableMessage('needs_attention', 'zh'), /原连接仍有效|连接已停用/);
  assert.doesNotMatch(agentDisableMessage('activating', 'en'), /is disabled|remains active/);
});

test('stable Codex merge conflicts explain the file problem instead of asking for stale retries', () => {
  const failure = { source: 'backend', envelope: { error: { code: 'CAPABILITY_DENIED', message_key: 'error.codex_profile_conflict' } } };
  assert.match(agentSettingsFailureMessage(failure, 'zh'), /无法安全合并/);
  assert.match(agentSettingsFailureMessage(failure, 'en'), /file format/);
  assert.doesNotMatch(agentSettingsFailureMessage(failure, 'zh'), /配置在保存前发生变化|再次保存以重新读取/);
  assert.equal(agentSettingsFailureMessage({ ...failure, source: 'native' }, 'zh'), null);
  assert.equal(agentSettingsFailureMessage({ source: 'backend', envelope: { error: { code: 'CHANGE_PREVIEW_STALE', message_key: 'error.codex_profile_conflict' } } }, 'en'), null);
  assert.equal(agentSettingsFailureMessage({ source: 'backend', envelope: { error: { code: 'CAPABILITY_DENIED', message_key: 'private native output' } } }, 'zh'), null);
});

test('DSH Desktop profile and bundled CLI failures explain different recovery actions', () => {
  const reason = message_key => ({ source: 'backend', envelope: { error: { code: 'ACTION_REQUIRED', message_key } } });
  assert.match(agentSettingsFailureMessage(reason('error.dsh_desktop_custom_profile'), 'zh'), /基础组合包和 Web 组合包/);
  assert.match(agentSettingsFailureMessage(reason('error.dsh_desktop_custom_profile'), 'en'), /default base and Web bundles/);
  assert.match(agentSettingsFailureMessage(reason('error.dsh_desktop_probe_failed'), 'zh'), /内置 CLI/);
  assert.match(agentSettingsFailureMessage(reason('error.dsh_desktop_probe_failed'), 'en'), /bundled .* CLI/);
  assert.equal(agentSettingsFailureMessage(reason('error.dsh_desktop_unknown'), 'en'), null);
});
