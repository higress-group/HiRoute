import type { Agent, AgentModelSelection, ModelStatus } from './types';

/** Every model selection projects its own explicit Plan references through this boundary. */
export function agentModelPlanIds(selection: AgentModelSelection | null | undefined): string[] {
  if (!selection) return [];
  if (selection.mode === 'hermes_default') return [selection.plan_id];
  if ('allowed_plan_ids' in selection) {
    const defaultId = selection.mode === 'codex_default' && selection.default_selection.kind === 'plan'
      ? [selection.default_selection.plan_id] : [];
    return [...new Set([...selection.allowed_plan_ids, ...defaultId])];
  }
  return [...new Set(Object.values(selection.preset_mappings).flatMap(choice => choice.kind === 'plan' ? [choice.plan_id] : []))];
}

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
