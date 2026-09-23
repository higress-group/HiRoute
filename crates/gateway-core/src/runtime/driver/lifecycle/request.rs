use super::*;

impl<S, P, F, T> GatewayCoreLifecycle<S, P, F, T>
where
    P: ProviderRuntimePort,
    S: SelectionPublicationPort<P::RouteRequestContext>,
    F: GatewayFilterManagerPort,
    T: AttemptTransportFactory,
{
    pub(super) async fn prepare_request(
        &self,
        session: &mut dyn GatewaySession,
        request_filters: &mut GatewayFilterRequestOwner<F::RequestFilters>,
    ) -> Result<RequestPreparation<P::LogicalRequest, S::Session>, GatewayExecutionError> {
        self.prepare_request_with_admission(session, request_filters, None, None, None)
            .await
    }
    pub(super) async fn prepare_bound_request(
        &self,
        session: &mut dyn GatewaySession,
        request_filters: &mut GatewayFilterRequestOwner<F::RequestFilters>,
        admission: BoundRequestAdmission,
    ) -> Result<RequestPreparation<P::LogicalRequest, S::Session>, GatewayExecutionError> {
        self.prepare_request_with_admission(session, request_filters, Some(admission), None, None)
            .await
    }
    pub(super) async fn prepare_bound_request_with_context(
        &self,
        session: &mut dyn GatewaySession,
        request_filters: &mut GatewayFilterRequestOwner<F::RequestFilters>,
        admission: BoundRequestAdmission,
        budget: StreamBudget,
        provider_context: Arc<dyn std::any::Any + Send + Sync>,
    ) -> Result<RequestPreparation<P::LogicalRequest, S::Session>, GatewayExecutionError> {
        self.prepare_request_with_admission(
            session,
            request_filters,
            Some(admission),
            Some(budget),
            Some(provider_context),
        )
        .await
    }
    async fn prepare_request_with_admission(
        &self,
        session: &mut dyn GatewaySession,
        request_filters: &mut GatewayFilterRequestOwner<F::RequestFilters>,
        admission: Option<BoundRequestAdmission>,
        budget_override: Option<StreamBudget>,
        provider_context: Option<Arc<dyn std::any::Any + Send + Sync>>,
    ) -> Result<RequestPreparation<P::LogicalRequest, S::Session>, GatewayExecutionError> {
        let request_started = Instant::now();
        let cancellation = session.cancellation_token();
        if cancellation.is_cancelled() {
            return Err(GatewayExecutionError::Cancelled);
        }
        let request_id = RequestId(self.next_request_id.fetch_add(1, Ordering::Relaxed) + 1);
        let bound_admission = admission.is_some();
        let (mut binding, request_telemetry, admitted_route, frozen_candidates) =
            if let Some(admission) = admission {
                let compiled_candidates = admission.binding.candidate_bindings()?;
                let mut seen = HashSet::new();
                if admission.overall_deadline <= request_started
                    || admission.max_attempts == 0
                    || admission.max_attempts > admission.binding.max_attempts()?
                    || admission.frozen_candidates.is_empty()
                    || admission.route_binding.plan_revision() != admission.binding.plan_revision()
                    || admission.frozen_candidates[0].binding != admission.route_binding
                    || admission.frozen_candidates.iter().any(|candidate| {
                        candidate.binding.plan_revision() != admission.binding.plan_revision()
                            || !compiled_candidates.contains(&candidate.binding)
                            || !seen.insert(candidate.binding)
                    })
                {
                    return Err(GatewayExecutionError::InvalidBoundAdmission);
                }
                let frozen_candidates = Arc::clone(&admission.frozen_candidates);
                (
                    admission.binding,
                    None,
                    Some((
                        admission.route_binding,
                        admission.accepted_response_body_plan,
                        admission.overall_deadline,
                        admission.max_attempts,
                    )),
                    Some(frozen_candidates),
                )
            } else if let Some(telemetry) = &self.telemetry {
                let publications = self
                    .publications
                    .as_ref()
                    .ok_or(GatewayExecutionError::InvalidBoundAdmission)?;
                let (binding, publication) = publications.bind_request_with_identity()?;
                let request_telemetry = RequestTelemetry::new(
                    Arc::clone(telemetry),
                    Correlation {
                        authority_id: publication.authority_id,
                        authority_epoch: publication.authority_epoch,
                        config_revision: publication.config_revision,
                        plan_revision: publication.plan_revision,
                        config_generations: Arc::new([]),
                        stable_target_key: None,
                        binding_local_id: None,
                        request_id: Some(request_id),
                        decision_id: None,
                        attempt_id: None,
                        attempt_generation: None,
                    },
                );
                (binding, Some(request_telemetry), None, None)
            } else {
                let publications = self
                    .publications
                    .as_ref()
                    .ok_or(GatewayExecutionError::InvalidBoundAdmission)?;
                (publications.bind_request()?, None, None, None)
            };
        let mut driver = LogicalRequestDriver::new(ScopeSupervisor::default())?;
        if let Some(telemetry) = request_telemetry.clone() {
            driver = driver.with_telemetry(telemetry);
        }
        driver.bind_publication()?;
        let budget = if let Some(budget) = budget_override {
            if !bound_admission || request_telemetry.is_some() {
                return Err(GatewayExecutionError::InvalidBoundAdmission);
            }
            budget
        } else if let Some(telemetry) = request_telemetry.clone() {
            self.budgets
                .stream_with_telemetry(self.limits.stream_memory_bytes, telemetry)?
        } else {
            self.budgets.stream(self.limits.stream_memory_bytes)?
        };
        let mut final_writer = RequestFinalWriter::new(request_telemetry.clone());
        let run_filter_callbacks = !self.filters.is_noop_fast_path();

        let mut head = session.request_head()?;
        let downstream_method = head.method.clone();
        let downstream_protocol = head.protocol;
        let route = if bound_admission {
            None
        } else {
            let route = match_compiled_route(binding.ingress_plan()?, &head);
            if let Some(route) = route.as_ref() {
                binding.bind_route(route)?;
            } else {
                binding.bind_local_response()?;
            }
            route
        };
        let (route_binding, route_accepted_body_plan, deadline, admitted_max_attempts) =
            if let Some(admitted) = admitted_route {
                admitted
            } else {
                let route_deadline = request_started
                    .checked_add(binding.overall_request_timeout()?)
                    .ok_or(GatewayExecutionError::Deadline)?;
                let deadline = match self.limits.bootstrap_hard_cap {
                    Some(hard_cap) => route_deadline.min(
                        request_started
                            .checked_add(hard_cap)
                            .ok_or(GatewayExecutionError::Deadline)?,
                    ),
                    None => route_deadline,
                };
                match route.as_ref() {
                    Some(route) => (
                        route.binding,
                        route.request_plan.accepted_response.body_plan.clone(),
                        deadline,
                        route.request_plan.max_attempts,
                    ),
                    None => (
                        ResolvedTargetBindingId::new(binding.plan_revision(), 0),
                        BodyPlan::PassThrough { max_chunk_bytes: 1 },
                        deadline,
                        0,
                    ),
                }
            };
        // Route matching holds only the immutable ingress segment. Once the
        // route is known, bind all RequestPinned groups in that route's
        // compiler-sealed candidate/accepted closure exactly once, then drop
        // the full attempt index and config catalog.
        let request_configs = binding.take_request_configs()?;
        let request_config_observation = request_telemetry.as_ref().map(|telemetry| {
            ConfigLeaseObservation::acquire(
                telemetry,
                ConfigAcquireScope::Request,
                request_configs.generations(),
            )
        });
        let request_configs =
            ObservedConfigSnapshot::new(request_configs, request_config_observation);
        if !bound_admission && route.is_none() {
            return emit_local_response(
                session,
                &mut final_writer,
                &self.filter_executors,
                &mut driver,
                &mut binding,
                &request_configs,
                &mut request_filters.filters,
                LocalReply {
                    status: StatusCode::NOT_FOUND,
                    headers: HeaderMap::new(),
                    body: Bytes::from_static(b"not found"),
                    provenance: SemanticProvenance::NonSemantic,
                },
                &budget,
                request_id,
                None,
                None,
                None,
                deadline,
                &cancellation,
                request_telemetry.clone(),
                &downstream_method,
                downstream_protocol,
                SessionReuse::Close,
            )
            .await
            .map(RequestPreparation::Completed);
        }
        let logical_binding = binding.take_logical_request()?;
        let has_logical_filters =
            run_filter_callbacks && !logical_binding.plan().filters.is_empty();
        let logical_request_plan = logical_binding.plan().body_plan.clone();
        let logical_request_chunk_capacity = logical_binding.plan().chunk_capacity;
        let logical_header_configs = if has_logical_filters {
            let phase = logical_binding.acquire_phase_configs()?;
            let event = logical_binding.acquire_event_configs()?;
            materialize_filter_configs(
                &logical_binding.plan().filters,
                None,
                &request_configs,
                None,
                &phase,
                &event,
            )?
        } else {
            FilterConfigSnapshot::default()
        };
        let mut logical_body_owner = BodyPlanExecutor::new(
            BodyDirection::LogicalRequest,
            logical_request_plan.clone(),
            self.limits.max_request_body_bytes,
        )?;
        let original_request_content_length = head.headers.get(CONTENT_LENGTH).cloned();
        let original_request_transfer_encoding = head.headers.get(TRANSFER_ENCODING).cloned();
        if let Some(content_length) = request_content_length(&head.headers)?
            && let Err(BodyError::BodyLimitExceeded) =
                logical_body_owner.preflight_content_length(content_length)
        {
            return emit_local_response(
                session,
                &mut final_writer,
                &self.filter_executors,
                &mut driver,
                &mut binding,
                &request_configs,
                &mut request_filters.filters,
                LocalReply {
                    status: StatusCode::PAYLOAD_TOO_LARGE,
                    headers: HeaderMap::new(),
                    body: Bytes::from_static(b"request body plan limit exceeded"),
                    provenance: SemanticProvenance::NonSemantic,
                },
                &budget,
                request_id,
                Some(route_binding),
                None,
                None,
                deadline,
                &cancellation,
                request_telemetry.clone(),
                &downstream_method,
                downstream_protocol,
                SessionReuse::Close,
            )
            .await
            .map(RequestPreparation::Completed);
        }
        // Pingora cannot expose a non-consuming disconnect watcher while the
        // downstream request body is still pending. Watching
        // `read_body_or_idle(true)` here races with the real body reader and
        // can misclassify ordinary DATA as a disconnect. Before request EOS,
        // deadline/cancellation bound callback work and the next body read is
        // the authoritative liveness check.
        let logical_filter_result = if has_logical_filters {
            let logical_filter_started = Instant::now();
            if let (Some(telemetry), Some(scope_id)) = (
                request_telemetry.as_ref(),
                driver.scope_id(ScopeKind::LogicalRequest),
            ) {
                telemetry.scope(
                    ScopeKind::LogicalRequest,
                    scope_id,
                    ScopePhase::Paused,
                    0,
                    Duration::ZERO,
                );
            }
            let logical_scope_id = driver
                .scope_id(ScopeKind::LogicalRequest)
                .ok_or(GatewayExecutionError::MissingFilterScope)?;
            let reply = await_session_operation(
                &cancellation,
                deadline,
                request_telemetry.as_ref(),
                async {
                    request_filters
                        .filters
                        .begin_logical_request(
                            &logical_binding.plan().filters,
                            filter_scope_context(
                                request_id,
                                binding.plan_revision(),
                                Some(route_binding),
                                None,
                                None,
                                logical_scope_id,
                                ScopeKind::LogicalRequest,
                                deadline,
                                &cancellation,
                                request_telemetry.clone(),
                                &budget,
                                logical_header_configs,
                                &self.filter_executors,
                            ),
                            &mut head,
                        )
                        .await
                        .map_err(GatewayExecutionError::Filter)
                },
            )
            .await?;
            if let (Some(telemetry), Some(scope_id)) = (
                request_telemetry.as_ref(),
                driver.scope_id(ScopeKind::LogicalRequest),
            ) {
                telemetry.scope(
                    ScopeKind::LogicalRequest,
                    scope_id,
                    ScopePhase::Body,
                    0,
                    logical_filter_started.elapsed(),
                );
            }
            reply
        } else {
            GatewayFilterResult::headers(None, None)
        };
        if let Some(reply) = logical_filter_result.local_reply {
            return emit_local_response(
                session,
                &mut final_writer,
                &self.filter_executors,
                &mut driver,
                &mut binding,
                &request_configs,
                &mut request_filters.filters,
                reply,
                &budget,
                request_id,
                Some(route_binding),
                None,
                None,
                deadline,
                &cancellation,
                request_telemetry.clone(),
                &downstream_method,
                downstream_protocol,
                SessionReuse::Close,
            )
            .await
            .map(RequestPreparation::Completed);
        }

        // Any header pause keeps the provider-facing head uncommitted. The
        // publication validator rejects unbounded body plans for such chains;
        // retaining here therefore uses the route's existing hard body limit.
        let logical_plan_buffered = logical_body_owner.is_buffered_transform();
        let mut logical_pause = logical_filter_result.pause;
        let mut logical_is_buffered = logical_plan_buffered || logical_pause.is_some();
        let mut logical_framing = FramingLedger::default();
        record_framing_mutations(
            &mut logical_framing,
            &head.headers,
            original_request_content_length.as_ref(),
            original_request_transfer_encoding.as_ref(),
        );
        let mut committed_logical_head = None;
        let mut logical = if logical_is_buffered {
            None
        } else {
            if matches!(logical_request_plan, BodyPlan::PassThrough { .. }) {
                logical_framing.pass_through()?;
            } else {
                logical_framing.streaming_transform()?;
            }
            logical_framing.finalize(
                &mut head.headers,
                protocol_framing(head.protocol),
                Some(&head.method),
                None,
            )?;
            if has_logical_filters {
                committed_logical_head = Some(head.clone());
            }
            let provider_head = if has_logical_filters {
                head.clone()
            } else {
                // With the explicit no-op manager there is no later callback
                // that can inspect or mutate this head. Move its potentially
                // allocated header map into the provider-owned IR instead of
                // cloning it on every streaming request.
                GatewayRequestHead {
                    method: head.method.clone(),
                    path_and_query: Arc::clone(&head.path_and_query),
                    authority: head.authority.clone(),
                    headers: std::mem::take(&mut head.headers),
                    protocol: head.protocol,
                }
            };
            Some(
                await_session_operation(
                    &cancellation,
                    deadline,
                    request_telemetry.as_ref(),
                    async {
                        self.provider
                            .begin_request(
                                provider_head,
                                LogicalRequestContext {
                                    budget: &budget,
                                    plan: &logical_request_plan,
                                    hard_total_limit: self.limits.max_request_body_bytes,
                                    chunk_capacity: logical_request_chunk_capacity,
                                    frozen_candidates: frozen_candidates.clone(),
                                    provider_context: provider_context.clone(),
                                },
                            )
                            .await
                            .map_err(GatewayExecutionError::Provider)
                    },
                )
                .await?,
            )
        };
        // BufferedTransform is the only logical mode allowed to retain body
        // frames in core. Its head remains uncommitted to #15 until EOS, so a
        // body filter can safely update held headers before exact framing.
        let mut buffered_logical_frames = Vec::new();
        let request_transport_chunk_bytes = self.limits.request_transport_chunk_bytes();

        loop {
            // Header StopIteration remains body-driven. StopAll(Watermark)
            // must not poll the sole request-body reader, while
            // StopAll(Buffer) may keep reading only under the BodyPlan hard
            // limit and services its continuation before every source poll.
            match logical_pause {
                Some(FilterPause::Watermark) => {
                    let resumed = await_session_operation(
                        &cancellation,
                        deadline,
                        request_telemetry.as_ref(),
                        async {
                            request_filters
                                .filters
                                .wait_logical_resume(&mut head)
                                .await
                                .map_err(GatewayExecutionError::Filter)
                        },
                    )
                    .await?;
                    if let Some(reply) = resumed.local_reply {
                        return emit_local_response(
                            session,
                            &mut final_writer,
                            &self.filter_executors,
                            &mut driver,
                            &mut binding,
                            &request_configs,
                            &mut request_filters.filters,
                            reply,
                            &budget,
                            request_id,
                            Some(route_binding),
                            None,
                            None,
                            deadline,
                            &cancellation,
                            request_telemetry.clone(),
                            &downstream_method,
                            downstream_protocol,
                            SessionReuse::Close,
                        )
                        .await
                        .map(RequestPreparation::Completed);
                    }
                    buffered_logical_frames.extend(resumed.frames);
                    logical_pause = resumed.pause;
                    continue;
                }
                Some(FilterPause::Buffer) => {
                    if let Some(resumed) = request_filters
                        .filters
                        .try_logical_resume(&mut head)
                        .await
                        .map_err(GatewayExecutionError::Filter)?
                    {
                        if let Some(reply) = resumed.local_reply {
                            return emit_local_response(
                                session,
                                &mut final_writer,
                                &self.filter_executors,
                                &mut driver,
                                &mut binding,
                                &request_configs,
                                &mut request_filters.filters,
                                reply,
                                &budget,
                                request_id,
                                Some(route_binding),
                                None,
                                None,
                                deadline,
                                &cancellation,
                                request_telemetry.clone(),
                                &downstream_method,
                                downstream_protocol,
                                SessionReuse::Close,
                            )
                            .await
                            .map(RequestPreparation::Completed);
                        }
                        buffered_logical_frames.extend(resumed.frames);
                        logical_pause = resumed.pause;
                        continue;
                    }
                }
                Some(FilterPause::HeaderIteration) | None => {}
            }

            if logical_is_buffered && !logical_plan_buffered && logical_pause.is_none() {
                record_framing_mutations(
                    &mut logical_framing,
                    &head.headers,
                    original_request_content_length.as_ref(),
                    original_request_transfer_encoding.as_ref(),
                );
                if matches!(logical_request_plan, BodyPlan::PassThrough { .. }) {
                    logical_framing.pass_through()?;
                } else {
                    logical_framing.streaming_transform()?;
                }
                logical_framing.finalize(
                    &mut head.headers,
                    protocol_framing(head.protocol),
                    Some(&head.method),
                    None,
                )?;
                committed_logical_head = Some(head.clone());
                let mut owner = await_session_operation(
                    &cancellation,
                    deadline,
                    request_telemetry.as_ref(),
                    async {
                        self.provider
                            .begin_request(
                                head.clone(),
                                LogicalRequestContext {
                                    budget: &budget,
                                    plan: &logical_request_plan,
                                    hard_total_limit: self.limits.max_request_body_bytes,
                                    chunk_capacity: logical_request_chunk_capacity,
                                    frozen_candidates: frozen_candidates.clone(),
                                    provider_context: provider_context.clone(),
                                },
                            )
                            .await
                            .map_err(GatewayExecutionError::Provider)
                    },
                )
                .await?;
                for frame in buffered_logical_frames.drain(..) {
                    await_session_operation(
                        &cancellation,
                        deadline,
                        request_telemetry.as_ref(),
                        async {
                            self.provider
                                .consume_request_body(&mut owner, frame)
                                .await
                                .map_err(GatewayExecutionError::Provider)
                        },
                    )
                    .await?;
                }
                logical = Some(owner);
                logical_is_buffered = false;
            }

            let chunk = await_session_operation(
                &cancellation,
                deadline,
                request_telemetry.as_ref(),
                async {
                    session
                        .read_request_body_charged(&budget, request_transport_chunk_bytes)
                        .await
                        .map_err(GatewayExecutionError::Transport)
                },
            )
            .await?;
            let Some(chunk) = chunk else {
                break;
            };
            if matches!(
                logical_body_owner.admit_chunk(chunk.bytes().len()),
                Err(BodyError::BodyLimitExceeded)
            ) {
                return emit_local_response(
                    session,
                    &mut final_writer,
                    &self.filter_executors,
                    &mut driver,
                    &mut binding,
                    &request_configs,
                    &mut request_filters.filters,
                    LocalReply {
                        status: StatusCode::PAYLOAD_TOO_LARGE,
                        headers: HeaderMap::new(),
                        body: Bytes::from_static(b"request body plan limit exceeded"),
                        provenance: SemanticProvenance::NonSemantic,
                    },
                    &budget,
                    request_id,
                    Some(route_binding),
                    None,
                    None,
                    deadline,
                    &cancellation,
                    request_telemetry.clone(),
                    &downstream_method,
                    downstream_protocol,
                    SessionReuse::Close,
                )
                .await
                .map(RequestPreparation::Completed);
            }
            let frame = LogicalRequestBodyFrame {
                bytes: Some(chunk.transfer_role(MemoryRole::RawRequest)?),
                end_stream: false,
                queue_metadata: BodyMetadataOwner::default(),
            };
            let logical_body_configs = if has_logical_filters {
                let phase = logical_binding.acquire_phase_configs()?;
                let event = logical_binding.acquire_event_configs()?;
                materialize_filter_configs(
                    &logical_binding.plan().filters,
                    None,
                    &request_configs,
                    None,
                    &phase,
                    &event,
                )?
            } else {
                FilterConfigSnapshot::default()
            };
            let filtered = if has_logical_filters {
                await_session_operation(
                    &cancellation,
                    deadline,
                    request_telemetry.as_ref(),
                    async {
                        request_filters
                            .filters
                            .filter_logical_request_body(&mut head, frame, logical_body_configs)
                            .await
                            .map_err(GatewayExecutionError::Filter)
                    },
                )
                .await?
            } else {
                GatewayFilterResult::forward(frame)
            };
            if let Some(reply) = filtered.local_reply {
                return emit_local_response(
                    session,
                    &mut final_writer,
                    &self.filter_executors,
                    &mut driver,
                    &mut binding,
                    &request_configs,
                    &mut request_filters.filters,
                    reply,
                    &budget,
                    request_id,
                    Some(route_binding),
                    None,
                    None,
                    deadline,
                    &cancellation,
                    request_telemetry.clone(),
                    &downstream_method,
                    downstream_protocol,
                    SessionReuse::Close,
                )
                .await
                .map(RequestPreparation::Completed);
            }
            logical_pause = filtered.pause;
            for frame in filtered.frames {
                if logical_is_buffered {
                    buffered_logical_frames.push(frame);
                } else {
                    if has_logical_filters && committed_logical_head.as_ref() != Some(&head) {
                        return Err(GatewayExecutionError::LogicalHeaderMutationAfterCommit);
                    }
                    await_session_operation(
                        &cancellation,
                        deadline,
                        request_telemetry.as_ref(),
                        async {
                            self.provider
                                .consume_request_body(
                                    logical.as_mut().expect("non-buffered logical owner exists"),
                                    frame,
                                )
                                .await
                                .map_err(GatewayExecutionError::Provider)
                        },
                    )
                    .await?;
                }
            }
        }
        let observed_body_bytes = match logical_body_owner.finish() {
            Ok(bytes) => bytes,
            Err(BodyError::ContentLengthMismatch) => {
                return emit_local_response(
                    session,
                    &mut final_writer,
                    &self.filter_executors,
                    &mut driver,
                    &mut binding,
                    &request_configs,
                    &mut request_filters.filters,
                    LocalReply {
                        status: StatusCode::BAD_REQUEST,
                        headers: HeaderMap::new(),
                        body: Bytes::from_static(b"request Content-Length mismatch"),
                        provenance: SemanticProvenance::NonSemantic,
                    },
                    &budget,
                    request_id,
                    Some(route_binding),
                    None,
                    None,
                    deadline,
                    &cancellation,
                    request_telemetry.clone(),
                    &downstream_method,
                    downstream_protocol,
                    SessionReuse::Close,
                )
                .await
                .map(RequestPreparation::Completed);
            }
            Err(error) => return Err(error.into()),
        };
        if let Some(telemetry) = request_telemetry.as_ref() {
            let queue_high_water_bytes = budget
                .snapshot()
                .map(|snapshot| snapshot.role_peak[MemoryRole::RawRequest as usize])
                .unwrap_or(0);
            telemetry.body(
                BodyDirection::LogicalRequest,
                &logical_request_plan,
                observed_body_bytes,
                queue_high_water_bytes,
            );
        }
        let eos = LogicalRequestBodyFrame {
            bytes: None,
            end_stream: true,
            queue_metadata: BodyMetadataOwner::default(),
        };
        let logical_eos_configs = if has_logical_filters {
            let phase = logical_binding.acquire_phase_configs()?;
            let event = logical_binding.acquire_event_configs()?;
            materialize_filter_configs(
                &logical_binding.plan().filters,
                None,
                &request_configs,
                None,
                &phase,
                &event,
            )?
        } else {
            FilterConfigSnapshot::default()
        };
        let filtered_eos = if has_logical_filters {
            await_request_operation(
                session,
                &cancellation,
                deadline,
                request_telemetry.as_ref(),
                async {
                    request_filters
                        .filters
                        .filter_logical_request_body(&mut head, eos, logical_eos_configs)
                        .await
                        .map_err(GatewayExecutionError::Filter)
                },
            )
            .await?
        } else {
            GatewayFilterResult::forward(eos)
        };
        if let Some(reply) = filtered_eos.local_reply {
            return emit_local_response(
                session,
                &mut final_writer,
                &self.filter_executors,
                &mut driver,
                &mut binding,
                &request_configs,
                &mut request_filters.filters,
                reply,
                &budget,
                request_id,
                Some(route_binding),
                None,
                None,
                deadline,
                &cancellation,
                request_telemetry.clone(),
                &downstream_method,
                downstream_protocol,
                SessionReuse::Reusable,
            )
            .await
            .map(RequestPreparation::Completed);
        }
        logical_pause = filtered_eos.pause;
        for frame in filtered_eos.frames {
            if logical_is_buffered {
                buffered_logical_frames.push(frame);
            } else {
                if has_logical_filters && committed_logical_head.as_ref() != Some(&head) {
                    return Err(GatewayExecutionError::LogicalHeaderMutationAfterCommit);
                }
                await_request_operation(
                    session,
                    &cancellation,
                    deadline,
                    request_telemetry.as_ref(),
                    async {
                        self.provider
                            .consume_request_body(
                                logical.as_mut().expect("non-buffered logical owner exists"),
                                frame,
                            )
                            .await
                            .map_err(GatewayExecutionError::Provider)
                    },
                )
                .await?;
            }
        }
        while let Some(pause) = logical_pause {
            if pause == FilterPause::HeaderIteration {
                return Err(GatewayExecutionError::Filter(Arc::from(
                    "logical header iteration remained paused after request EOS",
                )));
            }
            let resumed = await_session_operation(
                &cancellation,
                deadline,
                request_telemetry.as_ref(),
                async {
                    request_filters
                        .filters
                        .wait_logical_resume(&mut head)
                        .await
                        .map_err(GatewayExecutionError::Filter)
                },
            )
            .await?;
            if let Some(reply) = resumed.local_reply {
                return emit_local_response(
                    session,
                    &mut final_writer,
                    &self.filter_executors,
                    &mut driver,
                    &mut binding,
                    &request_configs,
                    &mut request_filters.filters,
                    reply,
                    &budget,
                    request_id,
                    Some(route_binding),
                    None,
                    None,
                    deadline,
                    &cancellation,
                    request_telemetry.clone(),
                    &downstream_method,
                    downstream_protocol,
                    SessionReuse::Close,
                )
                .await
                .map(RequestPreparation::Completed);
            }
            if logical_is_buffered {
                buffered_logical_frames.extend(resumed.frames);
            } else {
                for frame in resumed.frames {
                    await_session_operation(
                        &cancellation,
                        deadline,
                        request_telemetry.as_ref(),
                        async {
                            self.provider
                                .consume_request_body(
                                    logical.as_mut().expect("logical owner exists after EOS"),
                                    frame,
                                )
                                .await
                                .map_err(GatewayExecutionError::Provider)
                        },
                    )
                    .await?;
                }
            }
            logical_pause = resumed.pause;
        }
        debug_assert_eq!(logical_body_owner.observed_bytes(), observed_body_bytes);
        if logical_is_buffered {
            let emitted_body_bytes = buffered_logical_frames
                .iter()
                .try_fold(0usize, |total, frame| {
                    total.checked_add(frame.bytes.as_ref().map_or(0, |bytes| bytes.bytes().len()))
                })
                .ok_or(BodyError::BodyLimitExceeded)?;
            record_framing_mutations(
                &mut logical_framing,
                &head.headers,
                original_request_content_length.as_ref(),
                original_request_transfer_encoding.as_ref(),
            );
            logical_framing.buffered_eos(emitted_body_bytes)?;
            logical_framing.finalize(
                &mut head.headers,
                protocol_framing(head.protocol),
                Some(&head.method),
                None,
            )?;
            logical = Some(
                await_request_operation(
                    session,
                    &cancellation,
                    deadline,
                    request_telemetry.as_ref(),
                    async {
                        self.provider
                            .begin_request(
                                head,
                                LogicalRequestContext {
                                    budget: &budget,
                                    plan: &logical_request_plan,
                                    hard_total_limit: self.limits.max_request_body_bytes,
                                    chunk_capacity: logical_request_chunk_capacity,
                                    frozen_candidates: frozen_candidates.clone(),
                                    provider_context: provider_context.clone(),
                                },
                            )
                            .await
                            .map_err(GatewayExecutionError::Provider)
                    },
                )
                .await?,
            );
            for frame in buffered_logical_frames {
                await_request_operation(
                    session,
                    &cancellation,
                    deadline,
                    request_telemetry.as_ref(),
                    async {
                        self.provider
                            .consume_request_body(
                                logical
                                    .as_mut()
                                    .expect("buffered logical owner exists after EOS"),
                                frame,
                            )
                            .await
                            .map_err(GatewayExecutionError::Provider)
                    },
                )
                .await?;
            }
        } else if has_logical_filters && committed_logical_head.as_ref() != Some(&head) {
            return Err(GatewayExecutionError::LogicalHeaderMutationAfterCommit);
        }
        let mut logical = Some(logical.expect("logical owner exists after body execution"));
        let route_context = self
            .provider
            .finalize_route_request_context(
                logical
                    .as_mut()
                    .expect("logical owner exists before decision-session creation"),
            )
            .map_err(GatewayExecutionError::Provider)?;
        let max_attempts = admitted_max_attempts;
        let candidates: Arc<[DecisionCandidateAuthority]> = match frozen_candidates {
            Some(candidates) => candidates,
            None => binding
                .candidate_bindings()?
                .iter()
                .copied()
                .map(|candidate| {
                    let attempt = binding.resolve_attempt(candidate)?;
                    Ok(DecisionCandidateAuthority {
                        binding: candidate,
                        stable_target: ObservationLabel::new(
                            attempt.plan().stable_target_key.as_str(),
                        )
                        .map_err(GatewayExecutionError::Selection)?,
                        credential_refs: Arc::clone(&attempt.plan().credential_refs),
                        candidate_id: None,
                        profile_digest: None,
                        reason_ledger_identity: None,
                        provider_profile: None,
                    })
                })
                .collect::<Result<Vec<_>, GatewayExecutionError>>()?
                .into(),
        };
        let candidate_bindings: Arc<[ResolvedTargetBindingId]> = candidates
            .iter()
            .map(|candidate| candidate.binding)
            .collect::<Vec<_>>()
            .into();
        let decision_session = self
            .selection
            .begin_authorized_session(
                DecisionSessionRequest {
                    request_id,
                    route_binding,
                    candidate_bindings: Arc::clone(&candidate_bindings),
                    route_context,
                    overall_deadline: deadline,
                    max_attempts,
                },
                candidates,
            )
            .map_err(GatewayExecutionError::Selection)?;
        let route_decision_id = decision_session.route_decision_id();
        let request_telemetry =
            request_telemetry.map(|telemetry| telemetry.with_decision(route_decision_id.0));
        driver.finish_logical_filters()?;
        drop(logical_binding);
        let leases = RequestLeaseBook::new();
        let generation = AttemptGeneration(1);
        let completed_attempts = Vec::new();
        // A runtime observation source may legitimately reuse one immutable
        // fresh snapshot across fallbacks. Keep one request-local, attempt-
        // bounded identity table and reject only identity equivocation.
        let routing_snapshots: HashMap<RoutingFactsSnapshotId, Arc<[FreshRoutingFact]>> =
            HashMap::new();

        Ok(RequestPreparation::Ready(RequestRun {
            cancellation,
            request_id,
            binding,
            request_configs,
            driver,
            budget,
            final_writer,
            run_filter_callbacks,
            downstream_method,
            downstream_protocol,
            route_binding,
            route_accepted_body_plan,
            deadline,
            logical,
            decision_session,
            route_decision_id,
            request_telemetry,
            max_attempts,
            candidate_bindings,
            leases,
            generation,
            completed_attempts,
            routing_snapshots,
        }))
    }
}
