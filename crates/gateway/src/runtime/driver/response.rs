#[path = "response_prefix.rs"]
mod prefix;

use super::response_diagnostics;
use hiroute_diagnostics::event::{
    ResponseFailureReason as FailureReason, ResponseFailureStage as FailureStage,
};
use std::sync::Arc;

use hiroute_gateway_core::core::filter::LocalReply;
use hiroute_gateway_core::runtime::attempt::{
    ChargedResponseHead, Disposition, PrecommitEvent, PublishedDisposition,
};
use hiroute_gateway_core::runtime::body::{
    BodyMetadataOwner, ChargedBodyQueue, ChargedBytes, MemoryRole, StreamBudget,
};
use hiroute_gateway_core::runtime::driver::{
    AcceptedBodyFrame, ClassifiedAttemptResult, NormalizedAttemptLocalReply,
    PrecommitClassification, ProviderAcceptedEvent, ProviderAttemptCompletion,
    ProviderClassificationFacts, RetryabilityFact, SseTransformSources, UpstreamSideEffectSnapshot,
    UsageDimension, UsageFact,
};
use hiroute_gateway_core::runtime::sse::{EncodedOutputUnit, SemanticProvenance};
use hiroute_gateway_core::transport::GatewayResponseHead;
use http::header::CONTENT_TYPE;
use http::{HeaderMap, HeaderValue, StatusCode};

use crate::attempt_outcome::{
    AttemptFailure, AttemptFailureClass, ConnectorErrorProfile, ProviderFailureKind,
    RawAttemptFailure, classify_failure,
};
use crate::server::core_runtime::adapters::{self, RenderedClientResponse};
use crate::server::core_runtime::model_ir::{
    FinishReason, ModelError, ModelEvent, ModelResponseIRV1, ModelUsage,
};
use crate::server::core_runtime::profiles::CandidateProtocolProfile;
use crate::server::request_plan::IngressProtocol;

use super::{
    MATERIALIZATION_PROTOCOL_FAILED, ProductionAttemptState, ProductionDecodedSse,
    ProductionReadiness, SemanticTerminalOutcome, label, safe_error,
};

fn note_response_reasoning_loss(removed: usize) {
    if removed > 0
        && let Some(observation) = crate::server::core_runtime::observation::active_request()
    {
        observation.reasoning_cleanup(
            hiroute_diagnostics::event::ReasoningCleanupReason::ResponseProtocolProjection,
            removed,
            None,
        );
    }
}

