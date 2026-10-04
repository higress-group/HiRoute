import type { Agent, ModelStatus } from './types';

/** Collaboration-only status has no model facet; absence is not an unconfigured model. */
export function agentModelStatus(agent: Agent | null | undefined): ModelStatus | null {
  const settings = agent?.settings;
  return settings && 'state' in settings ? settings : null;
}

export function agentHasNoModelConnection(agent: Agent | null | undefined): boolean {
  const settings = agent?.settings;
  return Boolean(settings && 'schema' in settings
    && settings.schema === 'hiroute.agent-collaboration-only-settings-status/v2');
}
