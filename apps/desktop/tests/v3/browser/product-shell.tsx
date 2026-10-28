import React from 'react';
import { createRoot } from 'react-dom/client';
import { consoleFixture, mockIPC, mockWindows } from './host-mock';
import { invoke } from '../../../src/host/index.ts';
import { trialAgentRead } from './trial-agent-fixture';
import { agentHomeRead } from './agent-home-fixture';
import { DesktopApp } from '../../../src/product/DesktopApp';
import { fixturePlanAuthoring, mockProductInvoke, readyAgents, readyDesktop, readyManagement } from './product-fixtures';
import '../../../src/occami/styles.css';

// Only the IPC boundary is synthetic. Navigation, settings, forms and operation
// observation all run through DesktopApp; tests never call its private callbacks.
const root = createRoot(document.getElementById('root')!);
let generation = 0;
const control = {
  desktop: structuredClone(readyDesktop),
  agents: structuredClone(readyAgents),
  management: structuredClone(readyManagement),
  commands: [] as { command: string; payload: Record<string, unknown> }[],
  clipboard: [] as string[],
  downloads: [] as Blob[],
  callHost: invoke,
  handlers: {} as Record<string, (payload: Record<string, unknown>) => unknown>,
  fixtureResponse: (command: string, payload: Record<string, unknown>) => {
    const response = mockProductInvoke(command, payload, 'ready');
    const input = payload.input as { authoring?: { plan_id?: string } } | undefined;
    if (command === 'plan_editor_options' && input?.authoring) {
      return { ...response, authoring: fixturePlanAuthoring(control.desktop.catalog.plans.find(plan => plan.agent_plan_id === input.authoring?.plan_id)) };
    }
    return response;
  },
  remount: () => root.render(<DesktopApp key={++generation} />),
  reset: () => {
    control.desktop = structuredClone(readyDesktop);
    control.agents = structuredClone(readyAgents);
    control.management = structuredClone(readyManagement);
    control.commands = [];
    control.clipboard = [];
    control.downloads = [];
    control.handlers = {};
    localStorage.setItem('hiroute.language', 'zh');
    localStorage.setItem('hiroute.text-scale', '1');
    localStorage.removeItem('hiroute.subscription-proxy-return');
    control.remount();
  },
};
Object.assign(window, { productShell: control });
if (consoleFixture()) {
  const create = URL.createObjectURL.bind(URL);
  URL.createObjectURL = value => { if (value instanceof Blob) control.downloads.push(value); return create(value); };
}
Object.defineProperty(navigator, 'clipboard', { configurable: true, value: {
  writeText: async (value: string) => { control.clipboard.push(value); },
} });
mockIPC((command, payload) => {
  const args = (payload ?? {}) as Record<string, unknown>;
  control.commands.push({ command, payload: args });
  if (control.handlers[command]) return control.handlers[command](args);
  if (command === 'plugin:window|set_theme') return undefined;
  if (command === 'startup_status') return { state: 'ready', recovery_available: false };
  if (command === 'desktop_snapshot') return structuredClone(control.desktop);
  if (command === 'agent_snapshot') return structuredClone(control.agents);
  if (command === 'agent_home_summary' || command === 'agent_model_associations') return agentHomeRead(control.agents, command);
  if (command === 'trial_agent_choices' || command === 'trial_agent_model') return trialAgentRead(control.agents, command, args);
  if (command === 'compute_management_snapshot') return structuredClone(control.management);
  if (command === 'observe_operation' || command === 'web_confirmation_snapshot') return null;
  return control.fixtureResponse(command, args);
}, { shouldMockEvents: true });
mockWindows('main');
control.reset();
