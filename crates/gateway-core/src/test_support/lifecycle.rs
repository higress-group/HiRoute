//! Reusable production-lifecycle fixtures for socket, stability, and
//! benchmark harnesses.
//!
//! These values implement only the #13 selection and #15 provider ports. The
//! gateway under test remains the real `GatewayCoreLifecycle`, filter owner,
//! `AttemptExchange`, and selected transport factory.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use http::header::HOST;
use http::{HeaderValue, StatusCode};

use crate::core::execution_plan::{
    AcceptedResponseExecutionBinding, ConfigEventSnapshot, CredentialRef, ResolvedTargetBindingId,
};
use crate::core::filter::LocalReply;
use crate::runtime::attempt::{
    AcceptBlockedReason, AttemptId, ChargedResponseHead, Disposition, PrecommitEvent,
    PreparedAttemptBody, PreparedAttemptHttpRequest, PreparedRequestHead, PublishedDisposition,
};
use crate::runtime::body::{
    BodyMetadataOwner, BodyPlan, ChargedBodyQueue, ChargedBytes, MemoryRole,
};
use crate::runtime::driver::{
    AcceptedBodyFrame, AttemptBudgetGrant, AttemptMaterializationContext, ClassifiedAttemptResult,
    DecisionSessionPort, DecisionSessionRequest, DecodedSseToken, LogicalRequestBodyFrame,
    LogicalRequestContext, NormalizedAttemptLocalReply, ObservationLabel, PinnedConfigContext,
    PrecommitClassification, ProviderAcceptedEvent, ProviderClassificationFacts,
    ProviderRuntimePort, RealtimeRoutingFacts, RetryabilityFact, RouteDecisionId,
    SelectedGatewayAttempt, SelectionPublicationPort, SelectionRequest, SseTransformSources,
    UpstreamSideEffectSnapshot,
};
use crate::runtime::sse::{EncodedOutputUnit, SemanticProvenance};
use crate::transport::{GatewayRequestHead, GatewayResponseHead};

#[derive(Clone, Debug)]
pub struct BootstrapSelection {
    binding: ResolvedTargetBindingId,
    next_attempt_id: Arc<AtomicU64>,
    selected: Arc<AtomicUsize>,
    published: Arc<AtomicUsize>,
}

