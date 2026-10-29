import type { BrandKind } from '../../ui/BrandIcon';
import type { Agent, AgentModelSelection, AgentModelSurface } from './types';

/** A supported integration stays discoverable before a native profile is selected.
 * This entry is presentation only: no context or settings authority is invented.
 */
export function agentPageEntries(agents: Agent[]): Agent[] {
  const supported = ['agent_claude_default', 'agent_codex_default', 'agent_dsh_default', 'agent_dsh_desktop', 'agent_pi_default', 'agent_qoder_default', 'agent_hermes_default', 'agent_opencode_default'];
  const missing: Agent[] = supported.filter(id => !agents.some(agent => agent.agent_id === id)).map(agent_id => ({
    agent_id, version: '', context_id: null,
    configuration_state: agent_id === 'agent_hermes_default' ? 'profile_not_selected' : 'not_discovered',
    settings: null, status_error: null,
  }));
  return missing.length ? [...agents, ...missing] : agents;
}

export type AgentEcosystem = 'codex' | 'claude' | 'qoder' | 'pi' | 'dsh' | 'hermes' | 'opencode';

/** Only these current integrations have settings editors; discovery is independent. */
export function agentEcosystem(agentId: string): AgentEcosystem | null {
  switch (agentId) {
    case 'agent_codex_default': return 'codex';
    case 'agent_claude_default': return 'claude';
    case 'agent_opencode_default': return 'opencode';
    case 'agent_dsh_default':
    case 'agent_dsh_desktop': return 'dsh';
    case 'agent_pi_default': return 'pi';
    case 'agent_qoder_default': return 'qoder';
    case 'agent_hermes_default': return 'hermes';
    default: return null;
  }
}

export function agentSupportsModelRouting(agentId: string): boolean {
  const ecosystem = agentEcosystem(agentId);
  return ecosystem === 'codex' || ecosystem === 'claude' || ecosystem === 'qoder' || ecosystem === 'pi' || ecosystem === 'dsh' || ecosystem === 'hermes' || ecosystem === 'opencode';
}

/** A saved selection must belong to the actual integration, including token-only edits. */
export function agentModelSelectionMatches(agentId: string, selection: AgentModelSelection): boolean {
  switch (agentEcosystem(agentId)) {
    case 'codex': return selection.mode === 'codex_default';
    case 'claude': return selection.mode === 'claude_launcher';
    case 'opencode': return selection.mode === 'opencode_additional';
    case 'dsh': return selection.mode === 'dsh_additional';
    case 'pi': return selection.mode === 'pi_additional';
    case 'hermes': return selection.mode === 'hermes_default';
    case 'qoder': return selection.mode === 'qoder_additional';
    default: return false;
  }
}

export function agentDisplayName(agentId: string, language: 'zh' | 'en'): string {
  switch (agentEcosystem(agentId)) {
    case 'codex': return 'Codex';
    case 'claude': return 'Claude Code';
    case 'opencode': return 'OpenCode';
    case 'dsh': return agentId === 'agent_dsh_desktop' ? 'DeepSeek Harness Desktop' : 'DeepSeek Harness Web';
    case 'pi': return 'Pi';
    case 'qoder': return 'Qoder';
    case 'hermes': return 'Hermes';
    default: return agentId.toLowerCase().includes('cursor') ? 'Cursor' : language === 'zh' ? '本机 Agent' : 'Local Agent';
  }
}

export function agentBrand(agentId: string): BrandKind {
  switch (agentEcosystem(agentId)) {
    case 'codex': return 'codex';
    case 'claude': return 'claude-code';
    case 'dsh': return 'dsh';
    case 'pi': return 'pi';
    case 'qoder': return 'qoder';
    case 'opencode': return 'opencode';
    case 'hermes': return 'hermes';
    default: return 'agent';
  }
}

export function surfaceName(surface: AgentModelSurface): string {
  switch (surface) {
    case 'codex_desktop': return 'Codex Desktop';
    case 'codex_cli': return 'Codex CLI';
    case 'claude_cli': return 'Claude Code CLI';
    case 'dsh_cli': return 'DeepSeek Harness Web';
    case 'dsh_desktop': return 'DeepSeek Harness Desktop';
    case 'opencode_cli': return 'OpenCode';
    case 'pi_cli': return 'Pi CLI';
    case 'qoder_cli': return 'Qoder CLI';
    case 'hermes_cli': return 'Hermes';
    default: return surface;
  }
}

export function agentSupportsCollaboration(agentId: string): boolean {
  const ecosystem = agentEcosystem(agentId);
  return ecosystem !== null && agentId !== 'agent_dsh_desktop';
}

export function agentSupportsManagement(agentId: string): boolean {
  return agentEcosystem(agentId) !== null && agentId !== 'agent_dsh_desktop';
}
