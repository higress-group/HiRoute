import type { AgentSnapshot } from '../../../src/features/agents/types';

function facet(value: any) {
  const state = value?.state;
  const recorded = state === 'configured' || state === 'drift' || state === 'restored';
  return {
    state: recorded ? 'recorded' : state === 'not_configured' ? 'absent' : state === 'needs_attention' ? 'needs_attention' : state === 'pending' ? 'pending' : 'not_ready',
    coverage: state ? 'complete' : 'not_ready',
    action: recorded ? state === 'restored' ? 'restore' : 'configure' : null,
    native_checked: false,
  };
}

export function agentHomeRead(snapshot: AgentSnapshot, command: string) {
  if (command === 'agent_home_summary') return {
    trusted_authority: snapshot.trusted_authority,
    agents: snapshot.agents.map(agent => ({
      agent_id: agent.agent_id,
      configuration_state: agent.configuration_state,
      context_id: agent.context_id,
      facets: agent.context_id ? {
        context_id: agent.context_id,
        model: facet(agent.settings && 'state' in agent.settings ? agent.settings : null),
        collaboration: facet(agent.settings?.collaboration),
        management_skill: facet(agent.settings?.management_skill),
      } : null,
    })),
  };
  if (command === 'agent_model_associations') return {
    complete: snapshot.agents.every(agent => Boolean(agent.context_id && !agent.status_error)),
    agents: snapshot.agents.filter(agent => agent.context_id && !agent.status_error).map(agent => ({
      agent_id: agent.agent_id,
      current_selection: agent.settings && 'state' in agent.settings ? agent.settings.current_selection ?? null : null,
    })),
  };
}
