import { test } from 'node:test';
import assert from 'node:assert/strict';
import { agentEcosystem, agentPageEntries, agentSupportsCollaboration, agentSupportsManagement } from '../src/features/agents/ecosystems.ts';
import { agentSettingsSpec } from '../src/features/agents/settings-request.ts';
import { agentActionErrorMessage } from '../src/features/agents/mutation-feedback.ts';
import { nativeCodexDesktopLaunch, codexDesktopLaunchCommand } from '../src/features/agents/codex-desktop-launch.ts';

test('Desktop is a peer connection and never borrows CLI profile context', () => {
  const desktop = { agent_id: 'agent_codex_desktop', context_id: 'context/desktop', settings: null };
  const cli = { agent_id: 'agent_codex_default', context_id: 'context/cli', settings: null,
    codex_access: { slot_occupied: false, root_context_id: 'context/root', profile_context_id: 'context/cli' } };
  const entries = agentPageEntries([cli, desktop]);
  assert.equal(entries.filter(a => a.agent_id === desktop.agent_id).length, 1);
  assert.equal(entries.findIndex(a => a.agent_id === desktop.agent_id), entries.findIndex(a => a.agent_id === cli.agent_id) + 1);
  assert.equal(agentEcosystem(desktop.agent_id), 'codex');
  assert.equal(agentSupportsCollaboration(desktop.agent_id), false);
  assert.equal(agentSupportsManagement(desktop.agent_id), false);
  const draft = { codexMode: 'profile', values: { nativeModelMode: 'hiroute_only', fixedModels: [], allowedPlanIds: ['plan/one'], defaultChoice: { kind: 'plan', plan_id: 'plan/one' } } };
  assert.equal(agentSettingsSpec(desktop, 'model', false, draft).context_id, 'context/desktop');
  assert.equal(agentSettingsSpec(cli, 'model', false, draft).context_id, 'context/cli');
});

test('launch failure keeps completed configuration and asks for launch retry', () => {
  assert.match(agentActionErrorMessage('CODEX_DESKTOP_LAUNCH_FAILED', 'en'), /Saved model routing remains active/);
  assert.match(agentActionErrorMessage('CODEX_DESKTOP_LAUNCH_FAILED', 'zh'), /再次点击/);
});

test('native Desktop launch uses macOS Desktop and Windows Shell, while consoles copy a local command', () => {
  for (const [host, platform, supported] of [['tauri','macos',true],['shell-console','windows',true],['console','windows',false],['console','linux',false],['tauri','linux',false]]) {
    assert.equal(nativeCodexDesktopLaunch({host,platform}),supported);
  }
  const agent={agent_id:'agent_codex_desktop',context_id:'context/desktop',available_surfaces:['codex_desktop'],codex_desktop:{},settings:{state:'configured',operation_state:'succeeded',current_selection:{}}};
  assert.equal(codexDesktopLaunchCommand(agent),"hiroute agents desktop launch 'context/desktop' --output json");
  for (const change of [{agent_id:'agent_codex_default'},{context_id:"context/'$(command)"},{context_id:'context\nother'},{status_error:'unavailable'},{settings:{state:'pending'}},{codex_desktop:null}]) assert.equal(codexDesktopLaunchCommand({...agent,...change}),null);
});
