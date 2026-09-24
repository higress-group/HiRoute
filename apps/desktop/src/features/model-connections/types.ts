export type Language = 'zh' | 'en';
export type CanonicalDigest = `sha256:${string}`;
export type EntryKind = 'preset_api' | 'custom_api' | 'free_api_key' | 'free_direct';
export type UpstreamProtocol = 'responses' | 'messages' | 'chat_completions';
export type BaseKind = 'api_root' | 'native_messages_base' | 'native_responses_base';
export type FactBasis = 'registered_catalog' | 'runtime_fallback' | 'observed' | 'user_declared' | 'unknown';
export type TriState = 'supported' | 'unsupported' | 'unknown';

export type Authentication =
  | { kind: 'none' }
  | { kind: 'bearer' }
  | { kind: 'api_key_header'; header: string };

export type ModelConnectionEndpointDraft = {
  base_url: string;
  base_kind: BaseKind;
  request_path_override: string | null;
  inventory_path_override: string | null;
  protocol: UpstreamProtocol;
  protocol_profile_id: string;
  protocol_profile_revision: number;
  authentication: Authentication;
};

export type NativeReasoning =
  | { kind: 'unknown' }
  | { kind: 'fixed'; profile: string }
  | { kind: 'toggle'; parameter: string }
  | { kind: 'discrete'; parameter: string; profiles: string[] }
  | { kind: 'budget'; parameter: string; minimum_tokens: number; maximum_tokens: number; step_tokens: number };

export type FactValue<T> = { value: T | null; basis: FactBasis };

export type ModelDeclaration = {
  client_id: string;
  upstream_model_id: string;
  display_name: string;
  catalog_configuration_id: string | null;
  membership: 'observed' | 'user_declared';
  capabilities: {
    tool: FactValue<boolean>;
    vision: FactValue<boolean>;
    streaming: FactValue<boolean>;
    context_tokens: FactValue<number>;
    max_output_tokens: FactValue<number>;
    native_reasoning: FactValue<Exclude<NativeReasoning, { kind: 'unknown' }>>;
  };
};

export type ModelConnectionDraft = {
  display_template_id?: string | null;
  inference_model_id?: string | null;
  entry_kind: EntryKind;
  candidate_ref: string | null;
  lineage_ref: string;
  display_name: string;
  existing_source_id: string | null;
  expected_source_revision?: number | null;
  edit_revision: number;
  check_id: string;
  base_url: string;
  base_kind: BaseKind;
  request_path_override: string | null;
  inventory_path_override: string | null;
  protocol: UpstreamProtocol;
  protocol_profile_id: string;
  protocol_profile_revision: number;
  authentication: Authentication;
  additional_endpoints: ModelConnectionEndpointDraft[];
  provenance:
    | { kind: 'registered'; connection_option_id: string; registry_version: string; catalog_digest: CanonicalDigest }
    | { kind: 'user_configured'; configuration_revision: number };
  qualification: {
    free_access: 'direct' | 'api_key_required' | null;
    evidence_ref: string | null;
  };
  models: ModelDeclaration[];
};

