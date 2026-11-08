import type { NativeNetworkFailure, NetworkCheckResponse } from "../model-connections/native-network";

/** Backend-owned Product Session wire contract; UI code consumes but does not edit it. */
export type CanonicalDigest = string; // sha256:<64 lowercase hex digits>
export type DecisionWireProtocol = 'system_one' | 'openai_decisions';
export type DecisionAuthHeader = { name: string; value_secret_ref: string };

export type DecisionConnectionV1 =
  | { kind: 'system_one'; provider: string; model: string; endpoint: string; timeout_ms: number; auth_header: DecisionAuthHeader }
  | { kind: 'custom'; endpoint: string; timeout_ms: number; auth_header?: DecisionAuthHeader | null };
export type DecisionServiceV1 = { id: string; revision: number; name: string; connection: DecisionConnectionV1 };
export type DecisionAuthenticationV2 =
  | { kind: 'independent'; auth_header: DecisionAuthHeader }
  | { kind: 'shared_source'; source_id: string; source_lineage_digest: CanonicalDigest };
export type DecisionConnectionV2 =
  | { kind: 'native'; provider: string; wire_protocol: DecisionWireProtocol; model: string; endpoint: string; timeout_ms: number; authentication: DecisionAuthenticationV2 }
  | { kind: 'custom'; endpoint: string; timeout_ms: number; auth_header: DecisionAuthHeader | null };
export type DecisionServiceV2 = {
  schema: 'hiroute.decision-service/v2'; id: string; revision: number; name: string; connection: DecisionConnectionV2;
};
export type DecisionContextPolicy = {
  mode: 'automatic' | 'custom'; window_tokens?: number; per_question_tokens?: number;
  typesafe_long_context: 'automatic' | 'disabled';
};
export type ResolvedDecisionContext = {
  request_window_tokens: number; per_question_window_tokens: number;
  request_source: 'user' | 'metadata' | 'fallback'; per_question_source: 'user' | 'metadata' | 'fallback';
  catalog_revision: number; catalog_digest: CanonicalDigest;
  metadata: { binding_key: string; interface_key: string; constraint_key: string;
    basis: 'documented' | 'inferred' | 'maintainer_policy'; evidence_refs: string[]; note: string } | null;
  typesafe_long_context_header: boolean; accounting_algorithm: string; reserve_policy: string; context_digest: CanonicalDigest;
};
export type DecisionContextPreviewInput = { endpoint: string; model: string; wire_protocol: DecisionWireProtocol; policy: DecisionContextPolicy };
export type DecisionServiceV3 = {
  schema: 'hiroute.decision-service/v3'; id: string; revision: number; name: string;
  connection: DecisionConnectionV2; context?: { policy: DecisionContextPolicy; resolved: ResolvedDecisionContext };
};
export type DecisionServiceRecord = DecisionServiceV1 | DecisionServiceV2 | DecisionServiceV3;
export type DecisionClassifier =
  | { kind: 'local_rules' }
  | { kind: 'decision_service'; service: DecisionServiceV1 }
  | { kind: 'decision_service_v2'; service: DecisionServiceV2 }
  | { kind: 'decision_service_v3'; service: DecisionServiceV3 };

