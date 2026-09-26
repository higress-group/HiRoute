use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use hiroute_gateway_core::core::execution_plan::ResolvedTargetBindingId;
use hiroute_gateway_core::core::filter::LocalReply;
use hiroute_gateway_core::runtime::attempt::{
    AcceptBlockedReason, AttemptId, Disposition, PublishedDisposition,
};
use hiroute_gateway_core::runtime::driver::{
    AttemptBudgetGrant, AttemptCleanupOutcome, AttemptDownstreamOutcome, AttemptStreamOutcome,
    AttemptTerminationReason, CompletedAttemptObservation, DecisionCandidateAuthority,
    DecisionSessionPort, DecisionSessionRequest, ObservationLabel, ProviderClassificationFacts,
    RealtimeRoutingFacts, RetryabilityFact, RouteDecisionId, SelectedGatewayAttempt,
    SelectionPublicationPort, SelectionRequest,
};
use hiroute_gateway_core::runtime::sse::SemanticProvenance;
use http::header::{CONTENT_TYPE, RETRY_AFTER};
use http::{HeaderMap, HeaderValue, StatusCode};

use crate::context_hold::{ContextHoldStore, HoldCompletion};

use super::ProductionRouteContext;

#[derive(Clone, Default)]
pub struct ProductionSelection;

pub struct ProductionDecisionSession {
    request_id: hiroute_gateway_core::runtime::attempt::RequestId,
    route_decision_id: RouteDecisionId,
    candidates: Arc<[DecisionCandidateAuthority]>,
    candidate_index: usize,
    credential_index: usize,
    overall_deadline: Instant,
    hold: Option<(Arc<ContextHoldStore>, HoldCompletion)>,
    switch_fallback: Option<ResolvedTargetBindingId>,
    temporary_block_seen: bool,
    probe_busy_seen: bool,
    earliest_retry_at: Option<Instant>,
    non_temporary_block_seen: bool,
    upstream_attempted: bool,
}

