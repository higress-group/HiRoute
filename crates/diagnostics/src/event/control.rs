//! Protected native actions and the shared Client Core / Local Control call path.

use serde::{Deserialize, Serialize};

use crate::error::EventErrorCode;
use crate::identity::CorrelationToken;

/// The protected confirmation families owned by the Desktop bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionOperation {
    PlanEditor,
    Rename,
    RestoreName,
    PriceChange,
    AgentSettings,
    AgentCheck,
    ModelSave,
    SubscriptionCheck,
}

/// Result of a protected action: the confirmation-gated mutation and its stable result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionEnd {
    pub operation: ActionOperation,
    pub result: ActionResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_token: Option<CorrelationToken>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionResult {
    Applied,
    Previewed,
    Denied,
    Stale,
    Failed { code: EventErrorCode },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationPhase {
    Shown,
    Result,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmationEvent {
    pub operation: ActionOperation,
    pub phase: ConfirmationPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewPhase {
    Begin,
    End,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewEvent {
    pub operation: ActionOperation,
    pub phase: PreviewPhase,
    /// The real revision the preview was computed against; absent while it is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyPhase {
    Begin,
    End,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyEvent {
    pub operation: ActionOperation,
    pub phase: ApplyPhase,
    /// The real revision this apply was previewed against; absent while it is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ActionResult>,
}

/// A finished Local Control call. `submission` is the Client Core submission state, so a
/// pre-send failure stays distinguishable from an uncertain post-send result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlCallEnd {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_name: Option<ControlClientName>,
    pub operation: ControlOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_token: Option<CorrelationToken>,
    pub submission: SubmissionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<EventErrorCode>,
    pub elapsed_ms: u64,
}

/// Final product read result, after native context and snapshot fences. A successful
/// Local Control response alone does not establish that this read reached the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductReadEnd {
    pub operation: ControlOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<EventErrorCode>,
    pub elapsed_ms: u64,
}

/// A closed attribution label, never an arbitrary client-supplied string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlClientName {
    Console,
}

/// Submission state mirrored from Client Core without reinterpreting its semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionState {
    NotSent,
    Sent,
    Unknown,
}

/// Stages of one Local Control call. They describe transport progress only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlStageKind {
    EndpointValidate,
    PeerVerify,
    Hello,
    RequestWrite,
    ResponseRead,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlStage {
    pub stage: ControlStageKind,
    pub ok: bool,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent: Option<bool>,
}

/// Closed vocabulary of Local Control operations that may appear in diagnostics. An
/// arbitrary operation string never becomes a field value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlOperation {
    DesktopSnapshot,
    ObservationRead,
    ObservationDelete,
    ShowModel,
    GetEffectivePrices,
    GetPlanEditorOptions,
    GetClientServiceStatus,
    ListAgentPlanCatalog,
    GetAgentPlanStatus,
    FindOperationByIdempotency,
    ComputeSnapshot,
    ComputeScan,
    ComputeConnectionOptions,
    CheckModelConnection,
    ComputeSubscriptions,
    CheckSubscription,
    WorkerSettingsGet,
    WorkerSettingsSet,
    WorkerTaskPlans,
    WorkerExecutorAvailability,
    WorkerTaskList,
    WorkerTaskStatus,
    WorkerTaskResult,
    WorkerTaskWait,
    WorkerTaskCancel,
    WorkerTaskContinue,
    Other,
}

impl ControlOperation {
    /// Map a wire operation id to the closed vocabulary. Unknown ids fold into `other`
    /// instead of widening the enum.
    pub fn from_wire(operation: &str) -> Self {
        match operation {
            "DesktopSnapshot" | "GetDesktopSnapshot" => ControlOperation::DesktopSnapshot,
            "ObservationRead" => ControlOperation::ObservationRead,
            "ObservationDelete" => ControlOperation::ObservationDelete,
            "ShowModel" => ControlOperation::ShowModel,
            "GetEffectivePrices" => ControlOperation::GetEffectivePrices,
            "GetPlanEditorOptions" => ControlOperation::GetPlanEditorOptions,
            "GetClientServiceStatus" => ControlOperation::GetClientServiceStatus,
            "ListAgentPlanCatalog" => ControlOperation::ListAgentPlanCatalog,
            "GetAgentPlanStatus" => ControlOperation::GetAgentPlanStatus,
            "FindOperationByIdempotency" => ControlOperation::FindOperationByIdempotency,
            "ComputeSnapshot" => ControlOperation::ComputeSnapshot,
            "ComputeScan" => ControlOperation::ComputeScan,
            "ComputeConnectionOptions" => ControlOperation::ComputeConnectionOptions,
            "CheckModelConnection" => ControlOperation::CheckModelConnection,
            "ComputeSubscriptions" => ControlOperation::ComputeSubscriptions,
            "CheckSubscription" => ControlOperation::CheckSubscription,
            "WorkerSettingsGet" => ControlOperation::WorkerSettingsGet,
            "WorkerSettingsSet" => ControlOperation::WorkerSettingsSet,
            "WorkerTaskPlans" => ControlOperation::WorkerTaskPlans,
            "WorkerExecutorAvailability" => ControlOperation::WorkerExecutorAvailability,
            "WorkerTaskList" => ControlOperation::WorkerTaskList,
            "WorkerTaskStatus" => ControlOperation::WorkerTaskStatus,
            "WorkerTaskResult" => ControlOperation::WorkerTaskResult,
            "WorkerTaskWait" => ControlOperation::WorkerTaskWait,
            "WorkerTaskCancel" => ControlOperation::WorkerTaskCancel,
            "WorkerTaskContinue" => ControlOperation::WorkerTaskContinue,
            _ => ControlOperation::Other,
        }
    }
}

/// Generic bounded call outcome for operations without a dedicated event family.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundedCallEnd {
    pub operation: ControlOperation,
    pub ok: bool,
    pub elapsed_ms: u64,
}
