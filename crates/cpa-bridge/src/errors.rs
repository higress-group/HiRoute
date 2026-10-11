use thiserror::Error;

use crate::{CpaArtifactError, CpaConfigError, CpaExit, CpaProcessError};

#[derive(Debug, Error)]
pub enum CpaLifecycleError {
    #[error("CPA operation was cancelled or exceeded its absolute deadline")]
    OperationCancelled,
    #[error("CPA independent login has no saved credential")]
    ManagedOAuthCredentialsMissing,
    #[error("CPA independent login credential is invalid or unavailable")]
    InvalidManagedOAuthCredentials,
    #[error("CPA independent authorization has expired or been revoked; sign in again")]
    ManagedOAuthAuthenticationRequired,
    #[error("CPA independent login account changed")]
    ManagedOAuthAccountChanged,
    #[error("CPA independent login session is missing, stale or already used")]
    InvalidOAuthSession,
    #[error("Claude native authentication is missing")]
    BorrowedClaudeAuthMissing,
    #[error(
        "Claude native authentication is unavailable; unlock or authorize its credential store and retry"
    )]
    BorrowedClaudeAuthUnavailable,
    #[error("Claude native authentication is expired or invalid")]
    InvalidBorrowedClaudeAuth,
    #[error("Claude native account or authentication changed")]
    BorrowedClaudeAuthSourceChanged,
    #[error("CPA runtime specification is invalid")]
    InvalidSpec,
    #[error("CPA Release binding is invalid")]
    InvalidBinding,
    #[error("CPA instance is already owned by a live process")]
    AlreadyOwned,
    #[error("CPA owner state is invalid or raced")]
    OwnerState,
    #[error("CPA orphan does not match the trusted artifact")]
    UntrustedOrphan,
    #[error("CPA runtime has not been started")]
    NotStarted,
    #[error("CPA process exited during startup: {0:?}")]
    ExitedDuringStartup(CpaExit),
    #[error("CPA did not become ready before its bounded deadline")]
    StartupTimeout,
    #[error("CPA returned unsafe or mismatched control-plane data")]
    UnsafeControlResponse,
    #[error("CPA authenticated control plane is unavailable")]
    ControlUnavailable,
    #[error("CPA trusted artifact changed while restarting")]
    ArtifactChanged,
    #[error("CPA artifact version is not supported by this bridge contract")]
    UnsupportedArtifactVersion,
    #[error("CPA exceeded its bounded restart budget: {0:?}")]
    CrashLoop(CpaExit),
    #[error("CPA materialization is invalid")]
    InvalidMaterialization,
    #[error("CPA source management input is invalid")]
    InvalidSourceManagement,
    #[error("CPA source management revision is stale or conflicting")]
    StaleSourceManagement,
    #[error("CPA account state is invalid")]
    InvalidAccountState,
    #[error("CPA account state I/O failed: {0}")]
    AccountStateIo(std::io::Error),
    #[error("borrowed Codex authentication is invalid or unsafe")]
    InvalidBorrowedCodexAuth,
    #[error("borrowed Codex authentication source is missing")]
    BorrowedCodexAuthMissing,
    #[error("borrowed Codex authentication is no longer usable by CPA")]
    BorrowedCodexAuthUnavailable,
    #[error("borrowed Codex authentication changed while it was being checked")]
    BorrowedCodexAuthSourceChanged,
    #[error("borrowed Codex authentication or its managed auth directory is already leased")]
    BorrowedCodexAuthAlreadyLeased,
    #[error("borrowed Codex authentication state could not be accessed")]
    BorrowedCodexAuthIo,
    #[error("selected Codex client version is unavailable for subscription discovery")]
    BorrowedCodexClientVersionUnavailable,
    #[error("CPA loopback address could not be reserved: {0}")]
    LoopbackBind(std::io::Error),
    #[error(transparent)]
    Artifact(#[from] CpaArtifactError),
    #[error(transparent)]
    Config(#[from] CpaConfigError),
    #[error(transparent)]
    Process(#[from] CpaProcessError),
}
