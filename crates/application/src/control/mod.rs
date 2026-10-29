//! Dependency-injected read ports used by every Local Control client.
//!
//! These ports expose facts and durable records. Command validation, Preview digests, authority
//! decisions, and machine-envelope semantics stay in [`crate::ApplicationService`].

use std::{collections::BTreeMap, sync::Arc};

use hiroute_application_api::{
    AgentConnectSpecV1, AgentConnectionStatusRequestV1, AgentConnectionStatusV1,
    AgentLaunchDescriptorRequestV1, ApplyRequestV1, CanonicalDigest,
    ClassifierDecisionTestRequestV1, ClassifierDecisionTestResultV1,
    ComputeConnectionApplyRequestV1, ComputeConnectionChangeV1, ComputeConnectionOptionsResultV1,
    ComputeManagementQueryV2, ComputeManagementSnapshotV2, ComputeSavePreviewV2,
    ComputeSaveResultV2, ComputeScanResultV1, ComputeSubscriptionCandidatesV2,
    ComputeSubscriptionCheckPreviewV2, ComputeSubscriptionCheckResultV2,
    ManagedClaudeLaunchDescriptorV2, ModelConnectionCheckViewV1,
    NativeModelConnectionCheckRequestV1, OperationReferenceV1, PreviewRequestV1, PreviewResultV1,
    PrincipalKind, RegisteredModelConnectionCheckRequestV1, RevisionSetV1,
    SavedModelConnectionCheckRequestV1,
};
use hiroute_domain::{
    AgentPlanId, ObservationQueryPort, OperationId, OperationV1, PreparedComputeProjectionV1,
    SessionId, WorkspaceId,
};

use serde::Serialize;
use serde_json::Value;

mod collaboration_check;
pub use collaboration_check::CollaborationCheckError;
mod status;

pub use status::system_status;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlReadError {
    NotFound,
    /// Captures straddled a change; the caller must obtain a fresh Preview.
    SnapshotChanged,
    Unavailable,
    Corrupt,
    Denied,
    /// A retained service uses a representation that this legacy operation cannot return.
    ClientVersionRequired,
    /// A model-only Codex profile cannot own the collaboration Skill in the original HOME.
    CollaborationRootRequired,
}

/// Bounded observation composition never waits on a control writer mutex.
pub enum ControlReadAttempt<T> {
    Ready(T),
    Busy,
}

pub trait ClassifierDiagnosticPort: Send + Sync {
    fn test_decision_service_v2(
        &self,
        _service: &hiroute_domain::DecisionServiceRecord,
        _configuration_digest: &CanonicalDigest,
    ) -> Result<hiroute_application_api::DecisionServiceTestResultV2, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn test_classifier_c(
        &self,
        _request: &hiroute_application_api::HistoricalApplicabilityTestRequestV1,
        _service: &hiroute_domain::DecisionServiceV1,
    ) -> Result<hiroute_application_api::HistoricalApplicabilityTestResultV1, ControlReadError>
    {
        Err(ControlReadError::Unavailable)
    }
    fn test_classifier_decision(
        &self,
        request: &ClassifierDecisionTestRequestV1,
    ) -> Result<ClassifierDecisionTestResultV1, ControlReadError>;
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ControlStateSnapshotV1 {
    pub revisions: RevisionSetV1,
    pub desired_state: Option<Value>,
    pub recoverable_operations: Vec<OperationV1>,
}

pub use hiroute_application_api::{
    AgentConfigPermissionFindingV1, AgentModelCatalogMetadataSourceV1,
    AgentModelSourceCoverageStateV1, CodexAccessViewV1, DiscoveredAgentConfigurationV1,
    DiscoveredAgentModelCatalogV1, DiscoveredAgentModelSourceV1, DiscoveredAgentModelV1,
    DiscoveredAgentV1, DiscoveredCredentialInputV1,
};

pub trait ControlStatePort: Send + Sync {
    fn try_observation_revision(
        &self,
        _workspace: &WorkspaceId,
        _deadline: std::time::Instant,
    ) -> Result<ControlReadAttempt<u64>, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn try_validate_observation_capability(
        &self,
        _capability: &str,
        _workspace: &WorkspaceId,
        _principal: PrincipalKind,
        _operation: &str,
        _digest: &CanonicalDigest,
        _deadline: std::time::Instant,
    ) -> Result<ControlReadAttempt<u64>, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn trust_recovery_targets(
        &self,
        _plan_id: &str,
    ) -> Result<Vec<crate::agent_connection::PreviousRoutingTargetV1>, ControlReadError> {
        Ok(Vec::new())
    }