pub(super) fn classify_precommit(
    state: &mut ProductionAttemptState,
    event: PrecommitEvent,
) -> Result<PrecommitClassification<ProductionReadiness, ProductionDecodedSse>, Arc<str>> {
    match event {
        PrecommitEvent::ResponseHead(head) if head.status().is_informational() => {
            Ok(PrecommitClassification::pending())
        }
        PrecommitEvent::ResponseHead(head) => {
            if state.response_status.is_some() {
                return Err(Arc::from(
                    "provider emitted more than one final response head",
                ));
            }
            let status = head.status();
            state.response_status = Some(status);
            state.retry_after = trusted_retry_after(&state.profile, &head);
            if validate_native_content_type(&head, state.streaming && status.is_success()).is_err()
            {
                let raw = response_decode_failure(status, state.retry_after);
                return classify_state_failure(state, raw, response_decode_local_status(status));
            }
            if matches!(status.as_u16(), 401 | 403 | 500..=599) {
                return classify_state_failure(
                    state,
                    RawAttemptFailure::Http {
                        status: status.as_u16(),
                        kind: None,
                        retry_after: state.retry_after,
                    },
                    status,
                );
            }
            if state.native_output && status.is_success() {
                state.projector = Some(Box::new(
                    adapters::NativeResponseProjector::new_for_attempt(
                        &state.profile,
                        state.streaming,
                        state.served_model_alias.clone(),
                        state.chat_tool_projection.take(),
                        state.budget.clone(),
                    )
                    .map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?,
                ));
            } else {
                state.decoder = Some(
                    adapters::NativeResponseDecoder::new_for_attempt(
                        &state.profile,
                        status.as_u16(),
                        state.streaming && status.is_success(),
                        state.chat_tool_projection.take(),
                        state.budget.clone(),
                    )
                    .map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?,
                );
            }
            Ok(PrecommitClassification::pending())
        }
        PrecommitEvent::Body(bytes) => {
            let status = state
                .response_status
                .ok_or_else(|| Arc::from("provider body arrived before final response head"))?;
            if state.streaming && status.is_success() {
                return prefix::classify_stream_prefix(state, bytes.bytes(), false);
            }
            if state.native_output && status.is_success() {
                let projected = state
                    .projector
                    .as_mut()
                    .ok_or_else(|| Arc::from("native response projector is unavailable"))?
                    .feed(bytes.bytes(), false);
                drop(bytes);
                if projected.is_err() {
                    let raw = response_decode_failure(status, state.retry_after);
                    return classify_state_failure(
                        state,
                        raw,
                        response_decode_local_status(status),
                    );
                }
                Ok(PrecommitClassification::pending())
            } else {
                let decoded = state
                    .decoder
                    .as_mut()
                    .ok_or_else(|| Arc::from("native response decoder is unavailable"))?
                    .feed(bytes.bytes(), false);
                drop(bytes);
                if decoded.is_err() {
                    let raw = response_decode_failure(status, state.retry_after);
                    return classify_state_failure(
                        state,
                        raw,
                        response_decode_local_status(status),
                    );
                }
                Ok(PrecommitClassification::pending())
            }
        }
        PrecommitEvent::SseEvent { .. } => Err(Arc::from(
            "native SSE must enter through the profile decoder",
        )),
        PrecommitEvent::EndStream => {
            let status = state
                .response_status
                .ok_or_else(|| Arc::from("provider ended before final response head"))?;
            if state.streaming && status.is_success() {
                return prefix::classify_stream_prefix(state, &[], true);
            }
            if state.native_output && status.is_success() {
                let units = match state
                    .projector
                    .as_mut()
                    .ok_or_else(|| Arc::from("native response projector is unavailable"))?
                    .feed(&[], true)
                {
                    Ok(units) => units,
                    Err(_) => {
                        return classify_state_failure(
                            state,
                            RawAttemptFailure::Protocol,
                            StatusCode::BAD_GATEWAY,
                        );
                    }
                };
                let mut failure = None;
                for unit in units {
                    state.semantic_seen |= unit.semantic;
                    if let Some(outcome) = unit.terminal {
                        state.semantic_terminal = Some(native_terminal(outcome));
                    }
                    failure = failure.or(unit.failure);
                    push_native_prefix_unit(state, unit.bytes, unit.terminal.is_some())?;
                }
                if let Some(error) = failure {
                    return classify_model_error(state, error, status);
                }
                if known_terminal_failure(state.semantic_terminal) {
                    return classify_state_failure(
                        state,
                        RawAttemptFailure::Protocol,
                        StatusCode::BAD_GATEWAY,
                    );
                }
                if !state.semantic_seen || state.semantic_terminal.is_none() {
                    return classify_state_failure(
                        state,
                        RawAttemptFailure::Protocol,
                        StatusCode::BAD_GATEWAY,
                    );
                }
                Ok(PrecommitClassification::classified(
                    ClassifiedAttemptResult {
                        facts: semantic_response_facts(status),
                        readiness: take_readiness(
                            state,
                            StatusCode::OK,
                            "application/json",
                            false,
                        )?,
                    },
                ))
            } else {
                let decoded = match state
                    .decoder
                    .as_mut()
                    .ok_or_else(|| Arc::from("native response decoder is unavailable"))?
                    .feed(&[], true)
                    .and_then(|_| {
                        state
                            .decoder
                            .take()
                            .expect("decoder exists through nonstream EOF")
                            .finish()
                    }) {
                    Ok(decoded) => decoded,
                    Err(_) => {
                        let raw = response_decode_failure(status, state.retry_after);
                        return classify_state_failure(
                            state,
                            raw,
                            response_decode_local_status(status),
                        );
                    }
                };
                state.semantic_terminal = semantic_terminal_for_response(&decoded.response);
                if known_terminal_failure(state.semantic_terminal) {
                    let mut classified = if let Some(error) = decoded.response.error.clone() {
                        classify_model_error(state, error, status)?
                    } else {
                        classify_state_failure(
                            state,
                            RawAttemptFailure::Protocol,
                            StatusCode::BAD_GATEWAY,
                        )?
                    };
                    // Nonstream decoding consumes the decoder before rendering.
                    // Preserve its reported usage even when no client body is built.
                    if !decoded.response.usage.is_empty()
                        && let Some(result) = classified.classified.as_mut()
                    {
                        result.facts.usage = Some(model_usage_fact(&decoded.response.usage));
                    }
                    return Ok(classified);
                }
                if !decoded
                    .events
                    .iter()
                    .any(|event| event.is_semantic_output())
                {
                    return classify_state_failure(
                        state,
                        RawAttemptFailure::Protocol,
                        StatusCode::BAD_GATEWAY,
                    );
                }
                let rendered = match adapters::ClientResponseRenderer::render_nonstream_with_profile(
                    &state.client_profile,
                    &state.served_model_alias,
                    &decoded.response,
                ) {
                    Ok(rendered) => rendered,
                    Err(_) => {
                        return classify_state_failure(
                            state,
                            RawAttemptFailure::Protocol,
                            StatusCode::BAD_GATEWAY,
                        );
                    }
                };
                note_response_reasoning_loss(adapters::reasoning_loss::response_loss(
                    &decoded.response,
                    state.client_profile.protocol,
                ));
                let (rendered_status, content_type, bytes) = match rendered_response_parts(rendered)
                {
                    Ok(parts) => parts,
                    Err(_) => {
                        return classify_state_failure(
                            state,
                            RawAttemptFailure::Protocol,
                            StatusCode::BAD_GATEWAY,
                        );
                    }
                };
                push_prefix_bytes(state, bytes)?;
                Ok(PrecommitClassification::classified(
                    ClassifiedAttemptResult {
                        facts: semantic_response_facts(status),
                        readiness: take_readiness(state, rendered_status, content_type, true)?,
                    },
                ))
            }
        }
    }
}

