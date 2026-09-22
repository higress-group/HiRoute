//! Dependency-injected read ports used by every Local Control client.
//!
//! These ports expose facts and durable records. Command validation, Preview digests, authority
//! decisions, and machine-envelope semantics stay in [`crate::ApplicationService`].

use std::sync::Arc;

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
    AgentModelSurfaceV2, AgentPlanId, CanonicalDigest as DomainCanonicalDigest,
    NativeReasoningCapabilityV1, ObservationQueryPort, OperationId, OperationV1,
    PreparedComputeProjectionV1, SessionId, WorkspaceId,
};
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}

pub trait ClassifierDiagnosticPort: Send + Sync {
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredAgentV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    pub agent_id: String,
    pub profile_id: String,
    pub version: String,
    pub supported: bool,
    pub configuration_state: String,
    /// Independently located native entry points. Location is not an executable identity,
    /// version, digest, or installation admission gate.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub available_surfaces: BTreeSet<AgentModelSurfaceV2>,
    /// Bounded projection of the native client catalog plus independently proven source/account
    /// coverage. Large native metadata and all credentials remain behind the daemon boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_model_catalog: Option<DiscoveredAgentModelCatalogV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registered_configuration: Option<DiscoveredAgentConfigurationV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovered_credential: Option<DiscoveredCredentialInputV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_hardening: Option<AgentConfigPermissionFindingV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredAgentModelCatalogV1 {
    pub metadata_source: AgentModelCatalogMetadataSourceV1,
    pub native_default_model: String,
    pub models: Vec<DiscoveredAgentModelV1>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelCatalogMetadataSourceV1 {
    UserConfigured,
    TargetCache,
    TargetBundled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredAgentModelV1 {
    pub client_model_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_options: Vec<DiscoveredAgentModelSourceV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredAgentModelSourceV1 {
    pub binding_id: String,
    pub source_label: String,
    /// Opaque account/source identity. It is safe for correlation and contains no credential or
    /// human account claim.
    pub account_scope_ref: String,
    pub account_scope_digest: DomainCanonicalDigest,
    pub state: AgentModelSourceCoverageStateV1,
    pub reasoning: NativeReasoningCapabilityV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelSourceCoverageStateV1 {
    Ready,
    CredentialRequired,
    AuthorizationRequired,
    Disabled,
    ModelUnconfirmed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredAgentConfigurationV1 {
    pub connection_option_id: String,
    pub endpoint_profile_id: String,
    pub endpoint_profile_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_configuration_id: Option<String>,
    pub base_url: String,
    pub observed_model_id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_model_alias_hints: BTreeMap<String, String>,
    pub configuration_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredCredentialInputV1 {
    pub source: String,
    pub scanner_id: String,
    pub scanner_version: String,
    pub discovered_source_ref: String,
    pub field_selector: String,
    pub observed_revision: u64,
    /// Opaque descriptor understood only by the protected-input port. It contains no path or
    /// Secret and is bound to the exact scanner revision.
    pub protected_input_slot: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfigPermissionFindingV1 {
    pub scanner_id: String,
    pub scanner_version: String,
    pub discovered_source_ref: String,
    pub observed_identity: CanonicalDigest,
    pub observed_revision: u64,
    pub display_path: String,
    pub required_mode: u32,
}

pub trait ControlStatePort: Send + Sync {
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
    fn discover(&self) -> Result<Vec<DiscoveredAgentV1>, ControlReadError>;
}

pub trait ApplicationClockPort: Send + Sync {
    fn now_ms(&self) -> Result<i64, ControlReadError>;
    /// Current machine's calendar day boundary, including local DST rules.
    /// Adapters without a timezone authority must not substitute rolling 24h.
    fn local_day_start_ms(&self, _at_ms: i64) -> Result<i64, ControlReadError> {
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
    /// Explicit protected Check admission only; never called during discovery.
    fn check_native_authentication(&self, _agent_id: &str) -> Result<(), ControlReadError> {
        Err(ControlReadError::Unavailable)
    }
    fn check_collaboration(&self, _agent_id: &str) -> Result<(), ControlReadError> {
        Err(ControlReadError::Unavailable)
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
    ) -> Result<hiroute_application_api::AgentModelSettingsStatusV2, ControlReadError> {
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
    pub facts: crate::compiler::AgentPlanCompilationFactsV1,
    pub expected_revisions: RevisionSetV1,
    /// Exact active aggregate used only by Apply to preserve immutable aliases, existing Plans,
    /// and grants. Preview remains stateless over the captured facts and revisions above.
    pub active_publication: Option<hiroute_domain::GatewayPublicationV1>,
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

    fn connection_options(&self) -> Result<ComputeConnectionOptionsResultV1, ControlReadError>;

    fn prepare_compute_projection(
        &self,
        change: &ComputeConnectionChangeV1,
    ) -> Result<PreparedComputeProjectionV1, ComputeProjectionReadError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComputeManagementControlError {
    Invalid,
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
    fn compute_subscriptions(
        &self,
    ) -> Result<ComputeSubscriptionCandidatesV2, ComputeManagementControlError>;

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

    fn preview_compute_save(
        &self,
        change: hiroute_application_api::ComputeManagementChangeV2,
    ) -> Result<ComputeSavePreviewV2, ComputeManagementControlError>;

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
}

#[derive(Clone)]
pub struct ApplicationPorts {
    /// True only after the daemon has composed the Gateway and the combined role is ready.
    pub role_all_ready: bool,
    pub model_catalog: Option<Arc<dyn crate::model_catalog::ModelCatalogPort>>,
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
