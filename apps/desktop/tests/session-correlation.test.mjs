import assert from 'node:assert/strict';
import test from 'node:test';

import { sessionAgentLabel, sessionCorrelationLabel } from '../src/features/session-correlation.ts';

test('session correlation provenance has distinct user-facing labels', () => {
  assert.deepEqual(
    [
      'agent_supplied',
      'verified_worker',
      'inferred',
      'request_scoped',
      'unknown',
    ].map(sessionCorrelationLabel),
    ['Agent 提供', '执行关联已验证', '推断关联', '请求级', '关联未知'],
  );
});

test('missing legacy provenance is presented as unknown', () => {
  assert.equal(sessionCorrelationLabel(undefined), '关联未知');
  assert.equal(sessionCorrelationLabel(null), '关联未知');
});

test('verified executors have one shared label without acquiring a managed Agent identity', () => {
  for (const [executor, name] of [['codex', 'Codex'], ['claude', 'Claude Code'], ['dsh', 'DeepSeek Harness'],
    ['pi', 'Pi'], ['qoder', 'Qoder'], ['opencode', 'OpenCode']]) {
    const session = Object.freeze({ agent_id: '', executor });
    assert.equal(sessionAgentLabel(session, 'zh'), `${name}（委派执行）`);
    assert.equal(sessionAgentLabel(session, 'en'), `${name} (delegated)`);
    assert.equal(session.agent_id, '');
  }
});

test('missing, conflicting and unknown executor evidence cannot name an Agent by guesswork', () => {
  assert.equal(sessionAgentLabel({ agent_id: '' }, 'zh'), '未关联 Agent');
  assert.equal(sessionAgentLabel({ executor: null, correlation_kind: 'verified_worker' }, 'en'), 'Delegated executor unknown');
  for (const executor of ['future', '__proto__', '']) {
    assert.equal(sessionAgentLabel({ executor }, 'zh'), '委派执行器未确认');
  }
  assert.equal(sessionAgentLabel({ agent_id: 'agent_hermes_default' }, 'zh'), 'Hermes');
  assert.equal(sessionAgentLabel({ agent_id: 'agent_hermes_default', executor: 'codex' }, 'zh'), 'Codex（委派执行）');
});