    fn trust_plan_ids(&self, _workspace: &WorkspaceId) -> Result<Vec<String>, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }

    fn trust_context(
        &self,
        _workspace: &WorkspaceId,
        _plan_id: &str,
    ) -> Result<crate::trust::context::TrustContextV1, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn trust_primary_bindings(
        &self,
        _workspace: &WorkspaceId,
        _plan_id: &str,
        _revision: Option<u64>,
        _branch_id: Option<&str>,
    ) -> Result<Vec<String>, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn trust_task_population(
        &self,
        _workspace: &WorkspaceId,
        _query: &hiroute_domain::TrustPopulationQueryV1,
        _from_ms: i64,
        _to_ms: i64,
    ) -> Result<hiroute_domain::TrustTaskPopulationV1, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn trust_runs(
        &self,
        _workspace: &WorkspaceId,
        _ids: &[String],
    ) -> Result<BTreeMap<String, crate::trust::context::TrustRunV1>, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }

    fn snapshot(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<ControlStateSnapshotV1, ControlReadError>;

    fn operation(
        &self,
        operation_id: &OperationId,
    ) -> Result<Option<OperationV1>, ControlReadError>;

    /// Read-only early idempotency lookup for typed handlers that must avoid re-normalizing a
    /// now-stale successful request before the transaction coordinator can replay it. The
    /// coordinator remains the only admission authority and performs the same lookup again under
    /// its writer lock.
    fn operation_for_idempotency(
        &self,
        _workspace_id: &WorkspaceId,
        _principal: PrincipalKind,
        _operation_kind: &str,
        _idempotency_key: &str,
    ) -> Result<Option<OperationV1>, ControlReadError> {
        Ok(None)
    }

    /// Validates a short-lived digest/revision/principal/operation-bound protected token without
    /// consuming it. Write tokens are consumed atomically only by durable operation admission;
    /// read-content tokens remain bounded by their exact scope and expiry.
    fn validate_protected_capability(
        &self,
        raw_capability: &str,
        workspace_id: &WorkspaceId,
        principal: PrincipalKind,
        operation_kind: &str,
        accepted_digest: &CanonicalDigest,
        expected_revisions: &RevisionSetV1,
    ) -> Result<(), ControlReadError>;

    fn consume_agent_live_check_capability(
        &self,
        _raw_capability: &str,
        _workspace_id: &WorkspaceId,
        _principal: PrincipalKind,
        _accepted_digest: &CanonicalDigest,
        _expected_revisions: &RevisionSetV1,
    ) -> Result<(), ControlReadError> {
        Err(ControlReadError::Denied)
    }
}

pub trait AgentDiscoveryPort: Send + Sync {
    fn list(&self) -> Result<Vec<DiscoveredAgentV1>, ControlReadError> {
        self.discover()
    }
    fn discover(&self) -> Result<Vec<DiscoveredAgentV1>, ControlReadError>;
    fn discover_context(
        &self,
        _context_id: &str,
        _target: Option<&hiroute_domain::AgentNativeTargetV1>,
    ) -> Result<Vec<DiscoveredAgentV1>, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn discover_target(
        &self,
        _target: &hiroute_domain::AgentNativeTargetV1,
    ) -> Result<Vec<DiscoveredAgentV1>, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
}

