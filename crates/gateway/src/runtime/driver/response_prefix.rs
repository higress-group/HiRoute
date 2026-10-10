//! Hold upstream acceptance only until the first bounded, deliverable body unit.
//! Headers are not evidence that an upstream can produce a usable response.
use super::*;

pub(super) fn classify_stream_prefix(
    state: &mut ProductionAttemptState,
    bytes: &[u8],
    end_stream: bool,
) -> Result<PrecommitClassification<ProductionReadiness, ProductionDecodedSse>, Arc<str>> {
    let status = state.response_status.expect("stream prefix follows a head");
    let failure = if state.native_output {
        project_prefix(state, bytes, end_stream)
    } else {
        decode_prefix(state, bytes, end_stream)
    };
    match failure {
        Ok(Some(error)) => return classify_model_error(state, error, status),
        Err(_) => {
            return classify_state_failure(
                state,
                RawAttemptFailure::Protocol,
                StatusCode::BAD_GATEWAY,
            );
        }
        Ok(None) => {}
    }
    if known_terminal_failure(state.semantic_terminal) {
        return classify_state_failure(state, RawAttemptFailure::Protocol, StatusCode::BAD_GATEWAY);
    }
    let empty = state.prefix.as_ref().is_none_or(|prefix| prefix.is_empty());
    if end_stream && empty {
        return classify_state_failure(state, RawAttemptFailure::Protocol, StatusCode::BAD_GATEWAY);
    }
    // Native Responses certifies the terminal after its optional native tail.
    // Do not publish acceptance while that terminal still cannot be delivered.
    if empty
        || state
            .projector
            .as_ref()
            .is_some_and(|projector| projector.awaiting_responses_eof())
    {
        return Ok(PrecommitClassification::pending());
    }
    Ok(PrecommitClassification::classified(
        ClassifiedAttemptResult {
            facts: ProviderClassificationFacts {
                http_status: Some(status),
                readiness: label("response_body_ready"),
                retryability: RetryabilityFact::NonRetryable,
                ..ProviderClassificationFacts::default()
            },
            readiness: take_readiness(state, status, "text/event-stream", false)?,
        },
    ))
}

fn project_prefix(
    state: &mut ProductionAttemptState,
    bytes: &[u8],
    end_stream: bool,
) -> Result<Option<ModelError>, Arc<str>> {
    let units = state
        .projector
        .as_mut()
        .ok_or_else(|| Arc::from("native stream projector is unavailable"))?
        .feed(bytes, end_stream)
        .map_err(|error| response_diagnostics::adapter(FailureStage::NativeProjection, error))?;
    for unit in units {
        state.semantic_seen |= unit.semantic;
        if let Some(outcome) = unit.terminal {
            state.semantic_terminal = Some(native_terminal(outcome));
        }
        if let Some(error) = unit.failure {
            state.semantic_terminal = Some(SemanticTerminalOutcome::Failed);
            return Ok(Some(error));
        }
        push_native_prefix_unit(state, unit.bytes, unit.terminal.is_some())?;
    }
    Ok(None)
}

fn decode_prefix(
    state: &mut ProductionAttemptState,
    bytes: &[u8],
    end_stream: bool,
) -> Result<Option<ModelError>, Arc<str>> {
    let decoder = state
        .decoder
        .as_mut()
        .ok_or_else(|| Arc::from("native stream decoder is unavailable"))?;
    let renderer = state
        .renderer
        .as_mut()
        .ok_or_else(|| Arc::from("client stream renderer is unavailable"))?;
    let mut status = decoder
        .feed(bytes, end_stream)
        .map_err(|error| response_diagnostics::adapter(FailureStage::Decode, error))?;
    let mut output = Vec::new();
    loop {
        for event in decoder.take_events() {
            state.semantic_seen |= event.is_semantic_output();
            observe_semantic_terminal(&mut state.semantic_terminal, &event.event);
            // Render even a failed unit into the private prefix to retain usage
            // already reported in this chunk. Nothing is delivered until accept.
            for rendered in renderer
                .push(&event)
                .map_err(|error| response_diagnostics::adapter(FailureStage::Render, error))?
            {
                output.extend_from_slice(
                    &rendered
                        .wire_bytes()
                        .map_err(|_| Arc::from("stream prefix serialization failed"))?,
                );
            }
            if let ModelEvent::ResponseFailed { error } = event.event {
                return Ok(Some(error));
            }
        }
        note_response_reasoning_loss(renderer.take_reasoning_loss());
        if status != adapters::ResponseDecodeStatus::NeedDrain {
            break;
        }
        status = decoder
            .resume()
            .map_err(|_| Arc::from("stream prefix drain failed"))?;
    }
    let terminal = status == adapters::ResponseDecodeStatus::Terminal;
    if terminal && !end_stream {
        finish_native_stream_on_terminal(&mut state.decoder)?;
    } else if end_stream {
        state
            .decoder
            .take()
            .expect("prefix decoder is present")
            .finish()
            .map_err(|_| Arc::from("stream prefix ended without terminal"))?;
    }
    push_native_prefix_unit(state, output, terminal || end_stream)?;
    Ok(None)
}
