//! Canonical protocol adapters.

mod continuation;
mod ingress;
pub(crate) mod reasoning_loss;
mod request;
mod response;
mod responses_dialect;
mod schema_encoding;
pub(crate) use responses_dialect::FunctionEncodingProjection;
pub(crate) use schema_encoding::{SchemaChildren, schema_children};
#[cfg(test)]
mod responses_dialect_tests;
mod structured_output;
mod tool_projection;

pub(crate) use tool_projection::{ChatToolIdentity, ChatToolProjection};

pub(crate) use continuation::{
    AcceptedResponseDeliveryScanner, ActiveResponseDelivery, ToolIdProjection,
    with_active_response_delivery,
};
pub use ingress::decode_ingress_request;
#[cfg(test)]
pub(crate) use ingress::decode_ingress_request_with_replay;
pub(crate) use ingress::decode_owned_ingress_request_with_replay;
pub use request::{
    PreparedNativeRequest, PreparedNativeTemplate, project_candidate_request,
    project_candidate_request_template, sequential_attempt_body,
};
pub(crate) use request::{
    PreparedReplayTemplate, ReplacementEncoding, RequestedReplacement,
    prepare_replay_json_template, project_candidate_request_template_with_cleanup,
    sequential_replay_body,
};
pub use response::{
    ClientResponseRenderer, DecodedNativeResponse, IncrementalClientSseRenderer,
    NativeResponseDecoder, RenderedClientResponse, RenderedSseEvent, ResponseDecodeStatus,
};
pub(crate) use response::{NativeResponseProjector, NativeTerminalOutcome};

use thiserror::Error;

use super::model_ir::ModelIrError;
use super::profiles::{CapabilityError, ContextProjectionError};
use crate::server::request_plan::IngressProtocol;

/// Responses labels its ordinary assistant answer with `final_answer`. Messages and Chat
/// Completions have only that assistant-output channel, so the label is losslessly implicit in
/// those protocols. Live non-final output still needs its native phase/termination semantics.
fn can_represent_live_message_phase(protocol: IngressProtocol, phase: Option<&str>) -> bool {
    phase.is_none() || protocol == IngressProtocol::Responses || phase == Some("final_answer")
}

/// Bounded facts returned by the actual historical-message projection. These are
/// derived per candidate and never replace the original request or scoring history.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RequestProjectionLoss {
    pub message_phases_omitted: usize,
    /// Zero-based canonical request.messages coordinate, never a native item index.
    pub first_message_index: Option<usize>,
}

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
    #[error("request projection is unsupported: {reason:?}, canonical message {message_index:?}")]
    RequestUnrepresentable {
        reason: hiroute_diagnostics::event::RequestProjectionReason,
        message_index: Option<usize>,
    },
}

impl ProtocolAdapterError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ModelIr(ModelIrError::ResponsesPreviousResponseIdUnsupported) => {
                "RESPONSES_PREVIOUS_RESPONSE_ID_UNSUPPORTED"
            }
            Self::ModelIr(ModelIrError::ResponsesConversationUnsupported) => {
                "RESPONSES_CONVERSATION_UNSUPPORTED"
            }
            Self::ModelIr(
                ModelIrError::ExpectedObject
                | ModelIrError::InvalidField(_)
                | ModelIrError::InvalidJson(_)
                | ModelIrError::InvalidSse(_),
            ) => "PROTOCOL_INVALID_PAYLOAD",
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
            Self::Context(ContextProjectionError::ArithmeticOverflow) => "CONTEXT_LIMIT_UNKNOWN",
            Self::Serialization(_) => "PROTOCOL_SERIALIZATION_FAILED",
            Self::ClientUnrepresentable(_) | Self::RequestUnrepresentable { .. } => {
                "CLIENT_PROTOCOL_UNREPRESENTABLE"
            }
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