impl DecisionSessionPort for ProductionDecisionSession {
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
        let Some(candidate) = self.candidates.get(self.candidate_index) else {
            return Ok(None);
        };
        let credential_ref = candidate
            .credential_refs
            .get(self.credential_index)
            .cloned()
            .ok_or_else(|| Arc::from("candidate has no authorized credential reference"))?;
        let issued_at = Instant::now();
        let allocated = self
            .overall_deadline
            .saturating_duration_since(issued_at)
            .min(request.remaining_total);
        if allocated.is_zero() {
            return Ok(None);
        }
        Ok(Some(SelectedGatewayAttempt {
            request_id: self.request_id,
            // Core promotes this provisional candidate only after secret,
            // RuntimeState, protocol projection and DNS all succeed.
            attempt_id: AttemptId(0),
            generation: request.generation,
            binding: candidate.binding,
            credential_ref,
            route_decision_id: self.route_decision_id,
            budget: AttemptBudgetGrant {
                issued_at,
                allocated,
                deadline: self.overall_deadline,
            },
        }))
    }

    fn exhaustion_local_reply(&mut self, now: Instant) -> Result<Option<LocalReply>, Arc<str>> {
        if !self.temporary_block_seen || self.non_temporary_block_seen || self.upstream_attempted {
            return Ok(None);
        }
        let (code, retry_after_seconds) = if self.probe_busy_seen {
            ("RECOVERY_PROBE_IN_PROGRESS", 1)
        } else {
            let remaining = self
                .earliest_retry_at
                .and_then(|deadline| deadline.checked_duration_since(now))
                .unwrap_or(Duration::from_secs(1));
            let seconds = retry_after_seconds(remaining);
            ("CANDIDATES_COOLING_DOWN", seconds)
        };
        let retry_after_seconds = retry_after_seconds.max(1);
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            RETRY_AFTER,
            HeaderValue::from_str(&retry_after_seconds.to_string())
                .map_err(|_| Arc::from("invalid retry-after response"))?,
        );
        let body = Bytes::from(format!(
            "{{\"schema_version\":\"hiroute.gateway.error/v1\",\"code\":\"{code}\",\"phase\":\"runtime_state\",\"retry_after_seconds\":{retry_after_seconds}}}"
        ));
        Ok(Some(LocalReply {
            status: StatusCode::SERVICE_UNAVAILABLE,
            headers,
            body,
            provenance: SemanticProvenance::NonSemantic,
        }))
    }

    fn decide(
        &mut self,
        selected: &SelectedGatewayAttempt,
        facts: &ProviderClassificationFacts,
        _transport: &hiroute_gateway_core::runtime::attempt::AttemptTransportFacts,
    ) -> Result<Disposition, Arc<str>> {
        self.upstream_attempted |= selected.attempt_id.0 != 0;
        if facts.retryability != RetryabilityFact::Retryable {
            if facts.error_class.is_some() && self.can_switch_fallback(selected) {
                self.ensure_selected_matches_cursor(selected)?;
                self.advance_candidate();
                return Ok(Disposition::Continue);
            }
            return Ok(if facts.error_class.is_some() {
                Disposition::Terminate
            } else {
                Disposition::Accept
            });
        }
        self.ensure_selected_matches_cursor(selected)?;
        let next_key = facts
            .error_class
            .as_ref()
            .is_some_and(|class| matches!(class.as_str(), "credential" | "quota"));
        let has_next = if next_key {
            self.advance_credential()
        } else {
            self.advance_candidate()
        };
        Ok(if has_next {
            Disposition::Continue
        } else {
            Disposition::Terminate
        })
    }

    fn decide_failure(
        &mut self,
        selected: &SelectedGatewayAttempt,
        failure: &hiroute_gateway_core::runtime::driver::AttemptFailureFacts,
    ) -> Result<Disposition, Arc<str>> {
        self.upstream_attempted |= selected.attempt_id.0 != 0;
        let error_class = failure
            .provider
            .as_ref()
            .and_then(|provider| provider.error_class.as_ref())
            .map(ObservationLabel::as_str);
        let temporary = matches!(
            error_class,
            Some("runtime_state_cooling_down" | "runtime_state_probe_busy")
        );
        if temporary && selected.attempt_id.0 == 0 {
            self.temporary_block_seen = true;
            if error_class == Some("runtime_state_probe_busy") {
                self.probe_busy_seen = true;
            }
            if let Some(retry_after) = failure
                .provider
                .as_ref()
                .and_then(|provider| provider.retry_after)
                && let Some(deadline) = Instant::now().checked_add(retry_after)
            {
                self.earliest_retry_at = Some(
                    self.earliest_retry_at
                        .map_or(deadline, |current| current.min(deadline)),
                );
            }
        } else {
            self.non_temporary_block_seen = true;
        }
        if failure
            .provider
            .as_ref()
            .is_none_or(|provider| provider.retryability != RetryabilityFact::Retryable)
        {
            if failure.request_committed && self.can_switch_fallback(selected) {
                self.ensure_selected_matches_cursor(selected)?;
                self.advance_candidate();
                return Ok(Disposition::Continue);
            }
            return Ok(Disposition::Terminate);
        }
        self.ensure_selected_matches_cursor(selected)?;
        let has_next = if failure.class
            == hiroute_gateway_core::runtime::driver::AttemptFailureClass::Credential
        {
            self.advance_credential()
        } else {
            self.advance_candidate()
        };
        Ok(if has_next || temporary {
            Disposition::Continue
        } else {
            Disposition::Terminate
        })
    }

    fn replace_blocked_accept(
        &mut self,
        _selected: &SelectedGatewayAttempt,
        _reason: AcceptBlockedReason,
    ) -> Result<Disposition, Arc<str>> {
        Ok(Disposition::Terminate)
    }

    fn observe_published(&mut self, _disposition: &PublishedDisposition) -> Result<(), Arc<str>> {
        Ok(())
    }

    fn observe_completed(
        &mut self,
        observation: &CompletedAttemptObservation,
    ) -> Result<(), Arc<str>> {
        if observation.route_decision_id == self.route_decision_id
            && is_complete_accept(observation)
            && let Some((store, completion)) = &self.hold
            && let Some(preference) = completion.preference_for(observation.binding)
        {
            let _ = store.complete(&completion.ticket, preference.clone(), observation.ended_at);
        }
        Ok(())
    }
}