export type ComputeCandidateRef = { candidate_ref: string; candidate_revision: number };
export type ComputeCatalogProvenance = {
  product_release: string;
  catalog_binding_id: string;
  release_sequence: number;
  connector_registry_digest: CanonicalDigest;
  model_data_digest: CanonicalDigest;
  cross_reference_digest: CanonicalDigest;
};
export type ComputeConnectionOption = {
  endpoints?: { base_url: string; request_path: string; inventory_path?: string | null; protocol: UpstreamProtocol; authentication_semantics?: Authentication; stable_preference: number }[];
  known_models?: Record<string, string>;
  connection_option_id: string;
  display_name: string;
  connector_id: string;
  connector_revision: number;
  endpoint_profile_id: string;
  endpoint_profile_revision: number;
  origin: 'native_api' | 'agent_subscription' | 'free_catalog';
  billing_class: 'free' | 'subscription' | 'paid' | 'unknown';
  authentication: string;
  model_configuration_ids: string[];
  registered_check_available: boolean;
};
export type MetadataState = 'complete' | 'partial' | 'missing' | 'not_applicable';
export type MetadataCapabilityState = 'supported' | 'unsupported' | 'conditional' | 'unknown' | 'not_applicable';
export type MetadataTokenLimit = {
  state:
    | 'conflict'
    | 'known'
    | 'unknown'
    | 'runtime_required'
    | 'shared_total_budget'
    | 'entitlement_dependent'
    | 'not_applicable';
  value: number | null;
  candidates?: number[];
};
export type MetadataFieldProvenance = { basis: 'inferred'; rule_key: string };
export type MetadataExecutionFit = {
  state: 'native_text_representable' | 'unsupported' | 'not_applicable';
  reason?: string;
};
export type ProviderMetadataRecord = {
  provider_record_key: string;
  provider_id: string;
  display_name: string;
  description_candidates: string[];
  base_url_candidates: string[];
  protocol_candidates: UpstreamProtocol[];
  authentication_candidates: string[];
  default_model_ids: string[];
  metadata_completeness: {
    authentication: MetadataState;
    discovery: MetadataState;
    endpoint: MetadataState;
    identity: MetadataState;
  };
  usable_for: string[];
  field_provenance?: Record<string, MetadataFieldProvenance>;
};
export type ModelMetadataRecord = {
  model_record_key: string;
  provider_record_key: string;
  provider_id: string;
  upstream_model_id: string;
  display_name: string;
  context_tokens: MetadataTokenLimit;
  max_output_tokens: MetadataTokenLimit;
  input_modalities: string[];
  capability_hints: {
    reasoning: MetadataCapabilityState;
    streaming: MetadataCapabilityState;
    tool: MetadataCapabilityState;
    vision: MetadataCapabilityState;
  };
  reasoning_rendering_hints: {
    reasoning_effort_maps: Record<string, string | null>[];
    supported_reasoning_efforts: string[];
    thinking_level_maps: Record<string, string | null>[];
  };
  cost_hints: Record<string, unknown>[];
  lifecycle: string;
  replacement_upstream_ids: string[];
  normalized_model_matches: string[];
  metadata_completeness: {
    capabilities: MetadataState;
    cost: MetadataState;
    identity: MetadataState;
    lifecycle: MetadataState;
    limits: MetadataState;
    modalities: MetadataState;
    reasoning_rendering: MetadataState;
  };
  usable_for: string[];
  execution_fit: MetadataExecutionFit;
  cost_hint_state: 'recorded' | 'not_recorded';
  reasoning_rendering_state?: 'not_applicable';
  field_provenance?: Record<string, MetadataFieldProvenance>;
};
export type ModelMetadataCatalog = {
  schema: string;
  as_of: string;
  source_catalog_digest: CanonicalDigest;
  access_products: {
    product_key: string;
    interfaces: { interface_key: string; protocol: string; base_url: string | null; request_path: string | null }[];
    documented_upstream_model_ids: string[];
  }[];
  canonical_models: { model_key: string; display_name: string }[];
  endpoint_bindings: {
    product_key: string;
    model_key: string;
    upstream_model_id: string;
    interface_candidates: string[];
    lifecycle: string | null;
  }[];
  provider_records: ProviderMetadataRecord[];
  model_records: ModelMetadataRecord[];
};
export type ComputeConnectionOptions = {
  schema: string;
  catalog: ComputeCatalogProvenance;
  options: ComputeConnectionOption[];
  metadata_catalog?: ModelMetadataCatalog | null;
};
export type ComputeCheckCorrelation = {
  candidate_ref: string;
  edit_revision: number;
  check_id: string;
  input_digest: CanonicalDigest;
};
export type OperationReference = {
  operation_id: string;
  state: string;
  sequence: number;
  cancellable: boolean;
};
export type ComputeValidationRef = {
  approval_operation: OperationReference;
  validation_ref: string;
  validation_revision: string;
};
export type CandidateModelView = {
  model_ref: string;
  upstream_model_id: string;
  display_name: string;
  membership: 'catalog' | 'observed' | 'user_declared';
  fact_basis: FactBasis | 'connector_verified';
  selectable: boolean;
  reason?: string;
};
export type ComputeCandidateView = {
  candidate: ComputeCandidateRef;
  correlation: ComputeCheckCorrelation;
  producer: 'native' | 'cpa';
  provenance: 'registered' | 'user_configured' | 'connector_owned';
  display_name: string;
  existing_source_id?: string;
  models: CandidateModelView[];
  input_state: 'not_required' | 'provided' | 'missing' | 'unavailable';
  fact_state: 'pending_credential' | 'pending_approval' | 'complete';
  validation?: ComputeValidationRef;
  issues?: { code: string; message_key: string; retryable: boolean }[];
};

export type ModelConnectionCheckView = {
  inference_model_id?: string | null;
  candidate: ComputeCandidateView;
  target: {
    scheme: string;
    authority: string;
    port: number;
    request_path: string;
    upstream_protocol: UpstreamProtocol;
    protocol_profile_id: string;
    protocol_profile_revision: number;
  };
  inventory_path: string | null;
  reachability: 'not_run' | 'reachable' | 'transport_failed';
  authentication: 'not_run' | 'not_required' | 'verified' | 'rejected' | 'unknown';
  directory: 'not_run' | 'available' | 'empty' | 'unavailable' | 'invalid' | 'partial';
  protocol: 'selected' | 'inconclusive';
  inference: 'not_run' | 'verified' | 'failed';
  checked_model_count: number;
  invalid_model_count: number;
  pages_read: number;
  checked_at_unix_ms: number;
  input_digest: CanonicalDigest;
  issues?: { code: string; message_key: string; retryable: boolean }[];
};

