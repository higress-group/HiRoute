use std::sync::Arc;

use hiroute_gateway_core::core::filter::LocalReply;
use hiroute_gateway_core::runtime::attempt::{
    ChargedResponseHead, Disposition, PrecommitEvent, PublishedDisposition,
};
use hiroute_gateway_core::runtime::body::{
    BodyMetadataOwner, ChargedBodyQueue, ChargedBytes, MemoryRole, Reservation, StreamBudget,
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
    MATERIALIZATION_PROTOCOL_FAILED, MAX_RENDERED_PRECOMMIT_BYTES, ProductionAttemptState,
    ProductionDecodedSse, ProductionReadiness, SemanticTerminalOutcome, label, safe_error,
};

pub(super) struct PrecommitDecoderBudget {
    budget: StreamBudget,
    raw_bytes: usize,
    charged_bytes: usize,
    charges: Vec<Reservation>,
    _metadata: Reservation,
}

impl PrecommitDecoderBudget {
    const MAX_CHARGE_STEPS: usize = usize::BITS as usize + 1;

    pub(super) fn new(budget: &StreamBudget) -> Result<Self, Arc<str>> {
        let metadata_bytes = Self::MAX_CHARGE_STEPS
            .checked_mul(std::mem::size_of::<Reservation>())
            .ok_or_else(|| Arc::from("native decoder metadata budget overflow"))?;
        let metadata = budget
            .reserve(MemoryRole::ResponsePrefix, metadata_bytes)
            .map_err(safe_error)?;
        let charges = Vec::with_capacity(Self::MAX_CHARGE_STEPS);
        Ok(Self {
            budget: budget.clone(),
            raw_bytes: 0,
            charged_bytes: 0,
            charges,
            _metadata: metadata,
        })
    }

