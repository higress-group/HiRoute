import type {
  Agent, AgentFixedModel, AgentDefaultChoice, AgentClaudePresetChoice,
  AgentClaudePresetMappings, AgentCollaborationTriggerMode, CodexNativeModelMode,
} from './types';
import { agentEcosystem, agentModelSelectionMatches, agentSupportsModelRouting } from './ecosystems.ts';
import { agentModelStatus } from './status.ts';
export type {
  AgentDefaultChoice, AgentClaudePresetChoice, AgentClaudePresetMappings,
  AgentCollaborationTriggerMode, CodexNativeModelMode,
} from './types';

export type AgentEditorValues = {
  fixedModels: AgentFixedModel[];
  nativeModelMode: CodexNativeModelMode;
  allowedPlanIds: string[];
  routingMetadata?: boolean;
  planProtocols: Record<string, 'responses' | 'messages'>;
  defaultChoice: AgentDefaultChoice;
  claudePresets: AgentClaudePresetMappings;
  triggerMode: AgentCollaborationTriggerMode;
  hermesPlanId: string;
  hermesProtocol: 'chat_completions' | 'responses' | 'messages';
};

/** Apply a recovered selection to the ordinary editor without touching collaboration. */
export function withModelSelection(values: AgentEditorValues, selection: import('./types').AgentModelSelection): AgentEditorValues {
  if (selection.mode === 'claude_launcher') return { ...values, fixedModels: selection.fixed_models, claudePresets: selection.preset_mappings };
  if (selection.mode === 'hermes_default') return { ...values, hermesPlanId: selection.plan_id, hermesProtocol: selection.protocol };
  if (selection.mode === 'codex_default') return { ...values, fixedModels: selection.fixed_models, nativeModelMode: selection.native_model_mode, allowedPlanIds: selection.allowed_plan_ids, defaultChoice: selection.default_selection };
  return { ...values, allowedPlanIds: selection.allowed_plan_ids, planProtocols: selection.plan_protocols ?? {}, routingMetadata: selection.mode === 'opencode_additional' ? selection.routing_metadata : undefined };
}

export const EMPTY_EDITOR_VALUES: AgentEditorValues = {
  fixedModels: [],
  nativeModelMode: 'hiroute_only',
  allowedPlanIds: [],
  planProtocols: {},
  defaultChoice: { kind: 'preserve_native' },
  claudePresets: {
    opus: { kind: 'preserve_native' },
    sonnet: { kind: 'preserve_native' },
    haiku: { kind: 'preserve_native' },
  },
  triggerMode: 'explicit',
  hermesPlanId: '',
  hermesProtocol: 'responses',
};
const preserve = (): AgentClaudePresetChoice => ({ kind: 'preserve_native' });

