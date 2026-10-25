import type { AgentSnapshot } from '../../../src/features/agents/types';

export function trialAgentRead(snapshot: AgentSnapshot, command: string, payload: any): any {
  if (command === 'trial_agent_choices') return {
    choices: snapshot.agents
      .map(({ settings: _settings, status_error: _error, ...choice }) => structuredClone(choice)),
    trusted_authority: snapshot.trusted_authority,
  };
  if (command === 'trial_agent_model') {
    const agent = snapshot.agents.find(agent => agent.context_id === payload.contextId);
    if (!agent?.settings || agent.status_error) throw Error('AGENT_STATUS_UNAVAILABLE');
    const settings = structuredClone(agent.settings);
    delete settings.management_skill;
    if ('collaboration' in settings) delete (settings as any).collaboration;
    return { settings: { ...settings, context_id: agent.context_id }, plans: structuredClone(snapshot.plans), trusted_authority: snapshot.trusted_authority };
  }
}