pub trait ApplicationClockPort: Send + Sync {
    fn now_ms(&self) -> Result<i64, ControlReadError>;
    /// Current machine's calendar day boundary, including local DST rules.
    /// Adapters without a timezone authority must not substitute rolling 24h.
    fn local_day_start_ms(&self, _at_ms: i64) -> Result<i64, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    /// Civil-date boundaries in this host's timezone. A named day and a
    /// relative day are alternatives; a day is not assumed to have 24 hours.
    fn analysis_local_day(
        &self,
        _day: Option<&str>,
        _days_ago: u8,
        _at_ms: i64,
    ) -> Result<hiroute_domain::AnalysisLocalDayV1, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn local_utc_offset_seconds(&self, _at_ms: i64) -> Result<i32, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValuePlanScopeV1 {
    pub from_ms: i64,
    pub to_ms: i64,
    pub currency: String,
    pub session_id: Option<SessionId>,
}

/// Lists only plan identities present in the immutable value ledger for an exact scope. The
/// Application remains responsible for defaults and aggregation; the storage adapter owns SQL.
pub trait ValueScopePort: Send + Sync {
    fn value_plan_ids(
        &self,
        workspace_id: &WorkspaceId,
        scope: &ValuePlanScopeV1,
    ) -> Result<Vec<AgentPlanId>, ControlReadError>;
}

/// Production mutation composition. Implementations only adapt concrete daemon ports; all
/// planning, digest validation, capability admission, journaling, and recovery semantics remain
/// in Application's transaction coordinator.
pub trait ApplicationMutationPort: Send + Sync {
    fn preview_change(
        &self,
        request: PreviewRequestV1,
    ) -> Result<PreviewResultV1, crate::TransactionError>;

    fn apply_change(
        &self,
        principal_kind: PrincipalKind,
        request: ApplyRequestV1,
    ) -> Result<OperationV1, crate::TransactionError>;

    /// A current-user mutation from the owner-only Local Control transport. The Application
    /// still reproduces the plan, compares revisions and admits exactly one durable Operation.
    fn apply_local_change(
        &self,
        request: ApplyRequestV1,
    ) -> Result<OperationV1, crate::TransactionError>;

    /// Admits a command-specific plan that Application has reproduced from the exact current
    /// discovery/publication facts. The adapter cannot deserialize or manufacture this value.
    fn apply_prepared_change(
        &self,
        principal_kind: PrincipalKind,
        prepared: crate::PreparedTransactionV1,
    ) -> Result<OperationV1, crate::TransactionError>;

    /// Applies a prepared management plan admitted by owner-only Local Control.
    fn apply_local_prepared_change(
        &self,
        prepared: crate::PreparedTransactionV1,
    ) -> Result<OperationV1, crate::TransactionError>;
}

/// Read-only facts needed by the typed AgentConnection planner and managed-launch endpoint.
/// The daemon adapter owns filesystem/publication joins; Application owns every product decision.
pub trait AgentConnectionControlPort: Send + Sync {
    fn previous_routing_head(
        &self,
        _agent: &str,
    ) -> Result<crate::agent_connection::PreviousRoutingHead, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }

    fn retry_settings_operation(
        &self,
        _request: &hiroute_application_api::AgentSettingsRetryV1,
    ) -> Result<OperationV1, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }

    /// Explicit protected Check admission only; never called during discovery.
    fn check_native_authentication(&self, _agent_id: &str) -> Result<(), ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn check_collaboration(&self, _agent_id: &str) -> Result<(), CollaborationCheckError> {
        Err(ControlReadError::Unavailable.into())
    }
    fn validate_live_check_target(
        &self,
        _request: &hiroute_application_api::AgentCheckRequestV1,
    ) -> Result<(), ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    /// Executes exactly the already-admitted Live target and returns backend-owned evidence.
    /// Implementations must never return `Passed` without a complete matching Gateway receipt.
    fn execute_live_check(
        &self,
        _request: &hiroute_application_api::AgentCheckRequestV1,
        _request_digest: &CanonicalDigest,
    ) -> Result<hiroute_domain::AgentSurfaceCheckRecordV1, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    /// Commits the backend-owned result through the existing context/surface/revision CAS.
    fn save_live_check_result(
        &self,
        _record: &hiroute_domain::AgentSurfaceCheckRecordV1,
    ) -> Result<bool, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn settings_status(
        &self,
        _request: &hiroute_application_api::AgentSettingsStatusRequestV2,
    ) -> Result<hiroute_application_api::AgentSettingsStatusV2, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }

    fn settings_summary(
        &self,
        _request: &hiroute_application_api::AgentSettingsSummaryRequestV1,
    ) -> Result<hiroute_application_api::AgentSettingsSummaryV1, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }

    fn settings_facts(
        &self,
        _spec: &hiroute_application_api::AgentSettingsSpecV2,
    ) -> Result<crate::agent_connection::AgentSettingsPlanningInput, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }

    fn planning_facts(
        &self,
        spec: &AgentConnectSpecV1,
    ) -> Result<crate::agent_connection::AgentConnectionPlanningInputV1, ControlReadError>;

    fn connection_status(
        &self,
        request: &AgentConnectionStatusRequestV1,
    ) -> Result<AgentConnectionStatusV1, ControlReadError>;

    fn managed_launch_descriptor(
        &self,
        request: &AgentLaunchDescriptorRequestV1,
    ) -> Result<ManagedClaudeLaunchDescriptorV2, ControlReadError>;
}

