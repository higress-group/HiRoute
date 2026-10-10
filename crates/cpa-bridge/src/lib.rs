#![forbid(unsafe_code)]

//! Managed stock CLIProxyAPI connector runtime.
//!
//! The bridge owns process isolation, local capabilities, account-to-prefix pinning, and opaque
//! account references. It deliberately does not expose an OAuth token accessor and does not own
//! HiRoute candidate selection or retry policy.

mod accounts;
mod artifact;
mod attempt;
mod borrowed_claude;
mod borrowed_codex;
mod borrowed_subscription;
mod claude_profile;
pub use borrowed_claude::{BorrowedClaudeAuthSpec, BorrowedClaudeEvidence};
pub use borrowed_subscription::BorrowedSubscriptionEvidence;
mod config;
mod errors;
mod http;
mod managed_oauth;
pub use managed_oauth::{CpaManagedCredentialSummary, CpaManagedEvidence};
mod owner;
mod process;
mod proxy_environment;
mod request_context;
pub use request_context::CpaRequestContext;
mod runtime;
mod runtime_set;
pub use runtime_set::{CpaLoginSession, CpaLoginState, ManagedCpaRuntimeSet};
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
pub use runtime::{CpaOAuthLogin, CpaOAuthStatus};
pub use subscription::{
    CpaSubscriptionAvailability, CpaSubscriptionEffectContext, CpaSubscriptionMaterializer,
    CpaSubscriptionReleaseDecision, CpaSubscriptionSaveHandoff, cpa_subscription_availability,
    decide_subscription_release,
};

/// Stock CPA release whose CLI/config/management contract this implementation was grounded on.
pub const STOCK_CPA_CONTRACT_VERSION: &str = "7.2.140";
/// Exact managed binary: upstream protocol contract plus the private parent-pipe bootstrap.
pub const MANAGED_CPA_ARTIFACT_VERSION: &str = "8.0.4-hiroute.4";
/// The management PATCH synchronously refreshes upstream models before local pin checks.
/// Its whole-flow budget must leave room beyond CPA's five-second upstream deadline.
pub const MANAGED_CPA_CONTROL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
