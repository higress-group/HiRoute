export type RevisionSet = { target: number; dependencies: Record<string, number> };
export type CandidateRef = { candidate_ref: string; candidate_revision: number };
export type MaterializationState = 'needs_credential' | 'needs_authorization' | 'ready' | 'disabled';
export type Provenance = 'registered' | 'user_configured' | 'connector_owned';
export type KeyAvailability = 'available' | 'cooling_down' | 'disabled' | 'unavailable' | 'unknown';

export type ManagedModel = {
  model_ref: string;
  binding_id: string;
  revision: number;
  upstream_model_id: string;
  display_name: string;
  catalog_configuration_id?: string;
  membership: 'catalog' | 'observed' | 'user_declared';
  capabilities?: import('../model-connections/types').ModelDeclaration['capabilities'];
  native_reasoning?: unknown;
  presentation?: {
    billing_class: 'free' | 'subscription' | 'paid' | 'unknown';
    availability: 'available' | 'cooling_down' | 'disabled' | 'needs_credentials' | 'unavailable' | 'unknown';
    reason_code: string | null;
    evaluated_at_ms: number;
    price_contexts: {
      currency: string;
      valuation_kind: 'usage_estimate' | 'api_equivalent';
    }[];
  };
};

export type ManagedKey = {
  key_id: string;
  generation: number;
  fingerprint_hint: string;
  ordinal: number;
  enabled: boolean;
  model_statuses: {
    binding_id: string;
    availability: KeyAvailability;
    cooldown_until_ms?: number;
  }[];
};

export type ManagedSource = {
  source_id: string;
  revision: number;
  display_name: string;
  provenance: Provenance;
  display_template_id?: string | null;
  inventory_path?: string | null;
  connection_identity?: {
    access_kind: 'api' | 'subscription' | 'unknown';
    connection_option_id: string | null;
    product_label: string | null;
  };
  target: {
    scheme: string;
    authority: string;
    port: number;
    request_path: string;
    upstream_protocol: string;
    protocol_profile_id: string;
    protocol_profile_revision: number;
  };
  authentication:
    | { kind: 'api_key_header'; header: string }
    | { kind: 'bearer' }
    | { kind: 'none' };
  additional_native_endpoints?: {
    target: ManagedSource['target'];
    authentication: ManagedSource['authentication'];
    recheck?: { inventory_path: string | null } | null;
  }[];
  state: MaterializationState;
  models: ManagedModel[];
  keys: ManagedKey[];
  ready_model_count: number;
  actions: ('edit' | 'add_key' | 'recheck' | 'reauthorize' | 'enable' | 'disable')[];
};

export type ManagementSnapshot = {
  schema: string;
  revisions: RevisionSet;
  runtime_state: 'complete' | 'partial';
  sources: ManagedSource[];
};

export type KeyEdit =
  | { action: 'add'; input_candidate: CandidateRef }
  | { action: 'replace'; key_id: string; expected_generation: number; input_candidate: CandidateRef }
  | { action: 'remove'; key_id: string; expected_generation: number }
  | { action: 'set_enabled'; key_id: string; expected_generation: number; enabled: boolean }
  | { action: 'set_order'; key_ids: string[] };

export type ManagementChange = {
  schema: 'hiroute.compute-management-change/v2';
  subject:
    | { kind: 'saved_source'; source_id: string }
    | { kind: 'candidate'; candidate: CandidateRef };
  expected_revisions: RevisionSet;
  selected_model_refs: string[];
  intent: 'save_ready' | 'save_disabled';
  key_edits: KeyEdit[];
  validation?: {
    approval_operation: { operation_id: string; state: string; sequence: number; cancellable: boolean };
    validation_ref: string;
    validation_revision: string;
  };
};

export type SavePreview = {
  spec: unknown;
  accept_digest: string;
  expected_revisions: RevisionSet;
  changes: { resource_kind: string; resource_id: string; action: string }[];
  affected_plan_refs: string[];
};

export type ProtectedKeyInput = {
  draft_id: string;
  key_id?: string;
  expected_generation: number;
  value: string;
};
