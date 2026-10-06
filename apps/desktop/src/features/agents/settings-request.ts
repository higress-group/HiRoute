import type { Agent, AgentCheckScope, AgentFacet, AgentModelSelection, Preview } from './types';
import type { AgentEditorValues } from './editor-state';
import { agentEcosystem, agentModelSelectionMatches, agentSupportsModelRouting } from './ecosystems.ts';
import { agentHasNoModelConnection, agentModelStatus } from './status.ts';

type SettingsDraft = {
  values: AgentEditorValues;
  codexMode: 'profile' | 'root';
  restoreNativeModel: string;
};

/** Build only the public intent. Native Preview/confirmation owns authorization and Apply. */
export function agentSettingsSpec(agent: Agent, facet: AgentFacet, restore: boolean, draft: SettingsDraft) {
  const ecosystem = agentEcosystem(agent.agent_id);
  if (!agent.context_id || !ecosystem) throw new Error('AGENT_INPUT_INVALID');
  const { values, codexMode, restoreNativeModel } = draft;
  const keep = { intent: 'keep' as const };
  if (facet === 'model') {
    if (!agentSupportsModelRouting(agent.agent_id) || agentHasNoModelConnection(agent)) throw new Error('AGENT_INPUT_INVALID');
    const current = agentModelStatus(agent)?.current_selection;
    if (current && !agentModelSelectionMatches(agent.agent_id, current)) throw new Error('AGENT_INPUT_INVALID');
    let settings: AgentModelSelection;
    switch (ecosystem) {
      case 'codex':
        settings = {
          mode: 'codex_default',
          native_model_mode: values.nativeModelMode,
          fixed_models: values.fixedModels,
          allowed_plan_ids: values.allowedPlanIds,
          default_selection: values.defaultChoice,
        };
        break;
      case 'claude':
        settings = {
          mode: 'claude_launcher',
          surfaces: ['claude_cli'],
          fixed_models: values.fixedModels,
          preset_mappings: values.claudePresets,
        };
        break;
      case 'dsh':
        settings = { mode: 'dsh_additional', allowed_plan_ids: values.allowedPlanIds, plan_protocols: Object.fromEntries(values.allowedPlanIds.map(id => [id, values.planProtocols?.[id] ?? 'responses'])) };
        break;
      case 'pi':
        settings = { mode: 'pi_additional', allowed_plan_ids: values.allowedPlanIds, plan_protocols: Object.fromEntries(values.allowedPlanIds.map(id => [id, values.planProtocols?.[id] ?? 'responses'])) };
        break;
      case 'qoder':
        settings = { mode: 'qoder_additional', allowed_plan_ids: values.allowedPlanIds, plan_protocols: Object.fromEntries(values.allowedPlanIds.map(id => [id, values.planProtocols?.[id] ?? 'responses'])) };
        break;
      default: throw new Error('AGENT_INPUT_INVALID');
    }
    return {
      schema_version: { major: 2, minor: 0 },
      context_id: agent.codex_access && !agent.codex_access.slot_occupied
        ? (codexMode === 'root' ? agent.codex_access.root_context_id : agent.codex_access.profile_context_id)
        : agent.context_id,
      restore_native_model: restore && ecosystem === 'codex' && agent.codex_access?.selected_mode !== 'profile' && restoreNativeModel
        ? restoreNativeModel : undefined,
      model: restore
        ? { intent: 'restore', restore_point_ref: agentModelStatus(agent)?.restore_point_ref }
        : { intent: 'configure', settings },
      collaboration: keep,
    };
  }
  return {
    schema_version: { major: 2, minor: 0 },
    context_id: agent.codex_access?.root_context_id ?? agent.context_id,
    model: keep,
    collaboration: restore
      ? { intent: 'restore', restore_point_ref: agent.settings?.collaboration?.restore_point_ref }
      : { intent: 'configure', settings: { trigger_mode: values.triggerMode } },
  };
}

/** Token changes retain the current model selection and never resubmit an editor draft. */
export function agentTokenSpec(agent: Agent, regenerate: boolean) {
  const selection = agentModelStatus(agent)?.current_selection;
  if (!agent.context_id || !agentSupportsModelRouting(agent.agent_id) || !selection
    || !agentModelSelectionMatches(agent.agent_id, selection)) {
    throw new Error('AGENT_INPUT_INVALID');
  }
  return {
    schema_version: { major: 2, minor: 0 },
    context_id: agent.context_id,
    model: { intent: 'configure', settings: selection },
    collaboration: { intent: 'keep' },
    access_token: { intent: regenerate ? 'regenerate' : 'keep' },
  };
}

export function prerequisiteCheck(preview: Preview, facet: AgentFacet): AgentCheckScope | null {
  if (preview.applicable) return null;
  const missing = new Set(preview.blockers.flatMap(block => block.capabilities?.map(item => item.capability) ?? []));
  // Native ingress authentication can gate either facet: collaboration still
  // enters through the selected Agent even though it ultimately dispatches a
  // task. Treat the capability as the authority instead of inferring the check
  // from whichever settings form happened to expose the blocker.
  if (missing.has('ingress_authentication')) return 'native_authentication';
  if (facet === 'collaboration' && (missing.has('skill_loading') || missing.has('trusted_cli_execution'))) return 'collaboration';
  return null;
}