export function editorFingerprint(value: AgentEditorValues): string {
  // Compare settings rather than JSON map insertion order or seed-only metadata.
  // Preserve array order except for the set of allowed Plan identities.
  return JSON.stringify({
    fixedModels: value.fixedModels,
    nativeModelMode: value.nativeModelMode,
    allowedPlanIds: [...value.allowedPlanIds].sort(),
    planProtocols: value.planProtocols,
    routingMetadata: value.routingMetadata !== false,
    defaultChoice: value.defaultChoice,
    claudePresets: value.claudePresets,
    triggerMode: value.triggerMode,
    hermesPlanId: value.hermesPlanId,
    hermesProtocol: value.hermesProtocol,
  }, (_key, item: unknown) => item && typeof item === 'object' && !Array.isArray(item)
    ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b))) : item);
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
  facet: 'model' | 'collaboration' | 'management_skill',
  availablePlanIds: string[] = [],
): AgentEditorValues & { known: boolean } {
  const ecosystem = agentEcosystem(agent.agent_id);
  if (facet === 'management_skill') {
    const management = agent.settings?.management_skill;
    return { ...EMPTY_EDITOR_VALUES, known: ecosystem !== null && (ecosystem !== 'hermes' || !!agent.native_target)
      && !agent.status_error && !!management && ['not_configured', 'restored', 'configured'].includes(management.state) };
  }
  const status = facet === 'model' ? agentModelStatus(agent) : agent.settings?.collaboration;
  const selection = status?.current_selection;
  const initial = status?.state === 'not_configured' || status?.state === 'restored';
  const known = ecosystem !== null && !agent.status_error && (initial || (status?.state === 'configured' && !!selection));
  const currentModel = facet === 'model' ? agentModelStatus(agent)?.current_selection : undefined;
  const codex = currentModel?.mode === 'codex_default' ? currentModel : undefined;
  const claude = currentModel?.mode === 'claude_launcher' ? currentModel : undefined;
  const qoder = (currentModel?.mode === 'qoder_additional' || currentModel?.mode === 'pi_additional' || currentModel?.mode === 'dsh_additional' || currentModel?.mode === 'opencode_additional') ? currentModel : undefined;
  const collaboration = facet === 'collaboration'
    && selection
    && 'trigger_mode' in selection
    ? selection
    : undefined;
  const initialPlan = facet === 'model' && initial && known && !currentModel && availablePlanIds.length === 1
    ? availablePlanIds[0] : undefined;
  return {
    known: facet === 'model'
      ? known && agentSupportsModelRouting(agent.agent_id) && (initial || Boolean(currentModel && agentModelSelectionMatches(agent.agent_id, currentModel)))
      : known,
    hermesPlanId: currentModel?.mode === 'hermes_default' ? currentModel.plan_id : initialPlan && ecosystem === 'hermes' ? initialPlan : '',
    hermesProtocol: currentModel?.mode === 'hermes_default' ? currentModel.protocol : 'responses',
    planProtocols: qoder?.plan_protocols ?? {},
    routingMetadata: currentModel?.mode === 'opencode_additional' ? currentModel.routing_metadata : undefined,
    fixedModels: currentModel && 'fixed_models' in currentModel ? currentModel.fixed_models : [],
    nativeModelMode: codex?.native_model_mode ?? 'hiroute_only',
    allowedPlanIds: codex?.allowed_plan_ids ?? qoder?.allowed_plan_ids
      ?? (initialPlan && (ecosystem === 'codex' || ecosystem === 'qoder' || ecosystem === 'pi' || ecosystem === 'dsh' || ecosystem === 'opencode') ? [initialPlan] : []),
    defaultChoice: codex?.default_selection ?? (initialPlan && ecosystem === 'codex'
      ? { kind: 'plan', plan_id: initialPlan } : { kind: 'preserve_native' }),
    claudePresets: claude?.preset_mappings ?? {
      opus: initialPlan && ecosystem === 'claude' ? { kind: 'plan', plan_id: initialPlan } : preserve(),
      sonnet: initialPlan && ecosystem === 'claude' ? { kind: 'plan', plan_id: initialPlan } : preserve(),
      haiku: initialPlan && ecosystem === 'claude' ? { kind: 'plan', plan_id: initialPlan } : preserve(),
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

/** Local form completeness only; Preview remains the authority for capabilities and grants. */
export function agentModelFormInvalid(agent: Agent | undefined, editorValues: AgentEditorValues, enabledPlanIds: string[]): boolean {
  if (!agent || agentEcosystem(agent.agent_id) === null) return true;
  if (agentEcosystem(agent.agent_id) === 'hermes') {
    return !agent.native_target || !enabledPlanIds.includes(editorValues.hermesPlanId);
  }
  if (['qoder', 'pi', 'dsh', 'opencode'].includes(agentEcosystem(agent.agent_id) ?? '')) {
    return editorValues.allowedPlanIds.length === 0
      || editorValues.allowedPlanIds.some(id => !enabledPlanIds.includes(id));
  }
  const protectedNativeModelIds = new Set(agentModelStatus(agent)?.protected_native_model_ids ?? []);
  const catalogModels = (agent.native_model_catalog?.models ?? [])
    .map(model => ({ ...model, source_options: model.source_options ?? [] }));
  const selectedPlansEnabled = editorValues.allowedPlanIds.every(id => enabledPlanIds.includes(id));
  const defaultChoice = editorValues.defaultChoice;
  const selectedFixedSourcesValid = editorValues.fixedModels.every(fixed => {
    if (protectedNativeModelIds.has(fixed.client_model_id)) return true;
    const source = catalogModels
      .find(model => model.client_model_id === fixed.client_model_id)
      ?.source_options.find(option => option.binding_id === fixed.candidate.binding_id);
    if (!source || source.state !== 'ready') return false;
    const selection = fixed.candidate.reasoning;
    if (source.reasoning.kind === 'fixed') return selection === undefined;
    if (source.reasoning.kind === 'toggle') return selection?.kind === 'toggle';
    if (source.reasoning.kind === 'discrete') {
      return selection?.kind === 'profile' && source.reasoning.profiles.includes(selection.profile);
    }
    return selection?.kind === 'budget'
      && selection.tokens >= source.reasoning.minimum_tokens
      && selection.tokens <= source.reasoning.maximum_tokens
      && (selection.tokens - source.reasoning.minimum_tokens) % source.reasoning.step_tokens === 0;
  });
  const nativeDefault = agent.native_model_catalog?.native_default_model;
  const codexDefaultValid = codexDefaultChoiceValid(
    defaultChoice,
    editorValues.nativeModelMode,
    nativeDefault,
    editorValues.fixedModels,
    editorValues.allowedPlanIds,
  );
  const claudePlanIds = Object.values(editorValues.claudePresets)
    .flatMap(choice => choice.kind === 'plan' ? [choice.plan_id] : []);
  switch (agentEcosystem(agent.agent_id)) {
    case 'codex':
      return editorValues.allowedPlanIds.length + editorValues.fixedModels.length === 0
        || !selectedFixedSourcesValid || !selectedPlansEnabled || !codexDefaultValid;
    case 'claude':
      return claudePlanIds.length + editorValues.fixedModels.length === 0
        || claudePlanIds.some(id => !enabledPlanIds.includes(id));
    default:
      return true;
  }
}