#[derive(Clone)]
pub struct RoutingCompilationSnapshotV1 {
    /// Read-only diagnostics, excluded from executable facts and their canonical digest.
    pub unavailable_candidates: Vec<hiroute_application_api::PlanUnavailableCandidateV1>,
    pub facts: crate::compiler::AgentPlanCompilationFactsV1,
    pub expected_revisions: RevisionSetV1,
    /// Exact active aggregate used only by Apply to preserve immutable aliases, existing Plans,
    /// and grants. Preview remains stateless over the captured facts and revisions above.
    pub active_publication: Option<hiroute_domain::GatewayPublicationV1>,
}

#[derive(Clone)]
pub struct TrialRoutingSnapshotV1 {
    pub routing: RoutingCompilationSnapshotV1,
    pub management: hiroute_application_api::TrialManagementSnapshotV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComputeProjectionReadError {
    InvalidSelection,
    RevisionConflict,
    ActionRequired,
    Control(ControlReadError),
}

/// Read-only production projection boundary. Its implementation owns the verified ReleaseFacts,
/// exact scanner intersection, and durable compute reads; Application owns command semantics.
pub trait ComputeFactsPort: Send + Sync {
    fn scan_compute(&self) -> Result<ComputeScanResultV1, ControlReadError>;
    fn scan_compute_points(
        &self,
        _request: &hiroute_application_api::ComputeScanRequestV2,
    ) -> Result<hiroute_application_api::ComputeScanResultV2, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }

    fn connection_options(&self) -> Result<ComputeConnectionOptionsResultV1, ControlReadError>;

    fn prepare_compute_projection(
        &self,
        change: &ComputeConnectionChangeV1,
    ) -> Result<PreparedComputeProjectionV1, ComputeProjectionReadError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComputeManagementControlError {
    Invalid,
    InvalidModelConnectionUrl,
    UnsafeModelConnectionPath,
    UnsafeModelConnectionAuthenticationHeader,
    UnsafeModelConnectionProtocolHeader,
    NotFound,
    Conflict,
    RegisteredOptionUnavailable,
    RegisteredCatalogChanged,
    RegisteredSourceMismatch,
    SavedSourceMismatch,
    RecheckContextUnavailable,
    DiscoveryUnavailable,
    DiscoveryChanged,
    DiscoveryNotImportable,
    PreviewStale,
    ActionRequired,
    Unavailable,
    Corrupt,
}

/// Production adapter boundary for the source-level model connection aggregate. Public payloads
/// remain closed DTOs; candidate facts and protected input slots stay behind this port.
pub trait ComputeManagementControlPort: Send + Sync {
    fn compute_key_masks(
        &self,
        _query: &hiroute_application_api::ComputeKeyMasksQueryV1,
    ) -> Result<hiroute_application_api::ComputeKeyMasksViewV1, ComputeManagementControlError> {
        Err(ComputeManagementControlError::Unavailable)
    }

    fn manage_subscription_login(
        &self,
        _request: hiroute_application_api::ComputeSubscriptionLoginRequestV1,
    ) -> Result<
        hiroute_application_api::ComputeSubscriptionLoginResultV1,
        ComputeManagementControlError,
    > {
        Err(ComputeManagementControlError::Unavailable)
    }

    fn compute_subscriptions(
        &self,
    ) -> Result<ComputeSubscriptionCandidatesV2, ComputeManagementControlError>;

    fn saved_subscription_candidate(
        &self,
        _source: hiroute_application_api::ComputeSavedSubscriptionSourceV1,
    ) -> Result<ComputeSubscriptionCandidatesV2, ComputeManagementControlError> {
        Err(ComputeManagementControlError::Unavailable)
    }

