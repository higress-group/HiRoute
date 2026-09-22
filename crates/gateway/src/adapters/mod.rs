//! Canonical protocol adapters.

mod continuation;
mod ingress;
mod request;
mod response;
mod tool_projection;

pub(crate) use tool_projection::{ChatToolIdentity, ChatToolProjection};

pub(crate) use continuation::{
    AcceptedResponseDeliveryScanner, ActiveResponseDelivery, ToolIdProjection,
    with_active_response_delivery,
};
pub use ingress::{
    IngressRequestBindings, decode_ingress_request, decode_ingress_request_with_bindings,
};
pub use request::{
    PreparedNativeRequest, PreparedNativeTemplate, project_candidate_request,
    project_candidate_request_template, sequential_attempt_body,
};
pub(crate) use request::{
    PreparedReplayTemplate, ReplacementEncoding, RequestedReplacement,
    prepare_replay_json_template, sequential_replay_body,
};
pub use response::{
    ClientResponseRenderer, DecodedNativeResponse, IncrementalClientSseRenderer,
    NativeResponseDecoder, RenderedClientResponse, RenderedSseEvent, ResponseDecodeStatus,
};
pub(crate) use response::{NativeResponseProjector, NativeTerminalOutcome};

use thiserror::Error;

use super::model_ir::ModelIrError;
use super::profiles::{CapabilityError, ContextProjectionError};

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ProtocolAdapterError {
    #[error(transparent)]
    ModelIr(#[from] ModelIrError),
    #[error(transparent)]
    Capability(#[from] CapabilityError),
    #[error(transparent)]
    Context(#[from] ContextProjectionError),
    #[error("native request JSON serialization failed: {0}")]
    Serialization(String),
    #[error("client renderer cannot express canonical semantics: {0}")]
    ClientUnrepresentable(String),
}

impl ProtocolAdapterError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ModelIr(ModelIrError::ResponsesPreviousResponseIdUnsupported) => {
                "RESPONSES_PREVIOUS_RESPONSE_ID_UNSUPPORTED"
            }
            Self::ModelIr(_) => "PROTOCOL_SEMANTICS_UNSUPPORTED",
            Self::Capability(CapabilityError::ProtocolPathUnavailable) => {
                "PROTOCOL_PATH_UNAVAILABLE"
            }
            Self::Capability(CapabilityError::ReasoningProfileUnknown)
            | Self::Capability(CapabilityError::ReasoningProfileMismatch) => {
                "REASONING_PROFILE_MISMATCH"
            }
            Self::Capability(_) => "PROTOCOL_CAPABILITY_UNSUPPORTED",
            Self::Context(ContextProjectionError::UnknownLimit(_))
            | Self::Context(ContextProjectionError::UnknownEstimator) => "CONTEXT_LIMIT_UNKNOWN",
            Self::Context(ContextProjectionError::InputTooLarge { .. })
            | Self::Context(ContextProjectionError::TotalTooLarge { .. }) => "CONTEXT_TOO_LARGE",
            Self::Context(ContextProjectionError::ArithmeticOverflow) => "CONTEXT_LIMIT_UNKNOWN",
            Self::Serialization(_) => "PROTOCOL_SERIALIZATION_FAILED",
            Self::ClientUnrepresentable(_) => "CLIENT_PROTOCOL_UNREPRESENTABLE",
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "namespace_tests.rs"]
mod namespace_tests;

#[cfg(test)]
mod native_reasoning_tests;

#[cfg(test)]
mod cpa_responses_tests;
