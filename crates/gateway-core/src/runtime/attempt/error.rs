use super::*;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum AttemptError {
    #[error("proxy exchange failed: {0:?}")]
    Proxy(ProxyFailurePhase),
    #[error("transport target is invalid: {0}")]
    InvalidTarget(Arc<str>),
    #[error("write quantum must be non-zero")]
    ZeroWriteQuantum,
    #[error("precommit event capacity must be non-zero")]
    ZeroPrecommitCapacity,
    #[error("attempt cleanup timeout must be non-zero")]
    InvalidCleanupTimeout,
    #[error("compiled body plan is invalid: {0}")]
    InvalidBodyPlan(Arc<str>),
    #[error("prepared request chunk exceeds the bounded writer quantum")]
    RequestChunkExceedsWriteQuantum,
    #[error("a sequential attempt body cannot enter a buffering request filter")]
    SequentialBodyFilterUnsupported,
    #[error("a sequential attempt body disagreed with its declared length or quantum")]
    SequentialBodyContractViolation,
    #[error("transport does not implement cancellation-safe duplex request writes")]
    DuplexWriteUnsupported,
    #[error("duplex request writer was polled without a pending charged unit")]
    MissingPendingDuplexWrite,
    #[error("all connection sub-attempts failed")]
    ConnectFailed,
    #[error("upstream connection timed out")]
    ConnectTimeout,
    #[error("upstream request write timed out")]
    RequestWriteTimeout,
    #[error("upstream first byte timed out")]
    FirstByteTimeout,
    #[error("upstream response stream became idle")]
    StreamIdleTimeout,
    #[error("transport operation failed: {0}")]
    Transport(Arc<str>),
    #[error("cannot reconnect after semantic request bytes may have committed")]
    ReconnectAfterCommit,
    #[error("synthetic attempt reply arrived after the semantic upstream exchange started")]
    SyntheticReplyAfterUpstreamStart,
    #[error("a pre-exchange failure cannot publish Accept")]
    PreexchangeAccept,
    #[error("commit fence already advanced")]
    FenceAlreadyAdvanced,
    #[error("commit fence write has not started")]
    FenceNotStarted,
    #[error("exchange is finalized")]
    ExchangeFinalized,
    #[error("no disposition candidate was submitted")]
    NoDispositionCandidate,
    #[error("disposition was already published")]
    DispositionAlreadyPublished,
    #[error("Accept was not blocked")]
    AcceptWasNotBlocked,
    #[error("blocked Accept must become Continue or Terminate")]
    BlockedAcceptMustBecomeNonAccept,
    #[error("disposition permit is invalid or stale")]
    InvalidDispositionPermit,
    #[error("response liveness changed before Accept publication")]
    AcceptLivenessChanged,
    #[error("attempt deadline exceeded")]
    DeadlineExceeded,
    #[error("request was cancelled")]
    Cancelled,
    #[error("attempt cleanup exceeded its bounded join timeout")]
    CleanupTimeout,
    #[error("accepted response cannot finish before Accept publication and normal request EOS")]
    AcceptedResponseNotReady,
    #[error("disposition publication is forbidden after accepted scope or downstream commit")]
    DispositionAfterDownstreamCommit,
    #[error("downstream write is forbidden before terminal disposition publication")]
    DownstreamWriteBeforeDisposition,
    #[error("accepted response scope was already created")]
    AcceptedResponseScopeAlreadyCreated,
    #[error("accepted response scope was not created")]
    AcceptedResponseScopeNotCreated,
    #[error("semantic output cannot start before downstream headers are confirmed")]
    DownstreamHeaderNotCommitted,
    #[error("Continue disposition cannot create or write an accepted response")]
    ContinueCannotWriteDownstream,
    #[error(transparent)]
    Body(#[from] BodyError),
}