    fn charge_frame(&mut self, bytes: usize) -> Result<(), Arc<str>> {
        let raw_bytes = self
            .raw_bytes
            .checked_add(bytes)
            .ok_or_else(|| Arc::from("native precommit decoder byte limit overflow"))?;
        if raw_bytes > MAX_RENDERED_PRECOMMIT_BYTES {
            return Err(Arc::from("native precommit decoder byte limit exceeded"));
        }
        // Vec's amortized retained capacity is strictly below twice the live
        // byte length after its minimum allocation. Shadow-charge that upper
        // bound before the native decoder is allowed to copy this frame.
        let target_charge = if raw_bytes == 0 {
            0
        } else {
            raw_bytes
                .checked_mul(2)
                .and_then(|bytes| bytes.checked_add(8))
                .ok_or_else(|| Arc::from("native precommit decoder charge overflow"))?
        };
        if target_charge > self.charged_bytes {
            // Transport chunk boundaries are arbitrary and must not become a
            // correctness limit. Grow the accounting reservation geometrically
            // so a long sequence of tiny chunks uses bounded metadata while the
            // exact byte ceiling remains enforced.
            let maximum_charge = MAX_RENDERED_PRECOMMIT_BYTES
                .checked_mul(2)
                .and_then(|bytes| bytes.checked_add(8))
                .ok_or_else(|| Arc::from("native precommit decoder charge overflow"))?;
            let next_charge = target_charge
                .checked_next_power_of_two()
                .unwrap_or(maximum_charge)
                .min(maximum_charge);
            self.charges.push(
                self.budget
                    .reserve(MemoryRole::ResponsePrefix, next_charge - self.charged_bytes)
                    .map_err(safe_error)?,
            );
            self.charged_bytes = next_charge;
        }
        self.raw_bytes = raw_bytes;
        Ok(())
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
                    )
                    .map_err(|_| Arc::from(MATERIALIZATION_PROTOCOL_FAILED))?,
                );
            }
            Ok(PrecommitClassification::pending())
        }
        PrecommitEvent::Body(bytes) => {
            state
                .decoder_budget
                .as_mut()
                .ok_or_else(|| Arc::from("native decoder budget owner is unavailable"))?
                .charge_frame(bytes.retained_capacity())?;
            let status = state
                .response_status
                .ok_or_else(|| Arc::from("provider body arrived before final response head"))?;
            if state.streaming && status.is_success() {
                let outcome = match if state.native_output {
                    project_native_chunk_state(state, bytes.bytes(), false)
                } else {
                    decode_stream_chunk_state(state, bytes.bytes(), false)
                } {
                    Ok(outcome) => outcome,
                    Err(_) => {
                        drop(bytes);
                        return classify_state_failure(
                            state,
                            RawAttemptFailure::Protocol,
                            StatusCode::BAD_GATEWAY,
                        );
                    }
                };
                drop(bytes);
                if let Some(error) = outcome.failure {
                    return classify_model_error(state, error, status, true);
                }
                if outcome.semantic {
                    return Ok(PrecommitClassification::classified(
                        ClassifiedAttemptResult {
                            facts: semantic_response_facts(status),
                            readiness: take_readiness(
                                state,
                                StatusCode::OK,
                                "text/event-stream",
                                // Native protocol terminal metadata, rather
                                // than upstream transport EOF, marks the
                                // eventual downstream EOS frame.
                                false,
                            )?,
                        },
                    ));
                }
                Ok(PrecommitClassification::pending())
            } else if state.native_output && status.is_success() {
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
                let outcome = match if state.native_output {
                    project_native_chunk_state(state, &[], true)
                } else {
                    decode_stream_chunk_state(state, &[], true)
                } {
                    Ok(outcome) => outcome,
                    Err(_) => {
                        return classify_state_failure(
                            state,
                            RawAttemptFailure::Protocol,
                            StatusCode::BAD_GATEWAY,
                        );
                    }
                };
                if let Some(error) = outcome.failure {
                    return classify_model_error(state, error, status, true);
                }
                if !state.semantic_seen {
                    return classify_state_failure(
                        state,
                        RawAttemptFailure::Protocol,
                        StatusCode::BAD_GATEWAY,
                    );
                }
                if !state.native_output {
                    state
                        .decoder
                        .take()
                        .ok_or_else(|| Arc::from("native response decoder is unavailable"))?
                        .finish()
                        .map_err(|_| Arc::from("native response terminal is malformed"))?;
                }
                Ok(PrecommitClassification::classified(
                    ClassifiedAttemptResult {
                        facts: semantic_response_facts(status),
                        readiness: take_readiness(
                            state,
                            StatusCode::OK,
                            "text/event-stream",
                            true,
                        )?,
                    },
                ))
            } else if state.native_output && status.is_success() {
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
                    return classify_model_error(state, error, status, false);
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
                if let Some(error) = decoded.response.error.clone() {
                    return classify_model_error(state, error, status, false);
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
        .or_else(|| state.projector.as_ref().map(|projector| projector.usage()));
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
                (
                    decode_stream_chunk_readiness(readiness, bytes.bytes(), false)?,
                    false,
                )
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
                let rendered = decode_stream_chunk_readiness(readiness, &[], true)?;
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
    stream: bool,
) -> Result<PrecommitClassification<ProductionReadiness, ProductionDecodedSse>, Arc<str>> {
    let kind = sanitized_provider_kind(&state.profile, &error);
    let status = error.status.unwrap_or(response_status.as_u16());
    let raw = if stream && response_status.is_success() {
        RawAttemptFailure::Stream {
            kind,
            retry_after: state.retry_after,
        }
    } else {
        RawAttemptFailure::Http {
            status,
            kind,
            retry_after: state.retry_after,
        }
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
        _decoder_budget: state.decoder_budget.take(),
        _chat_tool_projection_budget: state.chat_tool_projection_budget.take(),
        budget: state.budget.clone(),
        streaming: state.streaming,
        prefix_eos_pending,
        prefix_terminal_chunks: state.prefix_terminal_chunks.take(),
        semantic_terminal: state.semantic_terminal,
    })
}

#[derive(Default)]
struct StreamDecodeOutcome {
    semantic: bool,
    terminal: bool,
    failure: Option<ModelError>,
}

fn project_native_chunk_state(
    state: &mut ProductionAttemptState,
    bytes: &[u8],
    end_stream: bool,
) -> Result<StreamDecodeOutcome, Arc<str>> {
    let units = state
        .projector
        .as_mut()
        .ok_or_else(|| Arc::from("native response projector is unavailable"))?
        .feed(bytes, end_stream)
        .map_err(|_| Arc::from("native stream response is malformed"))?;
    let mut outcome = StreamDecodeOutcome::default();
    for unit in units {
        let semantic_before = state.semantic_seen || outcome.semantic;
        if let Some(terminal) = unit.terminal {
            let terminal = native_terminal(terminal);
            state.semantic_terminal = Some(terminal);
            state.terminal_seen = true;
            outcome.terminal = true;
        }
        if unit.failure.is_some() && !semantic_before {
            outcome.failure = unit.failure;
        }
        outcome.semantic |= unit.semantic;
        push_native_prefix_unit(state, unit.bytes, unit.terminal.is_some())?;
    }
    state.semantic_seen |= outcome.semantic;
    Ok(outcome)
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
        .map_err(|_| Arc::from("accepted native stream is malformed"))?;
    let mut output = Vec::new();
    let mut terminal = false;
    for unit in units {
        if terminal {
            return Err(Arc::from("native stream emitted bytes after terminal"));
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

fn decode_stream_chunk_state(
    state: &mut ProductionAttemptState,
    bytes: &[u8],
    end_stream: bool,
) -> Result<StreamDecodeOutcome, Arc<str>> {
    let decoder = state
        .decoder
        .as_mut()
        .ok_or_else(|| Arc::from("native stream decoder is unavailable"))?;
    let renderer = state
        .renderer
        .as_mut()
        .ok_or_else(|| Arc::from("client stream renderer is unavailable"))?;
    let prefix = state
        .prefix
        .as_mut()
        .ok_or_else(|| Arc::from("response prefix owner is unavailable"))?;
    let mut status = decoder
        .feed(bytes, end_stream)
        .map_err(|_| Arc::from("native stream response is malformed"))?;
    let mut outcome = StreamDecodeOutcome::default();
    loop {
        for event in decoder.take_events() {
            observe_semantic_terminal(&mut state.semantic_terminal, &event.event);
            let semantic = event.is_semantic_output();
            if let ModelEvent::ResponseFailed { error } = &event.event
                && !state.semantic_seen
                && !outcome.semantic
            {
                outcome.failure = Some(error.clone());
            }
            for rendered in renderer
                .push(&event)
                .map_err(|_| Arc::from("client stream event is not representable"))?
            {
                push_queue_bytes(
                    prefix,
                    &state.budget,
                    rendered
                        .wire_bytes()
                        .map_err(|_| Arc::from("client stream event serialization failed"))?,
                )?;
            }
            outcome.semantic |= semantic;
            let terminal = matches!(
                event.event,
                ModelEvent::ResponseCompleted { .. } | ModelEvent::ResponseFailed { .. }
            );
            outcome.terminal |= terminal;
            state.terminal_seen |= terminal;
        }
        if status != adapters::ResponseDecodeStatus::NeedDrain {
            break;
        }
        status = decoder
            .resume()
            .map_err(|_| Arc::from("native stream response drain failed"))?;
    }
    state.semantic_seen |= outcome.semantic;
    Ok(outcome)
}

fn decode_stream_chunk_readiness(
    readiness: &mut ProductionReadiness,
    bytes: &[u8],
    end_stream: bool,
) -> Result<Option<Vec<u8>>, Arc<str>> {
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
        .map_err(|_| Arc::from("accepted native stream is malformed"))?;
    let mut output = Vec::new();
    loop {
        for event in decoder.take_events() {
            observe_semantic_terminal(&mut readiness.semantic_terminal, &event.event);
            for rendered in renderer
                .push(&event)
                .map_err(|_| Arc::from("accepted client stream event is not representable"))?
            {
                output.extend_from_slice(
                    &rendered
                        .wire_bytes()
                        .map_err(|_| Arc::from("accepted stream serialization failed"))?,
                );
            }
        }
        if status != adapters::ResponseDecodeStatus::NeedDrain {
            break;
        }
        status = decoder
            .resume()
            .map_err(|_| Arc::from("accepted native stream drain failed"))?;
    }
    Ok((!output.is_empty()).then_some(output))
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
    push_queue_bytes(prefix, &state.budget, bytes)
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
            .push_back(
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
mod tests {
    use super::*;
    use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
    use hiroute_gateway_core::runtime::body::BudgetTree;

    #[test]
    fn precommit_decoder_is_bounded_by_bytes_not_transport_chunk_count() {
        let budget = BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024)
            .unwrap()
            .stream(4 * 1024 * 1024)
            .unwrap();
        let mut decoder = PrecommitDecoderBudget::new(&budget).unwrap();
        for _ in 0..1_000 {
            decoder.charge_frame(128).unwrap();
        }
        assert!(
            decoder.charge_frame(MAX_RENDERED_PRECOMMIT_BYTES).is_err(),
            "the byte ceiling remains authoritative"
        );
    }

    #[test]
    fn connector_error_semantics_are_sealed_by_profile_not_connector_allowlist() {
        let mut profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "gpt-codex",
            fixed_reasoning("fixed"),
        );
        profile.connector.connector_id = "connector.cpa.codex".into();
        assert!(connector_error_profile(&profile).is_ok());

        profile.connector.request_path = "/registered/provider/responses".into();
        assert!(
            connector_error_profile(&profile).is_ok(),
            "a catalog-bound exact provider path need not equal the public ingress path"
        );

        profile.connector.schema_version = "hiroute.connector-profile/v2".into();
        assert!(connector_error_profile(&profile).is_err());
    }
}