    fn preview_subscription_check(
        &self,
        candidate: hiroute_application_api::ComputeCandidateRefV2,
    ) -> Result<ComputeSubscriptionCheckPreviewV2, ComputeManagementControlError>;

    fn prepare_subscription_check(
        &self,
        request: ComputeConnectionApplyRequestV1,
        apply_capability: Option<String>,
    ) -> Result<crate::PreparedTransactionV1, ComputeManagementControlError>;

    fn compute_subscription_check_result(
        &self,
        operation: &OperationReferenceV1,
    ) -> Result<ComputeSubscriptionCheckResultV2, ComputeManagementControlError>;

    fn release_subscription_check(
        &self,
        validation: &hiroute_application_api::ComputeValidationRefV2,
    ) -> Result<ComputeSubscriptionCheckResultV2, ComputeManagementControlError>;

    fn check_native_model_connection(
        &self,
        request: NativeModelConnectionCheckRequestV1,
    ) -> Result<ModelConnectionCheckViewV1, ComputeManagementControlError>;

    fn check_registered_model_connection(
        &self,
        _request: RegisteredModelConnectionCheckRequestV1,
    ) -> Result<ModelConnectionCheckViewV1, ComputeManagementControlError> {
        Err(ComputeManagementControlError::NotFound)
    }

    fn prepare_discovered_model_connection(
        &self,
        _request: hiroute_application_api::PrepareDiscoveredModelConnectionRequestV1,
    ) -> Result<hiroute_application_api::ComputeCandidateViewV2, ComputeManagementControlError>
    {
        Err(ComputeManagementControlError::DiscoveryUnavailable)
    }

    fn check_saved_model_connection(
        &self,
        _request: SavedModelConnectionCheckRequestV1,
    ) -> Result<ModelConnectionCheckViewV1, ComputeManagementControlError> {
        Err(ComputeManagementControlError::NotFound)
    }

    fn cancel_native_model_connection_check(
        &self,
        check_id: &str,
    ) -> Result<(), ComputeManagementControlError>;

    fn get_compute_candidate(
        &self,
        candidate: &hiroute_application_api::ComputeCandidateRefV2,
    ) -> Result<hiroute_application_api::ComputeCandidateViewV2, ComputeManagementControlError>;

    fn compute_management_snapshot(
        &self,
        query: &ComputeManagementQueryV2,
    ) -> Result<ComputeManagementSnapshotV2, ComputeManagementControlError>;

    fn compute_management_snapshot_v3(
        &self,
        _query: &ComputeManagementQueryV2,
    ) -> Result<hiroute_application_api::ComputeManagementSnapshotV3, ComputeManagementControlError>
    {
        Err(ComputeManagementControlError::Unavailable)
    }

    fn preview_compute_save(
        &self,
        change: hiroute_application_api::ComputeManagementChangeV2,
    ) -> Result<ComputeSavePreviewV2, ComputeManagementControlError>;

    fn compute_defaults(
        &self,
        _query: hiroute_application_api::ComputeDefaultsQueryV1,
    ) -> Result<hiroute_application_api::ComputeDefaultsResultV1, ComputeManagementControlError>
    {
        Err(ComputeManagementControlError::Unavailable)
    }
    fn preview_compute_save_v3(
        &self,
        _change: hiroute_application_api::ComputeManagementChangeV3,
    ) -> Result<ComputeSavePreviewV2, ComputeManagementControlError> {
        Err(ComputeManagementControlError::Unavailable)
    }

    fn prepare_compute_save(
        &self,
        request: ComputeConnectionApplyRequestV1,
    ) -> Result<crate::PreparedTransactionV1, ComputeManagementControlError>;

