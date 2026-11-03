import type { Agent } from './types';

/** Desktop is another view of the user Skill, not another installation reference. */
export function managementContext(agent: Agent, entries: Agent[]): string {
  if (agent.agent_id === 'agent_codex_desktop') {
    const owner = entries.find(value => value.agent_id === 'agent_codex_default');
    if (!owner?.codex_access?.root_context_id || !agent.codex_desktop
      || owner.codex_access.codex_home !== agent.codex_desktop.source_codex_home) throw new Error('AGENT_STATUS_UNVERIFIED');
    return owner.codex_access.root_context_id;
  }
  if (agent.agent_id === 'agent_dsh_desktop') {
    const context = entries.find(value => value.agent_id === 'agent_dsh_default')?.context_id;
    if (!context) throw new Error('AGENT_STATUS_UNVERIFIED');
    return context;
  }
  const context = agent.codex_access?.root_context_id ?? agent.context_id;
  if (!context) throw new Error('AGENT_INPUT_INVALID');
  return context;
}

export function managementAgentId(agentId: string): string {
  return agentId === 'agent_codex_desktop' ? 'agent_codex_default'
    : agentId === 'agent_dsh_desktop' ? 'agent_dsh_default' : agentId;
}

export function sharedManagement(agentId: string): boolean {
  return ['agent_codex_default', 'agent_codex_desktop', 'agent_dsh_default', 'agent_dsh_desktop'].includes(agentId);
}
