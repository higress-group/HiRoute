use super::*;

#[cfg(feature = "e2e-test-control")]
use crate::core::execution_plan::CaPolicy;

fn resolved_target_is_authorized(
    expected: &TransportTarget,
    target: &TransportTarget,
    policy: TransportTargetPolicy,
) -> bool {
    if target == expected {
        return true;
    }
    if policy == TransportTargetPolicy::ManagedLoopback
        && managed_loopback_rotation_is_authorized(expected, target)
    {
        return true;
    }
    #[cfg(feature = "e2e-test-control")]
    {
        e2e_test_control_override_is_authorized(expected, target)
    }
    #[cfg(not(feature = "e2e-test-control"))]
    false
}

fn managed_loopback_rotation_is_authorized(
    expected: &TransportTarget,
    target: &TransportTarget,
) -> bool {
    if !expected.is_numeric_loopback_http() || !target.is_numeric_loopback_http() {
        return false;
    }
    let mut authorized = expected.clone();
    authorized.authority = Arc::clone(&target.authority);
    authorized.addresses = Arc::clone(&target.addresses);
    authorized.connection_fingerprint = authorized.derive_connection_fingerprint();
    target == &authorized
}

#[cfg(feature = "e2e-test-control")]
fn e2e_test_control_override_is_authorized(
    expected: &TransportTarget,
    target: &TransportTarget,
) -> bool {
    if !matches!(expected.ca, CaPolicy::System) {
        return false;
    }
    let CaPolicy::Pem(pem) = &target.ca else {
        return false;
    };
    if pem.is_empty() {
        return false;
    }

    let mut authorized = expected.clone();
    authorized.ca = CaPolicy::Pem(Arc::clone(pem));
    authorized = authorized.with_derived_connection_fingerprint();
    target == &authorized && target.validate().is_ok()
}

