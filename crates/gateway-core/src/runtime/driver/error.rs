use super::*;

#[derive(Debug, Error)]
pub enum GatewayExecutionError {
    #[error("gateway lifecycle limits are invalid")]
    InvalidLimits,
    #[error("native filter registry is invalid")]
    InvalidFilterRegistry,
    #[error("logical request driver did not provide the required filter scope")]
    MissingFilterScope,
    #[error("compiled route did not match")]
    RouteNotFound,
    #[error("request body exceeded its hard limit")]
    RequestBodyLimit,
    #[error("request framing metadata is invalid")]
    InvalidRequestFraming,
    #[error("request deadline exceeded")]
    Deadline,
    #[error("selected attempt deadline exceeded before exchange creation")]
    PreexchangeAttemptDeadline,
    #[error("request was cancelled")]
    Cancelled,
    #[error("request-owned operation panicked")]
    OperationPanic,
    #[error("attempt cleanup exceeded its bounded join timeout")]
    CleanupTimeout,
    #[error("selection port returned an invalid attempt identity or plan binding")]
    InvalidSelection,
    #[error("already-authorized request admission is inconsistent or expired")]
    InvalidBoundAdmission,
    #[error("selection port returned malformed or unattributable realtime routing facts")]
    InvalidRoutingFacts,
    #[error("provider port returned mechanically inconsistent structured facts")]
    InvalidProviderFacts,
    #[error("selection port returned an invalid replacement for blocked Accept")]
    InvalidBlockedReplacement,
    #[error("selection port returned Accept for a mechanical attempt failure")]
    InvalidFailureDisposition,
    #[error("upstream response ended without a disposition candidate")]
    ResponseEndedWithoutDisposition,
    #[error("accepted upstream response ended without an EOS event")]
    ResponseEndedWithoutEos,
    #[error("accepted body filter mutated response headers after downstream commit")]
    AcceptedHeaderMutationAfterCommit,
    #[error("accepted body filter returned a local reply after response head commit")]
    AcceptedBodyLocalReplyAfterCommit,
    #[error("logical body filter mutated request headers after provider commit")]
    LogicalHeaderMutationAfterCommit,
    #[error("Accept publication did not carry provider readiness")]
    AcceptedWithoutReadiness,
    #[error("attempt local reply normalization cannot select Accept without readiness")]
    AcceptedLocalReplyWithoutReadiness,
    #[error("attempt local reply normalization changed the upstream side-effect snapshot")]
    InvalidAttemptLocalReplySideEffects,
    #[error("provider terminal encoder did not return one end-of-stream frame")]
    TerminalEncoderDidNotComplete,
    #[error("provider did not return decoded SSE ownership for sequence {0}")]
    MissingDecodedSse(u64),
    #[error("provider returned decoded SSE ownership for non-SSE sequence {0}")]
    UnexpectedDecodedSse(u64),
    #[error("provider changed SSE sequence from {expected} to {actual}")]
    DecodedSseSequenceMismatch { expected: u64, actual: u64 },
    #[error("local response metadata is invalid")]
    InvalidLocalReply,
    #[error("selection port failed: {0}")]
    Selection(Arc<str>),
    #[error("provider port failed: {0}")]
    Provider(Arc<str>),
    #[error("provider terminal request release failed: {0}")]
    ProviderTerminalRelease(Arc<str>),
    #[error("filter manager failed: {0}")]
    Filter(Arc<str>),
    #[error(transparent)]
    Driver(#[from] DriverError),
    #[error(transparent)]
    Attempt(#[from] AttemptError),
    #[error(transparent)]
    Plan(#[from] crate::core::execution_plan::PlanError),
    #[error(transparent)]
    Install(#[from] crate::core::publication::InstallError),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Body(#[from] BodyError),
    #[error(transparent)]
    Sse(#[from] SseError),
    #[error(transparent)]
    Executor(#[from] ExecutorError),
}

impl GatewayExecutionError {
    pub(super) fn is_cancelled(&self) -> bool {
        matches!(
            self,
            Self::Cancelled
                | Self::Attempt(AttemptError::Cancelled)
                | Self::Transport(TransportError::Cancelled)
        )
    }

    /// Keep the request's termination cause across the transport boundary.
    /// Cancellation tokens also stop deadline and cleanup work, so their
    /// final state cannot recover this information after execution finishes.
    pub(super) fn into_transport_error(self) -> TransportError {
        if self.is_cancelled() {
            TransportError::Cancelled
        } else {
            TransportError::Io(self.to_string().into())
        }
    }
}
