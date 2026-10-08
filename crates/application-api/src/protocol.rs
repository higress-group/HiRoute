use hiroute_domain::{
    AgentPlanId, CHANGE_SPEC_SCHEMA_V1, CanonicalDigest, ChangeSpecV1, ContentMode,
    ModelSwitchFilter, RevisionSetV1, SchemaVersion, SessionId, SessionSummaryV1, ValueGroupByV1,
    ValueViewV1,
};
use serde::{Deserialize, Serialize};

use crate::commands::CoverageState;

pub const LOCAL_CONTROL_SCHEMA_V2: SchemaVersion = SchemaVersion::new(2, 0);
pub const MACHINE_ENVELOPE_SCHEMA_V2: SchemaVersion = SchemaVersion::new(2, 0);
/// Product release used by the same-release Local Control handshake, independent
/// of the package versions of client transport and implementation libraries.
pub const LOCAL_CONTROL_RELEASE_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Maximum newline-delimited JSON frame size, including the trailing newline.
pub const LOCAL_CONTROL_MAX_FRAME_BYTES: usize = 10 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    InteractiveUser,
    Desktop,
    Skill,
    SealedCollaboration,
}

impl PrincipalKind {
    pub fn is_collaboration(self) -> bool {
        matches!(self, Self::Skill | Self::SealedCollaboration)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrincipalV1 {
    pub kind: PrincipalKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_connection_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

impl PrincipalV1 {
    pub fn interactive_user() -> Self {
        Self {
            kind: PrincipalKind::InteractiveUser,
            agent_connection_id: None,
            capabilities: Vec::new(),
        }
    }

    /// Authority established solely by an owner-authenticated local transport.
    ///
    /// A same-UID peer is intentionally represented by the least-privileged principal. Client
    /// names are diagnostics, not identity assertions. Interactive/Desktop authority is carried
    /// only by a protected, operation-bound grant delivered on an inherited descriptor.
    pub fn ambient_local_peer() -> Self {
        Self {
            kind: PrincipalKind::Skill,
            agent_connection_id: None,
            capabilities: vec!["same-os-user:query-preview".to_owned()],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientHelloV1 {
    pub api_version: SchemaVersion,
    pub machine_schema_version: SchemaVersion,
    pub client_name: String,
    pub client_version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerHelloV1 {
    pub api_version: SchemaVersion,
    pub machine_schema_version: SchemaVersion,
    pub daemon_version: String,
    pub release_version: String,
    pub capabilities: Vec<String>,
}

pub fn negotiate_hello(client: &ClientHelloV1) -> Result<ServerHelloV1, ErrorV1> {
    if client.api_version != LOCAL_CONTROL_SCHEMA_V2 {
        return Err(ErrorV1::new(ErrorCode::SchemaIncompatible));
    }
    if client.machine_schema_version != MACHINE_ENVELOPE_SCHEMA_V2 {
        return Err(ErrorV1::new(ErrorCode::MachineSchemaIncompatible));
    }
    if client.client_name.is_empty()
        || client.client_name.len() > 128
        || client.client_version != LOCAL_CONTROL_RELEASE_VERSION
    {
        return Err(ErrorV1::new(ErrorCode::InvalidArguments));
    }
    Ok(ServerHelloV1 {
        api_version: LOCAL_CONTROL_SCHEMA_V2,
        machine_schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
        daemon_version: LOCAL_CONTROL_RELEASE_VERSION.to_owned(),
        release_version: LOCAL_CONTROL_RELEASE_VERSION.to_owned(),
        capabilities: vec![
            "local-control-v2".to_owned(),
            "client-access-v1".to_owned(),
            "protected-client-grant-v2".to_owned(),
            "planned-command-registry-v1".to_owned(),
        ],
    })
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedClientGrantV2 {
    pub principal_kind: PrincipalKind,
    pub capability: String,
}

/// Hardened Local Control v2 request. The optional grant is supplied only by a protected launcher
/// descriptor. Claiming a principal kind without a matching scoped token grants no authority.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalControlWireRequestV2 {
    pub schema_version: SchemaVersion,
    pub request_id: String,
    pub operation_id: String,
    pub payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protected_grant: Option<ProtectedClientGrantV2>,
}

/// Transport-authenticated request consumed by the one Application service.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalControlRequestV2 {
    pub schema_version: SchemaVersion,
    pub request_id: String,
    pub principal: PrincipalV1,
    pub operation_id: String,
    pub payload: serde_json::Value,
    pub protected_grant: Option<ProtectedClientGrantV2>,
}

impl LocalControlWireRequestV2 {
    pub fn authenticate_ambient(self) -> LocalControlRequestV2 {
        LocalControlRequestV2 {
            schema_version: self.schema_version,
            request_id: self.request_id,
            principal: PrincipalV1::ambient_local_peer(),
            operation_id: self.operation_id,
            payload: self.payload,
            protected_grant: self.protected_grant,
        }
    }
}

pub const SETUP_REQUEST_SCHEMA_V1: &str = "hiroute.setup-request/v1";

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingModeV1 {
    #[default]
    SmartSaving,
    FreeFirst,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FreePoolModeV1 {
    #[default]
    AutomaticAllAvailable,
    Manual,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackPolicyV1 {
    FreeOnly,
    #[default]
    PrimaryFallback,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupSelectionV1 {
    #[default]
    Automatic,
    Enabled,
    Disabled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupRequestV1 {
    pub schema: String,
    #[serde(default)]
    pub agent_ids: Vec<String>,
    #[serde(default)]
    pub routing_mode: RoutingModeV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_purpose: Option<String>,
    #[serde(default)]
    pub free_pool_mode: FreePoolModeV1,
    #[serde(default)]
    pub fallback_policy: FallbackPolicyV1,
    #[serde(default)]
    pub native_subagent_routing: SetupSelectionV1,
    #[serde(default)]
    pub codex_catalog: SetupSelectionV1,
}

impl Default for SetupRequestV1 {
    fn default() -> Self {
        Self {
            schema: SETUP_REQUEST_SCHEMA_V1.to_owned(),
            agent_ids: Vec::new(),
            routing_mode: RoutingModeV1::default(),
            routing_purpose: None,
            free_pool_mode: FreePoolModeV1::default(),
            fallback_policy: FallbackPolicyV1::default(),
            native_subagent_routing: SetupSelectionV1::default(),
            codex_catalog: SetupSelectionV1::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupApplyRequestV1 {
    pub spec: SetupRequestV1,
    pub accept_digest: CanonicalDigest,
    pub expected_revision: u64,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCheckRequestV1 {
    pub agent_id: String,
    pub scope: AgentCheckScopeV1,
    pub suite: AgentCheckSuiteV1,
    #[serde(default)]
    pub allow_model_call: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<AgentModelCheckTargetV2>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentModelCheckTargetV2 {
    pub context_id: String,
    pub surface: crate::AgentModelSurfaceV2,
    pub expected_applied_revision: hiroute_domain::GatewayPublicationRevision,
    pub client_model_ids: Vec<String>,
}

impl AgentCheckRequestV1 {
    pub fn valid_target(&self) -> bool {
        match (self.scope, &self.target) {
            (AgentCheckScopeV1::Live, Some(target)) => {
                !target.context_id.is_empty()
                    && target.expected_applied_revision.get() > 0
                    && !target.client_model_ids.is_empty()
                    && target
                        .client_model_ids
                        .iter()
                        .all(|name| hiroute_domain::valid_client_model_name(name))
                    && target
                        .client_model_ids
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        == target.client_model_ids.len()
            }
            (AgentCheckScopeV1::Live, None) => false,
            (_, None) => !self.allow_model_call,
            (_, Some(_)) => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCheckScopeV1 {
    Configuration,
    /// Explicit local authentication challenge; no upstream model call.
    NativeAuthentication,
    /// Explicit isolated Skill loading and read-only trusted CLI challenge.
    Collaboration,
    Live,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCheckSuiteV1 {
    Quick,
    Tool,
    Conformance,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionLookupV1 {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<SessionContentModeV1>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum SessionContentModeV1 {
    #[default]
    #[serde(rename = "none")]
    None,
    #[serde(rename = "messages")]
    Messages,
    #[serde(rename = "messages-and-tools")]
    MessagesAndTools,
}

impl From<SessionContentModeV1> for ContentMode {
    fn from(value: SessionContentModeV1) -> Self {
        match value {
            SessionContentModeV1::None => Self::None,
            SessionContentModeV1::Messages => Self::Messages,
            SessionContentModeV1::MessagesAndTools => Self::MessagesAndTools,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionListRequestV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default)]
    pub model_switch: ModelSwitchFilter,
    #[serde(default)]
    pub include_unlinked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionListPageV1 {
    pub sessions: Vec<SessionSummaryV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValuePeriodV1 {
    Today,
    #[default]
    SevenDays,
    ThirtyDays,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValueRequestV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_plan_id: Option<AgentPlanId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<ValuePeriodV1>,
    #[serde(default = "default_value_group_by")]
    pub group_by: ValueGroupByV1,
    #[serde(default = "default_currency")]
    pub currency: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
}

fn default_value_group_by() -> ValueGroupByV1 {
    ValueGroupByV1::None
}

fn default_currency() -> String {
    "USD".to_owned()
}

impl Default for ValueRequestV1 {
    fn default() -> Self {
        Self {
            agent_plan_id: None,
            from_ms: None,
            to_ms: None,
            period: None,
            group_by: default_value_group_by(),
            currency: default_currency(),
            session_id: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValueScopeViewV1 {
    pub from_ms: i64,
    pub to_ms: i64,
    pub currency: String,
    pub group_by: ValueGroupByV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_plan_id: Option<AgentPlanId>,
    pub plans: Vec<ValueViewV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationLookupV1 {
    pub operation_id: String,
    #[serde(default)]
    pub after_sequence: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationCancelRequestV1 {
    pub operation_id: String,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SystemStatusV1 {
    pub schema: String,
    pub daemon: String,
    pub local_control: String,
    pub control_store: String,
    pub observation_store: String,
    pub discovery: String,
    pub gateway: String,
    pub setup_revision: u64,
    pub recoverable_operations: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupPreviewV1 {
    pub schema: String,
    pub applicable: bool,
    pub change_digest: CanonicalDigest,
    pub expected_revision: u64,
    pub normalized_spec: SetupRequestV1,
    pub discovered_agents: Vec<String>,
    pub blockers: Vec<String>,
    pub effects: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PreviewRequestV1 {
    pub schema_version: SchemaVersion,
    pub spec: ChangeSpecV1,
}

impl PreviewRequestV1 {
    pub fn new(spec: ChangeSpecV1) -> Self {
        Self {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            spec,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PreviewResultV1 {
    pub normalized_spec: ChangeSpecV1,
    pub change_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
    #[serde(default)]
    pub effects: Vec<EffectPreviewV1>,
    #[serde(default)]
    pub blockers: Vec<BlockerV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EffectPreviewV1 {
    pub effect_id: String,
    pub channel: String,
    pub action: String,
    pub target: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BlockerV1 {
    pub code: String,
    pub details_schema: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ApplyRequestV1 {
    pub schema_version: SchemaVersion,
    /// Apply always resubmits the complete structured spec; a preview handle is forbidden.
    pub spec: ChangeSpecV1,
    pub accept_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
    pub idempotency_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_capability: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApplyResultV1 {
    pub operation_id: String,
    pub accepted_digest: CanonicalDigest,
    pub state: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineStatus {
    Succeeded,
    Accepted,
    UsageError,
    Conflict,
    Denied,
    NotFound,
    Unavailable,
    ActionRequired,
    NeedsAttention,
    InternalError,
}

impl MachineStatus {
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Succeeded | Self::Accepted => 0,
            Self::InternalError => 1,
            Self::UsageError => 2,
            Self::Conflict => 3,
            Self::Denied => 4,
            Self::NotFound => 5,
            Self::Unavailable => 6,
            Self::ActionRequired => 7,
            Self::NeedsAttention => 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    UnknownCommand,
    InvalidArguments,
    SchemaIncompatible,
    MachineSchemaIncompatible,
    FeatureNotEnabled,
    NotImplemented,
    RevisionConflict,
    ControlWaitCapacityExceeded,
    WorkerCapacityExceeded,
    ChangePreviewStale,
    IdempotencyKeyReused,
    AgentAuthPrecedenceConflict,
    QoderModelBudgetConflict,
    CapabilityDenied,
    CapabilityUnavailable,
    ResourceNotFound,
    SnapshotUnavailable,
    DaemonUnavailable,
    GatewayUnavailable,
    ObservationUnavailable,
    NoSupportedAgent,
    ActionRequired,
    OperationNeedsAttention,
    Internal,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Usage,
    Conflict,
    Authorization,
    NotFound,
    Unavailable,
    ActionRequired,
    Recovery,
    Internal,
}

impl ErrorCode {
    pub const fn status(self) -> MachineStatus {
        match self {
            Self::UnknownCommand
            | Self::InvalidArguments
            | Self::SchemaIncompatible
            | Self::MachineSchemaIncompatible => MachineStatus::UsageError,
            Self::FeatureNotEnabled
            | Self::NotImplemented
            | Self::RevisionConflict
            | Self::ChangePreviewStale
            | Self::IdempotencyKeyReused => MachineStatus::Conflict,
            Self::NoSupportedAgent
            | Self::AgentAuthPrecedenceConflict
            | Self::QoderModelBudgetConflict => MachineStatus::Conflict,
            Self::CapabilityDenied => MachineStatus::Denied,
            Self::ResourceNotFound => MachineStatus::NotFound,
            Self::CapabilityUnavailable
            | Self::DaemonUnavailable
            | Self::GatewayUnavailable
            | Self::ObservationUnavailable
            | Self::SnapshotUnavailable
            | Self::ControlWaitCapacityExceeded
            | Self::WorkerCapacityExceeded => MachineStatus::Unavailable,
            Self::ActionRequired => MachineStatus::ActionRequired,
            Self::OperationNeedsAttention => MachineStatus::NeedsAttention,
            Self::Internal => MachineStatus::InternalError,
        }
    }

    pub const fn message_key(self) -> &'static str {
        match self {
            Self::UnknownCommand => "cli.error.unknown_command",
            Self::InvalidArguments => "cli.error.invalid_arguments",
            Self::SchemaIncompatible => "control.error.schema_incompatible",
            Self::MachineSchemaIncompatible => "control.error.machine_schema_incompatible",
            Self::FeatureNotEnabled => "application.error.feature_not_enabled",
            Self::NotImplemented => "application.error.not_implemented",
            Self::RevisionConflict => "application.error.revision_conflict",
            Self::ControlWaitCapacityExceeded => "control.error.wait_capacity_exceeded",
            Self::WorkerCapacityExceeded => "worker.error.capacity_exceeded",
            Self::ChangePreviewStale => "application.error.change_preview_stale",
            Self::IdempotencyKeyReused => "application.error.idempotency_key_reused",
            Self::AgentAuthPrecedenceConflict => "agent.error.auth_precedence_conflict",
            Self::QoderModelBudgetConflict => "agent.error.qoder_model_budget_conflict",
            Self::CapabilityUnavailable => "control.error.capability_unavailable",
            Self::CapabilityDenied => "control.error.capability_denied",
            Self::ResourceNotFound => "application.error.resource_not_found",
            Self::SnapshotUnavailable => "catalog.error.snapshot_unavailable",
            Self::DaemonUnavailable => "control.error.daemon_unavailable",
            Self::GatewayUnavailable => "application.error.gateway_unavailable",
            Self::ObservationUnavailable => "application.error.observation_unavailable",
            Self::NoSupportedAgent => "application.error.no_supported_agent",
            Self::ActionRequired => "application.error.action_required",
            Self::OperationNeedsAttention => "application.error.operation_needs_attention",
            Self::Internal => "internal.error.unclassified",
        }
    }

    pub const fn category(self) -> ErrorCategory {
        match self.status() {
            MachineStatus::UsageError => ErrorCategory::Usage,
            MachineStatus::Conflict => ErrorCategory::Conflict,
            MachineStatus::Denied => ErrorCategory::Authorization,
            MachineStatus::NotFound => ErrorCategory::NotFound,
            MachineStatus::Unavailable => ErrorCategory::Unavailable,
            MachineStatus::ActionRequired => ErrorCategory::ActionRequired,
            MachineStatus::NeedsAttention => ErrorCategory::Recovery,
            MachineStatus::Succeeded | MachineStatus::Accepted | MachineStatus::InternalError => {
                ErrorCategory::Internal
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ErrorV1 {
    pub code: ErrorCode,
    pub category: ErrorCategory,
    pub message_key: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    pub details_schema: String,
}

impl ErrorV1 {
    pub fn new(code: ErrorCode) -> Self {
        let retryable = matches!(
            code,
            ErrorCode::DaemonUnavailable | ErrorCode::ControlWaitCapacityExceeded
        );
        Self {
            code,
            category: code.category(),
            message_key: code.message_key().to_owned(),
            retryable,
            operation_id: None,
            details_schema: "hiroute.error-details/none-v1".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WarningV1 {
    pub code: String,
    pub details_schema: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NextActionV1 {
    pub command_id: String,
    pub input: serde_json::Value,
    pub reason_code: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OperationReferenceV1 {
    pub operation_id: String,
    pub state: String,
    pub sequence: u64,
    pub cancellable: bool,
}

/// Enriched machine envelope negotiated only with machine-schema v2 clients.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MachineEnvelopeV2<T> {
    pub schema_version: SchemaVersion,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub status: MachineStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<OperationReferenceV1>,
    #[serde(default)]
    pub warnings: Vec<WarningV1>,
    #[serde(default)]
    pub next_actions: Vec<NextActionV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorV1>,
}

impl<T> MachineEnvelopeV2<T> {
    pub fn succeeded(data: T, request_id: Option<String>) -> Self {
        Self {
            schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
            request_id,
            status: MachineStatus::Succeeded,
            data: Some(data),
            operation: None,
            warnings: Vec::new(),
            next_actions: Vec::new(),
            error: None,
        }
    }

    pub fn accepted(data: T, request_id: Option<String>) -> Self {
        Self {
            status: MachineStatus::Accepted,
            ..Self::succeeded(data, request_id)
        }
    }

    /// Project only typed data while preserving the authoritative status, operation, warnings,
    /// actions, and error metadata from the producer.
    pub fn map_data<U>(self, project: impl FnOnce(T) -> U) -> MachineEnvelopeV2<U> {
        MachineEnvelopeV2 {
            schema_version: self.schema_version,
            request_id: self.request_id,
            status: self.status,
            data: self.data.map(project),
            operation: self.operation,
            warnings: self.warnings,
            next_actions: self.next_actions,
            error: self.error,
        }
    }
}

impl MachineEnvelopeV2<serde_json::Value> {
    pub fn failed(error: ErrorV1, request_id: Option<String>) -> Self {
        Self {
            schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
            request_id,
            status: error.code.status(),
            data: None,
            operation: None,
            warnings: Vec::new(),
            next_actions: Vec::new(),
            error: Some(error),
        }
    }

    pub fn failed_with_data(
        error: ErrorV1,
        data: serde_json::Value,
        request_id: Option<String>,
    ) -> Self {
        Self {
            data: Some(data),
            ..Self::failed(error, request_id)
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MachineErrorV1 {
    pub code: ErrorCode,
    pub message_key: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    pub details_schema: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MachineNextActionV1 {
    pub command_id: String,
    pub input: serde_json::Value,
}

pub(crate) fn command_request_schema(command_id: &str) -> &'static str {
    match command_id {
        "worker.plans" => "hiroute.worker-plans-request/v1",
        "worker.dependencies.discover" => "hiroute.worker-dependencies-discover-request/v1",
        "worker.dependencies.select" => "hiroute.worker-dependencies-select-request/v1",
        "worker.list" => "hiroute.worker-list-request/v1",
        "worker.exec" => "hiroute.worker-exec-request/v1",
        "worker.status" => "hiroute.worker-status-request/v1",
        "worker.wait" => "hiroute.worker-wait-request/v1",
        "worker.result" => "hiroute.worker-result-request/v1",
        "worker.read" => "hiroute.worker-read-request/v1",
        "worker.cancel" => "hiroute.worker-cancel-request/v1",
        "worker.continue" => "hiroute.worker-continue-request/v1",
        "tasks.list" => "hiroute.delegation-list-request/v1",
        "tasks.show" => "hiroute.delegation-get-request/v1",
        "tasks.start" => "hiroute.delegation-start-request/v1",
        "tasks.wait" => "hiroute.delegation-wait-request/v1",
        "tasks.result" => "hiroute.delegation-result-request/v1",
        "tasks.cancel" => "hiroute.delegation-cancel-request/v1",
        "tasks.continue" => "hiroute.delegation-continue-request/v1",
        "routing.options" => "hiroute.plan-editor-options-request/v1",
        "routing.list" => "hiroute.agent-plan-catalog-query/v2",
        "routing.preview" => "hiroute.agent-plan-preview-request/union",
        "routing.apply" => "hiroute.agent-plan-apply-request/union",
        "decision.services.apply" => "hiroute.change-preview-or-apply-request/v1",
        "decision.services.test" => "hiroute.classifier-decision-test/v1",
        "work-plans.list" => "hiroute.work-plan-list-request/v1",
        "routing.show" => "hiroute.agent-plan-lookup/v1",
        "operations.find" => "hiroute.operation-idempotency-lookup/v1",
        "agent.launch" => "hiroute.agent-launch-descriptor-request/v1",
        "setup.preview" => "hiroute.setup-request/v1",
        "setup.apply" => "hiroute.setup-apply-request/v1",
        "setup.status" | "operations.get" | "operations.watch" => "hiroute.operation-lookup/v1",
        "operations.cancel" => "hiroute.operation-cancel-request/v1",
        "agents.connect.preview" | "agents.restore.preview" => {
            "hiroute.agent-connection-preview-request/union"
        }
        "agents.connect.apply" | "agents.restore.apply" => {
            "hiroute.agent-connection-apply-request/union"
        }
        "agents.connect.status" => "hiroute.agent-connection-status-request/union",
        "agents.check" => "hiroute.agent-check-request/v1",
        "compute.list" | "compute.show" => "hiroute.compute-management-query/v2",
        "compute.connection.preview" => "hiroute.compute-connection-preview-request/union",
        "compute.connection.apply" => "hiroute.compute-connection-apply-request/v1",
        "compute.connection.authorize" => "hiroute.compute-connection-authorization-request/v1",
        "compute.connection.test" => "hiroute.compute-connection-test-request/v1",
        "models.show" => "hiroute.model-catalog-query/v1",
        "sessions.list" => "hiroute.session-list-query/v1",
        "sessions.show" | "sessions.receipt" => "hiroute.session-lookup/v1",
        "value.show" => "hiroute.value-query/v1",
        "observation.plan-quality.samples" => "hiroute.observation.query/v2",
        _ => "hiroute.empty-request/v1",
    }
}

pub(crate) fn command_response_schema(command_id: &str) -> &'static str {
    match command_id {
        "worker.executors" => "hiroute.worker-executor-availability-list/v1",
        "worker.plans" => "hiroute.work-plan-list/v1",
        "worker.dependencies.discover" | "worker.dependencies.select" => {
            "hiroute.worker-dependencies-view/v1"
        }
        "worker.list" => "hiroute.delegation-list/v1",
        "worker.exec" | "worker.status" | "worker.wait" | "worker.result" | "worker.continue"
        | "worker.cancel" => "hiroute.worker-command-data/v1",
        "worker.read" => "hiroute.worker-read-data/v1",
        "tasks.list" => "hiroute.delegation-list/v1",
        "tasks.show" => "hiroute.delegation-get/v1",
        "tasks.start" => "hiroute.delegation-accepted/v1",
        "tasks.wait" => "hiroute.delegation-wait/v1",
        "tasks.result" => "hiroute.delegation-result/v1",
        "tasks.cancel" => "hiroute.delegation-cancel/v1",
        "tasks.continue" => "hiroute.delegation-accepted/v1",
        "work-plans.list" => "hiroute.work-plan-list/v1",
        "system.client-status" => "hiroute.client-service-status/v1",
        "routing.options" => "hiroute.plan-editor-options/v1",
        "routing.list" => "hiroute.agent-plan-catalog/v2",
        "routing.show" => "hiroute.agent-plan-status/v2",
        "routing.preview" => "hiroute.agent-plan-preview/union",
        "routing.apply" => "hiroute.operation-view/v1",
        "decision.services.list" => "hiroute.decision-services/v1",
        "decision.services.apply" => "hiroute.change-preview-or-operation/v1",
        "decision.services.test" => "hiroute.classifier-decision-test-result/v1",
        "operations.find" => "hiroute.operation-idempotency-result/v1",
        "agent.launch" => "hiroute.managed-claude-launch-descriptor/v2",
        "system.status" => "hiroute.system-status/v1",
        "setup.preview" => "hiroute.change-preview/v1",
        "setup.apply" => "hiroute.setup-apply-result/v1",
        "setup.status" => "hiroute.setup-status/v1",
        "operations.get" | "operations.cancel" => "hiroute.operation-view/v1",
        "operations.watch" => "hiroute.operation-event/v1",
        "agents.scan" | "agents.list" => "hiroute.agent-discovery-list/v1",
        "agents.connect.preview" | "agents.restore.preview" => {
            "hiroute.agent-connection-preview/union"
        }
        "agents.connect.apply" | "agents.restore.apply" => "hiroute.operation-view/v1",
        "agents.connect.status" => "hiroute.agent-connection-status/union",
        "agents.check" => "hiroute.agent-check/v1",
        "compute.scan" => "hiroute.compute-scan-result/v1",
        "compute.list" | "compute.show" => "hiroute.compute-management-snapshot/v2",
        "compute.connection.options" => "hiroute.compute-connection-options/v1",
        "compute.connection.preview" => "hiroute.compute-connection-preview/union",
        "compute.connection.apply" => "hiroute.operation-view/v1",
        "compute.connection.authorize" => "hiroute.compute-subscription-check-result/v2",
        "compute.connection.test" => "hiroute.model-connection-check/union",
        "models.show" => "hiroute.model-catalog-view/v1",
        "sessions.list" => "hiroute.session-list/v1",
        "sessions.show" => "hiroute.session-detail/v1",
        "sessions.receipt" => "hiroute.routing-receipt/v1",
        "sessions.status" => "hiroute.session-store-status/v1",
        "value.show" => "hiroute.value-view/v1",
        "observation.plan-quality.samples" => "hiroute.plan-quality-samples-page/v1",
        _ => "hiroute.command-result/v1",
    }
}

pub(crate) fn command_model_call(command_id: &str) -> &'static str {
    if matches!(
        command_id,
        "tasks.start" | "tasks.continue" | "worker.exec" | "worker.continue"
    ) {
        "caller_selected_worker_via_managed_loopback_gateway"
    } else if command_id == "agent.launch" {
        "caller_controlled_via_managed_loopback_gateway"
    } else if command_id == "agents.check" {
        "explicit_live_scope_only"
    } else if matches!(
        command_id,
        "compute.connection.test" | "decision.services.test"
    ) {
        "explicit_probe_may_call_model"
    } else {
        "never"
    }
}

pub(crate) fn command_stdin_channels(command_id: &str) -> &'static [&'static str] {
    match command_id {
        "worker.exec" | "worker.continue" => &["prompt_stdin", "file"],
        "worker.dependencies.select" => &["request_stdin"],
        value if value.starts_with("worker.") => &[],
        "work-plans.list" => &["request_stdin", "capability_fd"],
        value if value.starts_with("tasks.") => &["request_stdin", "capability_fd"],
        "setup.preview" => &["spec_stdin"],
        "setup.apply" => &["spec_fd", "capability_fd"],
        "models.show"
        | "compute.connection.preview"
        | "compute.connection.apply"
        | "compute.connection.authorize"
        | "compute.connection.test"
        | "decision.services.test"
        | "routing.options"
        | "routing.preview"
        | "routing.apply"
        | "decision.services.apply"
        | "agents.connect.preview"
        | "agents.connect.apply"
        | "agents.restore.preview"
        | "agents.restore.apply" => &["request_stdin"],
        "sessions.list" | "sessions.show" | "value.show" | "observation.plan-quality.samples" => {
            &["request_stdin", "capability_fd"]
        }
        "agents.check" | "operations.cancel" => &["capability_fd"],
        _ => &[],
    }
}

pub(crate) fn command_preview_apply_pair(command_id: &str) -> Option<&'static str> {
    match command_id {
        "setup.preview" => Some("setup.apply"),
        "setup.apply" => Some("setup.preview"),
        "compute.connection.preview" => Some("compute.connection.apply"),
        "compute.connection.apply" => Some("compute.connection.preview"),
        "routing.preview" => Some("routing.apply"),
        "routing.apply" => Some("routing.preview"),
        "agents.connect.preview" => Some("agents.connect.apply"),
        "agents.connect.apply" => Some("agents.connect.preview"),
        "agents.restore.preview" => Some("agents.restore.apply"),
        "agents.restore.apply" => Some("agents.restore.preview"),
        _ => None,
    }
}

pub(crate) fn command_idempotency(command_id: &str) -> &'static str {
    match command_id {
        "setup.apply"
        | "operations.cancel"
        | "compute.connection.apply"
        | "routing.apply"
        | "decision.services.apply"
        | "agents.connect.apply"
        | "agents.restore.apply"
        | "tasks.start"
        | "tasks.cancel"
        | "tasks.continue"
        | "worker.exec"
        | "worker.continue"
        | "worker.cancel" => "required",
        "worker.dependencies.select" => "deterministic_request_replay",
        _ => "not_applicable",
    }
}

pub(crate) fn staged_coverage(command_id: &str, positive: bool) -> Option<CoverageState> {
    match command_id {
        "setup.preview" | "setup.apply" | "setup.status" | "operations.watch"
        | "operations.cancel" => Some(if positive {
            CoverageState::Planned
        } else {
            CoverageState::Executable
        }),
        _ => None,
    }
}

pub(crate) fn command_usage(command_id: &str, joined_path: &str) -> String {
    match command_id {
        "worker.executors" => "hiroute worker executors [--output <json|text|quiet>]".into(),
        "worker.dependencies.discover" => "hiroute worker dependencies discover [--harness <codex_cli|claude_code>] [--output <json|text|quiet>]".into(),
        "worker.dependencies.select" => "hiroute worker dependencies select --request-stdin [--output <json|text|quiet>]".into(),
        "worker.plans" => "hiroute worker plans [--output <json|text|quiet>]".into(),
        "worker.list" => "hiroute worker list [--title <TITLE>] [--cursor <OPAQUE>] [--limit <1..200>] [--output <json|text|quiet>]".into(),
        "worker.exec" => "hiroute worker exec --plan <ID> --cwd <PATH> [--title <TITLE>] [--permission-policy <approve-all|approve-reads|deny-all>] [--run-timeout <1..86400>] [--no-wait | --wait-timeout <1..30>] [--submission-key <KEY>] (-- <TEXT> | --file <PATH> | < stdin)".into(),
        "worker.status" => "hiroute worker status (--run <ID> | --task <ID> [--run <ID>] | --submission <KEY> --operation <start|continue>)".into(),
        "worker.wait" => "hiroute worker wait --run <ID> [--after-revision <N>] [--wait-timeout <1..30>]".into(),
        "worker.result" => "hiroute worker result --run <ID> [--offset <N>] [--max-bytes <N>]".into(),
        "worker.read" => "hiroute worker read --run <ID> [--cursor <OPAQUE>] [--max-bytes <1..32768>]".into(),
        "worker.cancel" => "hiroute worker cancel --run <ID> [--reason <REFERENCE>] [--idempotency-key <KEY>]".into(),
        "worker.continue" => "hiroute worker continue --task <ID> --expected-latest-run <ID> [--cwd <PATH>] [--permission-policy <approve-all|approve-reads|deny-all>] [--run-timeout <1..86400>] [--no-wait | --wait-timeout <1..30>] [--submission-key <KEY>] (-- <TEXT> | --file <PATH> | < stdin)".into(),
        "tasks.list" => "hiroute tasks list --request-stdin (--agent <codex|claude-code> | --capability-fd <FD>) --json".into(),
        "tasks.show" => "hiroute tasks show (--task <ID> [--run <ID>] | --submission-key <KEY> --operation <start|continue>) --request-stdin (--agent <codex|claude-code> | --capability-fd <FD>) --json".into(),
        "tasks.start" => "hiroute tasks start --request-stdin (--agent <codex|claude-code> | --capability-fd <FD>) --json".into(),
        "tasks.wait" => "hiroute tasks wait --run <ID> --request-stdin (--agent <codex|claude-code> | --capability-fd <FD>) --json".into(),
        "tasks.result" => "hiroute tasks result --run <ID> --request-stdin (--agent <codex|claude-code> | --capability-fd <FD>) --json".into(),
        "tasks.cancel" => "hiroute tasks cancel --run <ID> --request-stdin (--agent <codex|claude-code> | --capability-fd <FD>) --json".into(),
        "tasks.continue" => "hiroute tasks continue --request-stdin (--agent <codex|claude-code> | --capability-fd <FD>) --json".into(),
        "work-plans.list" => "hiroute work-plans list --request-stdin (--agent <codex|claude-code> | --capability-fd <FD>) --json".to_owned(),
        "agent.launch" => "hiroute agent launch --agent claude-code --context <CONTEXT_ID> -- <CLAUDE_SESSION_ARGS...>".to_owned(),
        "schema.show" => "hiroute schema show --command-id <stable-command-id> --non-interactive --output json".to_owned(),
        "operations.find" => "hiroute operations find --request-stdin --output json".to_owned(),
        "models.show" => "hiroute models show --request-stdin --output json".to_owned(),
        "compute.list" => "hiroute compute list --output json".to_owned(),
        "compute.show" => "hiroute compute show <SOURCE_ID> --output json".to_owned(),
        "compute.connection.preview" | "compute.connection.apply" | "compute.connection.authorize" | "compute.connection.test" => {
            format!("hiroute {joined_path} --request-stdin --output json")
        }
        "routing.options" | "routing.preview" | "routing.apply" | "decision.services.apply" | "decision.services.test" => {
            format!("hiroute {joined_path} --request-stdin --output json")
        }
        "routing.list" => "hiroute routing list [--request-stdin] --output json".to_owned(),
        "decision.services.list" => "hiroute decision services list --output json".to_owned(),
        "routing.show" => "hiroute routing show <PLAN_ID> --output json".to_owned(),
        "agents.connect.preview" | "agents.connect.apply" | "agents.restore.preview" | "agents.restore.apply" => {
            format!("hiroute {joined_path} --request-stdin --output json")
        }
        "agents.connect.status" => "hiroute agents connect status <CONNECTION_OR_CONTEXT_ID> --output json".to_owned(),
        "sessions.list" | "value.show" | "observation.plan-quality.samples" => {
            format!("hiroute {joined_path} ([OPTIONS] | --request-stdin) [--capability-fd <FD>] --non-interactive --output json")
        }
        "sessions.show" => "hiroute sessions show (<SESSION_ID> [--content <none|messages|messages-and-tools>] | --request-stdin) [--capability-fd <FD>] --non-interactive --output json".to_owned(),
        "system.status" | "agents.scan" | "agents.list" | "sessions.status" => {
            format!("hiroute {joined_path} [OPTIONS] --non-interactive --output json")
        }
        "setup.preview" => "hiroute setup preview [HUMAN OPTIONS] | --spec-stdin".to_owned(),
        "setup.apply" => "hiroute setup apply --spec-fd <FD> --accept-digest <SHA256> --expected-revision <REVISION> --idempotency-key <KEY>".to_owned(),
        "setup.status" => "hiroute setup status <OPERATION_ID>".to_owned(),
        "operations.get" | "operations.watch" | "operations.cancel" => {
            format!("hiroute {joined_path} <OPERATION_ID> [OPTIONS]")
        }
        "agents.check" => "hiroute agents check <AGENT_ID> --scope <configuration|native-authentication|collaboration|live> --suite <quick|tool|conformance> [--target <JSON>] [--allow-model-call]; live requires an explicit context/surface/revision/model target and protected consent; local checks use same-UID trust and make no upstream model call".to_owned(),
        "sessions.receipt" => "hiroute sessions receipt <RECEIPT_ID>".to_owned(),
        _ => format!("hiroute {joined_path} [typed options] --non-interactive --output json"),
    }
}

pub(crate) fn command_arguments(command_id: &str) -> String {
    match command_id {
        "worker.executors" => "No Agent, Plan, allowlist, capability FD, or request body is accepted. The query checks configured entry metadata and executability, not program contents or compatibility. Ready allows a launch attempt; it does not prove optional capabilities, install, configure, authorize, or admit a run.".into(),
        "worker.dependencies.discover" => "Optionally restrict discovery to one Harness. The query checks the selected path, PATH, common locations, npm globals and npx cache without installing or selecting anything.".into(),
        "worker.dependencies.select" => "Same-UID local management only. --request-stdin reads one strict JSON object with harness (codex_cli or claude_code), absolute adapter_path and cli_path, optional absolute node_path, and expected_selection_revision copied from the matching selection_revisions entry returned by worker dependencies discover --output json. All paths and the revision are validated again before an atomic selection update; the command never installs software or accepts an Apply capability.".into(),
        "worker.plans" => "Lists every currently published and delegation-enabled Worker Plan in this daemon instance; no main-Agent selector or allowlist participates.".into(),
        "worker.list" => "Pages the daemon instance's durable tasks newest-first. --title is normalized exact matching; the signed opaque cursor is bound to that filter. Default 50, maximum 200, and no total count.".into(),
        "worker.exec" => "Plan, cwd, and exactly one text source are required. Permission policy defaults to approve-all; approve-reads and deny-all are explicit run restrictions and fail unavailable unless the exact Harness mapping proves them. cwd is scheduling context, not directory authorization. A private receipt is saved before transmission; uncertain delivery keeps the same submission key.".into(),
        "worker.status" => "Select exactly a run, a task (and optional historical run), or an original submission key plus start|continue. The daemon instance is the task namespace; no caller, Agent, grant, permit, or workspace-root identity is accepted.".into(),
        "worker.read" => "Reads a bounded best-effort public assistant-message progress window. It is not a complete log or completion proof; cursors are opaque, signed and run-bound.".into(),
        "worker.wait" => "Wait is bounded to 1..30 seconds and never cancels the run. A timeout is a pending observation with a reusable run locator.".into(),
        "worker.result" => "Reads the actual persisted result, optionally by UTF-8 byte page. Query success can honestly report a failed, cancelled, active, incomplete, or unavailable run.".into(),
        "worker.cancel" => "Cancellation is idempotent for one exact run. Omitting the idempotency key creates one; only the owned Worker scope is stopped.".into(),
        "worker.continue" => "Task and expected latest run are mandatory. cwd is optional but, when present, must canonicalize to the persisted task root. Permission policy defaults to approve-all and configures only this run; Continue never falls back to a new task, newest Plan, or session/new.".into(),
        "tasks.list" => "Required identity: --agent or --capability-fd. stdin carries the typed caller plus optional cursor/limit; default 50, maximum 200. Only tasks visible to the current verified collaboration grant are returned.".into(),
        "tasks.show" => "Required identity: --agent or --capability-fd. Select exactly one task (and optional historical run) or the original submission key plus start|continue. A missing key is not permission to retry with a replacement key.".into(),
        "tasks.start" => "Required identity: --agent reads the private sealed installation, or --capability-fd supplies an inherited collaboration pipe; never both. stdin schema: hiroute.delegation-start-request/v1. Start returns accepted promptly. Retry the identical request with the original idempotency key; never replace the key after uncertain delivery. A result query does not cancel, and Continue is a separate operation, not a new Start.".into(),
        "tasks.wait" => "Required identity: --agent or --capability-fd. --run must match typed stdin. after_revision and wait_ms provide a bounded observation (default 20 seconds, maximum 30); timeout or stopping the client never cancels the run.".into(),
        "tasks.result" => "Required identity: --agent or --capability-fd. --run must match the typed run_id in stdin; caller selectors never grant access. stdin schema: hiroute.delegation-result-request/v1 (caller, run_id, optional offset/max_bytes). Reads actual persisted text, at most 1 MiB; next_offset advances UTF-8 byte pagination. An unfinished run remains active; querying or stopping polling does not cancel it.".into(),
        "tasks.cancel" => "Required identity: --agent or --capability-fd. --run and typed run_id must match; idempotency_key is mandatory. The response reports cancelling or the already-known terminal state; only owned-scope stop evidence can report cancelled/cleanup complete.".into(),
        "tasks.continue" => "Required identity: --agent or --capability-fd. stdin names task_id, expected_latest_run_id, a current permit, execution bounds, new instruction, and idempotency_key. Continue uses the retained exact Plan and verified native session with a new run/token; it never falls back to Start, the latest Plan, or session/new.".into(),
        "work-plans.list" => "Required identity: --agent reads the private sealed collaboration installation, or --capability-fd supplies an inherited collaboration pipe; never both. stdin JSON contains workspace_id, context_id, and grant_id as non-secret selectors. The latest allowlist, published Worker metadata, and behavior-verified installation availability are read on every query.".to_owned(),
        "agent.launch" => "--agent claude-code and --context <CONTEXT_ID> are required, followed by the exact -- delimiter. Session arguments after -- are passed as native OsString values. The last --settings object or file supplies the managed overlay, matching native repeated-option precedence (unrelated fields survive; routing, authentication, and provider entries stay managed). --model, --setting-sources, and --fallback-model pass through unchanged. Cloud-provider, authentication, and routing override arguments are rejected before Local Control access or spawn.".to_owned(),
        "schema.list" => "Global --output json and --request-id options only.".to_owned(),
        "schema.show" => "Required: --command-id <stable-command-id>. Global --output json and --request-id options are supported.".to_owned(),
        "setup.apply" => "The setup document is read from --spec-fd. Digest, expected revision, and idempotency key are mandatory. Mutation authority is accepted only from the protected inherited capability channel.".to_owned(),
        "agents.check" => "Configuration, native-authentication and collaboration are bounded local checks admitted by the owner-only same-UID transport and make no upstream model call. Live requires --target JSON containing context_id, surface, expected_applied_revision and client_model_ids, --allow-model-call and a separately delivered one-shot capability bound to that target. Targets are forbidden on local checks; provider, credential, prompt and URL overrides are forbidden.".to_owned(),
        "compute.connection.preview" => "--request-stdin accepts exactly one registered v1 projection, compute-management v2 save, or subscription-check v2 preview request. Preview has no durable side effect and returns the exact digest and revisions required by Apply.".to_owned(),
        "compute.connection.apply" => "--request-stdin accepts only the exact preview spec, digest, revisions, and a non-empty idempotency key. Same-UID Local Control reproduces the plan; replay uses the original key and changed payloads are rejected.".to_owned(),
        "compute.connection.test" => "--request-stdin selects native, registered, discovered, or saved with its strict nested request. Credentials are referenced only through a protected-input candidate; plaintext credential fields are rejected. The explicit probe may contact the configured provider and may consume quota when an inference model is selected.".to_owned(),
        "compute.connection.authorize" => "--request-stdin reads one exact subscription result operation or releases one exact validation. Starting authorization still uses connection Preview/Apply, including revision, digest, and idempotency checks.".to_owned(),
        "decision.services.list" => "No request body or secret material. Returns the latest saved version of each service; published plans retain their exact immutable version.".to_owned(),
        "decision.services.apply" => "--request-stdin accepts a ChangeSpec preview request first, then the exact ApplyRequest with accept_digest, expected_revisions and idempotency_key. spec.desired_state contains id, expected_revision, service (null to delete), and optional protected input_slot. Replacing a credential requires a new reference. A referenced service cannot be deleted.".to_owned(),
        "decision.services.test" => "--request-stdin accepts schema hiroute.classifier-decision-test/v1 and classifier {kind: decision_service, service: <saved version>}. Sends one fixed synthetic decision; no session history is read. Failure is reported in data.outcome and failure_code, even when the command query succeeds.".to_owned(),
        "routing.preview" | "routing.apply" => "--request-stdin accepts only the existing strict draft, content, or lifecycle schema. Apply must reproduce Preview and carry its exact revisions, digest, and idempotency key; publication remains one recoverable Operation.".to_owned(),
        "agents.connect.preview" | "agents.connect.apply" | "agents.restore.preview" | "agents.restore.apply" => "--request-stdin accepts only the strict v1 connection or v2 managed-settings request. Apply is same-UID, revision-checked, idempotent, and writes only owned Agent fields; restore refuses concurrent ownership drift.".to_owned(),
        "sessions.list" => "Human options retain the v1 query. Fact-only listing is available to the same-UID Local Control peer; --query searches retained text and therefore still requires a separately delivered protected capability. --request-stdin accepts only a strict v2 sessions intent.".to_owned(),
        "observation.plan-quality.samples" => "Provide at least --plan-id or --session-id. Use --competence below-floor|meets-floor to compare the latest reliable score with the floor saved for that stage, or --unrated for missing/partial scores. Score bounds remain strict open bounds. Detail filters do not alter full-scope summaries. Facts are same-UID reads; protected evidence content is never returned.".to_owned(),
        "value.show" => "Human options retain the v1 value query. Alternatively, --request-stdin accepts only a strict v2 value or home_value intent. Same-UID reads return recorded known and unknown amounts without computing or filling missing values.".to_owned(),
        "sessions.show" => "Human options retain the v1 lookup and content defaults to none. Same-UID facts/timeline reads need no extra token; messages, tool content, catalog, ancestry, content pages, and search still require an exact protected capability.".to_owned(),
        "sessions.receipt" | "setup.status" | "operations.get" | "operations.watch" | "operations.cancel" => "An exact typed resource ID is required; arbitrary storage keys and SQL are forbidden.".to_owned(),
        _ => "Only the typed options declared by the generated request schema are accepted; arbitrary patches, URLs, shell, or plaintext Secret argv are forbidden.".to_owned(),
    }
}
