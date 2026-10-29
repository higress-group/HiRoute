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
export type DecisionServiceRecord = DecisionServiceV1 | DecisionServiceV2;
export type DecisionClassifier =
  | { kind: 'local_rules' }
  | { kind: 'decision_service'; service: DecisionServiceV1 }
  | { kind: 'decision_service_v2'; service: DecisionServiceV2 };

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
export type DecisionServices = { services: DecisionServiceView[] };
export type DecisionExpectedSourceRevision = { source_id: string; revision: number };
export type DecisionServiceChange = {
  id: string; expected_revision: number; service: DecisionServiceV2 | null;
  expected_source_revisions: DecisionExpectedSourceRevision[];
};
export type DecisionServiceSaveArgs = { input: { change: DecisionServiceChange; secret: string | null } };
export type DecisionServiceApplyResult = { operation_id: string; accepted_digest: CanonicalDigest; state: string };
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

/** Names and wrappers are identical for Tauri and web-console invoke. */
export type DecisionProductCalls = {
  decision_connection_sources: { args: Record<string, never>; result: DecisionConnectionSources };
  decision_services_v2: { args: Record<string, never>; result: DecisionServices };
  save_decision_service_v2: { args: DecisionServiceSaveArgs; result: DecisionServiceApplyResult };
  test_decision_service_v2: { args: DecisionServiceTestArgs; result: DecisionServiceTestResult };
};
