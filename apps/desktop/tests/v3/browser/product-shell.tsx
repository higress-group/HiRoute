import React from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import { DesktopApp } from '../../../src/product/DesktopApp';
import { mockProductInvoke, readyAgents, readyDesktop, readyManagement } from './product-fixtures';
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
  handlers: {} as Record<string, (payload: Record<string, unknown>) => unknown>,
  fixtureResponse: (command: string, payload: Record<string, unknown>) => mockProductInvoke(command, payload, 'ready'),
  reset: () => {
    control.desktop = structuredClone(readyDesktop);
    control.agents = structuredClone(readyAgents);
    control.management = structuredClone(readyManagement);
    control.commands = [];
    control.clipboard = [];
    control.handlers = {};
    localStorage.setItem('hiroute.language', 'zh');
    localStorage.setItem('hiroute.text-scale', '1');
    root.render(<DesktopApp key={++generation} />);
  },
};
Object.assign(window, { productShell: control });
Object.defineProperty(navigator, 'clipboard', { configurable: true, value: {
  writeText: async (value: string) => { control.clipboard.push(value); },
} });
mockIPC((command, payload) => {
  const args = (payload ?? {}) as Record<string, unknown>;
  control.commands.push({ command, payload: args });
  if (control.handlers[command]) return control.handlers[command](args);
  if (command === 'startup_status') return { state: 'ready', recovery_available: false };
  if (command === 'desktop_snapshot') return structuredClone(control.desktop);
  if (command === 'agent_snapshot') return structuredClone(control.agents);
  if (command === 'compute_management_snapshot') return structuredClone(control.management);
  if (command === 'observe_operation' || command === 'web_confirmation_snapshot') return null;
  return mockProductInvoke(command, args, 'ready');
}, { shouldMockEvents: true });
mockWindows('main');
control.reset();