fn retry_after_seconds(remaining: Duration) -> u64 {
    remaining
        .as_secs()
        .saturating_add(u64::from(remaining.subsec_nanos() != 0))
        .max(1)
}

fn is_complete_accept(observation: &CompletedAttemptObservation) -> bool {
    observation.disposition == Disposition::Accept
        && observation
            .provider
            .as_ref()
            .and_then(|facts| facts.model_event.as_ref())
            .map(ObservationLabel::as_str)
            == Some("response_complete")
        && observation.failure.is_none()
        && observation.stream == AttemptStreamOutcome::CompletedEos
        && observation.downstream == AttemptDownstreamOutcome::Completed
        && observation.cleanup == AttemptCleanupOutcome::Completed
        && observation.termination_reason == AttemptTerminationReason::AcceptedEos
}

impl SelectionPublicationPort<ProductionRouteContext> for ProductionSelection {
    type Session = ProductionDecisionSession;

    fn begin_session(
        &self,
        _request: DecisionSessionRequest<ProductionRouteContext>,
    ) -> Result<Self::Session, Arc<str>> {
        Err(Arc::from(
            "production selection requires an authorized candidate closure",
        ))
    }

    fn begin_authorized_session(
        &self,
        request: DecisionSessionRequest<ProductionRouteContext>,
        candidates: Arc<[DecisionCandidateAuthority]>,
    ) -> Result<Self::Session, Arc<str>> {
        if candidates.is_empty()
            || candidates.len() != request.candidate_bindings.len()
            || candidates
                .iter()
                .zip(request.candidate_bindings.iter())
                .any(|(candidate, binding)| candidate.binding != *binding)
        {
            return Err(Arc::from("authorized candidate order is inconsistent"));
        }
        let route_context = request.route_context;
        if route_context.switch_fallback.is_some_and(|fallback| {
            candidates
                .first()
                .is_none_or(|first| first.binding == fallback)
                || candidates
                    .get(1)
                    .is_none_or(|second| second.binding != fallback)
        }) {
            return Err(Arc::from(
                "switch fallback is outside the frozen second candidate",
            ));
        }
        Ok(ProductionDecisionSession {
            request_id: request.request_id,
            route_decision_id: RouteDecisionId(request.request_id.0),
            candidates,
            candidate_index: 0,
            credential_index: 0,
            overall_deadline: request.overall_deadline,
            hold: route_context
                .hold_completion
                .map(|completion| (route_context.context_holds, completion)),
            switch_fallback: route_context.switch_fallback,
            temporary_block_seen: false,
            probe_busy_seen: false,
            earliest_retry_at: None,
            non_temporary_block_seen: false,
            upstream_attempted: false,
        })
    }
}

impl ProductionDecisionSession {
    fn can_switch_fallback(&self, selected: &SelectedGatewayAttempt) -> bool {
        selected.attempt_id.0 != 0
            && self.candidate_index == 0
            && self.switch_fallback.is_some_and(|fallback| {
                self.candidates
                    .get(1)
                    .is_some_and(|candidate| candidate.binding == fallback)
            })
            && self
                .candidates
                .first()
                .is_some_and(|candidate| candidate.binding == selected.binding)
    }

    fn ensure_selected_matches_cursor(
        &self,
        selected: &SelectedGatewayAttempt,
    ) -> Result<(), Arc<str>> {
        let candidate = self
            .candidates
            .get(self.candidate_index)
            .ok_or_else(|| Arc::from("selected candidate is outside the frozen closure"))?;
        if selected.binding != candidate.binding
            || candidate.credential_refs.get(self.credential_index)
                != Some(&selected.credential_ref)
        {
            return Err(Arc::from(
                "selected credential is outside the current frozen cursor",
            ));
        }
        Ok(())
    }

