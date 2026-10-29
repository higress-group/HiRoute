//! The client renderer owns intentional omission, including capture's view of it.
use super::{
    ClientProtocolProfile, IncrementalClientSseRenderer, IngressProtocol, ModelEvent,
    ResponseBlockKind,
};

impl IncrementalClientSseRenderer {
    /// Whether this client's protocol projection retains an event's meaning.
    /// `true` includes deferred events and final snapshots represented by earlier
    /// deltas; an empty result from `push` is not evidence of omission. This does
    /// not validate an event or prove that its wire bytes were accepted.
    ///
    /// Apply this to converted delivery only. Native passthrough has its own wire
    /// projection. Buffered delivery must still pass its response-level rendering
    /// validation before any retained event can be considered accepted.
    pub(crate) fn retains_event(profile: &ClientProtocolProfile, event: &ModelEvent) -> bool {
        match event {
            ModelEvent::ProviderState { state } => match profile.protocol {
                IngressProtocol::Responses => state.kind == "encrypted_content",
                IngressProtocol::Messages => matches!(
                    state.kind.as_str(),
                    "thinking_signature" | "redacted_thinking"
                ),
                IngressProtocol::ChatCompletions => false,
            },
            ModelEvent::ContentBlockStarted {
                block_kind: ResponseBlockKind::Reasoning,
                ..
            }
            | ModelEvent::ReasoningDelta { .. }
            | ModelEvent::ReasoningFinished { .. } => {
                profile.protocol != IngressProtocol::Messages
                    || profile.source_protocol == IngressProtocol::Messages
            }
            _ => true,
        }
    }
}

#[cfg(test)]
#[path = "client_projection_tests.rs"]
mod tests;
