import type { HostCapabilities } from '../../host/types.ts';
import type { Agent } from './types';
import { agentModelStatus } from './status.ts';

export function nativeCodexDesktopLaunch(host: HostCapabilities): boolean {
  return (host.host === 'tauri' && host.platform === 'macos')
    || (host.host === 'shell-console' && host.platform === 'windows' && host.codex_desktop_launch === true);
}

export function codexDesktopLaunchCommand(agent: Agent): string | null {
  const model = agentModelStatus(agent);
  const context = agent.context_id;
  if (agent.agent_id !== 'agent_codex_desktop' || !context || context.length > 256
    || !/^[A-Za-z0-9_:/.-]+$/.test(context) || !agent.codex_desktop
    || !agent.available_surfaces?.includes('codex_desktop') || agent.status_error
    || model?.state !== 'configured' || model.operation_state !== 'succeeded' || !model.current_selection) return null;
  // Context IDs use this same literal syntax in POSIX shells and PowerShell; no path/token.
  return `hiroute agents desktop launch '${context}' --output json`;
}