pub(super) fn normalize_attempt_local_reply(
    state: &mut ProductionAttemptState,
    reply: LocalReply,
    upstream_side_effects: UpstreamSideEffectSnapshot,
) -> Result<NormalizedAttemptLocalReply<ProductionReadiness>, Arc<str>> {
    Ok(NormalizedAttemptLocalReply {
        classified: ClassifiedAttemptResult {
            facts: ProviderClassificationFacts {
                error_class: Some(label("filter_local_reply")),
                retryability: RetryabilityFact::NonRetryable,
                readiness: label("local_reply"),
                ..ProviderClassificationFacts::default()
            },
            readiness: take_readiness(state, reply.status, "application/json", true)?,
        },
        reply,
        upstream_side_effects,
    })
}

pub(super) fn finalize_attempt_facts(
    state: &ProductionAttemptState,
    readiness: Option<&ProductionReadiness>,
    published_facts: Option<&ProviderClassificationFacts>,
    completion: &ProviderAttemptCompletion,
) -> ProviderClassificationFacts {
    let mut facts = published_facts.cloned().unwrap_or_default();
    if let Some(outcome) = readiness
        .and_then(|readiness| readiness.semantic_terminal)
        .or(state.semantic_terminal)
    {
        // A prebody failure keeps the classification that authorized relay.
        // Only a failure discovered after acceptance closes retryability here.
        if matches!(
            outcome,
            SemanticTerminalOutcome::Failed | SemanticTerminalOutcome::Incomplete
        ) && state.classified_failure.is_none()
        {
            facts.error_class = Some(label("provider_rejected"));
            facts.retryability = RetryabilityFact::NonRetryable;
        }
        facts.model_event = Some(label(match outcome {
            SemanticTerminalOutcome::Complete => "response_complete",
            SemanticTerminalOutcome::Incomplete => "response_incomplete",
            SemanticTerminalOutcome::Failed => "response_failed",
            SemanticTerminalOutcome::Unknown => "response_unknown",
        }));
    }
    let usage = readiness
        .and_then(|readiness| readiness.projector.as_ref())
        .map(|projector| projector.usage())
        .or_else(|| state.projector.as_ref().map(|projector| projector.usage()))
        .or_else(|| {
            readiness
                .and_then(|r| r.renderer.as_ref())
                .map(|r| r.usage())
        })
        .or_else(|| state.renderer.as_ref().map(|r| r.usage()));
    if let Some(usage) = usage.filter(|usage| !usage.is_empty()) {
        facts.usage = Some(model_usage_fact(usage));
    }
    facts.ended_at = Some(completion.ended_at);
    facts
}

pub(super) fn accepted_response_head(
    readiness: &ProductionReadiness,
    published: &PublishedDisposition,
) -> Result<GatewayResponseHead, Arc<str>> {
    match published.disposition {
        Disposition::Accept | Disposition::Terminate => {
            let mut headers = HeaderMap::new();
            headers.insert(
                CONTENT_TYPE,
                HeaderValue::from_static(readiness.content_type),
            );
            Ok(GatewayResponseHead {
                status: readiness.response_status,
                headers,
            })
        }
        Disposition::Continue => Err(Arc::from("Continue cannot emit a final response")),
    }
}