export type DecisionSourceState = 'ready' | 'disabled' | 'missing_credentials' | 'missing' | 'unavailable';
export type DecisionSourceSuggestion = {
  model: string | null; endpoint: string | null; wire_protocol: DecisionWireProtocol | null;
};
export type DecisionSourceSummary = {
  source_id: string; lineage_digest: CanonicalDigest | null; revision: number | null;
  display_name: string | null; provider_label: string | null; endpoint_host: string | null;
  auth_kind: 'native_api_key' | null; state: DecisionSourceState;
  credential_state_revision: CanonicalDigest | null; suggestion: DecisionSourceSuggestion | null;
};
export type DecisionConnectionSources = { sources: DecisionSourceSummary[] };
export type DecisionServiceView = {
  service: DecisionServiceRecord; configuration_digest: CanonicalDigest;
  credential_state_revision: CanonicalDigest | null; source: DecisionSourceSummary | null;
  related_sources: DecisionSourceSummary[];
};
export type DecisionServices = { schema: 'hiroute.decision-services/v3'; services: DecisionServiceView[] };
export type DecisionExpectedSourceRevision = { source_id: string; revision: number };
export type DecisionServiceChange = {
  id: string; expected_revision: number; service: DecisionServiceV3 | null;
  expected_source_revisions: DecisionExpectedSourceRevision[];
};
export type DecisionServiceSaveArgs = { input: { change: DecisionServiceChange; secret: string | null; expected_configuration_digest?: CanonicalDigest } };
export type DecisionServiceApplyResult = { operation_id: string; accepted_digest: CanonicalDigest; state: string };
export type DecisionRemovalProtection = 'current_head' | 'prepared_version' | 'unresolved_hold' | 'draft' | 'active_publication' | 'prepared_publication' | 'lkg_publication';
export type DecisionRemovalBlocker = { plan_id: string | null; draft_id: string | null; revision: number; protection: DecisionRemovalProtection };
export type DecisionRemovalPreview = { id: string; revision: number; configuration_digest: CanonicalDigest; failure: DecisionFailureCode | null; blockers: DecisionRemovalBlocker[] };
export type DecisionRemovalOutcome = 'removed' | 'rolled_back' | 'unconfirmed';
export type DecisionServiceTestArgs = { input: { id: string; revision: number; expected_configuration_digest: CanonicalDigest } };
export type DecisionFailureCode =
  | 'DECISION_INVALID_CONFIGURATION' | 'DECISION_SOURCE_NOT_FOUND' | 'DECISION_SOURCE_LINEAGE_MISMATCH'
  | 'DECISION_SOURCE_DISABLED' | 'DECISION_SOURCE_UNAVAILABLE' | 'DECISION_NO_USABLE_CREDENTIALS'
  | 'DECISION_REVISION_CONFLICT' | 'DECISION_IN_USE' | 'DECISION_TIMEOUT'
  | 'DECISION_AUTHENTICATION_REJECTED' | 'DECISION_RATE_LIMITED' | 'DECISION_INPUT_REJECTED'
  | 'DECISION_MALFORMED_OUTPUT' | 'DECISION_TRANSPORT_UNAVAILABLE';
export type DecisionServiceTestResult = {
  schema: 'hiroute.decision-service-test-result/v2'; outcome: 'passed' | 'failed';
  failure_code: DecisionFailureCode | null; duration_millis: number | null;
  tested_configuration_digest: CanonicalDigest; tested_credential_state_revision: CanonicalDigest | null;
};

export type DecisionServiceNetworkTestResult = {
  schema: 'hiroute.decision-service-test-result/v3'; check: DecisionServiceTestResult;
  network: NetworkCheckResponse['network'] | null; failure: NativeNetworkFailure | null;
  decision_failure?: DecisionFailureDetail;
};

export type DecisionFailureDetail = {
  kind: 'timeout' | 'unavailable' | 'source_disabled' | 'no_usable_credentials'
    | 'budget_exhausted' | 'budget_unavailable'
    | 'authentication_rejected' | 'proxy_authentication_rejected' | 'rate_limited'
    | 'endpoint_rejected' | 'input_rejected' | 'context_limit' | 'upstream_error' | 'invalid_output';
  http_status: number | null;
};

/** Names and wrappers are identical for Tauri and web-console invoke. */
export type DecisionProductCalls = {
  decision_connection_sources: { args: Record<string, never>; result: DecisionConnectionSources };
  decision_services_v2: { args: Record<string, never>; result: { services: DecisionServiceView[] } };
  save_decision_service_v2: { args: { input: { change: Omit<DecisionServiceChange, 'service'> & { service: DecisionServiceV2 | null }; secret: string | null } }; result: DecisionServiceApplyResult };
  decision_services_v3: { args: Record<string, never>; result: DecisionServices };
  preview_decision_context: { args: { input: DecisionContextPreviewInput }; result: ResolvedDecisionContext };
  preview_decision_removal: { args: { input: { id: string; expected_revision: number } }; result: DecisionRemovalPreview };
  save_decision_service_v3: { args: DecisionServiceSaveArgs; result: DecisionServiceApplyResult };
  test_decision_service_v2: { args: DecisionServiceTestArgs; result: DecisionServiceTestResult };
  test_decision_service_v3: { args: DecisionServiceTestArgs; result: DecisionServiceNetworkTestResult };
};
