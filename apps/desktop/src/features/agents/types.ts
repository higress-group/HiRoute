import type { Plan } from '../../plan-editor';
import type { OperationReference } from '../model-connections/types';

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

export type CodexAccess = {
  codex_home: string;
  slot_id: string;
  profile_context_id: string;
  root_context_id: string;
  selected_mode: 'profile' | 'root';
  slot_occupied: boolean;
  target_file: string;
  profile_name: string;
  commands: Record<string, string>;
  pending_operation: string | null;
  access_revoked: boolean;
  conflict_fields: string[];
};

export type CollaborationStatus = {
  state: string;
  restore_point_ref: string | null;
  current_selection?: { trigger_mode: AgentCollaborationTriggerMode } | null;
};
export type AgentModelSurface = 'codex_cli' | 'codex_desktop' | 'claude_cli' | 'qoder_cli' | 'pi_cli';
export type AgentReasoningSelection =
  | { kind: 'profile'; profile: string }
  | { kind: 'toggle'; enabled: boolean }
  | { kind: 'budget'; tokens: number };
export type AgentNativeReasoning =
  | { kind: 'fixed'; profile: string }
  | { kind: 'toggle'; parameter: string }
  | { kind: 'discrete'; parameter: string; profiles: string[] }
  | { kind: 'budget'; parameter: string; minimum_tokens: number; maximum_tokens: number; step_tokens: number };
export type AgentFixedModel = {
  client_model_id: string;
  candidate: { binding_id: string; reasoning?: AgentReasoningSelection };
};
export type AgentModelSourceCoverage = {
  binding_id: string;
  source_label: string;
  account_scope_ref: string;
  account_scope_digest: string;
  state: 'ready' | 'credential_required' | 'authorization_required' | 'disabled' | 'model_unconfirmed';
  reasoning: AgentNativeReasoning;
};
export type AgentNativeModel = {
  client_model_id: string;
  display_name: string;
  source_options?: AgentModelSourceCoverage[];
};
export type AgentNativeModelCatalog = {
  metadata_source: 'user_configured' | 'target_cache' | 'target_bundled';
  native_default_model: string;
  models: AgentNativeModel[];
};
export type CodexModelSelection = {
  mode: 'codex_default';
  native_model_mode: CodexNativeModelMode;
  fixed_models: AgentFixedModel[];
  allowed_plan_ids: string[];
  default_selection: AgentDefaultChoice;
};
export type ClaudeModelSelection = {
  mode: 'claude_launcher';
  surfaces: ['claude_cli'];
  fixed_models: AgentFixedModel[];
  preset_mappings: AgentClaudePresetMappings;
};
export type QoderModelSelection = {
  mode: 'qoder_additional';
  allowed_plan_ids: string[];
};
export type PiModelSelection = { mode: 'pi_additional'; allowed_plan_ids: string[] };
export type AgentModelSelection = CodexModelSelection | ClaudeModelSelection | QoderModelSelection | PiModelSelection;
export type AgentSurfaceResult = {
  surface: AgentModelSurface;
  applied_revision: number;
  state: 'not_verified' | 'passed' | 'failed';
  reason_code: string | null;
};
export type AgentLiveCheckTarget = {
  context_id: string;
  surface: AgentModelSurface;
  expected_applied_revision: number;
  client_model_ids: string[];
};
export type AgentLiveCheckCompletion = {
  accepted: boolean;
  scope: string;
  model_call: boolean;
  state?: string;
  call_count: number;
  requested_call_count: number;
};
export type ModelStatus = {
  state: string;
  operation_id?: string | null;
  operation_state?: string | null;
  model_verified: boolean;
  applied_revision?: number | null;
  surface_results?: AgentSurfaceResult[];
  live_check_targets?: AgentLiveCheckTarget[];
  current_selection?: AgentModelSelection | null;
  protected_native_model_ids?: string[];
  restore_point_ref: string | null;
  collaboration?: CollaborationStatus | null;
};
export type CollaborationOnlyStatus = {
  schema: 'hiroute.agent-collaboration-only-settings-status/v2';
  context_id: string;
  collaboration: CollaborationStatus;
};
export type AgentSettingsStatus = ModelStatus | CollaborationOnlyStatus;
export type Agent = {
  codex_access?: CodexAccess | null;
  agent_id: string;
  version: string;
  context_id: string | null;
  configuration_state: string;
  available_surfaces?: AgentModelSurface[];
  native_model_catalog?: AgentNativeModelCatalog | null;
  settings: AgentSettingsStatus | null;
  status_error: string | null;
};
export type AgentSnapshot = {
  agents: Agent[];
  plans: { plans: Plan[] };
  trusted_authority: boolean;
};
export type Preview = {
  applicable: boolean;
  unproven_native_model_ids?: string[];
  blockers: { reason: string; capabilities?: { capability: string; reason: string }[]; model_ids?: string[] }[];
};
export type AgentMutationOutcome = {
  state: string;
  operation: OperationReference | null;
};

export type Outcome = {
  preview: Preview;
  mutation: AgentMutationOutcome | null;
};
export type AgentFacet = 'model' | 'collaboration';
export type AgentCheckScope = 'native_authentication' | 'collaboration';
