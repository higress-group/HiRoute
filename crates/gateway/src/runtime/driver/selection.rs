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

mod preparation;
pub(crate) use preparation::PreparationFailure;

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
    reasoning_cleanup: Arc<super::ReasoningCleanup>,
    temporary_block_seen: bool,
    probe_busy_seen: bool,
    earliest_retry_at: Option<Instant>,
    non_temporary_block_seen: bool,
    upstream_attempted: bool,
    last_context_overflow: bool,
    preparation_failure: Option<PreparationFailure>,
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
        if self.upstream_attempted {
            if self.last_context_overflow && now < self.overall_deadline {
                return Ok(Some(super::context_overflow::local_reply()));
            }
            return Ok(None);
        }
        if !self.temporary_block_seen || self.non_temporary_block_seen {
            return Ok(self
                .preparation_failure
                .map(PreparationFailure::local_reply));
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
        self.last_context_overflow = selected.attempt_id.0 != 0
            && facts
                .error_class
                .as_ref()
                .is_some_and(|class| class.as_str() == "context_length");
        if facts
            .error_class
            .as_ref()
            .is_some_and(|class| class.as_str() == "reasoning_history")
            && self.candidate_index == 0
            && self
                .reasoning_cleanup
                .eligible
                .is_some_and(|(binding, _)| binding == selected.binding)
        {
            self.ensure_selected_matches_cursor(selected)?;
            if self.reasoning_cleanup.consume(selected.binding) {
                return Ok(Disposition::Continue);
            }
            return Ok(if self.advance_candidate() {
                Disposition::Continue
            } else {
                Disposition::Terminate
            });
        }
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
        // A later transport, authorization or preparation failure supersedes
        // an earlier response cause; never instruct compaction for that failure.
        self.last_context_overflow = false;
        let error_class = failure
            .provider
            .as_ref()
            .and_then(|provider| provider.error_class.as_ref())
            .map(ObservationLabel::as_str);
        let temporary = matches!(
            error_class,
            Some("runtime_state_cooling_down" | "runtime_state_probe_busy")
        );
        if selected.attempt_id.0 == 0 {
            let cause = PreparationFailure::from_error_class(error_class);
            self.preparation_failure = Some(
                self.preparation_failure
                    .map_or(cause, |previous| previous.merge(cause)),
            );
        }
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
            let _ = store.complete_with_cleanup(
                &completion.ticket,
                preference.clone(),
                self.reasoning_cleanup.prefix_for(observation.binding),
                observation.ended_at,
            );
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
            reasoning_cleanup: route_context.reasoning_cleanup,
            temporary_block_seen: false,
            probe_busy_seen: false,
            earliest_retry_at: None,
            non_temporary_block_seen: route_context.admission_preparation_failure.is_some(),
            upstream_attempted: false,
            last_context_overflow: false,
            preparation_failure: route_context.admission_preparation_failure,
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
mod tests;
