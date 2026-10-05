//! Worker execution values. Identity locates records; it never grants authority.
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod installation;
pub use installation::*;
mod progress;
pub use progress::*;
mod process;
pub use process::*;
mod native_cleanup;
pub use native_cleanup::*;
mod records;
pub use records::*;

pub const DEFAULT_RUN_DURATION_MS: u64 = 60 * 60 * 1_000;
pub const MAX_RUN_DURATION_MS: u64 = 24 * DEFAULT_RUN_DURATION_MS;
pub const DEFAULT_WORKER_CONCURRENCY: u16 = 10;
pub const MAX_WORKER_CONCURRENCY: u16 = 1_000;

/// The sole current Worker concurrency setting for one daemon-local runtime store.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConcurrencySettingsV1 {
    pub max_concurrent: u16,
}

impl Default for WorkerConcurrencySettingsV1 {
    fn default() -> Self {
        Self {
            max_concurrent: DEFAULT_WORKER_CONCURRENCY,
        }
    }
}

impl WorkerConcurrencySettingsV1 {
    pub fn validate(self) -> Result<Self, DelegationErrorV1> {
        if !(1..=MAX_WORKER_CONCURRENCY).contains(&self.max_concurrent) {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerHarnessV1 {
    CodexCli,
    ClaudeCode,
    QoderCli,
    Pi,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAccessV1 {
    ReadOnly,
    WorkspaceWrite,
    /// Explicitly approved Harness-native mode; not an OS filesystem sandbox.
    TrustedNative,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerToolV1 {
    Read,
    Edit,
    Shell,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerNetworkV1 {
    /// Model ingress is allowed; Worker tools have no general network access.
    GatewayOnly,
    Allowed,
}

/// Caller-selected native ACP permission response policy for one Worker run.
///
/// This is execution configuration inside the same-UID local trust boundary, not a caller
/// credential, directory grant, or operating-system sandbox.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerPermissionPolicyV1 {
    /// Use the selected Harness's native autonomous mode. An unexpected ACP permission
    /// request may receive one lifecycle-bounded AllowOnce as a compatibility fallback.
    #[default]
    ApproveAll,
    /// Request the selected Harness's proven restricted native configuration and accept only an
    /// unexpected request that the Harness classifies as a read. Unproven mappings are
    /// unavailable rather than widened.
    ApproveReads,
    /// Request a proven no-active-tool native configuration and reject every ACP permission
    /// request. Unproven mappings are unavailable rather than widened.
    DenyAll,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceExecutionPermitV1 {
    pub permit_id: String,
    pub generation: u64,
    /// Opaque identity from the platform's canonical root validation, not raw cwd.
    pub root_identity: String,
    pub access: WorkspaceAccessV1,
    pub tools: Vec<WorkerToolV1>,
    pub network: WorkerNetworkV1,
    pub expires_at_ms: u64,
    pub max_run_ms: u64,
    /// Legacy serialized compatibility field. Current Worker capacity is governed only by
    /// `WorkerConcurrencySettingsV1` in the daemon-local runtime store.
    pub max_concurrent: u16,
    pub revoked: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerExecutionIntentV1 {
    pub root_identity: String,
    pub access: WorkspaceAccessV1,
    pub tools: Vec<WorkerToolV1>,
    pub network: WorkerNetworkV1,
    pub duration_ms: u64,
    pub delegation_depth: u8,
}

impl WorkspaceExecutionPermitV1 {
    pub fn validate(&self) -> Result<(), DelegationErrorV1> {
        if self.permit_id.is_empty()
            || self.permit_id.len() > 128
            || self.root_identity.is_empty()
            || self.root_identity.len() > 512
            || self.generation == 0
            || self.max_run_ms == 0
            || self.max_run_ms > MAX_RUN_DURATION_MS
            || self.max_concurrent == 0
            || self.expires_at_ms == 0
            || self.tools.len() > 3
            || (self.access == WorkspaceAccessV1::ReadOnly
                && self.tools.contains(&WorkerToolV1::Edit))
        {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        Ok(())
    }

    /// Called with the current grant/permit under the one shared admission gate.
    /// Native mode compatibility and collaboration Plan authorization are separate checks.
    pub fn authorize(
        &self,
        intent: &WorkerExecutionIntentV1,
        now_ms: u64,
        expected_generation: u64,
    ) -> Result<u64, DelegationErrorV1> {
        self.validate()?;
        if self.revoked
            || expected_generation != self.generation
            || intent.root_identity != self.root_identity
            || intent.delegation_depth != 1
            || intent.tools.iter().any(|tool| !self.tools.contains(tool))
            || (intent.access == WorkspaceAccessV1::ReadOnly
                && intent.tools.contains(&WorkerToolV1::Edit))
            || (self.access != WorkspaceAccessV1::TrustedNative
                && intent.access == WorkspaceAccessV1::TrustedNative)
            || (self.access == WorkspaceAccessV1::ReadOnly
                && intent.access != WorkspaceAccessV1::ReadOnly)
            || (self.network == WorkerNetworkV1::GatewayOnly
                && intent.network != WorkerNetworkV1::GatewayOnly)
        {
            return Err(DelegationErrorV1::PermissionDenied);
        }
        let deadline = now_ms
            .checked_add(intent.duration_ms)
            .ok_or(DelegationErrorV1::DeadlineExceeded)?;
        if intent.duration_ms == 0
            || intent.duration_ms > self.max_run_ms
            || now_ms >= self.expires_at_ms
            || deadline > self.expires_at_ms
        {
            return Err(DelegationErrorV1::DeadlineExceeded);
        }
        Ok(deadline)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Error, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationErrorV1 {
    #[error("delegation arguments are invalid")]
    InvalidArguments,
    #[error("delegation permission is missing or no longer valid")]
    PermissionDenied,
    #[error("delegation deadline is outside the current permit")]
    DeadlineExceeded,
    #[error("delegation was cancelled")]
    Cancelled,
    #[error("delegation task already has an active run")]
    Busy,
    #[error("the Worker concurrency capacity has been reached")]
    CapacityExceeded,
    #[error("delegation state conflicts with the requested transition")]
    Conflict,
    #[error("delegation task or run was not found")]
    NotFound,
    #[error("required Worker capability is unavailable or unverified")]
    CapabilityUnavailable,
    #[error("the selected Worker dependency tuple is incomplete or missing")]
    DependenciesMissing,
    #[error("the selected Worker dependency metadata is invalid")]
    DependenciesInvalid,
    #[error("the selected Worker dependency metadata cannot be inspected")]
    DependenciesUnavailable,
    #[error("the original native session cannot be resumed")]
    ResumeUnavailable,
    #[error("run content is unavailable or incomplete")]
    ContentUnavailable,
    #[error("delegation storage is unavailable")]
    StorageUnavailable,
    #[error("delegation protocol connection failed")]
    ProtocolFailed,
    #[error("Worker reported a terminal prompt failure")]
    PromptFailed,
}

#[cfg(test)]
mod tests;

mod authorization;
pub use authorization::*;
