import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from '@tauri-apps/api/mocks';
import { Agents, type AgentSnapshot } from '../../../src/agents';
import { PresentationRoot } from '../../../src/ui';
import { readyAgents } from './product-fixtures';
import '../../../src/occami/styles.css';

const claudeOf = (snapshot: AgentSnapshot) => snapshot.agents.find(agent => agent.agent_id === 'agent_claude_default')!;
const codexOf = (snapshot: AgentSnapshot) => snapshot.agents.find(agent => agent.agent_id === 'agent_codex_default')!;

function registered(): AgentSnapshot {
  return structuredClone(readyAgents);
}
function notRunnable(): AgentSnapshot {
  const snapshot = registered();
  const agent = claudeOf(snapshot);
  agent.configuration_state = 'executable_not_runnable';
  agent.version = '';
  agent.settings = {
    state: 'not_configured',
    model_verified: false,
    restore_point_ref: null,
    current_selection: null,
    collaboration: { state: 'not_configured', restore_point_ref: null, current_selection: null },
  };
  return snapshot;
}
function notRunnableWithRestore(): AgentSnapshot {
  const snapshot = registered();
  claudeOf(snapshot).configuration_state = 'executable_not_runnable';
  return snapshot;
}
function unregisteredEndpoint(): AgentSnapshot {
  const snapshot = registered();
  const agent = claudeOf(snapshot);
  agent.configuration_state = 'unregistered_endpoint';
  agent.version = '';
  agent.settings = null;
  return snapshot;
}
function freshCodex(surface: 'codex_desktop' | 'codex_cli'): AgentSnapshot {
  const snapshot = registered();
  const agent = codexOf(snapshot);
  agent.available_surfaces = [surface];
  agent.configuration_state = 'not_configured';
  agent.settings = {
    ...agent.settings!,
    state: 'not_configured',
    model_verified: false,
    restore_point_ref: null,
    applied_revision: null,
    surface_results: [],
    live_check_targets: [],
    current_selection: null,
  };
  return snapshot;
}
function desktopWithFixed(): AgentSnapshot {
  const snapshot = freshCodex('codex_desktop');
  const settings = codexOf(snapshot).settings!;
  settings.state = 'configured';
  settings.current_selection = {
    mode: 'codex_default',
    native_model_mode: 'preserve_available',
    fixed_models: [{ client_model_id: 'gpt-5.6-sol', candidate: { binding_id: 'binding/codex/previous', reasoning: { kind: 'profile', profile: 'high' } } }],
    allowed_plan_ids: [],
    default_selection: { kind: 'preserve_native' },
  };
  return snapshot;
}
function protectedCodex(): AgentSnapshot {
  const snapshot = desktopWithFixed();
  codexOf(snapshot).settings!.protected_native_model_ids = ['gpt-5.6-sol'];
  return snapshot;
}
function splitCodexStatus(): AgentSnapshot {
  const snapshot = registered();
  const agent = codexOf(snapshot);
  agent.settings = {
    ...agent.settings!,
    model_verified: false,
    surface_results: [
      { surface: 'codex_cli', applied_revision: 19, state: 'passed', reason_code: null },
      { surface: 'codex_desktop', applied_revision: 19, state: 'not_verified', reason_code: null },
    ],
    current_selection: {
      ...agent.settings!.current_selection!,
      mode: 'codex_default',
    },
  };
  return snapshot;
}

type Handler = (payload: Record<string, unknown>) => unknown;
const control = {
  agents: registered(),
  commands: [] as { command: string; payload: Record<string, unknown> }[],
  handlers: {} as Record<string, Handler>,
  mutations: 0,
  operations: [] as { operation: { operation_id: string; state: string }; presentation: { kind: string; target: string } }[],
  refresh: () => {},
  reset: () => {},
  registered,
  notRunnable,
  notRunnableWithRestore,
  unregisteredEndpoint,
  desktopOnly: () => freshCodex('codex_desktop'),
  desktopWithFixed,
  protectedCodex,
  cliOnly: () => freshCodex('codex_cli'),
  splitCodexStatus,
};
Object.assign(window, { agentTrust: control });

mockIPC(async (command, payload) => {
  const args = (payload ?? {}) as Record<string, unknown>;
  control.commands.push({ command, payload: args });
  if (control.handlers[command]) return control.handlers[command](args);
  switch (command) {
    case 'agent_snapshot': return structuredClone(control.agents);
    case 'preview_agent_settings': {
      return { preview: { applicable: true, blockers: [] }, mutation: { state: 'applied', operation: { operation_id: 'operation/agent-settings-fixture', state: 'succeeded', sequence: 1, cancellable: false } } };
    }
    case 'check_agent_authentication': return true;
    case 'check_agent_live': return { accepted: true, scope: 'live', model_call: true, state: 'passed', call_count: 1, requested_call_count: 1 };
    default: throw new Error(`Unexpected agent-trust command: ${command}`);
  }
});

function Harness() {
  const [generation, setGeneration] = useState(0);
  const [refresh, setRefresh] = useState(0);
  control.refresh = () => setRefresh(value => value + 1);
  control.reset = () => {
    control.agents = registered();
    control.commands = [];
    control.handlers = {};
    control.mutations = 0;
    control.operations = [];
    setRefresh(0);
    setGeneration(value => value + 1);
  };
  return <PresentationRoot language="zh" theme="dark" textScale={1}>
    <div className="app-window"><main className="main" style={{ marginLeft: 0 }}>
      <div role="note">组件测试：所有 IPC 为 mock；不连接 Tauri、daemon 或真实安装。</div>
      <Agents key={generation} language="zh" refreshVersion={refresh} mutationAllowed onMutation={() => { control.mutations += 1; }} onOperation={(operation, presentation) => { control.operations.push({ operation, presentation }); }} />
    </main></div>
  </PresentationRoot>;
}

createRoot(document.getElementById('root')!).render(<Harness />);
