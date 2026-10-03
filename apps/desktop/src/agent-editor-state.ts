import type { Agent, AgentFixedModel } from './agents';

export type AgentCollaborationTriggerMode = 'explicit' | 'delegate_by_default';
export type CodexNativeModelMode = 'hiroute_only' | 'preserve_available';
export type AgentDefaultChoice =
  | { kind: 'preserve_native' }
  | { kind: 'fixed_model'; client_model_id: string }
  | { kind: 'plan'; plan_id: string };
export type AgentClaudePresetChoice =
  | { kind: 'preserve_native' }
  | { kind: 'plan'; plan_id: string };
export type AgentClaudePresetMappings = {
  opus: AgentClaudePresetChoice;
  sonnet: AgentClaudePresetChoice;
  haiku: AgentClaudePresetChoice;
};
export type AgentEditorValues = {
  fixedModels: AgentFixedModel[];
  nativeModelMode: CodexNativeModelMode;
  allowedPlanIds: string[];
  defaultChoice: AgentDefaultChoice;
  claudePresets: AgentClaudePresetMappings;
  triggerMode: AgentCollaborationTriggerMode;
};

const preserve = (): AgentClaudePresetChoice => ({ kind: 'preserve_native' });

export function editorFingerprint(value: AgentEditorValues): string {
  return JSON.stringify({
    ...value,
    allowedPlanIds: [...value.allowedPlanIds].sort(),
  });
}

export function codexDefaultChoiceValid(
  choice: AgentDefaultChoice,
  mode: CodexNativeModelMode,
  nativeDefault: string | undefined,
  fixedModels: AgentFixedModel[],
  allowedPlanIds: string[],
): boolean {
  if (choice.kind === 'preserve_native') {
    if (mode === 'hiroute_only') return false;
    // The backend checks whether this name resolves to an authorized fixed model or selected
    // plan alias. The WebView only checks that a current name exists before Preview.
    return Boolean(nativeDefault);
  }
  return choice.kind === 'plan'
    ? allowedPlanIds.includes(choice.plan_id)
    : fixedModels.some(model => model.client_model_id === choice.client_model_id);
}

export function agentEditorSeed(
  agent: Agent,
  facet: 'model' | 'collaboration',
  availablePlanIds: string[] = [],
): AgentEditorValues & { known: boolean } {
  const status = facet === 'model' ? agent.settings : agent.settings?.collaboration;
  const selection = status?.current_selection;
  const initial = status?.state === 'not_configured' || status?.state === 'restored';
  const known = !agent.status_error && (initial || (status?.state === 'configured' && !!selection));
  const currentModel = facet === 'model' ? agent.settings?.current_selection : undefined;
  const codex = currentModel?.mode === 'codex_default' ? currentModel : undefined;
  const claude = currentModel?.mode === 'claude_launcher' ? currentModel : undefined;
  const collaboration = facet === 'collaboration'
    && selection
    && 'trigger_mode' in selection
    ? selection
    : undefined;
  const initialPlan = facet === 'model' && initial && known && !currentModel && availablePlanIds.length === 1
    ? availablePlanIds[0] : undefined;
  return {
    known: facet === 'model'
      ? known && (initial || (agent.agent_id === 'agent_codex_default' ? !!codex : !!claude))
      : known,
    fixedModels: currentModel?.fixed_models ?? [],
    nativeModelMode: codex?.native_model_mode ?? 'hiroute_only',
    allowedPlanIds: codex?.allowed_plan_ids ?? (initialPlan && agent.agent_id === 'agent_codex_default' ? [initialPlan] : []),
    defaultChoice: codex?.default_selection ?? (initialPlan && agent.agent_id === 'agent_codex_default'
      ? { kind: 'plan', plan_id: initialPlan } : { kind: 'preserve_native' }),
    claudePresets: claude?.preset_mappings ?? {
      opus: initialPlan && agent.agent_id === 'agent_claude_default' ? { kind: 'plan', plan_id: initialPlan } : preserve(),
      sonnet: initialPlan && agent.agent_id === 'agent_claude_default' ? { kind: 'plan', plan_id: initialPlan } : preserve(),
      haiku: initialPlan && agent.agent_id === 'agent_claude_default' ? { kind: 'plan', plan_id: initialPlan } : preserve(),
    },
    triggerMode: collaboration?.trigger_mode ?? 'explicit',
  };
}

export function commonClaudePlan(mappings: AgentClaudePresetMappings): string | null {
  const ids = Object.values(mappings).map(choice => choice.kind === 'plan' ? choice.plan_id : '');
  return ids.every(id => id === ids[0]) ? ids[0] : null;
}

export function sharedClaudePlan(planId: string): AgentClaudePresetMappings {
  return {
    opus: { kind: 'plan', plan_id: planId },
    sonnet: { kind: 'plan', plan_id: planId },
    haiku: { kind: 'plan', plan_id: planId },
  };
}
