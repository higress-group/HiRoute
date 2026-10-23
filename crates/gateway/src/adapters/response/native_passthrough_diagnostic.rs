//! Fixed-size first-cause observation, never used by native protocol decisions.
use super::{IngressProtocol, NativeResponseProjector, NativeTerminalOutcome, ProjectionState};
use hiroute_diagnostics::event::{
    IngressProtocol as DiagnosticProtocol, NativeCompletionEvidence,
    NativeCompletionReason as Reason, ResponseEventShape,
};

#[cfg(test)]
#[path = "native_passthrough_diagnostic_tests.rs"]
mod tests;

#[derive(Default)]
pub(super) struct CompletionDiagnostic {
    frame_index: u64,
    pub(super) shape: Option<ResponseEventShape>,
    first: Option<NativeCompletionEvidence>,
    emitted: bool,
}

impl CompletionDiagnostic {
    pub(super) fn generic(&mut self) {
        self.first.get_or_insert(NativeCompletionEvidence {
            reason: Reason::Generic,
            frame_index: None,
            event: None,
        });
    }

    pub(super) fn note(&mut self, reason: Reason) {
        if self.first.is_none() {
            self.first = Some(NativeCompletionEvidence {
                reason,
                frame_index: (self.frame_index != 0).then_some(self.frame_index),
                event: self.shape,
            });
        }
    }

    pub(super) fn note_output(&mut self, reason: Reason, index: u32) {
        let mut event = self.shape;
        if let Some(shape) = &mut event {
            shape.output_index = Some(u64::from(index));
        }
        if self.first.is_none() {
            self.note(reason);
            self.first.as_mut().expect("recorded first cause").event = event;
        }
    }
}

impl ProjectionState {
    pub(in crate::server::core_runtime::adapters::response) fn begin_native_diagnostic_frame(
        &mut self,
    ) {
        self.completion_diagnostic.frame_index =
            self.completion_diagnostic.frame_index.saturating_add(1);
        self.completion_diagnostic.shape = None;
    }
}

impl NativeResponseProjector {
    /// Called only by the authoritative driver, never by canonical capture.
    pub(crate) fn take_completion_diagnostic(
        &mut self,
    ) -> Option<(DiagnosticProtocol, NativeCompletionEvidence)> {
        if self.state.terminal != Some(NativeTerminalOutcome::Unknown)
            || self.state.completion_diagnostic.emitted
            || self.awaiting_responses_eof()
            || self.pending_terminal_tail
        {
            return None;
        }
        self.state.completion_diagnostic.emitted = true;
        let protocol = match self.state.protocol {
            IngressProtocol::Responses => DiagnosticProtocol::OpenAiResponses,
            IngressProtocol::Messages => DiagnosticProtocol::AnthropicMessages,
            IngressProtocol::ChatCompletions => DiagnosticProtocol::OpenAiChat,
        };
        Some((
            protocol,
            self.state
                .completion_diagnostic
                .first
                .unwrap_or(NativeCompletionEvidence {
                    reason: Reason::Generic,
                    frame_index: None,
                    event: None,
                }),
        ))
    }
}