pub(super) fn encode_accepted_event(
    readiness: &mut ProductionReadiness,
    event: ProviderAcceptedEvent<ProductionDecodedSse>,
) -> Result<Option<AcceptedBodyFrame>, Arc<str>> {
    let event = match event {
        ProviderAcceptedEvent::Terminal {
            body, provenance, ..
        } => {
            let body = readiness.terminal_body.take().or(body);
            let output = body
                .map(|bytes| {
                    bytes
                        .transfer_role(MemoryRole::OutputQueue)
                        .map(|bytes| EncodedOutputUnit { bytes, provenance })
                })
                .transpose()
                .map_err(safe_error)?;
            return Ok(Some(AcceptedBodyFrame {
                output,
                end_stream: true,
                sse_sources: SseTransformSources::default(),
                queue_metadata: BodyMetadataOwner::default(),
            }));
        }
        ProviderAcceptedEvent::Raw(event) => event,
        ProviderAcceptedEvent::DecodedSse {
            decoded,
            provenance,
            ..
        } => {
            return Ok(Some(AcceptedBodyFrame {
                output: Some(EncodedOutputUnit {
                    bytes: decoded
                        .bytes
                        .transfer_role(MemoryRole::OutputQueue)
                        .map_err(safe_error)?,
                    provenance,
                }),
                end_stream: decoded.end_stream,
                sse_sources: SseTransformSources::default(),
                queue_metadata: BodyMetadataOwner::default(),
            }));
        }
    };
    match event {
        PrecommitEvent::Body(bytes) if readiness.streaming => {
            let (rendered, end_stream) = if readiness.projector.is_some() {
                project_native_chunk_readiness(readiness, bytes.bytes(), false)?
            } else {
                decode_stream_chunk_readiness(readiness, bytes.bytes(), false)?
            };
            drop(bytes);
            queue_accepted_stream_output(readiness, rendered, end_stream)
        }
        PrecommitEvent::EndStream
            if readiness.decoder.is_none() && readiness.projector.is_none() =>
        {
            Ok(Some(AcceptedBodyFrame {
                output: None,
                end_stream: true,
                sse_sources: SseTransformSources::default(),
                queue_metadata: BodyMetadataOwner::default(),
            }))
        }
        PrecommitEvent::EndStream if readiness.streaming => {
            let (rendered, projected_terminal) = if readiness.projector.is_some() {
                project_native_chunk_readiness(readiness, &[], true)?
            } else {
                let (rendered, _) = decode_stream_chunk_readiness(readiness, &[], true)?;
                readiness
                    .decoder
                    .take()
                    .ok_or_else(|| Arc::from("accepted native decoder is unavailable"))?
                    .finish()
                    .map_err(|_| Arc::from("accepted native stream ended without terminal"))?;
                (rendered, true)
            };
            queue_accepted_stream_output(readiness, rendered, projected_terminal)
        }
        PrecommitEvent::Body(_) | PrecommitEvent::EndStream => Err(Arc::from(
            "nonstream response had an unclassified transport tail",
        )),
        PrecommitEvent::ResponseHead(_) => Ok(None),
        PrecommitEvent::SseEvent { .. } => Err(Arc::from("raw SSE bypassed decoder ownership")),
    }
}

fn queue_accepted_stream_output(
    readiness: &mut ProductionReadiness,
    rendered: Option<Vec<u8>>,
    terminal: bool,
) -> Result<Option<AcceptedBodyFrame>, Arc<str>> {
    if let Some(bytes) = rendered.filter(|bytes| !bytes.is_empty()) {
        push_queue_bytes(&mut readiness.prefix, &readiness.budget, bytes)?;
    }
    if !readiness.prefix.is_empty() {
        if terminal {
            readiness.prefix_terminal_chunks = Some(readiness.prefix.len());
        }
        // The core drains one bounded prefix chunk before reading the next
        // upstream event. An SSE event can exceed the accepted plan's 64 KiB
        // transport frame even though it is within the SSE event limit.
        return Ok(None);
    }
    Ok(terminal.then_some(AcceptedBodyFrame {
        output: None,
        end_stream: true,
        sse_sources: SseTransformSources::default(),
        queue_metadata: BodyMetadataOwner::default(),
    }))
}

pub(super) fn take_accepted_prefix(
    readiness: &mut ProductionReadiness,
) -> Option<ProviderAcceptedEvent<ProductionDecodedSse>> {
    // Responses certifies its terminal only after checking the optional native
    // tail. Keep that terminal in the existing budgeted queue until its final
    // write can carry EOS too: native clients exit as soon as they see it.
    if readiness
        .projector
        .as_ref()
        .is_some_and(|p| p.awaiting_responses_eof())
    {
        return None;
    }
    if let Some(bytes) = readiness.prefix.pop_front() {
        let end_stream = readiness
            .prefix_terminal_chunks
            .as_mut()
            .is_some_and(|remaining| {
                *remaining = remaining.saturating_sub(1);
                *remaining == 0
            });
        if end_stream {
            readiness.prefix_terminal_chunks = None;
        }
        return Some(ProviderAcceptedEvent::DecodedSse {
            sequence: 0,
            decoded: ProductionDecodedSse { bytes, end_stream },
            provenance: SemanticProvenance::ProducesSemantic,
        });
    }
    if readiness.prefix_eos_pending {
        readiness.prefix_eos_pending = false;
        return Some(ProviderAcceptedEvent::Raw(PrecommitEvent::EndStream));
    }
    None
}