impl<T: AttemptTransport> AttemptExchange<T> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request_id: RequestId,
        attempt_id: AttemptId,
        generation: AttemptGeneration,
        plan_revision: PlanRevision,
        target: TransportTarget,
        transport: T,
        request: PreparedAttemptHttpRequest,
        body_plans: AttemptBodyPlans,
        timeouts: AttemptTimeouts,
        attempt_deadline: Instant,
        precommit_event_capacity: usize,
        budget: StreamBudget,
        write_quantum: usize,
    ) -> Result<Self, AttemptError> {
        Self::new_with_cancellation(
            request_id,
            attempt_id,
            generation,
            plan_revision,
            target,
            transport,
            request,
            body_plans,
            timeouts,
            attempt_deadline,
            precommit_event_capacity,
            budget,
            write_quantum,
            CancellationToken::new(),
        )
    }

    /// Constructs a dynamic exchange bound to its source request's cancellation
    /// token. This is used by sealed auxiliary HTTP calls that do not have a
    /// compiled provider attempt plan but still share the request lifecycle.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_cancellation(
        request_id: RequestId,
        attempt_id: AttemptId,
        generation: AttemptGeneration,
        plan_revision: PlanRevision,
        target: TransportTarget,
        transport: T,
        request: PreparedAttemptHttpRequest,
        body_plans: AttemptBodyPlans,
        timeouts: AttemptTimeouts,
        attempt_deadline: Instant,
        precommit_event_capacity: usize,
        budget: StreamBudget,
        write_quantum: usize,
        cancellation: CancellationToken,
    ) -> Result<Self, AttemptError> {
        target
            .validate()
            .map_err(|error| AttemptError::InvalidTarget(error.to_string().into()))?;
        body_plans
            .validate()
            .map_err(|error| AttemptError::InvalidBodyPlan(error.to_string().into()))?;
        timeouts
            .validate()
            .map_err(|error| AttemptError::InvalidTarget(error.to_string().into()))?;
        if attempt_deadline <= Instant::now() {
            return Err(AttemptError::DeadlineExceeded);
        }
        Self::new_validated(
            request_id,
            attempt_id,
            generation,
            plan_revision,
            AttemptTargetOwner::Dynamic(target),
            transport,
            request,
            body_plans,
            timeouts,
            attempt_deadline,
            precommit_event_capacity,
            budget,
            write_quantum,
            cancellation,
        )
    }

    /// Constructs an exchange from an immutable publication-owned attempt
    /// plan. Publication preparation has already validated the target
    /// fingerprint and both attempt-local body plans, so the request hot path must not
    /// repeat those compilation-time checks (including SHA-256 derivation).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_from_compiled_plan(
        request_id: RequestId,
        attempt_id: AttemptId,
        generation: AttemptGeneration,
        plan_revision: PlanRevision,
        plan: Arc<CompiledAttemptPlan>,
        transport: T,
        request: PreparedAttemptHttpRequest,
        attempt_deadline: Instant,
        budget: StreamBudget,
        write_quantum: usize,
        cancellation: CancellationToken,
        resolved_target: Option<TransportTarget>,
    ) -> Result<Self, AttemptError> {
        let body_plans = plan.body_plans.clone();
        let timeouts = plan.timeouts;
        let precommit_event_capacity = plan.precommit_event_capacity;
        let target = match resolved_target {
            Some(target) => {
                let authorized = std::iter::once(&plan.transport_target)
                    .chain(plan.authorized_native_targets.iter())
                    .any(|expected| {
                        let resolved_expected = if expected.requires_resolution() {
                            expected
                                .clone()
                                .with_resolved_addresses(Arc::clone(&target.addresses))
                                .ok()
                        } else {
                            Some(expected.clone())
                        };
                        resolved_expected.is_some_and(|expected| {
                            resolved_target_is_authorized(
                                &expected,
                                &target,
                                plan.transport_target_policy,
                            )
                        })
                    });
                if !authorized {
                    return Err(AttemptError::InvalidTarget(
                        "resolved target does not match compiled target".into(),
                    ));
                }
                AttemptTargetOwner::Dynamic(target)
            }
            None => AttemptTargetOwner::Compiled(Arc::clone(&plan)),
        };
        Self::new_validated(
            request_id,
            attempt_id,
            generation,
            plan_revision,
            target,
            transport,
            request,
            body_plans,
            timeouts,
            attempt_deadline,
            precommit_event_capacity,
            budget,
            write_quantum,
            cancellation,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_validated(
        request_id: RequestId,
        attempt_id: AttemptId,
        generation: AttemptGeneration,
        plan_revision: PlanRevision,
        target: AttemptTargetOwner,
        transport: T,
        request: PreparedAttemptHttpRequest,
        body_plans: AttemptBodyPlans,
        timeouts: AttemptTimeouts,
        attempt_deadline: Instant,
        precommit_event_capacity: usize,
        budget: StreamBudget,
        write_quantum: usize,
        cancellation: CancellationToken,
    ) -> Result<Self, AttemptError> {
        if write_quantum == 0 {
            return Err(AttemptError::ZeroWriteQuantum);
        }
        if attempt_deadline <= Instant::now() {
            return Err(AttemptError::DeadlineExceeded);
        }
        let attempt_started_at = Instant::now();
        if precommit_event_capacity == 0 {
            return Err(AttemptError::ZeroPrecommitCapacity);
        }
        if request.body.max_chunk_bytes() > write_quantum {
            return Err(AttemptError::RequestChunkExceedsWriteQuantum);
        }
        let mut attempt_request_owner = BodyPlanExecutor::new(
            BodyDirection::AttemptRequest,
            body_plans.attempt_request.clone(),
            usize::MAX,
        )?;
        request
            .body
            .validate_body_plan(&mut attempt_request_owner)?;
        let precommit_body_owner = BodyPlanExecutor::new(
            BodyDirection::AttemptResponsePrecommit,
            body_plans.attempt_response_precommit.clone(),
            usize::MAX,
        )?;
        let mailbox_bytes = precommit_event_capacity
            .checked_mul(size_of::<PrecommitEvent>())
            .ok_or(AttemptError::Body(BodyError::BudgetExceeded))?;
        let response_mailbox_reservation =
            budget.reserve(MemoryRole::ResponsePrefix, mailbox_bytes)?;
        // Reserve the full logical mailbox before allocation, but keep the
        // empty fast path allocation-free until the transport actually
        // produces a response event.
        let response_window = PrecommitWindow::new();
        Ok(Self {
            request_id,
            attempt_id,
            generation,
            plan_revision,
            target,
            transport: Some(transport),
            request,
            semantic_upstream_calls: 0,
            connection_sub_attempts: 0,
            connected: false,
            request_framing_reconciled: false,
            pending_request_write: None,
            writer_state: WriterState::NotStarted,
            upstream_request_fence: CommitFence::Clear,
            downstream_header_fence: CommitFence::Clear,
            downstream_semantic_fence: CommitFence::Clear,
            accepted_response_scope_created: false,
            response_window,
            response_window_capacity: precommit_event_capacity,
            response_mailbox_reservation: Some(response_mailbox_reservation),
            transport_codec_reservation: None,
            precommit_body_owner,
            body_plans,
            timeouts,
            attempt_deadline,
            attempt_started_at,
            connect_elapsed: None,
            request_write_started_at: None,
            request_write_elapsed: None,
            first_upstream_receipt_at: None,
            last_upstream_progress_at: None,
            local_read_suppressed: Duration::ZERO,
            transport_suppression_observed: Duration::ZERO,
            idle_suppression_credit: Duration::ZERO,
            upstream_body_bytes: 0,
            timeout_kind: None,
            budget,
            response_live: true,
            cancellation,
            candidate: None,
            accept_blocked: false,
            permit_nonce: 0,
            published: None,
            reset_count: 0,
            response_window_high_water: 0,
            finalized: false,
            telemetry: None,
        })
    }

    pub fn with_telemetry(mut self, telemetry: RequestTelemetry) -> Self {
        telemetry.body(
            BodyDirection::AttemptRequest,
            &self.body_plans.attempt_request,
            self.request.body.visible_bytes(),
            self.request.body.high_water_bytes(),
        );
        self.telemetry = Some(telemetry);
        self
    }

    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub fn plan_revision(&self) -> PlanRevision {
        self.plan_revision
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub fn transport_facts(&self) -> AttemptTransportFacts {
        let pending_suppression = self
            .transport
            .as_ref()
            .and_then(AttemptTransport::local_read_suppression_total)
            .unwrap_or(self.transport_suppression_observed)
            .saturating_sub(self.transport_suppression_observed);
        AttemptTransportFacts {
            started_at: self.attempt_started_at,
            connect_elapsed: self.connect_elapsed,
            request_write_elapsed: self.request_write_elapsed,
            upstream_ttfb: self
                .first_upstream_receipt_at
                .map(|received| received.saturating_duration_since(self.attempt_started_at)),
            last_upstream_progress_at: self.last_upstream_progress_at,
            local_read_suppressed: self
                .local_read_suppressed
                .saturating_add(pending_suppression),
            upstream_body_bytes: self.upstream_body_bytes,
            timeout: self.timeout_kind,
        }
    }

    pub(crate) fn mark_attempt_deadline_exceeded(&mut self) {
        self.timeout_kind = Some(AttemptTimeoutKind::AttemptDeadline);
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use crate::core::execution_plan::{
        CaPolicy, PoolEpoch, TransportReuseClassId, TransportScheme,
    };

    use super::*;

    fn system_target() -> TransportTarget {
        TransportTarget {
            scheme: TransportScheme::Https,
            authority: Arc::from("provider.invalid"),
            addresses: Arc::from([SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 443)]),
            sni: Some(Arc::from("provider.invalid")),
            ca: CaPolicy::System,
            alpn: Arc::from([Arc::from("h2"), Arc::from("http/1.1")]),
            connect_timeout: Duration::from_secs(3),
            transport_read_buffer_bytes: 64 * 1024,
            h2_stream_window_bytes: 64 * 1024,
            h2_connection_window_bytes: 256 * 1024,
            h2_max_concurrent_streams: 16,
            reuse_class: TransportReuseClassId(7),
            pool_epoch: PoolEpoch(11),
            connection_fingerprint: [0; 32],
        }
        .with_derived_connection_fingerprint()
    }

    fn test_ca_target(expected: &TransportTarget) -> TransportTarget {
        let mut target = expected.clone();
        target.ca = CaPolicy::Pem(Arc::from(
            b"-----BEGIN CERTIFICATE-----\nZmFrZQ==\n-----END CERTIFICATE-----\n".as_slice(),
        ));
        target.with_derived_connection_fingerprint()
    }

    #[test]
    fn exact_resolved_target_is_authorized() {
        let expected = system_target();
        assert!(resolved_target_is_authorized(
            &expected,
            &expected,
            TransportTargetPolicy::Exact
        ));
    }

    #[cfg(not(feature = "e2e-test-control"))]
    #[test]
    fn default_build_rejects_ca_override() {
        let expected = system_target();
        assert!(!resolved_target_is_authorized(
            &expected,
            &test_ca_target(&expected),
            TransportTargetPolicy::Exact,
        ));
    }

    #[cfg(feature = "e2e-test-control")]
    #[test]
    fn e2e_override_accepts_only_system_to_nonempty_pem() {
        let expected = system_target();
        let authorized = test_ca_target(&expected);
        assert!(resolved_target_is_authorized(
            &expected,
            &authorized,
            TransportTargetPolicy::Exact,
        ));

        let mut empty_ca = expected.clone();
        empty_ca.ca = CaPolicy::Pem(Arc::from([]));
        empty_ca = empty_ca.with_derived_connection_fingerprint();
        assert!(!resolved_target_is_authorized(
            &expected,
            &empty_ca,
            TransportTargetPolicy::Exact,
        ));

        let pem_expected = authorized.clone();
        let mut replacement = pem_expected.clone();
        replacement.ca = CaPolicy::Pem(Arc::from(b"replacement".as_slice()));
        replacement = replacement.with_derived_connection_fingerprint();
        assert!(!resolved_target_is_authorized(
            &pem_expected,
            &replacement,
            TransportTargetPolicy::Exact,
        ));
    }

    #[cfg(feature = "e2e-test-control")]
    #[test]
    fn e2e_override_rejects_every_other_transport_drift() {
        let expected = system_target();
        let authorized = test_ca_target(&expected);
        let mut drifts: Vec<TransportTarget> = Vec::new();

        let mut target = authorized.clone();
        target.scheme = TransportScheme::Http;
        drifts.push(target.with_derived_connection_fingerprint());
        let mut target = authorized.clone();
        target.authority = Arc::from("other.invalid");
        drifts.push(target.with_derived_connection_fingerprint());
        let mut target = authorized.clone();
        target.sni = Some(Arc::from("other.invalid"));
        drifts.push(target.with_derived_connection_fingerprint());
        let mut target = authorized.clone();
        target.alpn = Arc::from([Arc::from("http/1.1")]);
        drifts.push(target.with_derived_connection_fingerprint());
        let mut target = authorized.clone();
        target.connect_timeout = Duration::from_secs(4);
        drifts.push(target.with_derived_connection_fingerprint());
        let mut target = authorized.clone();
        target.transport_read_buffer_bytes += 1;
        drifts.push(target.with_derived_connection_fingerprint());
        let mut target = authorized.clone();
        target.h2_stream_window_bytes += 1;
        drifts.push(target.with_derived_connection_fingerprint());
        let mut target = authorized.clone();
        target.h2_connection_window_bytes += 1;
        drifts.push(target.with_derived_connection_fingerprint());
        let mut target = authorized.clone();
        target.h2_max_concurrent_streams += 1;
        drifts.push(target.with_derived_connection_fingerprint());
        let mut target = authorized.clone();
        target.reuse_class = TransportReuseClassId(8);
        drifts.push(target.with_derived_connection_fingerprint());
        let mut target = authorized.clone();
        target.pool_epoch = PoolEpoch(12);
        drifts.push(target.with_derived_connection_fingerprint());
        let mut target = authorized.clone();
        target.connection_fingerprint[0] ^= 1;
        drifts.push(target);

        for drift in drifts {
            assert!(!resolved_target_is_authorized(
                &expected,
                &drift,
                TransportTargetPolicy::Exact,
            ));
        }
    }

    fn managed_loopback_target(address: SocketAddr) -> TransportTarget {
        TransportTarget {
            scheme: TransportScheme::Http,
            authority: address.to_string().into(),
            addresses: Arc::from([address]),
            sni: None,
            ca: CaPolicy::System,
            alpn: Arc::from([Arc::from("http/1.1")]),
            connect_timeout: Duration::from_secs(3),
            transport_read_buffer_bytes: 64 * 1024,
            h2_stream_window_bytes: 64 * 1024,
            h2_connection_window_bytes: 256 * 1024,
            h2_max_concurrent_streams: 16,
            reuse_class: TransportReuseClassId(7),
            pool_epoch: PoolEpoch(11),
            connection_fingerprint: [0; 32],
        }
        .with_derived_connection_fingerprint()
    }

    #[test]
    fn managed_loopback_policy_allows_only_socket_rotation() {
        let expected = managed_loopback_target("127.0.0.1:4317".parse().unwrap());
        let rotated = managed_loopback_target("127.0.0.1:51803".parse().unwrap());
        assert!(resolved_target_is_authorized(
            &expected,
            &rotated,
            TransportTargetPolicy::ManagedLoopback,
        ));
        assert!(!resolved_target_is_authorized(
            &expected,
            &rotated,
            TransportTargetPolicy::Exact,
        ));

        let mut drift = rotated.clone();
        drift.reuse_class = TransportReuseClassId(8);
        drift = drift.with_derived_connection_fingerprint();
        assert!(!resolved_target_is_authorized(
            &expected,
            &drift,
            TransportTargetPolicy::ManagedLoopback,
        ));

        let non_loopback = managed_loopback_target("192.0.2.10:51803".parse().unwrap());
        assert!(!resolved_target_is_authorized(
            &expected,
            &non_loopback,
            TransportTargetPolicy::ManagedLoopback,
        ));
    }
}
