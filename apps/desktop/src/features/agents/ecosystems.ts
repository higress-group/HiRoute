import type { AgentModelSelection, AgentModelSurface } from './types';

export type AgentEcosystem = 'codex' | 'claude' | 'qoder' | 'pi' | 'dsh';

/** Only these current integrations have settings editors; discovery is independent. */
export function agentEcosystem(agentId: string): AgentEcosystem | null {
  switch (agentId) {
    case 'agent_codex_default': return 'codex';
    case 'agent_claude_default': return 'claude';
    case 'agent_dsh_default': return 'dsh';
    case 'agent_pi_default': return 'pi';
    case 'agent_qoder_default': return 'qoder';
    default: return null;
  }
}

export function agentSupportsModelRouting(agentId: string): boolean {
  const ecosystem = agentEcosystem(agentId);
  return ecosystem === 'codex' || ecosystem === 'claude' || ecosystem === 'qoder' || ecosystem === 'pi' || ecosystem === 'dsh';
}

/** A saved selection must belong to the actual integration, including token-only edits. */
export function agentModelSelectionMatches(agentId: string, selection: AgentModelSelection): boolean {
  switch (agentEcosystem(agentId)) {
    case 'codex': return selection.mode === 'codex_default';
    case 'claude': return selection.mode === 'claude_launcher';
    case 'dsh': return selection.mode === 'dsh_additional';
    case 'pi': return selection.mode === 'pi_additional';
    case 'qoder': return selection.mode === 'qoder_additional';
    default: return false;
  }
}

export function agentDisplayName(agentId: string, language: 'zh' | 'en'): string {
  switch (agentEcosystem(agentId)) {
    case 'codex': return 'Codex';
    case 'claude': return 'Claude Code';
    case 'dsh': return 'DeepSeek Harness';
    case 'pi': return 'Pi';
    case 'qoder': return 'Qoder';
    default: return agentId.toLowerCase().includes('cursor') ? 'Cursor' : language === 'zh' ? '本机 Agent' : 'Local Agent';
  }
}

export function agentBrand(agentId: string): 'codex' | 'claude-code' | 'qoder' | 'pi' | 'dsh' | 'agent' {
  switch (agentEcosystem(agentId)) {
    case 'codex': return 'codex';
    case 'claude': return 'claude-code';
    case 'dsh': return 'dsh';
    case 'pi': return 'pi';
    case 'qoder': return 'qoder';
    default: return 'agent';
  }
}

export function surfaceName(surface: AgentModelSurface): string {
  switch (surface) {
    case 'codex_desktop': return 'Codex Desktop';
    case 'codex_cli': return 'Codex CLI';
    case 'claude_cli': return 'Claude Code CLI';
    case 'dsh_cli': return 'DeepSeek Harness';
    case 'pi_cli': return 'Pi CLI';
    case 'qoder_cli': return 'Qoder CLI';
    default: return surface;
  }
}
