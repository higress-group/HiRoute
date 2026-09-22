use std::collections::VecDeque;

use hiroute_gateway_core::runtime::body::{ChargedBytes, MemoryRole, StreamBudget};
use hiroute_gateway_core::runtime::sse::{
    BoundedEventEmitter, BoundedOutputSink, SseError, SseEventView, SseFeedOutcome, SseFramer,
    SseVisitor,
};

use crate::server::core_runtime::model_ir::ModelIrError;

use super::ProtocolAdapterError;
use super::native_passthrough::{
    NativeProjectedUnit, NativeTerminalOutcome, ProjectionMetadata, ProjectionState,
};

pub(super) fn feed_projected_sse(
    framer: &mut SseFramer,
    budget: &StreamBudget,
    state: &mut ProjectionState,
    pending_terminal_tail: &mut bool,
    bytes: &[u8],
    end_stream: bool,
) -> Result<Vec<NativeProjectedUnit>, ProtocolAdapterError> {
    let charged = ChargedBytes::copy_from_opaque(budget, MemoryRole::TransportInflight, bytes)
        .map_err(|error| ModelIrError::InvalidSse(error.to_string()))?;
    let mut metadata = VecDeque::new();
    let mut sink = ProjectedSink::default();
    let mut visitor = ProjectionVisitor {
        budget,
        state,
        metadata: &mut metadata,
        protocol_error: None,
    };
    let outcome = framer.feed(charged, end_stream, &mut visitor, &mut sink);
    if let Some(error) = visitor.protocol_error {
        return Err(error);
    }
    if matches!(outcome, Ok(SseFeedOutcome::NeedDrain { .. })) {
        return Err(ModelIrError::InvalidSse(
            "native projector unexpectedly required output drain".into(),
        )
        .into());
    }
    outcome.map_err(|error| ModelIrError::InvalidSse(error.to_string()))?;
    if sink.output.len() != metadata.len() {
        return Err(
            ModelIrError::InvalidSse("native projector lost SSE output metadata".into()).into(),
        );
    }
    let mut output: Vec<NativeProjectedUnit> = sink
        .output
        .into_iter()
        .zip(metadata)
        .map(|(bytes, metadata)| NativeProjectedUnit {
            bytes,
            source_bytes: metadata.source_bytes,
            semantic: metadata.semantic,
            terminal: metadata.terminal,
            failure: metadata.failure,
        })
        .collect();
    if state.protocol == super::IngressProtocol::Responses {
        // Responses may append a native [DONE] marker after its response terminal.
        // Preserve both wire events, but certify the single terminal only after
        // transport EOF has proved that no later semantic event follows them.
        for unit in &mut output {
            unit.terminal = None;
        }
        if end_stream && let Some(terminal) = state.terminal {
            output.push(NativeProjectedUnit {
                bytes: Vec::new(),
                source_bytes: 0,
                semantic: false,
                terminal: Some(terminal),
                failure: None,
            });
        }
        return Ok(output);
    }
    if let Some(index) = output.iter().position(|unit| unit.terminal.is_some())
        && (index + 1 < output.len() || framer.pending_bytes() > 0)
    {
        state.withhold_terminal_completion();
        output[index].terminal = None;
        *pending_terminal_tail = true;
    }
    // The terminal may precede a later SSE event whose bytes span chunks.
    // Preserve the framed predecessor but delay downstream EOS until the
    // complete tail can be emitted.
    if *pending_terminal_tail
        && framer.pending_bytes() == 0
        && let Some(last) = output.last_mut()
    {
        last.terminal = Some(NativeTerminalOutcome::Unknown);
        *pending_terminal_tail = false;
    }
    Ok(output)
}

struct ProjectionVisitor<'a> {
    budget: &'a StreamBudget,
    state: &'a mut ProjectionState,
    metadata: &'a mut VecDeque<ProjectionMetadata>,
    protocol_error: Option<ProtocolAdapterError>,
}

impl SseVisitor for ProjectionVisitor<'_> {
    fn on_event(
        &mut self,
        event: SseEventView<'_>,
        emitter: &mut BoundedEventEmitter<'_, '_>,
    ) -> Result<(), SseError> {
        let data = event.data(self.budget)?;
        if data.as_ref().is_empty() {
            if self.state.protocol == super::IngressProtocol::Responses
                && self.state.terminal.is_some()
            {
                self.state.withhold_terminal_completion();
            }
            self.metadata.push_back(ProjectionMetadata {
                semantic: false,
                terminal: None,
                failure: None,
                source_bytes: event.raw().len(),
            });
            return emitter.pass_raw();
        }
        let event_type = match event.event_type() {
            Some(value) => match std::str::from_utf8(value) {
                Ok(value) => Some(value),
                Err(error) => {
                    self.protocol_error = Some(ModelIrError::InvalidSse(error.to_string()).into());
                    return Err(SseError::FramerFailed);
                }
            },
            None => None,
        };
        let (rewritten, mut metadata) = match self.state.project_sse(event_type, data.as_ref()) {
            Ok(projected) => projected,
            Err(error) => {
                self.protocol_error = Some(error);
                return Err(SseError::FramerFailed);
            }
        };
        metadata.source_bytes = event.raw().len();
        self.metadata.push_back(metadata);
        if let Some(data) = rewritten {
            let bytes = render_rewritten_event(&event, &data);
            let mut output = emitter.output_builder(bytes.len())?;
            output
                .extend_from_slice(&bytes)
                .map_err(|_| SseError::BudgetExceeded)?;
            emitter.emit_owned(output.finish())
        } else {
            emitter.pass_raw()
        }
    }
}

#[derive(Default)]
struct ProjectedSink {
    output: Vec<Vec<u8>>,
}

impl BoundedOutputSink for ProjectedSink {
    fn emit_borrowed(&mut self, bytes: &[u8]) -> Result<(), SseError> {
        self.output.push(bytes.to_vec());
        Ok(())
    }

    fn emit_owned(&mut self, bytes: ChargedBytes) -> Result<(), SseError> {
        self.output.push(bytes.bytes().to_vec());
        Ok(())
    }
}

fn render_rewritten_event(event: &SseEventView<'_>, data: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(event.raw().len().max(data.len() + 16));
    let mut wrote_data = false;
    for field in event.fields() {
        if !field.comment && field.name == b"data" {
            if !wrote_data {
                output.extend_from_slice(b"data: ");
                output.extend_from_slice(data);
                output.push(b'\n');
                wrote_data = true;
            }
            continue;
        }
        if field.comment {
            output.push(b':');
            if !field.value.is_empty() {
                output.push(b' ');
                output.extend_from_slice(field.value);
            }
        } else {
            output.extend_from_slice(field.name);
            output.push(b':');
            if !field.value.is_empty() {
                output.push(b' ');
                output.extend_from_slice(field.value);
            }
        }
        output.push(b'\n');
    }
    if !wrote_data {
        output.extend_from_slice(b"data: ");
        output.extend_from_slice(data);
        output.push(b'\n');
    }
    output.push(b'\n');
    output
}
