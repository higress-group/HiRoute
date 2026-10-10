//! CPA (Codex Proxy Adapter) lifecycle events owned by the product bridge. CPA's own
//! stdout/stderr and credential locators never enter diagnostics.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpaLifecycle {
    pub phase: CpaPhase,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpaPhase {
    Start,
    Ready,
    Exit,
}

/// Result of a credential lease request; a stable code, not a credential or path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialLease {
    pub result: LeaseResultCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseResultCode {
    Granted,
    Denied,
    Expired,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpaStage {
    pub stage: CpaStageKind,
    pub elapsed_ms: u64,
    pub restart_generation: u64,
    /// `entered` is written before the step blocks, so a paused start still shows the step
    /// it reached; the same step then reports `completed` or `failed` with a stable code.
    #[serde(default)]
    pub outcome: CpaStageOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpaStageKind {
    NativeCredentialRead,
    ManagedCredentialRead,
    ArtifactLocate,
    ArtifactValidate,
    ProcessSpawn,
    ReadyWait,
    ControlCall,
}

/// A stage report without an outcome predates the entry/end split; it is read as a step that
/// ran to completion rather than one that is still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CpaStageOutcome {
    Entered,
    #[default]
    Completed,
    Failed {
        code: CpaFailureCode,
    },
}

/// Why a CPA start step failed. A cause outside this vocabulary is reported as `unknown`
/// rather than attributed to a step that did not fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpaFailureCode {
    CredentialSourceMissing,
    CredentialReadFailed,
    CredentialStoreUnsupported,
    CredentialLoginUnsupported,
    CredentialAccountMissing,
    CredentialInvalid,
    ManagedLoginRequired,
    /// The trusted artifact could not be located or verified.
    ArtifactUnavailable,
    /// The artifact was found but refused by the bridge contract.
    ArtifactRejected,
    /// The process could not be started.
    SpawnFailed,
    /// Another live process owns this instance.
    AlreadyOwned,
    /// The bounded ready deadline expired.
    ReadyTimeout,
    /// The process exited or answered unsafely before it became ready.
    ReadyRejected,
    /// A management socket or the complete control deadline timed out.
    ControlTimeout,
    /// The local management transport could not complete.
    ControlTransport,
    /// Local management or downstream capability authentication was rejected.
    ControlAuthentication,
    /// Account controls or the prefixed model catalog did not converge in time.
    ControlPinNotApplied,
    /// Explicit subscription discovery could not obtain the selected native client version.
    NativeClientVersionUnavailable,
    /// Control data was invalid, unsafe, or no longer described the expected account.
    ControlRejected,
    /// The cause is not part of this vocabulary.
    Unknown,
}
