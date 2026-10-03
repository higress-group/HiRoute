#![forbid(unsafe_code)]

//! Managed stock CLIProxyAPI connector runtime.
//!
//! The bridge owns process isolation, local capabilities, account-to-prefix pinning, and opaque
//! account references. It deliberately does not expose an OAuth token accessor and does not own
//! HiRoute candidate selection or retry policy.

mod accounts;
mod artifact;
mod attempt;
mod borrowed_codex;
mod config;
mod errors;
mod http;
mod owner;
mod process;
mod runtime;
mod state;
mod subscription;

pub use accounts::{CpaAccountKind, CpaProfileBinding};
pub use artifact::{
    CpaArtifactError, CpaBinaryLocator, PinnedCpaArtifact, PinnedCpaBinaryLocator,
    VerifiedCpaBinary,
};
pub use attempt::{
    CpaAttemptError, CpaDownstreamCredentialCapability, CpaDownstreamCredentialPort,
    ExactCpaAttemptRequest, ExactCpaCredentialRequest, PreparedCpaTarget,
};
pub use borrowed_codex::{BorrowedCodexAuthSpec, BorrowedCodexEvidence};
pub use config::{CpaConfigError, CpaManagedConfigContract};
pub use errors::CpaLifecycleError;
pub use process::{
    CpaExit, CpaLaunch, CpaProcessBackend, CpaProcessError, CpaProcessHandle, StdCpaProcessBackend,
};
pub use runtime::{
    CpaHealth, CpaRegisteredSourcePort, CpaRoutingBatch, CpaRuntimeSpec, CpaSourceManagementState,
    ManagedCpaRuntime, RestartPolicy,
};
pub use subscription::{
    CpaSubscriptionAvailability, CpaSubscriptionEffectContext, CpaSubscriptionMaterializer,
    CpaSubscriptionReleaseDecision, CpaSubscriptionSaveHandoff, cpa_subscription_availability,
    decide_subscription_release,
};

/// Stock CPA release whose CLI/config/management contract this implementation was grounded on.
pub const STOCK_CPA_CONTRACT_VERSION: &str = "7.2.140";
/// Exact managed binary: upstream protocol contract plus the private parent-pipe bootstrap.
pub const MANAGED_CPA_ARTIFACT_VERSION: &str = "8.0.4-hiroute.2";
