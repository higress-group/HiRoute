import { agentEcosystem } from '../agents/ecosystems.ts';
import type { Agent, AgentSnapshot, ModelStatus } from '../agents/types';

/** Discovery is only a choice hint. It never means empty or editable settings. */
export type TrialAgentChoice = Omit<Agent, 'settings' | 'status_error'>;
export type TrialAgentChoices = { choices: TrialAgentChoice[]; trusted_authority: boolean };
export type TrialAgentModel = { settings: ModelStatus & { context_id: string }; plans: AgentSnapshot['plans']; trusted_authority: boolean };
export type TrialAgentSnapshot = TrialAgentChoices & { selected: Agent | null; plans: AgentSnapshot['plans'] };

export function preferredTrialAgent(choices: TrialAgentChoice[], requested: string) {
  const usable = trialModelChoices(choices);
  return requested ? usable.find(choice => choice.agent_id === requested)
    : usable.find(choice => choice.agent_id === 'agent_claude_default') ?? usable[0];
}

export function selectedTrialSnapshot(choices: TrialAgentChoices, choice: TrialAgentChoice, model: TrialAgentModel): TrialAgentSnapshot {
  if (choice.context_id !== model.settings.context_id) throw Error('AGENT_STATUS_UNAVAILABLE');
  return { ...choices, plans: model.plans, selected: { ...choice, settings: model.settings, status_error: null },
    trusted_authority: choices.trusted_authority && model.trusted_authority };
}

export function trialModelChoices(choices: TrialAgentChoice[]): (TrialAgentChoice & { context_id: string })[] {
  return choices.filter((choice): choice is TrialAgentChoice & { context_id: string } => !!choice.context_id && ['claude','codex','dsh','pi','qoder','opencode'].includes(agentEcosystem(choice.agent_id) ?? ''));
}
