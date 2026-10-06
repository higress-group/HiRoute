import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from '@tauri-apps/api/mocks';
import { Agents, type Agent, type AgentSnapshot, type ModelStatus } from '../../../src/agents';
import type { OperationReference } from '../../../src/features/model-connections/types';
import { PresentationRoot } from '../../../src/ui';
import { readyAgents } from './product-fixtures';
import '../../../src/occami/styles.css';

function modelAgent(snapshot: AgentSnapshot, id: string): Agent & { settings: ModelStatus } {
  const agent = snapshot.agents.find(agent => agent.agent_id === id);
  if (!agent?.settings || !('state' in agent.settings)) throw new Error('Model scenario requires model settings');
  return agent as Agent & { settings: ModelStatus };
}
const claudeOf = (snapshot: AgentSnapshot) => modelAgent(snapshot, 'agent_claude_default');
const codexOf = (snapshot: AgentSnapshot) => modelAgent(snapshot, 'agent_codex_default');

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
  const agent = snapshot.agents.find(agent => agent.agent_id === 'agent_claude_default')!;
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
  const selection = agent.settings?.current_selection;
  if (selection?.mode !== 'codex_default') throw new Error('Codex status fixture requires a Codex selection');
  agent.codex_access = {
    codex_home: '/fixture/codex', slot_id: 'slot/fixture',
    profile_context_id: 'context/fixture/profile', root_context_id: agent.context_id!,
    selected_mode: 'root', slot_occupied: true, target_file: '/fixture/codex/config.toml',
    profile_name: 'hiroute', commands: {}, pending_operation: null, access_revoked: false, conflict_fields: [],
  };
  agent.settings = {
    ...agent.settings!,
    model_verified: false,
    surface_results: [
      { surface: 'codex_cli', applied_revision: 19, state: 'passed', reason_code: null },
      { surface: 'codex_desktop', applied_revision: 19, state: 'not_verified', reason_code: null },
    ],
    current_selection: selection,
  };
  return snapshot;
}

function additionalAgent(configured: boolean, ecosystem: 'qoder' | 'pi' | 'dsh' = 'qoder'): AgentSnapshot {
  const context = `agent-context/${ecosystem}/sha256:${'a'.repeat(64)}`;
  return {
    trusted_authority: true, plans: { plans: [] },
    agents: [{
      agent_id: `agent_${ecosystem}_default`, version: 'fixture', context_id: context,
      configuration_state: configured ? 'configured' : 'not_configured',
      available_surfaces: [`${ecosystem}_cli`], native_model_catalog: null, status_error: null,
      settings: {
        state: 'not_configured', model_verified: false, restore_point_ref: null, current_selection: null,
        collaboration: {
          state: configured ? 'configured' : 'not_configured',
          restore_point_ref: configured ? `task-restore/${ecosystem}` : null,
          current_selection: configured ? { trigger_mode: 'explicit' } : null,
        },
      },
    }],
  };
}

function additionalRouted(ecosystem: 'qoder' | 'pi' | 'dsh' = 'qoder'): AgentSnapshot {
  const snapshot = additionalAgent(true, ecosystem);
  snapshot.plans = structuredClone(readyAgents.plans);
  const agent = modelAgent(snapshot, `agent_${ecosystem}_default`);
  const plans = snapshot.plans.plans.slice(0, 2);
  agent.settings = {
    ...agent.settings, state: 'configured', applied_revision: 23, restore_point_ref: `model-restore/${ecosystem}`,
    current_selection: { mode: `${ecosystem}_additional`, allowed_plan_ids: plans.map(plan => plan.agent_plan_id) },
    live_check_targets: [{ context_id: agent.context_id!, surface: `${ecosystem}_cli`, expected_applied_revision: 23,
      client_model_ids: plans.map(plan => `fixture-hiroute/${plan.model_alias}`) }],
    surface_results: [{ surface: `${ecosystem}_cli`, applied_revision: 23, state: 'not_verified', reason_code: null }],
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
  qoderFresh: () => additionalAgent(false),
  qoderConfigured: () => additionalAgent(true),
  qoderRouted: () => additionalRouted(),
  additionalRouted,
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
  const [operation, setOperation] = useState<OperationReference | null>(null);
  control.refresh = () => setRefresh(value => value + 1);
  control.reset = () => {
    control.agents = registered();
    control.commands = [];
    control.handlers = {};
    control.mutations = 0;
    control.operations = [];
    setOperation(null);
    setRefresh(0);
    setGeneration(value => value + 1);
  };
  return <PresentationRoot language="zh" theme="dark" textScale={1}>
    <div className="app-window">
      <div role="note" style={{ gridColumn: '1 / -1', gridRow: 1 }}>组件测试：所有 IPC 为 mock；不连接 Tauri、daemon 或真实安装。</div>
      <main className="main" style={{ gridColumn: '1 / -1', gridRow: 2 }}>
      <Agents key={generation} language="zh" refreshVersion={refresh} operation={operation} mutationAllowed onMutation={() => { control.mutations += 1; }} onOperation={(operation, presentation) => { control.operations.push({ operation, presentation }); setOperation(operation); }} />
    </main></div>
  </PresentationRoot>;
}

createRoot(document.getElementById('root')!).render(<Harness />);
