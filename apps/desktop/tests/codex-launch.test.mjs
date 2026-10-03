import { test } from 'node:test';
import assert from 'node:assert/strict';
import { confirmedCodexLaunchCommand, preferredCodexShell } from '../src/features/agents/codex-launch.ts';

const target = { operationId: 'op/new-profile', contextId: 'context/profile' };
const operation = { operation_id: target.operationId, state: 'succeeded', sequence: 2, cancellable: false };
const agent = {
  agent_id: 'agent_codex_default', status_error: null,
  settings: { state: 'configured' },
  codex_access: {
    slot_occupied: true, selected_mode: 'profile', profile_context_id: target.contextId,
    pending_operation: null, access_revoked: false,
    commands: { 'bash/zsh': "CODEX_HOME='/path with spaces' codex --profile hiroute", powershell: 'powershell-command' },
  },
};

test('copy uses the confirmed target command verbatim and chooses the platform shell', () => {
  assert.equal(confirmedCodexLaunchCommand(target, operation, agent, 'MacIntel'), agent.codex_access.commands['bash/zsh']);
  assert.equal(confirmedCodexLaunchCommand(target, operation, agent, 'Win32'), 'powershell-command');
  assert.equal(preferredCodexShell('Linux', { fish: 'fish-command' }), 'fish');
});

test('accepted, failed, cancelled, missing and unrelated Operations cannot trigger copying', () => {
  for (const state of ['accepted', 'preparing', 'pending', 'failed', 'cancelled']) {
    assert.equal(confirmedCodexLaunchCommand(target, { ...operation, state }, agent, 'MacIntel'), null);
  }
  assert.equal(confirmedCodexLaunchCommand(target, { ...operation, operation_id: 'op/old' }, agent, 'MacIntel'), null);
  assert.equal(confirmedCodexLaunchCommand(target, null, agent, 'MacIntel'), null);
});

test('unconfirmed, replaced, root and revoked targets cannot trigger copying after success', () => {
  for (const patch of [
    { slot_occupied: false }, { selected_mode: 'root' }, { profile_context_id: 'context/other' },
    { pending_operation: 'op/cleanup' }, { access_revoked: true }, { commands: {} },
  ]) {
    assert.equal(confirmedCodexLaunchCommand(target, operation, { ...agent, codex_access: { ...agent.codex_access, ...patch } }, 'MacIntel'), null);
  }
  assert.equal(confirmedCodexLaunchCommand(target, operation, { ...agent, settings: { state: 'pending' } }, 'MacIntel'), null);
  assert.equal(confirmedCodexLaunchCommand(target, operation, { ...agent, status_error: 'UNAVAILABLE' }, 'MacIntel'), null);
});