    fn advance_credential(&mut self) -> bool {
        let next = self.credential_index.saturating_add(1);
        if self
            .candidates
            .get(self.candidate_index)
            .is_some_and(|candidate| next < candidate.credential_refs.len())
        {
            self.credential_index = next;
            true
        } else {
            self.advance_candidate()
        }
    }

    fn advance_candidate(&mut self) -> bool {
        self.candidate_index = self.candidate_index.saturating_add(1);
        self.credential_index = 0;
        self.candidate_index < self.candidates.len()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use hiroute_gateway_core::core::execution_plan::{
        CredentialRef, PlanRevision, ResolvedTargetBindingId,
    };
    use hiroute_gateway_core::runtime::attempt::{
        AttemptGeneration, AttemptTransportFacts, CommitFence, RequestId,
    };
    use hiroute_gateway_core::runtime::driver::{
        AttemptCommitFacts, AttemptFailureClass, AttemptFailureFacts,
    };

    use super::*;

    fn candidate(binding_local_id: u32, credential_refs: &[&str]) -> DecisionCandidateAuthority {
        DecisionCandidateAuthority {
            binding: ResolvedTargetBindingId::new(PlanRevision(1), binding_local_id),
            stable_target: ObservationLabel::new(format!("binding-{binding_local_id}")).unwrap(),
            credential_refs: credential_refs
                .iter()
                .map(|value| CredentialRef::new(*value).unwrap())
                .collect::<Vec<_>>()
                .into(),
            candidate_id: None,
            profile_digest: None,
            reason_ledger_identity: None,
            provider_profile: None,
        }
    }

    fn selection_request<'a>(
        _now: Instant,
        generation: u64,
        remaining_attempts: u32,
        realtime_facts: &'a RealtimeRoutingFacts,
    ) -> SelectionRequest<'a> {
        SelectionRequest {
            request_id: RequestId(1),
            generation: AttemptGeneration(generation),
            route_binding: ResolvedTargetBindingId::new(PlanRevision(1), 1),
            remaining_total: Duration::from_secs(1),
            remaining_attempts,
            realtime_facts,
            previous_attempts: &[],
        }
    }

    fn session(
        now: Instant,
        candidates: Vec<DecisionCandidateAuthority>,
    ) -> ProductionDecisionSession {
        ProductionDecisionSession {
            request_id: RequestId(1),
            route_decision_id: RouteDecisionId(1),
            candidates: candidates.into(),
            candidate_index: 0,
            credential_index: 0,
            overall_deadline: now + Duration::from_secs(10),
            hold: None,
            switch_fallback: None,
            temporary_block_seen: false,
            probe_busy_seen: false,
            earliest_retry_at: None,
            non_temporary_block_seen: false,
            upstream_attempted: false,
        }
    }

    fn materialization_failure(
        now: Instant,
        error_class: &str,
        retry_after: Option<Duration>,
    ) -> AttemptFailureFacts {
        AttemptFailureFacts {
            class: AttemptFailureClass::Materialization,
            provider: Some(ProviderClassificationFacts {
                error_class: Some(ObservationLabel::new(error_class).unwrap()),
                retryability: RetryabilityFact::Retryable,
                retry_after,
                readiness: ObservationLabel::new("materialization_failed").unwrap(),
                ..ProviderClassificationFacts::default()
            }),
            transport: AttemptTransportFacts {
                started_at: now,
                connect_elapsed: None,
                request_write_elapsed: None,
                upstream_ttfb: None,
                last_upstream_progress_at: None,
                local_read_suppressed: Duration::ZERO,
                upstream_body_bytes: 0,
                timeout: None,
            },
            request_committed: false,
            termination_reason: ObservationLabel::new("materialization").unwrap(),
        }
    }

    #[test]
    fn retryable_quota_advances_exact_credential_before_binding() {
        let now = Instant::now();
        let realtime_facts = RealtimeRoutingFacts::default();
        let transport = AttemptTransportFacts {
            started_at: now,
            connect_elapsed: None,
            request_write_elapsed: None,
            upstream_ttfb: None,
            last_upstream_progress_at: None,
            local_read_suppressed: Duration::ZERO,
            upstream_body_bytes: 0,
            timeout: None,
        };
        let mut session = ProductionDecisionSession {
            request_id: RequestId(1),
            route_decision_id: RouteDecisionId(1),
            candidates: vec![
                candidate(1, &["credential-a", "credential-b"]),
                candidate(2, &["credential-c"]),
            ]
            .into(),
            candidate_index: 0,
            credential_index: 0,
            overall_deadline: now + Duration::from_secs(1),
            hold: None,
            switch_fallback: None,
            temporary_block_seen: false,
            probe_busy_seen: false,
            earliest_retry_at: None,
            non_temporary_block_seen: false,
            upstream_attempted: false,
        };
        let first = session
            .select_next(selection_request(now, 1, 3, &realtime_facts))
            .unwrap()
            .unwrap();
        assert_eq!(first.binding.local_id(), 1);
        assert_eq!(first.credential_ref.as_str(), "credential-a");

        let quota = ProviderClassificationFacts {
            error_class: Some(ObservationLabel::new("quota").unwrap()),
            retryability: RetryabilityFact::Retryable,
            readiness: ObservationLabel::new("rejected").unwrap(),
            ..ProviderClassificationFacts::default()
        };
        assert_eq!(
            session.decide(&first, &quota, &transport).unwrap(),
            Disposition::Continue
        );
        let second = session
            .select_next(selection_request(now, 2, 2, &realtime_facts))
            .unwrap()
            .unwrap();
        assert_eq!(second.binding.local_id(), 1);
        assert_eq!(second.credential_ref.as_str(), "credential-b");

        assert_eq!(
            session.decide(&second, &quota, &transport).unwrap(),
            Disposition::Continue
        );
        let third = session
            .select_next(selection_request(now, 3, 1, &realtime_facts))
            .unwrap()
            .unwrap();
        assert_eq!(third.binding.local_id(), 2);
        assert_eq!(third.credential_ref.as_str(), "credential-c");
    }

    #[test]
    fn switched_candidate_precommit_400_falls_back_to_previous_once() {
        let now = Instant::now();
        let realtime_facts = RealtimeRoutingFacts::default();
        let mut session = session(
            now,
            vec![candidate(1, &["new-key"]), candidate(2, &["original-key"])],
        );
        session.switch_fallback = Some(ResolvedTargetBindingId::new(PlanRevision(1), 2));
        let mut selected = session
            .select_next(selection_request(now, 1, 2, &realtime_facts))
            .unwrap()
            .unwrap();
        // The core promotes the candidate after pre-exchange materialization.
        selected.attempt_id = AttemptId(1);
        let rejected = ProviderClassificationFacts {
            error_class: Some(ObservationLabel::new("permanent_client").unwrap()),
            retryability: RetryabilityFact::NonRetryable,
            http_status: Some(StatusCode::BAD_REQUEST),
            ..ProviderClassificationFacts::default()
        };
        assert_eq!(
            session
                .decide(
                    &selected,
                    &rejected,
                    &AttemptTransportFacts {
                        started_at: now,
                        connect_elapsed: None,
                        request_write_elapsed: None,
                        upstream_ttfb: None,
                        last_upstream_progress_at: None,
                        local_read_suppressed: Duration::ZERO,
                        upstream_body_bytes: 0,
                        timeout: None,
                    }
                )
                .unwrap(),
            Disposition::Continue
        );
        let original = session
            .select_next(selection_request(now, 2, 1, &realtime_facts))
            .unwrap()
            .unwrap();
        assert_eq!(original.binding.local_id(), 2);
        assert_eq!(
            session
                .decide(
                    &original,
                    &rejected,
                    &AttemptTransportFacts {
                        started_at: now,
                        connect_elapsed: None,
                        request_write_elapsed: None,
                        upstream_ttfb: None,
                        last_upstream_progress_at: None,
                        local_read_suppressed: Duration::ZERO,
                        upstream_body_bytes: 0,
                        timeout: None,
                    }
                )
                .unwrap(),
            Disposition::Terminate
        );
    }

    #[test]
    fn unknown_or_nonretryable_materialization_never_advances_selection() {
        let now = Instant::now();
        let selected = SelectedGatewayAttempt {
            request_id: RequestId(1),
            attempt_id: AttemptId(0),
            generation: AttemptGeneration(1),
            binding: ResolvedTargetBindingId::new(PlanRevision(1), 1),
            credential_ref: CredentialRef::new("credential").unwrap(),
            route_decision_id: RouteDecisionId(1),
            budget: AttemptBudgetGrant {
                issued_at: now,
                allocated: Duration::from_secs(1),
                deadline: now + Duration::from_secs(1),
            },
        };
        let mut session = ProductionDecisionSession {
            request_id: RequestId(1),
            route_decision_id: RouteDecisionId(1),
            candidates: Arc::new([]),
            candidate_index: 0,
            credential_index: 0,
            overall_deadline: now + Duration::from_secs(1),
            hold: None,
            switch_fallback: None,
            temporary_block_seen: false,
            probe_busy_seen: false,
            earliest_retry_at: None,
            non_temporary_block_seen: false,
            upstream_attempted: false,
        };
        for retryability in [RetryabilityFact::Unknown, RetryabilityFact::NonRetryable] {
            let failure = AttemptFailureFacts {
                class: AttemptFailureClass::Materialization,
                provider: Some(ProviderClassificationFacts {
                    retryability,
                    readiness: ObservationLabel::new("materialization_failed").unwrap(),
                    ..ProviderClassificationFacts::default()
                }),
                transport: AttemptTransportFacts {
                    started_at: now,
                    connect_elapsed: None,
                    request_write_elapsed: None,
                    upstream_ttfb: None,
                    last_upstream_progress_at: None,
                    local_read_suppressed: Duration::ZERO,
                    upstream_body_bytes: 0,
                    timeout: None,
                },
                request_committed: false,
                termination_reason: ObservationLabel::new("materialization").unwrap(),
            };
            assert_eq!(
                session.decide_failure(&selected, &failure).unwrap(),
                Disposition::Terminate
            );
            assert_eq!(session.candidate_index, 0);
            assert_eq!(session.credential_index, 0);
        }
    }

    #[test]
    fn switched_provisional_nonretryable_failure_does_not_call_previous_model() {
        let now = Instant::now();
        let realtime_facts = RealtimeRoutingFacts::default();
        let mut session = session(
            now,
            vec![candidate(1, &["new-key"]), candidate(2, &["original-key"])],
        );
        session.switch_fallback = Some(ResolvedTargetBindingId::new(PlanRevision(1), 2));
        let selected = session
            .select_next(selection_request(now, 1, 2, &realtime_facts))
            .unwrap()
            .unwrap();
        assert_eq!(selected.attempt_id, AttemptId(0));

        let mut failure = materialization_failure(now, "materialization_fail_closed", None);
        failure.provider.as_mut().unwrap().retryability = RetryabilityFact::NonRetryable;
        assert_eq!(
            session.decide_failure(&selected, &failure).unwrap(),
            Disposition::Terminate
        );
        assert_eq!(session.candidate_index, 0);
    }

    #[test]
    fn switched_promoted_preexchange_failure_does_not_call_previous_model() {
        let now = Instant::now();
        let realtime_facts = RealtimeRoutingFacts::default();
        let mut session = session(
            now,
            vec![candidate(1, &["new-key"]), candidate(2, &["original-key"])],
        );
        session.switch_fallback = Some(ResolvedTargetBindingId::new(PlanRevision(1), 2));
        let mut selected = session
            .select_next(selection_request(now, 1, 2, &realtime_facts))
            .unwrap()
            .unwrap();
        selected.attempt_id = AttemptId(1);

        let mut failure = materialization_failure(now, "preexchange_filter_failure", None);
        failure.provider.as_mut().unwrap().retryability = RetryabilityFact::NonRetryable;
        assert!(!failure.request_committed);
        assert_eq!(
            session.decide_failure(&selected, &failure).unwrap(),
            Disposition::Terminate
        );
        assert_eq!(session.candidate_index, 0);
    }

    #[test]
    fn pure_runtime_state_exhaustion_returns_retryable_503_and_probe_busy_wins() {
        let now = Instant::now();
        let realtime = RealtimeRoutingFacts::default();
        let mut session = session(
            now,
            vec![
                candidate(1, &["credential-a"]),
                candidate(2, &["credential-b"]),
            ],
        );
        let first = session
            .select_next(selection_request(now, 1, 3, &realtime))
            .unwrap()
            .unwrap();
        assert_eq!(
            session
                .decide_failure(
                    &first,
                    &materialization_failure(
                        now,
                        "runtime_state_cooling_down",
                        Some(Duration::from_millis(1_500)),
                    ),
                )
                .unwrap(),
            Disposition::Continue
        );
        let second = session
            .select_next(selection_request(now, 1, 3, &realtime))
            .unwrap()
            .unwrap();
        assert_eq!(
            session
                .decide_failure(
                    &second,
                    &materialization_failure(
                        now,
                        "runtime_state_probe_busy",
                        Some(Duration::from_secs(1)),
                    ),
                )
                .unwrap(),
            Disposition::Continue
        );
        assert!(
            session
                .select_next(selection_request(now, 1, 3, &realtime))
                .unwrap()
                .is_none()
        );
        let reply = session
            .exhaustion_local_reply(now)
            .unwrap()
            .expect("pure temporary exhaustion has a retry response");
        assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(reply.headers.get(RETRY_AFTER).unwrap(), "1");
        assert_eq!(reply.headers.get(CONTENT_TYPE).unwrap(), "application/json");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&reply.body).unwrap(),
            serde_json::json!({
                "schema_version": "hiroute.gateway.error/v1",
                "code": "RECOVERY_PROBE_IN_PROGRESS",
                "phase": "runtime_state",
                "retry_after_seconds": 1,
            })
        );
    }

    #[test]
    fn cooling_exhaustion_uses_earliest_deadline_and_rounds_up() {
        assert_eq!(retry_after_seconds(Duration::from_nanos(1)), 1);
        assert_eq!(retry_after_seconds(Duration::from_nanos(1_000_000_001)), 2);

        let now = Instant::now();
        let realtime = RealtimeRoutingFacts::default();
        let mut session = session(
            now,
            vec![
                candidate(1, &["credential-a"]),
                candidate(2, &["credential-b"]),
            ],
        );
        let first = session
            .select_next(selection_request(now, 1, 3, &realtime))
            .unwrap()
            .unwrap();
        assert_eq!(
            session
                .decide_failure(
                    &first,
                    &materialization_failure(
                        now,
                        "runtime_state_cooling_down",
                        Some(Duration::from_secs(12)),
                    ),
                )
                .unwrap(),
            Disposition::Continue
        );
        let second = session
            .select_next(selection_request(now, 1, 3, &realtime))
            .unwrap()
            .unwrap();
        assert_eq!(
            session
                .decide_failure(
                    &second,
                    &materialization_failure(
                        now,
                        "runtime_state_cooling_down",
                        Some(Duration::from_millis(3_200)),
                    ),
                )
                .unwrap(),
            Disposition::Continue
        );
        assert!(
            session
                .select_next(selection_request(now, 1, 3, &realtime))
                .unwrap()
                .is_none()
        );
        let reply = session
            .exhaustion_local_reply(now)
            .unwrap()
            .expect("pure cooldown exhaustion has a retry response");
        assert_eq!(reply.headers.get(RETRY_AFTER).unwrap(), "4");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&reply.body).unwrap(),
            serde_json::json!({
                "schema_version": "hiroute.gateway.error/v1",
                "code": "CANDIDATES_COOLING_DOWN",
                "phase": "runtime_state",
                "retry_after_seconds": 4,
            })
        );
    }

    #[test]
    fn mixed_runtime_state_exhaustion_keeps_generic_failure() {
        let now = Instant::now();
        let realtime = RealtimeRoutingFacts::default();
        let mut session = session(
            now,
            vec![
                candidate(1, &["credential-a"]),
                candidate(2, &["credential-b"]),
            ],
        );
        let first = session
            .select_next(selection_request(now, 1, 3, &realtime))
            .unwrap()
            .unwrap();
        session
            .decide_failure(
                &first,
                &materialization_failure(
                    now,
                    "runtime_state_cooling_down",
                    Some(Duration::from_secs(2)),
                ),
            )
            .unwrap();
        let second = session
            .select_next(selection_request(now, 1, 3, &realtime))
            .unwrap()
            .unwrap();
        assert_eq!(
            session
                .decide_failure(
                    &second,
                    &materialization_failure(now, "runtime_state_disabled", None),
                )
                .unwrap(),
            Disposition::Terminate
        );
        assert!(session.exhaustion_local_reply(now).unwrap().is_none());
    }

    #[test]
    fn hold_success_requires_full_accepted_eos_downstream_and_cleanup() {
        let now = Instant::now();
        let accepted = CompletedAttemptObservation {
            route_decision_id: RouteDecisionId(1),
            attempt_id: AttemptId(1),
            generation: AttemptGeneration(1),
            binding: ResolvedTargetBindingId::new(PlanRevision(1), 1),
            credential_ref: CredentialRef::new("credential").unwrap(),
            budget: AttemptBudgetGrant {
                issued_at: now,
                allocated: Duration::from_secs(1),
                deadline: now + Duration::from_secs(1),
            },
            routing_facts: RealtimeRoutingFacts::default(),
            provider: Some(ProviderClassificationFacts {
                model_event: Some(ObservationLabel::new("response_complete").unwrap()),
                ..ProviderClassificationFacts::default()
            }),
            failure: None,
            transport: AttemptTransportFacts {
                started_at: now,
                connect_elapsed: None,
                request_write_elapsed: None,
                upstream_ttfb: None,
                last_upstream_progress_at: None,
                local_read_suppressed: Duration::ZERO,
                upstream_body_bytes: 1,
                timeout: None,
            },
            disposition: Disposition::Accept,
            commits: AttemptCommitFacts {
                upstream_request: CommitFence::WriteConfirmed,
                downstream_headers: CommitFence::WriteConfirmed,
                downstream_semantic: CommitFence::WriteConfirmed,
            },
            stream: AttemptStreamOutcome::CompletedEos,
            downstream: AttemptDownstreamOutcome::Completed,
            cleanup: AttemptCleanupOutcome::Completed,
            ended_at: now,
            termination_reason: AttemptTerminationReason::AcceptedEos,
        };
        assert!(is_complete_accept(&accepted));
        let mut interrupted = accepted.clone();
        interrupted.stream = AttemptStreamOutcome::StreamStartedNoRetry;
        assert!(!is_complete_accept(&interrupted));
        let mut downstream_failed = accepted.clone();
        downstream_failed.downstream = AttemptDownstreamOutcome::Failed;
        assert!(!is_complete_accept(&downstream_failed));
        let mut cleanup_failed = accepted.clone();
        cleanup_failed.cleanup = AttemptCleanupOutcome::Failed;
        assert!(!is_complete_accept(&cleanup_failed));
        let mut incomplete = accepted.clone();
        incomplete.provider.as_mut().unwrap().model_event =
            Some(ObservationLabel::new("response_incomplete").unwrap());
        assert!(!is_complete_accept(&incomplete));
        let mut failed = accepted.clone();
        failed.provider.as_mut().unwrap().model_event =
            Some(ObservationLabel::new("response_failed").unwrap());
        assert!(!is_complete_accept(&failed));
        let mut missing_terminal = accepted.clone();
        missing_terminal.provider.as_mut().unwrap().model_event = None;
        assert!(!is_complete_accept(&missing_terminal));
        let mut terminated = accepted;
        terminated.disposition = Disposition::Terminate;
        assert!(!is_complete_accept(&terminated));
    }
}