export type RevisionSet = { target: number; dependencies: Record<string, number> };
export type ChangeSpec = {
  schema_version: { major: number; minor: number };
  command_id: string;
  resource_id?: string;
  desired_state: unknown;
};
export type ComputeManagementChange = {
  schema: 'hiroute.compute-management-change/v2';
  subject: { kind: 'candidate'; candidate: ComputeCandidateRef };
  expected_revisions: RevisionSet;
  selected_model_refs: string[];
  intent: 'save_ready' | 'save_disabled';
  key_edits: { action: 'add'; input_candidate: ComputeCandidateRef }[];
  validation?: ComputeValidationRef;
};
export type ComputeSavePreview = {
  candidate?: ComputeCandidateRef;
  validation?: ComputeValidationRef;
  spec: ChangeSpec;
  accept_digest: CanonicalDigest;
  expected_revisions: RevisionSet;
  changes: { resource_kind: string; resource_id: string; action: string }[];
  affected_plan_refs: string[];
};
export type ComputeConnectionApplyRequest = {
  spec: ChangeSpec;
  accept_digest: CanonicalDigest;
  expected_revisions: RevisionSet;
  idempotency_key: string;
};
export type ComputeSaveResult = {
  candidate?: ComputeCandidateRef;
  validation?: ComputeValidationRef;
  disposition: 'saved' | 'pending' | 'needs_input' | 'conflict' | 'failed';
  source_id?: string;
  bindings: { model_ref: string; binding_id: string; revision: number }[];
  saved_revision?: number;
  management_state?: string;
  operation?: OperationReference;
  reason?: string;
};

export type ProtectedInputRegistration = { input_candidate: ComputeCandidateRef };
export type ModelConnectionDraftInput = Omit<
  ModelConnectionDraft,
  'entry_kind' | 'models' | 'provenance' | 'qualification'
> & {
  configuration_revision: number;
  models: Omit<ModelDeclaration, 'client_id'>[];
};
export type ModelConnectionCheckRequest = {
  draft: ModelConnectionDraftInput;
  input_candidate?: ComputeCandidateRef;
};
export type RegisteredModelConnectionCheckRequest = {
  inference_model_id?: string | null;
  models?: Omit<ModelDeclaration, 'client_id'>[];
  connection_option_id: string;
  expected_catalog: ComputeCatalogProvenance;
  candidate_ref?: string | null;
  lineage_ref: string;
  edit_revision: number;
  check_id: string;
  input_candidate: ComputeCandidateRef;
  existing_source_id?: string | null;
  expected_source_revision?: number | null;
};

export type SavedModelConnectionCheckRequest = {
  source_id: string;
  expected_source_revision: number;
  candidate_ref?: string | null;
  edit_revision: number;
  check_id: string;
};

/** Implemented by the native host through Client Core and its protected-input boundary. */
export type ModelConnectionBackend = {
  registerProtectedInput(secret: string): Promise<ProtectedInputRegistration>;
  releaseProtectedInput(input: ComputeCandidateRef): Promise<void>;
  listConnectionOptions(): Promise<ComputeConnectionOptions>;
  checkModelConnection(request: ModelConnectionCheckRequest): Promise<ModelConnectionCheckView>;
  checkRegisteredModelConnection(request: RegisteredModelConnectionCheckRequest): Promise<ModelConnectionCheckView>;
  cancelModelConnectionCheck(checkId: string): Promise<void>;
  previewComputeSave(change: ComputeManagementChange): Promise<ComputeSavePreview>;
  applyComputeSave(request: ComputeConnectionApplyRequest): Promise<{
    result: { operation_id: string; accepted_digest: CanonicalDigest; state: string };
    operation: OperationReference;
  }>;
  getComputeSaveResult(operation: OperationReference): Promise<ComputeSaveResult>;
};

export type ModelConnectionFormProps = {
  language: Language;
  mutable: boolean;
  initialDraft: ModelConnectionDraft;
  expectedRevisions: RevisionSet;
  backend: ModelConnectionBackend;
  onBack(): void;
  onCancel(): void;
  onSaveAccepted(operation: OperationReference): void;
  onSaveResult(result: ComputeSaveResult): void;
  onSaveUncertain(input: { idempotency_key: string; candidate: ComputeCandidateRef }): void;
  onConfigurePrice?(modelClientId: string): void;
  restoreFocus(): void;
};