impl BootstrapSelection {
    pub fn new(binding: ResolvedTargetBindingId) -> Self {
        Self {
            binding,
            next_attempt_id: Arc::new(AtomicU64::new(1)),
            selected: Arc::new(AtomicUsize::new(0)),
            published: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn selected(&self) -> usize {
        self.selected.load(Ordering::Relaxed)
    }

    pub fn published(&self) -> usize {
        self.published.load(Ordering::Relaxed)
    }
}

pub struct BootstrapDecisionSession {
    binding: ResolvedTargetBindingId,
    route_decision_id: RouteDecisionId,
    overall_deadline: std::time::Instant,
    next_attempt_id: Arc<AtomicU64>,
    selected: Arc<AtomicUsize>,
    published: Arc<AtomicUsize>,
}

impl DecisionSessionPort for BootstrapDecisionSession {
    fn route_decision_id(&self) -> RouteDecisionId {
        self.route_decision_id
    }

    fn snapshot_realtime_facts(&mut self, _now: Instant) -> Result<RealtimeRoutingFacts, Arc<str>> {
        Ok(RealtimeRoutingFacts::default())
    }

    fn select_next(
        &mut self,
        request: SelectionRequest<'_>,
    ) -> Result<Option<SelectedGatewayAttempt>, Arc<str>> {
        if request.remaining_attempts == 0 {
            return Ok(None);
        }
        self.selected.fetch_add(1, Ordering::Relaxed);
        let issued_at = Instant::now();
        let allocated = self
            .overall_deadline
            .saturating_duration_since(issued_at)
            .min(request.remaining_total);
        Ok(Some(SelectedGatewayAttempt {
            request_id: request.request_id,
            attempt_id: AttemptId(self.next_attempt_id.fetch_add(1, Ordering::Relaxed)),
            generation: request.generation,
            binding: self.binding,
            credential_ref: CredentialRef::new(format!(
                "bootstrap-credential-{}",
                self.binding.local_id()
            ))
            .map_err(|error| Arc::from(error.to_string()))?,
            route_decision_id: self.route_decision_id,
            budget: AttemptBudgetGrant {
                issued_at,
                allocated,
                deadline: self.overall_deadline,
            },
        }))
    }

    fn decide(
        &mut self,
        _selected: &SelectedGatewayAttempt,
        facts: &ProviderClassificationFacts,
        _transport: &crate::runtime::attempt::AttemptTransportFacts,
    ) -> Result<Disposition, Arc<str>> {
        let disposition =
            if facts.model_event.as_ref().map(ObservationLabel::as_str) == Some("local_reply") {
                Disposition::Terminate
            } else if facts.retryability == RetryabilityFact::Retryable {
                Disposition::Continue
            } else {
                Disposition::Accept
            };
        Ok(disposition)
    }

    fn decide_failure(
        &mut self,
        _selected: &SelectedGatewayAttempt,
        _failure: &crate::runtime::driver::AttemptFailureFacts,
    ) -> Result<Disposition, Arc<str>> {
        Ok(Disposition::Terminate)
    }

    fn replace_blocked_accept(
        &mut self,
        _selected: &SelectedGatewayAttempt,
        _reason: AcceptBlockedReason,
    ) -> Result<Disposition, Arc<str>> {
        Ok(Disposition::Terminate)
    }

    fn observe_published(&mut self, _disposition: &PublishedDisposition) -> Result<(), Arc<str>> {
        self.published.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn observe_completed(
        &mut self,
        _observation: &crate::runtime::driver::CompletedAttemptObservation,
    ) -> Result<(), Arc<str>> {
        Ok(())
    }
}

impl SelectionPublicationPort<(Arc<str>, u64)> for BootstrapSelection {
    type Session = BootstrapDecisionSession;

    fn begin_session(
        &self,
        request: DecisionSessionRequest<(Arc<str>, u64)>,
    ) -> Result<Self::Session, Arc<str>> {
        Ok(BootstrapDecisionSession {
            binding: self.binding,
            route_decision_id: RouteDecisionId(request.request_id.0),
            overall_deadline: request.overall_deadline,
            next_attempt_id: Arc::clone(&self.next_attempt_id),
            selected: Arc::clone(&self.selected),
            published: Arc::clone(&self.published),
        })
    }
}

#[derive(Debug)]
pub struct BootstrapLogicalRequest {
    head: GatewayRequestHead,
    body: Option<ChargedBodyQueue>,
}

#[derive(Debug, Default)]
pub struct BootstrapAttemptState {
    response_head: Option<ChargedResponseHead>,
}

#[derive(Debug)]
pub struct BootstrapReadiness {
    response_head: Option<ChargedResponseHead>,
    local_status: Option<StatusCode>,
}

impl BootstrapReadiness {
    fn upstream(response_head: ChargedResponseHead) -> Self {
        Self {
            response_head: Some(response_head),
            local_status: None,
        }
    }

    fn local(status: StatusCode) -> Self {
        Self {
            response_head: None,
            local_status: Some(status),
        }
    }
}

#[derive(Debug)]
pub struct BootstrapDecodedSse {
    bytes: ChargedBytes,
}

#[derive(Clone, Debug, Default)]
pub struct PassthroughBootstrapProvider {
    accept_on_first_semantic_sse: bool,
    classified_sse_events: Arc<AtomicUsize>,
    encoded_sse_events: Arc<AtomicUsize>,
    terminal_releases: Arc<AtomicUsize>,
    logical_body_bytes: Arc<AtomicU64>,
}

impl PassthroughBootstrapProvider {
    pub fn accepting_first_semantic_sse() -> Self {
        Self {
            accept_on_first_semantic_sse: true,
            ..Self::default()
        }
    }

    pub fn classified_sse_events(&self) -> usize {
        self.classified_sse_events.load(Ordering::Relaxed)
    }

    pub fn encoded_sse_events(&self) -> usize {
        self.encoded_sse_events.load(Ordering::Relaxed)
    }

    pub fn terminal_releases(&self) -> usize {
        self.terminal_releases.load(Ordering::Relaxed)
    }

    pub fn logical_body_bytes(&self) -> u64 {
        self.logical_body_bytes.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl ProviderRuntimePort for PassthroughBootstrapProvider {
    type LogicalRequest = BootstrapLogicalRequest;
    type RouteRequestContext = (Arc<str>, u64);
    type AttemptState = BootstrapAttemptState;
    type Readiness = BootstrapReadiness;
    type DecodedSseEvent = BootstrapDecodedSse;

    async fn begin_request(
        &self,
        head: GatewayRequestHead,
        context: LogicalRequestContext<'_>,
    ) -> Result<Self::LogicalRequest, Arc<str>> {
        let body = if matches!(context.plan, BodyPlan::PassThrough { .. }) {
            None
        } else {
            Some(
                ChargedBodyQueue::new(
                    context.budget,
                    MemoryRole::RawRequest,
                    context.plan,
                    context.hard_total_limit,
                    context.chunk_capacity,
                )
                .map_err(string_error)?,
            )
        };
        Ok(BootstrapLogicalRequest { head, body })
    }

    async fn consume_request_body(
        &self,
        logical: &mut Self::LogicalRequest,
        frame: LogicalRequestBodyFrame,
    ) -> Result<(), Arc<str>> {
        if let Some(bytes) = frame.bytes {
            self.logical_body_bytes
                .fetch_add(bytes.bytes().len() as u64, Ordering::Relaxed);
            logical
                .body
                .as_mut()
                .ok_or_else(|| {
                    Arc::<str>::from(
                        "bootstrap PassThrough fixture only supports an empty request body",
                    )
                })?
                .push_back(bytes)
                .map_err(string_error)?;
        }
        Ok(())
    }

    fn finalize_route_request_context(
        &self,
        logical: &mut Self::LogicalRequest,
    ) -> Result<Self::RouteRequestContext, Arc<str>> {
        Ok((
            Arc::clone(&logical.head.path_and_query),
            self.logical_body_bytes.load(Ordering::Relaxed),
        ))
    }

    async fn materialize_attempt(
        &self,
        logical: &mut Self::LogicalRequest,
        context: AttemptMaterializationContext<'_>,
    ) -> Result<(PreparedAttemptHttpRequest, Self::AttemptState), Arc<str>> {
        let plan = context.plan();
        let mut headers = logical.head.headers.clone();
        headers.insert(
            HOST,
            HeaderValue::from_str(&plan.transport_target.authority)
                .map_err(|_| Arc::<str>::from("invalid upstream authority"))?,
        );
        let hard_limit = plan
            .body_plans
            .attempt_request
            .max_retained_bytes()
            .unwrap_or(context.write_quantum);
        let mut wire = ChargedBodyQueue::new(
            context.budget,
            MemoryRole::AttemptWire,
            &plan.body_plans.attempt_request,
            hard_limit,
            plan.attempt_request_chunk_capacity,
        )
        .map_err(string_error)?;
        let quantum = context
            .write_quantum
            .min(plan.body_plans.attempt_request.max_chunk_bytes());
        if let Some(body) = &logical.body {
            for raw in body.chunks() {
                for chunk in raw.bytes().chunks(quantum) {
                    wire.push_back(
                        ChargedBytes::copy_from_opaque(
                            context.budget,
                            MemoryRole::AttemptWire,
                            chunk,
                        )
                        .map_err(string_error)?,
                    )
                    .map_err(string_error)?;
                }
            }
        }
        let lease = context.leases.acquire().map_err(string_error)?;
        Ok((
            PreparedAttemptHttpRequest {
                network_route: None,
                head: PreparedRequestHead {
                    method: logical.head.method.clone(),
                    path_and_query: Arc::clone(&logical.head.path_and_query),
                    headers,
                },
                body: PreparedAttemptBody::new(wire, lease).map_err(string_error)?,
            },
            BootstrapAttemptState::default(),
        ))
    }

    fn classify_materialization_failure(
        &self,
        _error: &Arc<str>,
    ) -> crate::runtime::driver::AttemptMaterializationFailure {
        crate::runtime::driver::AttemptMaterializationFailure {
            class: crate::runtime::driver::AttemptMaterializationFailureClass::Materialization,
            provider: ProviderClassificationFacts {
                error_class: Some(ObservationLabel::new("materialization").unwrap()),
                retryability: RetryabilityFact::NonRetryable,
                readiness: ObservationLabel::new("materialization_failed").unwrap(),
                ..ProviderClassificationFacts::default()
            },
            termination_reason: ObservationLabel::new("materialization")
                .expect("static materialization label is safe"),
        }
    }

    fn classify_precommit(
        &self,
        state: &mut Self::AttemptState,
        event: PrecommitEvent,
        _pinned_configs: &crate::runtime::driver::PinnedConfigContext<'_>,
        _event_configs: &ConfigEventSnapshot,
    ) -> Result<PrecommitClassification<Self::Readiness, Self::DecodedSseEvent>, Arc<str>> {
        if self.accept_on_first_semantic_sse {
            return match event {
                PrecommitEvent::ResponseHead(head) => {
                    if !head.status().is_informational() {
                        state.response_head = Some(head);
                    }
                    Ok(PrecommitClassification::pending())
                }
                PrecommitEvent::SseEvent {
                    sequence,
                    bytes,
                    provenance,
                } => {
                    self.classified_sse_events.fetch_add(1, Ordering::Relaxed);
                    let classified = if provenance == SemanticProvenance::NonSemantic {
                        None
                    } else {
                        let response_head = state.response_head.take().ok_or_else(|| {
                            Arc::<str>::from("SSE event arrived before response head")
                        })?;
                        Some(ClassifiedAttemptResult {
                            facts: ProviderClassificationFacts {
                                readiness: ObservationLabel::new("semantic_sse_ready").unwrap(),
                                model_event: Some(ObservationLabel::new("semantic_sse").unwrap()),
                                ..ProviderClassificationFacts::default()
                            },
                            readiness: BootstrapReadiness::upstream(response_head),
                        })
                    };
                    Ok(PrecommitClassification::decoded(
                        DecodedSseToken {
                            sequence,
                            decoded: BootstrapDecodedSse { bytes },
                        },
                        classified,
                    ))
                }
                PrecommitEvent::Body(_) | PrecommitEvent::EndStream => {
                    Err(Arc::from("non-SSE event in SSE classification mode"))
                }
            };
        }

        let PrecommitEvent::ResponseHead(response_head) = event else {
            return Err(Arc::from(
                "response body arrived before response head decision",
            ));
        };
        if response_head.status().is_informational() {
            return Ok(PrecommitClassification::pending());
        }
        let retryable = response_head.status() == StatusCode::TOO_MANY_REQUESTS;
        Ok(PrecommitClassification::classified(
            ClassifiedAttemptResult {
                facts: ProviderClassificationFacts {
                    error_class: retryable.then(|| ObservationLabel::new("rate_limited").unwrap()),
                    retryability: if retryable {
                        RetryabilityFact::Retryable
                    } else {
                        RetryabilityFact::NonRetryable
                    },
                    readiness: ObservationLabel::new("response_head_ready").unwrap(),
                    ..ProviderClassificationFacts::default()
                },
                readiness: BootstrapReadiness::upstream(response_head),
            },
        ))
    }

    fn normalize_attempt_local_reply(
        &self,
        _state: &mut Self::AttemptState,
        reply: LocalReply,
        upstream_side_effects: UpstreamSideEffectSnapshot,
    ) -> Result<NormalizedAttemptLocalReply<Self::Readiness>, Arc<str>> {
        Ok(NormalizedAttemptLocalReply {
            classified: ClassifiedAttemptResult {
                facts: ProviderClassificationFacts {
                    retryability: RetryabilityFact::NonRetryable,
                    readiness: ObservationLabel::new("local_reply_ready").unwrap(),
                    model_event: Some(ObservationLabel::new("local_reply").unwrap()),
                    ..ProviderClassificationFacts::default()
                },
                readiness: BootstrapReadiness::local(reply.status),
            },
            reply,
            upstream_side_effects,
        })
    }

    fn finalize_attempt_facts(
        &self,
        _state: &mut Self::AttemptState,
        _readiness: Option<&mut Self::Readiness>,
        published_facts: Option<&ProviderClassificationFacts>,
        completion: &crate::runtime::driver::ProviderAttemptCompletion,
    ) -> Result<ProviderClassificationFacts, Arc<str>> {
        let mut facts = published_facts.cloned().unwrap_or_default();
        facts.ended_at = Some(completion.ended_at);
        Ok(facts)
    }

    fn accepted_response_head(
        &self,
        readiness: &mut Self::Readiness,
        published: &PublishedDisposition,
        _accepted: &AcceptedResponseExecutionBinding,
        _configs: &PinnedConfigContext<'_>,
    ) -> Result<GatewayResponseHead, Arc<str>> {
        match published.disposition {
            Disposition::Accept => {
                let response_head = readiness
                    .response_head
                    .as_mut()
                    .ok_or_else(|| Arc::from("Accept readiness has no upstream response head"))?;
                Ok(GatewayResponseHead {
                    status: response_head.status(),
                    // The accepted owner is linear and no later provider callback
                    // inspects the response head. Move the retained map into the wire
                    // value while the readiness reservation remains alive through
                    // response completion, avoiding an unaccounted duplicate map.
                    headers: std::mem::take(response_head.headers_mut()),
                })
            }
            Disposition::Terminate => Ok(GatewayResponseHead {
                status: readiness.local_status.unwrap_or(StatusCode::BAD_GATEWAY),
                headers: Default::default(),
            }),
            Disposition::Continue => Err(Arc::from(
                "final response requested for Continue disposition",
            )),
        }
    }

    fn encode_accepted_event(
        &self,
        _readiness: &mut Self::Readiness,
        event: ProviderAcceptedEvent<Self::DecodedSseEvent>,
        published: &PublishedDisposition,
        _accepted: &AcceptedResponseExecutionBinding,
        _pinned_configs: &PinnedConfigContext<'_>,
        _event_configs: &ConfigEventSnapshot,
    ) -> Result<Option<AcceptedBodyFrame>, Arc<str>> {
        let event = match event {
            ProviderAcceptedEvent::Terminal {
                body,
                provenance,
                upstream_side_effects: _,
            } => {
                if published.disposition != Disposition::Terminate {
                    return Err(Arc::from(
                        "terminal event requested for non-Terminate disposition",
                    ));
                }
                return Ok(Some(AcceptedBodyFrame {
                    output: body.map(|bytes| EncodedOutputUnit { bytes, provenance }),
                    end_stream: true,
                    sse_sources: SseTransformSources::default(),
                    queue_metadata: BodyMetadataOwner::default(),
                }));
            }
            ProviderAcceptedEvent::Raw(event) => event,
            ProviderAcceptedEvent::DecodedSse {
                sequence: _,
                decoded,
                provenance,
            } => {
                self.encoded_sse_events.fetch_add(1, Ordering::Relaxed);
                return Ok(Some(AcceptedBodyFrame {
                    output: Some(EncodedOutputUnit {
                        bytes: decoded
                            .bytes
                            .transfer_role(MemoryRole::OutputQueue)
                            .map_err(string_error)?,
                        provenance,
                    }),
                    end_stream: false,
                    sse_sources: SseTransformSources::default(),
                    queue_metadata: BodyMetadataOwner::default(),
                }));
            }
        };
        if published.disposition != Disposition::Accept {
            return Err(Arc::from(
                "accepted event requested for non-Accept disposition",
            ));
        }
        match event {
            PrecommitEvent::Body(bytes) => Ok(Some(AcceptedBodyFrame {
                output: Some(EncodedOutputUnit {
                    bytes: bytes
                        .transfer_role(MemoryRole::OutputQueue)
                        .map_err(string_error)?,
                    provenance: SemanticProvenance::ProducesSemantic,
                }),
                end_stream: false,
                sse_sources: SseTransformSources::default(),
                queue_metadata: BodyMetadataOwner::default(),
            })),
            PrecommitEvent::SseEvent {
                bytes, provenance, ..
            } => {
                self.encoded_sse_events.fetch_add(1, Ordering::Relaxed);
                Ok(Some(AcceptedBodyFrame {
                    output: Some(EncodedOutputUnit {
                        bytes: bytes
                            .transfer_role(MemoryRole::OutputQueue)
                            .map_err(string_error)?,
                        provenance,
                    }),
                    end_stream: false,
                    sse_sources: SseTransformSources::default(),
                    queue_metadata: BodyMetadataOwner::default(),
                }))
            }
            PrecommitEvent::EndStream => Ok(Some(AcceptedBodyFrame {
                output: None,
                end_stream: true,
                sse_sources: SseTransformSources::default(),
                queue_metadata: BodyMetadataOwner::default(),
            })),
            PrecommitEvent::ResponseHead(_) => Ok(None),
        }
    }

    fn release_terminal_request(
        &self,
        mut logical: Self::LogicalRequest,
        _published: &PublishedDisposition,
    ) -> Result<(), Arc<str>> {
        if let Some(body) = logical.body.as_mut() {
            body.clear_and_release();
        }
        self.terminal_releases.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

fn string_error(error: impl ToString) -> Arc<str> {
    Arc::from(error.to_string())
}
