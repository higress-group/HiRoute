//! Startup, readiness and child-process events for the Desktop and daemon entry paths.

use serde::{Deserialize, Serialize};

use super::{OutcomeKind, ProcessRole};
use crate::error::StableErrorCode;
use crate::identity::{SessionId, TargetTriple, VersionToken};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessStart {
    pub role: ProcessRole,
    pub version: VersionToken,
    pub source_revision: VersionToken,
    pub target: TargetTriple,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<SessionId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupOutcome {
    Success,
    Failure { code: StartupFailureCode },
    Cancelled,
}

/// Stable startup failure vocabulary. Startup never carries an arbitrary error string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupFailureCode {
    InvalidConfiguration,
    StorageUnavailable,
    UpgradeSourceUnsupported,
    UpgradeStorageFailed,
    ReleaseFactsInvalid,
    DependencyUnavailable,
    WorkerRejected,
    GatewayUnavailable,
    ControlUnavailable,
    ReadyChannelFailed,
    Cancelled,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartupEnd {
    pub outcome: StartupOutcome,
    pub elapsed_ms: u64,
}

/// Stage identifiers shared by the Desktop bootstrap and the daemon role startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupStage {
    RootValidate,
    ResidentLock,
    ArtifactValidate,
    Spawn,
    ReadyWait,
    CatalogLoad,
    GatewayStart,
    WorkerRevalidate,
    ControlBind,
    PublicationReconcile,
}

impl StartupStage {
    pub const ALL: [StartupStage; 10] = [
        StartupStage::RootValidate,
        StartupStage::ResidentLock,
        StartupStage::ArtifactValidate,
        StartupStage::Spawn,
        StartupStage::ReadyWait,
        StartupStage::CatalogLoad,
        StartupStage::GatewayStart,
        StartupStage::WorkerRevalidate,
        StartupStage::ControlBind,
        StartupStage::PublicationReconcile,
    ];
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageBegin {
    pub stage: StartupStage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageEnd {
    pub stage: StartupStage,
    pub elapsed_ms: u64,
    pub outcome: StageOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageOutcome {
    Completed,
    Failed { code: StartupFailureCode },
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageProgress {
    pub stage: StartupStage,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_ms: Option<u64>,
}

/// A readiness pipe result. `direction` distinguishes the daemon writing `ready` from the
/// parent reading it, so a closed pipe is attributable to one side.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadyIo {
    pub direction: ReadyDirection,
    pub ok: bool,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ReadyIoError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_errno: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadyDirection {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadyIoError {
    BrokenPipe,
    Closed,
    Timeout,
    Protocol,
    Io,
}

/// The verified exit of a daemon child process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildExit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
    pub verified: bool,
}

/// Generic bounded outcome for operations that do not need a dedicated payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundedOutcome {
    pub outcome: OutcomeKind,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<StableErrorCode>,
}
