use super::*;

#[derive(Debug)]
pub(crate) struct PassthroughLogicalRequest {
    pub(crate) head: GatewayRequestHead,
    pub(crate) body: ChargedBodyQueue,
}

#[derive(Debug)]
pub(crate) struct PassthroughAttemptState {
    pub(crate) response_head: Option<ChargedResponseHead>,
    pub(crate) started_at: Instant,
}

impl Default for PassthroughAttemptState {
    fn default() -> Self {
        Self {
            response_head: None,
            started_at: Instant::now(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct PassthroughReadiness {
    pub(crate) response_head: Option<ChargedResponseHead>,
    pub(crate) local_status: Option<StatusCode>,
    pub(crate) ttft: Option<Duration>,
    pub(crate) final_usage: Option<UsageFact>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum LocalReplyClassification {
    Acceptable,
    Retryable,
}

impl PassthroughReadiness {
    pub(crate) fn upstream(response_head: ChargedResponseHead) -> Self {
        Self {
            response_head: Some(response_head),
            local_status: None,
            ttft: None,
            final_usage: None,
        }
    }

    pub(crate) fn local(status: StatusCode) -> Self {
        Self {
            response_head: None,
            local_status: Some(status),
            ttft: None,
            final_usage: None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct PassthroughDecodedSse {
    pub(crate) bytes: ChargedBytes,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PassthroughProvider {
    pub(crate) local_reply_normalizations: Arc<AtomicUsize>,
    pub(crate) terminal_head_encodes: Arc<AtomicUsize>,
    pub(crate) terminal_body_encodes: Arc<AtomicUsize>,
    pub(crate) normalized_upstream_effects: Arc<Mutex<Vec<UpstreamSideEffectSnapshot>>>,
    pub(crate) encoded_terminal_effects: Arc<Mutex<Vec<UpstreamSideEffectSnapshot>>>,
    pub(crate) terminal_request_releases: Arc<AtomicUsize>,
    pub(crate) materialized_attempts: Arc<AtomicUsize>,
    pub(crate) materialize_delay: Option<Duration>,
    pub(crate) materialization_failure: Option<AttemptMaterializationFailure>,
    pub(crate) materialization_failures_remaining: Arc<AtomicUsize>,
    pub(crate) materialization_raw_error: Option<Arc<str>>,
    pub(crate) classified_materialization_errors: Arc<Mutex<Vec<Arc<str>>>>,
    pub(crate) logical_body_frames: Arc<AtomicUsize>,
    pub(crate) observed_logical_body: Arc<Mutex<Vec<(Bytes, MemoryRole)>>>,
    pub(crate) drop_logical_body: bool,
    pub(crate) invalid_attempt_framing: bool,
    pub(crate) accept_on_first_semantic_sse: bool,
    pub(crate) end_stream_on_semantic_sse: bool,
    pub(crate) classified_sse: Arc<Mutex<Vec<(u64, SemanticProvenance)>>>,
    pub(crate) encoded_sse: Arc<Mutex<Vec<(u64, SemanticProvenance)>>>,
    pub(crate) decoded_prefix_sse: Arc<Mutex<Vec<u64>>>,
    pub(crate) raw_tail_sse: Arc<Mutex<Vec<u64>>>,
    pub(crate) config_publish_between_reads: Option<(ConfigCellHandle, Arc<ConfigBundle>)>,
    pub(crate) observed_pinned_config_generations:
        Arc<Mutex<Vec<(ConfigGeneration, ConfigGeneration)>>>,
    pub(crate) observed_accepted_config_generations:
        Arc<Mutex<Vec<(ConfigGeneration, ConfigGeneration)>>>,
    pub(crate) local_reply_classification: Option<LocalReplyClassification>,
    pub(crate) finalized_attempts: Arc<AtomicUsize>,
    pub(crate) finalized_without_readiness: Arc<AtomicUsize>,
    pub(crate) fail_terminal_encoder: bool,
    pub(crate) fail_terminal_request_release: bool,
    pub(crate) classification_usage: Option<UsageFact>,
    pub(crate) readiness_completion_usage: Option<UsageFact>,
    pub(crate) preexchange_completion_trace: Option<Arc<Mutex<Vec<PreexchangeCompletionStep>>>>,
}

#[async_trait]
impl ProviderRuntimePort for PassthroughProvider {
    type LogicalRequest = PassthroughLogicalRequest;
    type RouteRequestContext = PassthroughRouteContext;
    type AttemptState = PassthroughAttemptState;
    type Readiness = PassthroughReadiness;
    type DecodedSseEvent = PassthroughDecodedSse;

    async fn begin_request(
        &self,
        head: GatewayRequestHead,
        context: LogicalRequestContext<'_>,
    ) -> Result<Self::LogicalRequest, Arc<str>> {
        let body = ChargedBodyQueue::new(
            context.budget,
            MemoryRole::RawRequest,
            context.plan,
            context.hard_total_limit,
            context.chunk_capacity,
        )
        .map_err(|error| Arc::from(error.to_string()))?;
        Ok(PassthroughLogicalRequest { head, body })
    }

    async fn consume_request_body(
        &self,
        logical: &mut Self::LogicalRequest,
        frame: LogicalRequestBodyFrame,
    ) -> Result<(), Arc<str>> {
        self.logical_body_frames.fetch_add(1, Ordering::Relaxed);
        if let Some(bytes) = frame.bytes {
            self.observed_logical_body
                .lock()
                .map_err(|_| Arc::from("poisoned logical body facts"))?
                .push((bytes.bytes().clone(), bytes.role()));
            if self.drop_logical_body {
                return Ok(());
            }
            logical
                .body
                .push_back(bytes)
                .map_err(|error| Arc::from(error.to_string()))?;
        }
        Ok(())
    }

    fn finalize_route_request_context(
        &self,
        logical: &mut Self::LogicalRequest,
    ) -> Result<Self::RouteRequestContext, Arc<str>> {
        Ok(PassthroughRouteContext {
            method: logical.head.method.clone(),
            path_and_query: Arc::clone(&logical.head.path_and_query),
            body_bytes: logical.body.visible_bytes(),
        })
    }

    async fn materialize_attempt(
        &self,
        logical: &mut Self::LogicalRequest,
        context: AttemptMaterializationContext<'_>,
    ) -> Result<(PreparedAttemptHttpRequest, Self::AttemptState), Arc<str>> {
        self.materialized_attempts.fetch_add(1, Ordering::Relaxed);
        if self.materialization_failure.is_some()
            && self
                .materialization_failures_remaining
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
        {
            return Err(self
                .materialization_raw_error
                .clone()
                .unwrap_or_else(|| Arc::from("provider-private materialization failure")));
        }
        if let Some((publisher, candidate)) = &self.config_publish_between_reads {
            let first = context
                .config(ConfigCellId(101))
                .ok_or_else(|| Arc::from("missing first pinned config"))?
                .generation;
            publisher
                .publish(Arc::clone(candidate))
                .map_err(|error| Arc::from(error.to_string()))?;
            let second = context
                .config(ConfigCellId(102))
                .ok_or_else(|| Arc::from("missing second pinned config"))?
                .generation;
            self.observed_pinned_config_generations
                .lock()
                .map_err(|_| Arc::from("poisoned pinned config facts"))?
                .push((first, second));
        }
        if let Some(delay) = self.materialize_delay {
            tokio::time::sleep(delay).await;
        }
        let mut headers = logical.head.headers.clone();
        headers.insert(
            HOST,
            HeaderValue::from_str(&context.plan().transport_target.authority)
                .map_err(|_| Arc::from("invalid upstream authority"))?,
        );
        if self.invalid_attempt_framing {
            headers.insert(CONTENT_LENGTH, HeaderValue::from_static("1"));
            headers.insert(
                http::header::TRANSFER_ENCODING,
                HeaderValue::from_static("chunked"),
            );
        }
        let lease = context
            .leases
            .acquire()
            .map_err(|error| Arc::from(error.to_string()))?;
        let hard_limit = context
            .plan()
            .body_plans
            .attempt_request
            .max_retained_bytes()
            .unwrap_or(64 * 1024);
        let mut wire = ChargedBodyQueue::new(
            context.budget,
            MemoryRole::AttemptWire,
            &context.plan().body_plans.attempt_request,
            hard_limit,
            context.plan().attempt_request_chunk_capacity,
        )
        .map_err(|error| Arc::from(error.to_string()))?;
        let quantum = context
            .write_quantum
            .min(context.plan().body_plans.attempt_request.max_chunk_bytes());
        for raw in logical.body.chunks() {
            for chunk in raw.bytes().chunks(quantum) {
                wire.push_back(
                    ChargedBytes::copy_from_opaque(context.budget, MemoryRole::AttemptWire, chunk)
                        .map_err(|error| Arc::from(error.to_string()))?,
                )
                .map_err(|error| Arc::from(error.to_string()))?;
            }
        }
        Ok((
            PreparedAttemptHttpRequest {
                network_route: None,
                head: PreparedRequestHead {
                    method: logical.head.method.clone(),
                    path_and_query: Arc::clone(&logical.head.path_and_query),
                    headers,
                },
                body: PreparedAttemptBody::new(wire, lease)
                    .map_err(|error| Arc::from(error.to_string()))?,
            },
            PassthroughAttemptState::default(),
        ))
    }

    fn classify_materialization_failure(&self, error: &Arc<str>) -> AttemptMaterializationFailure {
        self.classified_materialization_errors
            .lock()
            .expect("classified materialization errors")
            .push(Arc::clone(error));
        self.materialization_failure
            .clone()
            .unwrap_or_else(|| AttemptMaterializationFailure {
                class: AttemptMaterializationFailureClass::Materialization,
                provider: ProviderClassificationFacts {
                    error_class: Some(ObservationLabel::new("materialization").unwrap()),
                    retryability: RetryabilityFact::NonRetryable,
                    readiness: ObservationLabel::new("materialization_failed").unwrap(),
                    ..ProviderClassificationFacts::default()
                },
                termination_reason: ObservationLabel::new("materialization")
                    .expect("static materialization label is safe"),
            })
    }

    fn classify_precommit(
        &self,
        state: &mut Self::AttemptState,
        event: PrecommitEvent,
        _pinned_configs: &hiroute_gateway_core::runtime::driver::PinnedConfigContext<'_>,
        _event_configs: &hiroute_gateway_core::core::execution_plan::ConfigEventSnapshot,
    ) -> Result<PrecommitClassification<Self::Readiness, Self::DecodedSseEvent>, Arc<str>> {
        if self.accept_on_first_semantic_sse {
            match event {
                PrecommitEvent::ResponseHead(response_head) => {
                    if !response_head.status().is_informational() {
                        state.response_head = Some(response_head);
                    }
                    return Ok(PrecommitClassification::pending());
                }
                PrecommitEvent::SseEvent {
                    sequence,
                    bytes,
                    provenance,
                } => {
                    self.classified_sse
                        .lock()
                        .map_err(|_| Arc::from("poisoned SSE classification facts"))?
                        .push((sequence, provenance));
                    let candidate = if provenance == SemanticProvenance::NonSemantic {
                        None
                    } else {
                        let ttft = state.started_at.elapsed();
                        let response_head = state
                            .response_head
                            .take()
                            .ok_or_else(|| Arc::from("SSE event arrived before response head"))?;
                        Some(ClassifiedAttemptResult {
                            facts: ProviderClassificationFacts {
                                readiness: ObservationLabel::new("semantic_response").unwrap(),
                                model_event: Some(
                                    ObservationLabel::new("semantic-response").unwrap(),
                                ),
                                retryability: RetryabilityFact::NonRetryable,
                                usage: self.classification_usage,
                                ttft: Some(ttft),
                                ..ProviderClassificationFacts::default()
                            },
                            readiness: PassthroughReadiness {
                                ttft: Some(ttft),
                                final_usage: self.readiness_completion_usage,
                                ..PassthroughReadiness::upstream(response_head)
                            },
                        })
                    };
                    return Ok(PrecommitClassification::decoded(
                        DecodedSseToken {
                            sequence,
                            decoded: PassthroughDecodedSse { bytes },
                        },
                        candidate,
                    ));
                }
                PrecommitEvent::EndStream | PrecommitEvent::Body(_) => {
                    return Err(Arc::from("invalid event in SSE classification mode"));
                }
            }
        }
        let PrecommitEvent::ResponseHead(response_head) = event else {
            return Err(Arc::from("response body arrived before disposition head"));
        };
        let status = response_head.status();
        if status.is_informational() {
            return Ok(PrecommitClassification::pending());
        }
        let mut readiness = PassthroughReadiness::upstream(response_head);
        readiness.final_usage = self.readiness_completion_usage;
        Ok(PrecommitClassification::classified(
            ClassifiedAttemptResult {
                facts: ProviderClassificationFacts {
                    error_class: (status == StatusCode::TOO_MANY_REQUESTS)
                        .then(|| ObservationLabel::new("rate_limited").unwrap()),
                    retryability: if status == StatusCode::TOO_MANY_REQUESTS {
                        RetryabilityFact::Retryable
                    } else {
                        RetryabilityFact::NonRetryable
                    },
                    http_status: Some(status),
                    retry_after: (status == StatusCode::TOO_MANY_REQUESTS)
                        .then_some(Duration::from_secs(2)),
                    reset_at: (status == StatusCode::TOO_MANY_REQUESTS)
                        .then(|| Instant::now() + Duration::from_secs(2)),
                    provider_code: (status == StatusCode::TOO_MANY_REQUESTS)
                        .then(|| ObservationLabel::new("rate_limit_exceeded").unwrap()),
                    provider_request_id: Some(
                        ObservationLabel::new("provider-request-test").unwrap(),
                    ),
                    readiness: ObservationLabel::new("response_head").unwrap(),
                    usage: self.classification_usage,
                    ..ProviderClassificationFacts::default()
                },
                readiness,
            },
        ))
    }

    fn normalize_attempt_local_reply(
        &self,
        _state: &mut Self::AttemptState,
        reply: LocalReply,
        upstream_side_effects: UpstreamSideEffectSnapshot,
    ) -> Result<NormalizedAttemptLocalReply<Self::Readiness>, Arc<str>> {
        self.local_reply_normalizations
            .fetch_add(1, Ordering::Relaxed);
        self.normalized_upstream_effects
            .lock()
            .map_err(|_| Arc::from("poisoned normalized upstream effects"))?
            .push(upstream_side_effects);
        Ok(NormalizedAttemptLocalReply {
            classified: ClassifiedAttemptResult {
                facts: ProviderClassificationFacts {
                    error_class: Some(ObservationLabel::new("local_reply").unwrap()),
                    retryability: match self.local_reply_classification {
                        Some(LocalReplyClassification::Retryable) => RetryabilityFact::Retryable,
                        _ => RetryabilityFact::NonRetryable,
                    },
                    readiness: ObservationLabel::new("local_reply").unwrap(),
                    model_event: Some(
                        ObservationLabel::new(match self.local_reply_classification {
                            Some(LocalReplyClassification::Acceptable) => "local-reply-acceptable",
                            Some(LocalReplyClassification::Retryable) => "local-reply-retryable",
                            None => "local-reply-terminal",
                        })
                        .unwrap(),
                    ),
                    ..ProviderClassificationFacts::default()
                },
                readiness: PassthroughReadiness::local(reply.status),
            },
            reply,
            upstream_side_effects,
        })
    }

    fn finalize_attempt_facts(
        &self,
        _state: &mut Self::AttemptState,
        readiness: Option<&mut Self::Readiness>,
        published_facts: Option<&ProviderClassificationFacts>,
        completion: &hiroute_gateway_core::runtime::driver::ProviderAttemptCompletion,
    ) -> Result<ProviderClassificationFacts, Arc<str>> {
        if let Some(trace) = self.preexchange_completion_trace.as_ref() {
            trace
                .lock()
                .map_err(|_| Arc::from("poisoned pre-exchange completion trace"))?
                .push(PreexchangeCompletionStep::ProviderFinalize);
        }
        self.finalized_attempts.fetch_add(1, Ordering::Relaxed);
        if readiness.is_none() {
            self.finalized_without_readiness
                .fetch_add(1, Ordering::Relaxed);
        }
        let mut facts = published_facts.cloned().unwrap_or_default();
        if let Some(readiness) = readiness {
            if readiness.ttft.is_some() {
                facts.ttft = readiness.ttft;
            }
            if readiness.final_usage.is_some() {
                facts.usage = readiness.final_usage;
            }
        }
        facts.ended_at = Some(completion.ended_at);
        Ok(facts)
    }

    fn accepted_response_head(
        &self,
        readiness: &mut Self::Readiness,
        published: &PublishedDisposition,
        _accepted: &hiroute_gateway_core::core::execution_plan::AcceptedResponseExecutionBinding,
        configs: &hiroute_gateway_core::runtime::driver::PinnedConfigContext<'_>,
    ) -> Result<GatewayResponseHead, Arc<str>> {
        if let (Some(first), Some(second)) = (
            configs.value(ConfigCellId(101)),
            configs.value(ConfigCellId(102)),
        ) {
            self.observed_accepted_config_generations
                .lock()
                .map_err(|_| Arc::from("poisoned accepted config facts"))?
                .push((first.generation, second.generation));
        }
        match published.disposition {
            Disposition::Accept => {
                if let Some(response_head) = readiness.response_head.as_mut() {
                    Ok(GatewayResponseHead {
                        status: response_head.status(),
                        headers: std::mem::take(response_head.headers_mut()),
                    })
                } else {
                    Ok(GatewayResponseHead {
                        status: readiness
                            .local_status
                            .ok_or_else(|| Arc::from("Accept readiness has no response head"))?,
                        headers: HeaderMap::new(),
                    })
                }
            }
            Disposition::Terminate => {
                self.terminal_head_encodes.fetch_add(1, Ordering::Relaxed);
                Ok(GatewayResponseHead {
                    status: readiness.local_status.unwrap_or(StatusCode::BAD_GATEWAY),
                    headers: HeaderMap::new(),
                })
            }
            Disposition::Continue => Err(Arc::from("encoder called for Continue disposition")),
        }
    }

    fn encode_accepted_event(
        &self,
        readiness: &mut Self::Readiness,
        event: ProviderAcceptedEvent<Self::DecodedSseEvent>,
        published: &PublishedDisposition,
        _accepted: &hiroute_gateway_core::core::execution_plan::AcceptedResponseExecutionBinding,
        _pinned_configs: &hiroute_gateway_core::runtime::driver::PinnedConfigContext<'_>,
        _event_configs: &hiroute_gateway_core::core::execution_plan::ConfigEventSnapshot,
    ) -> Result<Option<AcceptedBodyFrame>, Arc<str>> {
        let event = match event {
            ProviderAcceptedEvent::Terminal {
                body,
                provenance,
                upstream_side_effects,
            } => {
                if !matches!(
                    published.disposition,
                    Disposition::Accept | Disposition::Terminate
                ) {
                    return Err(Arc::from(
                        "terminal encoder called without a terminal publication",
                    ));
                }
                self.terminal_body_encodes.fetch_add(1, Ordering::Relaxed);
                if self.fail_terminal_encoder {
                    return Err(Arc::from("synthetic terminal encoder failure"));
                }
                self.encoded_terminal_effects
                    .lock()
                    .map_err(|_| Arc::from("poisoned encoded terminal effects"))?
                    .push(upstream_side_effects);
                return Ok(Some(AcceptedBodyFrame {
                    output: body.map(|bytes| EncodedOutputUnit { bytes, provenance }),
                    end_stream: true,
                    sse_sources: SseTransformSources::default(),
                    queue_metadata: BodyMetadataOwner::default(),
                }));
            }
            ProviderAcceptedEvent::Raw(event) => event,
            ProviderAcceptedEvent::DecodedSse {
                sequence,
                decoded,
                provenance,
            } => {
                self.decoded_prefix_sse
                    .lock()
                    .map_err(|_| Arc::from("poisoned decoded SSE facts"))?
                    .push(sequence);
                self.encoded_sse
                    .lock()
                    .map_err(|_| Arc::from("poisoned SSE encoding facts"))?
                    .push((sequence, provenance));
                return Ok(Some(AcceptedBodyFrame {
                    output: Some(EncodedOutputUnit {
                        bytes: decoded
                            .bytes
                            .transfer_role(MemoryRole::OutputQueue)
                            .map_err(|error| Arc::from(error.to_string()))?,
                        provenance,
                    }),
                    end_stream: self.end_stream_on_semantic_sse
                        && provenance == SemanticProvenance::ProducesSemantic,
                    sse_sources: SseTransformSources::default(),
                    queue_metadata: BodyMetadataOwner::default(),
                }));
            }
        };
        if published.disposition != Disposition::Accept {
            return Err(Arc::from(
                "accepted encoder called before Accept publication",
            ));
        }
        match event {
            PrecommitEvent::Body(bytes) => Ok(Some(AcceptedBodyFrame {
                output: Some(EncodedOutputUnit {
                    bytes: bytes
                        .transfer_role(MemoryRole::OutputQueue)
                        .map_err(|error| Arc::from(error.to_string()))?,
                    provenance: SemanticProvenance::ProducesSemantic,
                }),
                end_stream: false,
                sse_sources: SseTransformSources::default(),
                queue_metadata: BodyMetadataOwner::default(),
            })),
            PrecommitEvent::SseEvent {
                sequence,
                bytes,
                provenance,
            } => {
                self.raw_tail_sse
                    .lock()
                    .map_err(|_| Arc::from("poisoned raw SSE facts"))?
                    .push(sequence);
                self.encoded_sse
                    .lock()
                    .map_err(|_| Arc::from("poisoned SSE encoding facts"))?
                    .push((sequence, provenance));
                Ok(Some(AcceptedBodyFrame {
                    output: Some(EncodedOutputUnit {
                        bytes: bytes
                            .transfer_role(MemoryRole::OutputQueue)
                            .map_err(|error| Arc::from(error.to_string()))?,
                        provenance,
                    }),
                    end_stream: false,
                    sse_sources: SseTransformSources::default(),
                    queue_metadata: BodyMetadataOwner::default(),
                }))
            }
            PrecommitEvent::EndStream => {
                readiness.final_usage = Some(UsageFact {
                    input: UsageDimension::reported(11),
                    output: UsageDimension::reported(7),
                    billable: UsageDimension::reported(18),
                    cache_read: UsageDimension::reported(3),
                    cache_write: UsageDimension::unknown(),
                    reasoning: UsageDimension::estimated(2),
                });
                Ok(Some(AcceptedBodyFrame {
                    output: None,
                    end_stream: true,
                    sse_sources: SseTransformSources::default(),
                    queue_metadata: BodyMetadataOwner::default(),
                }))
            }
            PrecommitEvent::ResponseHead(_) => Ok(None),
        }
    }

    fn release_terminal_request(
        &self,
        mut logical: Self::LogicalRequest,
        _published: &PublishedDisposition,
    ) -> Result<(), Arc<str>> {
        self.terminal_request_releases
            .fetch_add(1, Ordering::Relaxed);
        logical.body.clear_and_release();
        if self.fail_terminal_request_release {
            Err(Arc::from("synthetic provider request release failure"))
        } else {
            Ok(())
        }
    }
}