    fn compute_save_result(
        &self,
        operation: &OperationReferenceV1,
    ) -> Result<ComputeSaveResultV2, ComputeManagementControlError>;
}

pub trait RoutingFactsPort: Send + Sync {
    fn decision_source_summary(
        &self,
        _source_id: &str,
    ) -> Result<hiroute_application_api::DecisionSourceSummaryV1, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn decision_connection_sources(
        &self,
        _workspace: &WorkspaceId,
    ) -> Result<hiroute_application_api::DecisionConnectionSourcesV1, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn decision_services_v2(
        &self,
        _workspace: &WorkspaceId,
    ) -> Result<hiroute_application_api::DecisionServicesV2, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn decision_service_record(
        &self,
        _workspace: &WorkspaceId,
        _id: &str,
        _revision: u64,
    ) -> Result<Option<hiroute_domain::DecisionServiceRecord>, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn decision_service(
        &self,
        _workspace: &WorkspaceId,
        _id: &str,
        _revision: u64,
    ) -> Result<Option<hiroute_domain::DecisionServiceV1>, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn publication_checkpoint_snapshot(
        &self,
        _workspace: &WorkspaceId,
    ) -> Result<
        (
            hiroute_domain::PublicationRecordV1,
            hiroute_domain::RevisionSetV1,
        ),
        ControlReadError,
    > {
        Err(ControlReadError::Unavailable)
    }
    fn decision_services(
        &self,
        _workspace: &WorkspaceId,
    ) -> Result<Vec<hiroute_domain::DecisionServiceV1>, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn claude_client_capability_preview(
        &self,
        _plan: &hiroute_domain::CompiledAgentPlanV1,
    ) -> Result<hiroute_application_api::ClaudeClientCapabilityPreviewV1, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }

    fn codex_client_capability_preview(
        &self,
        _plan: &hiroute_domain::CompiledAgentPlanV1,
    ) -> Result<hiroute_application_api::CodexClientCapabilityPreviewV1, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }

    fn dsh_client_capability_preview(
        &self,
        _plan: &hiroute_domain::CompiledAgentPlanV1,
    ) -> Result<hiroute_application_api::DshClientCapabilityPreviewV1, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }

    fn plan_draft_snapshot(
        &self,
        _workspace: &WorkspaceId,
        _change: &hiroute_domain::PlanDraftChangeV1,
    ) -> Result<crate::routing::PlanDraftSnapshotV1, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }

    fn plan_lifecycle_snapshot(
        &self,
        _workspace: &WorkspaceId,
        _change: &hiroute_application_api::PlanLifecycleChangeV1,
    ) -> Result<crate::routing::PlanLifecycleSnapshotV1, ControlReadError> {
        // No default empty reference list: a production reference adapter is required.
        Err(ControlReadError::Unavailable)
    }

    fn resolve_model_ratings(
        &self,
        _query: &hiroute_application_api::ResolveModelRatingsV1,
    ) -> Result<
        hiroute_application_api::ResolveModelRatingsResultV1,
        crate::model_catalog::RatingQueryError,
    > {
        Err(crate::model_catalog::RatingQueryError::SnapshotUnavailable)
    }

    fn free_plan_suggestions(
        &self,
        _workspace: &WorkspaceId,
        _requirements: &hiroute_domain::CapabilityRequirementsV1,
        _selections: &std::collections::BTreeMap<String, hiroute_domain::ReasoningSelectionV1>,
        _snapshot: hiroute_application_api::RatingSnapshotSelectionV1,
    ) -> Result<crate::routing::FreeSuggestionsV1, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }

    fn plan_authoring_snapshot(
        &self,
        _workspace: &WorkspaceId,
        _change: &hiroute_application_api::PlanContentChangeV2,
    ) -> Result<crate::routing::PlanAuthoringSnapshotV2, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }

    fn routing_compilation_snapshot(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<RoutingCompilationSnapshotV1, ControlReadError>;

    fn trial_routing_snapshot(
        &self,
        _workspace_id: &WorkspaceId,
    ) -> Result<TrialRoutingSnapshotV1, ControlReadError> {
        Err(ControlReadError::Unavailable)
    }

    fn try_active_publication(
        &self,
        _workspace: &WorkspaceId,
        _deadline: std::time::Instant,
    ) -> Result<ControlReadAttempt<Option<hiroute_domain::GatewayPublicationV1>>, ControlReadError>
    {
        Err(ControlReadError::Unavailable)
    }
}

