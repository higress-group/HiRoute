use thiserror::Error;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ModelIrError {
    #[error("protocol document must be a JSON object")]
    ExpectedObject,
    #[error("required field is missing or has the wrong type: {0}")]
    InvalidField(&'static str),
    #[error("unsupported field would be lost: {0}")]
    UnsupportedField(String),
    #[error("unsupported semantic value: {0}")]
    UnsupportedValue(String),
    #[error("Responses previous_response_id continuation is unsupported")]
    ResponsesPreviousResponseIdUnsupported,
    #[error("Tool arguments are not complete JSON: {0}")]
    InvalidToolArguments(String),
    #[error("Tool call identity is missing: {0}")]
    MissingToolIdentity(String),
    #[error("provider state is not portable to the requested protocol")]
    ProviderStateNotPortable,
    #[error("provider state has no exact owner binding")]
    ProviderStateOwnershipRequired,
    #[error("Tool call has no exact logical/native ID binding: {0}")]
    ToolIdBindingRequired(String),
    #[error("Tool continuation identity is ambiguous or conflicting")]
    ToolContinuationConflict,
    #[error("native response lifecycle is invalid: {0}")]
    InvalidResponseLifecycle(String),
    #[error("native response ended without an explicit terminal event")]
    MissingTerminalEvent,
    #[error("native response emitted more than one terminal event")]
    DuplicateTerminalEvent,
    #[error("native response JSON is invalid: {0}")]
    InvalidJson(String),
    #[error("SSE framing is invalid: {0}")]
    InvalidSse(String),
    #[error("bounded protocol buffer exceeded {0} bytes")]
    BufferLimit(usize),
}