pub(super) fn connector_error_profile(
    profile: &CandidateProtocolProfile,
) -> Result<ConnectorErrorProfile, Arc<str>> {
    if !sealed_connector_semantics(profile) {
        return Err(Arc::from(MATERIALIZATION_PROTOCOL_FAILED));
    }
    let errors = profile
        .connector
        .errors
        .exact()
        .ok_or_else(|| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?;
    Ok(ConnectorErrorProfile {
        http_status_typed: errors.http_status_typed,
        stream_error_typed: errors.sse_error_typed,
        retry_after_typed: errors.retry_after_header.is_some(),
    })
}

fn response_decode_failure(
    status: StatusCode,
    retry_after: Option<std::time::Duration>,
) -> RawAttemptFailure {
    if status.is_success() {
        RawAttemptFailure::Protocol
    } else {
        RawAttemptFailure::Http {
            status: status.as_u16(),
            kind: None,
            retry_after,
        }
    }
}

fn response_decode_local_status(status: StatusCode) -> StatusCode {
    if status.is_success() {
        StatusCode::BAD_GATEWAY
    } else {
        status
    }
}

fn validate_native_content_type(
    head: &ChargedResponseHead,
    streaming: bool,
) -> Result<(), Arc<str>> {
    let content_type = head
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    let expected = if streaming {
        "text/event-stream"
    } else {
        "application/json"
    };
    if content_type.is_some_and(|value| value.eq_ignore_ascii_case(expected)) {
        Ok(())
    } else {
        Err(Arc::from(
            "provider response content type is incompatible with frozen profile",
        ))
    }
}

fn trusted_retry_after(
    profile: &CandidateProtocolProfile,
    head: &ChargedResponseHead,
) -> Option<std::time::Duration> {
    let header = profile
        .connector
        .errors
        .exact()?
        .retry_after_header
        .as_deref()?;
    head.headers()
        .get(header)?
        .to_str()
        .ok()?
        .parse::<u64>()
        .ok()
        .map(std::time::Duration::from_secs)
}

fn sanitized_provider_kind(
    profile: &CandidateProtocolProfile,
    error: &ModelError,
) -> Option<ProviderFailureKind> {
    if !sealed_connector_semantics(profile) {
        return None;
    }
    let code = error.code.as_deref()?.to_ascii_lowercase();
    // Preserve a precise cause without guessing one for generic InvalidParameter.
    // Relay eligibility is independent of this diagnostic classification.
    if code == "invalidparameter"
        && error
            .message
            .as_deref()
            .is_some_and(reasoning_budget_rejected)
    {
        return Some(ProviderFailureKind::Protocol);
    }
    if code == "invalid_encrypted_content" {
        return Some(ProviderFailureKind::ReasoningHistory);
    }
    if profile.capability.upstream_protocol == IngressProtocol::Messages
        && code == "invalid_request_error"
        && error.message.as_deref().is_some_and(|message| {
            let message = message.to_ascii_lowercase();
            message.contains("invalid `signature` in `thinking` block")
                || message.contains("invalid signature in thinking block")
                || message.contains("expected `thinking` or `redacted_thinking`, but found")
        })
    {
        return Some(ProviderFailureKind::ReasoningHistory);
    }
    match profile.capability.upstream_protocol {
        IngressProtocol::Responses | IngressProtocol::ChatCompletions => match code.as_str() {
            "invalid_api_key" | "authentication_error" | "unauthorized" => {
                Some(ProviderFailureKind::Credential)
            }
            "insufficient_quota" | "quota_exceeded" => Some(ProviderFailureKind::Quota),
            "rate_limit_exceeded" | "server_overloaded" => {
                Some(ProviderFailureKind::BindingOverload)
            }
            "invalid_request_error" | "invalid_request" => {
                Some(ProviderFailureKind::PermanentClient)
            }
            "protocol_error" => Some(ProviderFailureKind::Protocol),
            "server_error" | "internal_error" => Some(ProviderFailureKind::Transient),
            _ => None,
        },
        IngressProtocol::Messages => match code.as_str() {
            "authentication_error" | "permission_error" => Some(ProviderFailureKind::Credential),
            "rate_limit_error" | "overloaded_error" => Some(ProviderFailureKind::BindingOverload),
            "invalid_request_error" | "not_found_error" => {
                Some(ProviderFailureKind::PermanentClient)
            }
            "api_error" => Some(ProviderFailureKind::Transient),
            _ => None,
        },
    }
}

fn reasoning_budget_rejected(message: &str) -> bool {
    const PREFIX: &str =
        "The thinking_budget parameter must be a positive integer and not greater than ";
    message
        .strip_prefix("<400> InternalError.Algo.InvalidParameter: ")
        .unwrap_or(message)
        .strip_prefix(PREFIX)
        .is_some_and(|limit| {
            !limit.is_empty()
                && limit.len() <= 10
                && limit.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn sealed_connector_semantics(profile: &CandidateProtocolProfile) -> bool {
    profile.schema_version == "hiroute.candidate-protocol-profile/v1"
        && profile.capability.schema_version == "hiroute.candidate-capability/v1"
        && profile.connector.schema_version == "hiroute.connector-profile/v1"
        && !profile.adapter_revision.trim().is_empty()
        && !profile.serializer_revision.trim().is_empty()
        && !profile.decoder_revision.trim().is_empty()
        && profile.capability.upstream_protocol == profile.connector.upstream_protocol
        && profile.connector.critical_facts_are_exact()
        && profile.exact_provider_path().is_ok()
        && profile
            .selected_reasoning()
            .is_ok_and(|reasoning| reasoning.validate_for(profile.capability.upstream_protocol))
}

fn classify_model_error(
    state: &mut ProductionAttemptState,
    error: ModelError,
    response_status: StatusCode,
) -> Result<PrecommitClassification<ProductionReadiness, ProductionDecodedSse>, Arc<str>> {
    response_diagnostics::model_error(&error, Some(response_status.as_u16()));
    let kind = sanitized_provider_kind(&state.profile, &error);
    let status = error.status.unwrap_or(response_status.as_u16());
    let raw = RawAttemptFailure::Http {
        status,
        kind,
        retry_after: state.retry_after,
    };
    let local_status = StatusCode::from_u16(status)
        .ok()
        .filter(|status| !status.is_success())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    classify_state_failure(state, raw, local_status)
}

fn classify_state_failure(
    state: &mut ProductionAttemptState,
    raw: RawAttemptFailure,
    local_status: StatusCode,
) -> Result<PrecommitClassification<ProductionReadiness, ProductionDecodedSse>, Arc<str>> {
    let failure = classify_failure(&raw, connector_error_profile(&state.profile)?);
    let facts = failure_facts(&failure);
    state.classified_failure = Some(failure);
    let body = serde_json::to_vec(&serde_json::json!({
        "error": {
            "type": "upstream_error",
            "code": "UPSTREAM_ATTEMPT_FAILED"
        }
    }))
    .map_err(|_| Arc::from("sanitized error serialization failed"))?;
    let terminal_body =
        ChargedBytes::from_exact_vec(&state.budget, MemoryRole::ResponsePrefix, body)
            .map_err(safe_error)?;
    if let Some(prefix) = state.prefix.as_mut() {
        prefix.clear_and_release();
    }
    state.prefix_terminal_chunks = None;
    let mut readiness = take_readiness(state, local_status, "application/json", true)?;
    readiness.terminal_body = Some(terminal_body);
    Ok(PrecommitClassification::classified(
        ClassifiedAttemptResult { facts, readiness },
    ))
}

fn semantic_response_facts(status: StatusCode) -> ProviderClassificationFacts {
    ProviderClassificationFacts {
        http_status: Some(status),
        readiness: label("semantic_response"),
        model_event: Some(label("semantic_response")),
        retryability: RetryabilityFact::NonRetryable,
        ..ProviderClassificationFacts::default()
    }
}

fn take_readiness(
    state: &mut ProductionAttemptState,
    response_status: StatusCode,
    content_type: &'static str,
    prefix_eos_pending: bool,
) -> Result<ProductionReadiness, Arc<str>> {
    Ok(ProductionReadiness {
        response_status,
        content_type,
        prefix: state
            .prefix
            .take()
            .ok_or_else(|| Arc::from("response prefix owner was already transferred"))?,
        terminal_body: None,
        decoder: state.decoder.take(),
        renderer: state.renderer.take(),
        projector: state.projector.take(),
        _chat_tool_projection_budget: state.chat_tool_projection_budget.take(),
        budget: state.budget.clone(),
        streaming: state.streaming,
        prefix_eos_pending,
        prefix_terminal_chunks: state.prefix_terminal_chunks.take(),
        semantic_terminal: state.semantic_terminal,
    })
}

fn project_native_chunk_readiness(
    readiness: &mut ProductionReadiness,
    bytes: &[u8],
    end_stream: bool,
) -> Result<(Option<Vec<u8>>, bool), Arc<str>> {
    let units = readiness
        .projector
        .as_mut()
        .ok_or_else(|| Arc::from("accepted native projector is unavailable"))?
        .feed(bytes, end_stream)
        .map_err(|error| response_diagnostics::adapter(FailureStage::NativeProjection, error))?;
    let mut output = Vec::new();
    let mut terminal = false;
    for unit in units {
        if terminal {
            return Err(Arc::from("native stream emitted bytes after terminal"));
        }
        if let Some(error) = &unit.failure {
            response_diagnostics::model_error(error, Some(readiness.response_status.as_u16()));
            response_diagnostics::note(
                FailureStage::ProviderStream,
                FailureReason::ProviderRejected,
            );
        }
        if let Some(outcome) = unit.terminal {
            readiness.semantic_terminal = Some(native_terminal(outcome));
            terminal = true;
        }
        output.extend_from_slice(&unit.bytes);
    }
    Ok(((!output.is_empty()).then_some(output), terminal))
}

fn push_native_prefix_unit(
    state: &mut ProductionAttemptState,
    bytes: Vec<u8>,
    terminal: bool,
) -> Result<(), Arc<str>> {
    if state.prefix_terminal_chunks.is_some() {
        response_diagnostics::note(FailureStage::PrefixBuffer, FailureReason::InvalidLifecycle);
        return Err(Arc::from("native response emitted output after terminal"));
    }
    push_prefix_bytes(state, bytes)?;
    if terminal {
        let chunks = state
            .prefix
            .as_ref()
            .ok_or_else(|| Arc::from("response prefix owner is unavailable"))?
            .len();
        if chunks == 0 {
            response_diagnostics::note(FailureStage::PrefixBuffer, FailureReason::InvalidLifecycle);
            return Err(Arc::from("native terminal produced no wire bytes"));
        }
        state.prefix_terminal_chunks = Some(chunks);
    }
    Ok(())
}

fn native_terminal(outcome: adapters::NativeTerminalOutcome) -> SemanticTerminalOutcome {
    match outcome {
        adapters::NativeTerminalOutcome::Complete => SemanticTerminalOutcome::Complete,
        adapters::NativeTerminalOutcome::Incomplete => SemanticTerminalOutcome::Incomplete,
        adapters::NativeTerminalOutcome::Failed => SemanticTerminalOutcome::Failed,
        adapters::NativeTerminalOutcome::Unknown => SemanticTerminalOutcome::Unknown,
    }
}

fn model_usage_fact(usage: &ModelUsage) -> UsageFact {
    fn reported(value: Option<u64>) -> UsageDimension {
        value.map_or_else(UsageDimension::unknown, UsageDimension::reported)
    }
    UsageFact {
        input: reported(usage.input_tokens),
        output: reported(usage.output_tokens),
        billable: UsageDimension::unknown(),
        cache_read: reported(usage.cache_read_tokens),
        cache_write: reported(usage.cache_write_tokens),
        reasoning: reported(usage.reasoning_tokens),
    }
}

fn decode_stream_chunk_readiness(
    readiness: &mut ProductionReadiness,
    bytes: &[u8],
    end_stream: bool,
) -> Result<(Option<Vec<u8>>, bool), Arc<str>> {
    let decoder = readiness
        .decoder
        .as_mut()
        .ok_or_else(|| Arc::from("accepted native decoder is unavailable"))?;
    let renderer = readiness
        .renderer
        .as_mut()
        .ok_or_else(|| Arc::from("accepted client renderer is unavailable"))?;
    let mut status = decoder
        .feed(bytes, end_stream)
        .map_err(|error| response_diagnostics::adapter(FailureStage::Decode, error))?;
    let mut output = Vec::new();
    loop {
        for event in decoder.take_events() {
            observe_semantic_terminal(&mut readiness.semantic_terminal, &event.event);
            if let ModelEvent::ResponseFailed { error } = &event.event {
                response_diagnostics::model_error(error, Some(readiness.response_status.as_u16()));
                response_diagnostics::note(
                    FailureStage::ProviderStream,
                    FailureReason::ProviderRejected,
                );
            }
            for rendered in renderer
                .push(&event)
                .map_err(|error| response_diagnostics::adapter(FailureStage::Render, error))?
            {
                output.extend_from_slice(
                    &rendered
                        .wire_bytes()
                        .map_err(|_| Arc::from("accepted stream serialization failed"))?,
                );
            }
        }
        note_response_reasoning_loss(renderer.take_reasoning_loss());
        if status != adapters::ResponseDecodeStatus::NeedDrain {
            break;
        }
        status = decoder
            .resume()
            .map_err(|_| Arc::from("accepted native stream drain failed"))?;
    }
    let terminal = status == adapters::ResponseDecodeStatus::Terminal;
    if terminal && !end_stream {
        finish_native_stream_on_terminal(&mut readiness.decoder)?;
    }
    Ok(((!output.is_empty()).then_some(output), terminal))
}

fn finish_native_stream_on_terminal(
    decoder: &mut Option<adapters::NativeResponseDecoder>,
) -> Result<(), Arc<str>> {
    let active = decoder
        .as_mut()
        .ok_or_else(|| Arc::from("native stream decoder is unavailable"))?;
    if active
        .feed(&[], true)
        .map_err(|_| Arc::from("native stream has a malformed terminal tail"))?
        != adapters::ResponseDecodeStatus::Terminal
        || !active.take_events().is_empty()
    {
        return Err(Arc::from("native stream has events after terminal"));
    }
    decoder
        .take()
        .expect("terminal decoder is present")
        .finish()
        .map_err(|_| Arc::from("native stream terminal is incomplete"))?;
    Ok(())
}

fn known_terminal_failure(terminal: Option<SemanticTerminalOutcome>) -> bool {
    matches!(
        terminal,
        Some(SemanticTerminalOutcome::Failed | SemanticTerminalOutcome::Incomplete)
    )
}

fn semantic_terminal_for_response(response: &ModelResponseIRV1) -> Option<SemanticTerminalOutcome> {
    if response.error.is_some() {
        Some(SemanticTerminalOutcome::Failed)
    } else {
        response
            .finish_reason
            .as_ref()
            .map(semantic_terminal_for_finish_reason)
    }
}

fn observe_semantic_terminal(current: &mut Option<SemanticTerminalOutcome>, event: &ModelEvent) {
    match event {
        ModelEvent::FinishReason { reason } => {
            *current = Some(semantic_terminal_for_finish_reason(reason));
        }
        ModelEvent::ResponseFailed { .. } => {
            *current = Some(SemanticTerminalOutcome::Failed);
        }
        _ => {}
    }
}

fn semantic_terminal_for_finish_reason(reason: &FinishReason) -> SemanticTerminalOutcome {
    match reason {
        FinishReason::Stop | FinishReason::ToolCall => SemanticTerminalOutcome::Complete,
        FinishReason::Length
        | FinishReason::Refusal
        | FinishReason::Cancelled
        | FinishReason::Other(_) => SemanticTerminalOutcome::Incomplete,
    }
}

fn push_prefix_bytes(state: &mut ProductionAttemptState, bytes: Vec<u8>) -> Result<(), Arc<str>> {
    let prefix = state
        .prefix
        .as_mut()
        .ok_or_else(|| Arc::from("response prefix owner is unavailable"))?;
    push_queue_bytes(prefix, &state.budget, bytes).inspect_err(|_| {
        response_diagnostics::note(FailureStage::PrefixBuffer, FailureReason::ResourceLimit);
    })
}

fn push_queue_bytes(
    queue: &mut ChargedBodyQueue,
    budget: &StreamBudget,
    bytes: Vec<u8>,
) -> Result<(), Arc<str>> {
    if bytes.is_empty() {
        return Ok(());
    }
    let quantum = queue.max_chunk_bytes();
    for chunk in bytes.chunks(quantum) {
        queue
            .push_back_growing(
                budget,
                ChargedBytes::copy_from_opaque(budget, MemoryRole::ResponsePrefix, chunk)
                    .map_err(safe_error)?,
            )
            .map_err(safe_error)?;
    }
    Ok(())
}

fn rendered_response_parts(
    rendered: RenderedClientResponse,
) -> Result<(StatusCode, &'static str, Vec<u8>), Arc<str>> {
    match rendered {
        RenderedClientResponse::Json {
            status,
            content_type,
            bytes,
            ..
        } => Ok((
            StatusCode::from_u16(status)
                .map_err(|_| Arc::from("rendered response status is invalid"))?,
            content_type,
            bytes,
        )),
        RenderedClientResponse::Sse { .. } => {
            Err(Arc::from("nonstream renderer returned an SSE response"))
        }
    }
}

pub(super) fn failure_facts(failure: &AttemptFailure) -> ProviderClassificationFacts {
    ProviderClassificationFacts {
        error_class: Some(label(match failure.class {
            AttemptFailureClass::ReasoningHistory => "reasoning_history",
            AttemptFailureClass::Credential => "credential",
            AttemptFailureClass::Quota => "quota",
            AttemptFailureClass::BindingOverload => "binding_overload",
            AttemptFailureClass::Protocol => "protocol",
            AttemptFailureClass::PermanentClient => "permanent_client",
            AttemptFailureClass::Transient => "transient",
            AttemptFailureClass::Timeout(_) => "timeout",
            AttemptFailureClass::Disconnect => "disconnect",
            AttemptFailureClass::PreOutputStream(_) => "preoutput_stream",
            AttemptFailureClass::PostCommit => "postcommit",
            AttemptFailureClass::Unclassified => "unclassified",
        })),
        retryability: if failure.is_precommit_relayable() {
            RetryabilityFact::Retryable
        } else {
            RetryabilityFact::NonRetryable
        },
        http_status: failure
            .status
            .and_then(|status| StatusCode::from_u16(status).ok()),
        retry_after: failure.retry_after,
        readiness: label("classified_response"),
        ..ProviderClassificationFacts::default()
    }
}

#[cfg(test)]
#[path = "response_terminal_tests.rs"]
mod terminal_tests;

#[cfg(test)]
#[path = "response_budget_tests.rs"]
mod tests;