#[derive(Clone)]
pub struct ApplicationPorts {
    /// True only after the daemon has composed the Gateway and the combined role is ready.
    pub role_all_ready: bool,
    pub model_catalog: Option<Arc<dyn crate::model_catalog::ModelCatalogPort>>,
    pub model_feed: Option<Arc<dyn crate::model_feed::ModelFeedPort>>,
    pub prices: Option<Arc<dyn crate::prices::SourcePriceControlPort>>,
    pub work_plans: Option<Arc<crate::delegation::work_plans::WorkPlanDirectory>>,
    pub delegation_tasks: Option<Arc<crate::delegation::tasks::DelegationTasks>>,
    pub client_access: Option<Arc<dyn crate::client_access::ClientAccessPort>>,
    pub control: Arc<dyn ControlStatePort>,
    pub discovery: Arc<dyn AgentDiscoveryPort>,
    pub observation: Arc<dyn ObservationQueryPort>,
    pub clock: Arc<dyn ApplicationClockPort>,
    pub value_scope: Arc<dyn ValueScopePort>,
    pub mutation: Option<Arc<dyn ApplicationMutationPort>>,
    pub compute: Option<Arc<dyn ComputeFactsPort>>,
    pub compute_management: Option<Arc<dyn ComputeManagementControlPort>>,
    pub routing: Option<Arc<dyn RoutingFactsPort>>,
    pub agent_connection: Option<Arc<dyn AgentConnectionControlPort>>,
    pub classifier_diagnostic: Option<Arc<dyn ClassifierDiagnosticPort>>,
}

impl ApplicationPorts {
    pub fn with_work_plans(
        mut self,
        port: Arc<crate::delegation::work_plans::WorkPlanDirectory>,
    ) -> Self {
        self.work_plans = Some(port);
        self
    }

    pub fn with_delegation_tasks(
        mut self,
        tasks: Arc<crate::delegation::tasks::DelegationTasks>,
    ) -> Self {
        self.delegation_tasks = Some(tasks);
        self
    }

    pub fn with_client_access(
        mut self,
        port: Arc<dyn crate::client_access::ClientAccessPort>,
    ) -> Self {
        self.client_access = Some(port);
        self
    }

    pub fn new(
        control: Arc<dyn ControlStatePort>,
        discovery: Arc<dyn AgentDiscoveryPort>,
        observation: Arc<dyn ObservationQueryPort>,
        clock: Arc<dyn ApplicationClockPort>,
        value_scope: Arc<dyn ValueScopePort>,
    ) -> Self {
        Self {
            role_all_ready: false,
            model_catalog: None,
            model_feed: None,
            prices: None,
            client_access: None,
            work_plans: None,
            delegation_tasks: None,
            control,
            discovery,
            observation,
            clock,
            value_scope,
            mutation: None,
            compute: None,
            compute_management: None,
            routing: None,
            agent_connection: None,
            classifier_diagnostic: None,
        }
    }

    pub fn with_role_all_ready(mut self) -> Self {
        self.role_all_ready = true;
        self
    }

    pub fn with_model_feed(mut self, port: Arc<dyn crate::model_feed::ModelFeedPort>) -> Self {
        self.model_feed = Some(port);
        self
    }
    pub fn with_model_catalog(
        mut self,
        port: Arc<dyn crate::model_catalog::ModelCatalogPort>,
    ) -> Self {
        self.model_catalog = Some(port);
        self
    }

    pub fn with_prices(mut self, port: Arc<dyn crate::prices::SourcePriceControlPort>) -> Self {
        self.prices = Some(port);
        self
    }

    pub fn with_mutation(mut self, mutation: Arc<dyn ApplicationMutationPort>) -> Self {
        self.mutation = Some(mutation);
        self
    }

    pub fn with_compute_routing<T: ComputeFactsPort + RoutingFactsPort + 'static>(
        mut self,
        port: Arc<T>,
    ) -> Self {
        self.compute = Some(port.clone());
        self.routing = Some(port);
        self
    }

    pub fn with_compute_management(mut self, port: Arc<dyn ComputeManagementControlPort>) -> Self {
        self.compute_management = Some(port);
        self
    }

    pub fn with_agent_connection(mut self, port: Arc<dyn AgentConnectionControlPort>) -> Self {
        self.agent_connection = Some(port);
        self
    }

    pub fn with_classifier_diagnostic(mut self, port: Arc<dyn ClassifierDiagnosticPort>) -> Self {
        self.classifier_diagnostic = Some(port);
        self
    }
}
