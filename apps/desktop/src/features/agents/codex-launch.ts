import { agentModelStatus } from './status.ts';
import type { Agent } from './types';
import type { OperationReference } from '../model-connections/types';

export type PendingCodexLaunchCopy = { operationId: string; contextId: string };

export function preferredCodexShell(platform: string, commands: Record<string, string>): string {
  const preferred = /win/i.test(platform) ? 'powershell' : 'bash/zsh';
  return commands[preferred] ? preferred : Object.keys(commands).find(shell => Boolean(commands[shell])) ?? preferred;
}

/** A successful submission is insufficient: copy only a fresh, confirmed target. */
export function confirmedCodexLaunchCommand(
  target: PendingCodexLaunchCopy,
  operation: OperationReference | null | undefined,
  agent: Agent | undefined,
  platform: string,
): string | null {
  const access = agent?.codex_access;
  if (operation?.operation_id !== target.operationId || operation.state !== 'succeeded'
    || agent?.status_error || agentModelStatus(agent)?.state !== 'configured'
    || !access?.slot_occupied || access.selected_mode !== 'profile'
    || access.profile_context_id !== target.contextId || access.pending_operation || access.access_revoked) return null;
  return access.commands[preferredCodexShell(platform, access.commands)] || null;
}
