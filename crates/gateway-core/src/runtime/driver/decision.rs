use super::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RouteDecisionId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptBudgetGrant {
    /// Monotonic instant at which the decision session issued this grant.
    /// Carrying the anchor makes validation independent of queueing or
    /// scheduling delay between selection and core consumption.
    pub issued_at: Instant,
    /// Duration granted by the decision session at selection time.
    pub allocated: Duration,
    /// Absolute monotonic deadline. Core rejects a grant outside the frozen
    /// overall request budget before any credential or transport work.
    pub deadline: Instant,
}

/// An explicit request-owned decision result. One value creates exactly one
/// `AttemptExchange`; gateway core never derives a fallback binding from an
/// active exchange.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedGatewayAttempt {
    pub request_id: RequestId,
    pub attempt_id: AttemptId,
    pub generation: AttemptGeneration,
    pub binding: ResolvedTargetBindingId,
    pub credential_ref: CredentialRef,
    pub route_decision_id: RouteDecisionId,
    pub budget: AttemptBudgetGrant,
}

/// A bounded, display-safe identifier used by routing and provider facts.
/// Adapters must keep secrets in their private state and expose only stable,
/// low-cardinality identifiers through this type.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ObservationLabel(Arc<str>);

impl ObservationLabel {
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, Arc<str>> {
        let value = value.into();
        let lower = value.to_ascii_lowercase();
        let resembles_secret = lower == "authorization"
            || lower.starts_with("authorization:")
            || lower == "proxy-authorization"
            || lower.starts_with("proxy-authorization:")
            || lower.starts_with("bearer:")
            || lower.starts_with("basic:")
            || lower.starts_with("sk-");
        if value.is_empty()
            || value.len() > 128
            || resembles_secret
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
            })
        {
            return Err(Arc::from(
                "observation label must be 1..=128 safe ASCII characters",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactSubject {
    /// The binding identity is meaningful only inside this immutable plan
    /// revision; it must never be compared as a process-global identifier.
    pub plan_revision: PlanRevision,
    pub binding: ResolvedTargetBindingId,
    pub stable_target: ObservationLabel,
    pub provider: Option<ObservationLabel>,
    pub model: Option<ObservationLabel>,
    pub entitlement: Option<ObservationLabel>,
    /// Stable, non-secret selector from the binding's compiled credential
    /// closure. Secret material never enters a routing fact.
    pub credential: Option<CredentialRef>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FactScope {
    Candidate,
    Provider,
    Model,
    Entitlement,
    Credential,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FactConfidence {
    Reported,
    Measured,
    Estimated,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RoutingFactsSnapshotId(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FreshRoutingFact {
    pub source: ObservationLabel,
    pub subject: FactSubject,
    pub scope: FactScope,
    pub confidence: FactConfidence,
    pub observed_at: Instant,
    pub valid_until: Instant,
    pub state: RoutingFactState,
}

impl FreshRoutingFact {
    pub fn is_fresh_at(&self, now: Instant) -> bool {
        matches!(self.state, RoutingFactState::Known(_))
            && self.observed_at <= now
            && now < self.valid_until
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RoutingFactState {
    Known(RealtimeRoutingFact),
    Unknown,
    /// The last value is retained for observability, but a policy must opt in
    /// explicitly before using it. Core never promotes it back to Known.
    Stale {
        last_known: Option<RealtimeRoutingFact>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RealtimeRoutingFact {
    QuotaRemaining {
        units: u64,
    },
    PriceMicrosPerMillion {
        micros: u64,
    },
    HealthScore {
        basis_points: u16,
    },
    ComplianceEligible {
        eligible: bool,
    },
    CooldownUntil {
        until: Instant,
    },
    QuotaResetAt {
        at: Instant,
    },
    Capability {
        capability: ObservationLabel,
        available: bool,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RealtimeRoutingFacts {
    /// Identifies immutable snapshot content. A still-fresh snapshot may be
    /// reused across fallback selections; the same ID must never name changed
    /// content. The selected value is retained in completion observation.
    pub snapshot_id: Option<RoutingFactsSnapshotId>,
    pub facts: Arc<[FreshRoutingFact]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryabilityFact {
    Unknown,
    Retryable,
    NonRetryable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsageProvenance {
    Reported,
    Estimated,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsageDimension {
    pub units: Option<u64>,
    pub provenance: UsageProvenance,
}

impl UsageDimension {
    pub const fn reported(units: u64) -> Self {
        Self {
            units: Some(units),
            provenance: UsageProvenance::Reported,
        }
    }

    pub const fn estimated(units: u64) -> Self {
        Self {
            units: Some(units),
            provenance: UsageProvenance::Estimated,
        }
    }

    pub const fn unknown() -> Self {
        Self {
            units: None,
            provenance: UsageProvenance::Unknown,
        }
    }
}

impl Default for UsageDimension {
    fn default() -> Self {
        Self::unknown()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UsageFact {
    pub input: UsageDimension,
    pub output: UsageDimension,
    pub billable: UsageDimension,
    pub cache_read: UsageDimension,
    pub cache_write: UsageDimension,
    pub reasoning: UsageDimension,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderClassificationFacts {
    pub error_class: Option<ObservationLabel>,
    pub retryability: RetryabilityFact,
    pub retry_after: Option<Duration>,
    pub http_status: Option<StatusCode>,
    pub provider_code: Option<ObservationLabel>,
    pub provider_request_id: Option<ObservationLabel>,
    pub reset_at: Option<Instant>,
    pub usage: Option<UsageFact>,
    /// Bounded classification only. The real readiness value remains in the
    /// provider-owned opaque `Readiness` state held by core.
    pub readiness: ObservationLabel,
    /// Bounded event class only. Raw model events remain provider-private.
    pub model_event: Option<ObservationLabel>,
    /// Time to the first provider/model event. Transport TTFB remains in
    /// `AttemptTransportFacts`; a heartbeat cannot populate this field.
    pub ttft: Option<Duration>,
    pub ended_at: Option<Instant>,
}

impl Default for ProviderClassificationFacts {
    fn default() -> Self {
        Self {
            error_class: None,
            retryability: RetryabilityFact::Unknown,
            retry_after: None,
            http_status: None,
            provider_code: None,
            provider_request_id: None,
            reset_at: None,
            usage: None,
            readiness: ObservationLabel(Arc::from("unspecified")),
            model_event: None,
            ttft: None,
            ended_at: None,
        }
    }
}

#[derive(Debug)]
pub struct ClassifiedAttemptResult<R> {
    pub facts: ProviderClassificationFacts,
    pub readiness: R,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptFailureClass {
    Credential,
    Dns,
    Materialization,
    AttemptRequestFilter,
    Connect,
    RequestWrite,
    FirstByte,
    StreamIdle,
    AttemptDeadline,
    Transport,
}

/// Provider-owned classification space for failures that occur while turning
/// one selected grant into a wire request. Transport and filter timeout classes
/// are intentionally absent because only gateway core may assert those facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptMaterializationFailureClass {
    Credential,
    Dns,
    Materialization,
}

impl From<AttemptMaterializationFailureClass> for AttemptFailureClass {
    fn from(value: AttemptMaterializationFailureClass) -> Self {
        match value {
            AttemptMaterializationFailureClass::Credential => Self::Credential,
            AttemptMaterializationFailureClass::Dns => Self::Dns,
            AttemptMaterializationFailureClass::Materialization => Self::Materialization,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AttemptFailureFacts {
    pub class: AttemptFailureClass,
    pub provider: Option<ProviderClassificationFacts>,
    pub transport: AttemptTransportFacts,
    pub request_committed: bool,
    pub termination_reason: ObservationLabel,
}

#[derive(Clone, Debug)]
pub struct AttemptMaterializationFailure {
    pub class: AttemptMaterializationFailureClass,
    pub provider: ProviderClassificationFacts,
    /// Stable, low-cardinality and non-secret reason suitable for policy and
    /// observation. The raw provider error remains inside the adapter.
    pub termination_reason: ObservationLabel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptCommitFacts {
    pub upstream_request: CommitFence,
    pub downstream_headers: CommitFence,
    pub downstream_semantic: CommitFence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptStreamOutcome {
    NotStarted,
    CompletedEos,
    AbortedBeforeSemanticCommit,
    StreamStartedNoRetry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptDownstreamOutcome {
    NotStarted,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptCleanupOutcome {
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptTerminationReason {
    FallbackComplete,
    AcceptedEos,
    TerminatedResponseComplete,
    PreexchangeFailure,
    AttemptFailure,
    StreamStartedNoRetry,
    Cancelled,
    DownstreamFailure,
    ProviderReleaseFailure,
    ProviderEncoderFailure,
    FilterFailure,
    CleanupFailure,
}

/// Product-neutral, immutable technical result supplied to the provider at
/// true attempt completion. The provider may use its private state/readiness
/// to add final Usage and model semantics; core never invents them.
#[derive(Clone, Debug)]
pub struct ProviderAttemptCompletion {
    pub disposition: Disposition,
    pub commits: AttemptCommitFacts,
    pub stream: AttemptStreamOutcome,
    pub downstream: AttemptDownstreamOutcome,
    pub cleanup: AttemptCleanupOutcome,
    pub transport: AttemptTransportFacts,
    pub ended_at: Instant,
    pub termination_reason: AttemptTerminationReason,
}

#[derive(Clone, Debug)]
pub struct CompletedAttemptObservation {
    pub route_decision_id: RouteDecisionId,
    pub attempt_id: AttemptId,
    pub generation: AttemptGeneration,
    pub binding: ResolvedTargetBindingId,
    pub credential_ref: CredentialRef,
    pub budget: AttemptBudgetGrant,
    /// Exact fresh routing snapshot used when this attempt was selected.
    pub routing_facts: RealtimeRoutingFacts,
    pub provider: Option<ProviderClassificationFacts>,
    pub failure: Option<AttemptFailureFacts>,
    pub transport: AttemptTransportFacts,
    pub disposition: Disposition,
    pub commits: AttemptCommitFacts,
    pub stream: AttemptStreamOutcome,
    pub downstream: AttemptDownstreamOutcome,
    pub cleanup: AttemptCleanupOutcome,
    pub ended_at: Instant,
    pub termination_reason: AttemptTerminationReason,
}

pub struct DecisionSessionRequest<C> {
    pub request_id: RequestId,
    pub route_binding: ResolvedTargetBindingId,
    pub candidate_bindings: Arc<[ResolvedTargetBindingId]>,
    pub route_context: C,
    pub overall_deadline: Instant,
    pub max_attempts: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionCandidateAuthority {
    pub binding: ResolvedTargetBindingId,
    pub stable_target: ObservationLabel,
    pub credential_refs: Arc<[CredentialRef]>,
    /// Stable planner candidate identity. Core never interprets it, but keeps
    /// it bound to the exact provider profile selected for this request.
    pub candidate_id: Option<ObservationLabel>,
    /// Digest of the immutable provider protocol/connector profile.
    pub profile_digest: Option<ObservationLabel>,
    /// Digest binding the Planner output, including its reason ledger.
    pub reason_ledger_identity: Option<ObservationLabel>,
    /// Product-owned, non-secret profile envelope. It is opaque to core and
    /// remains request-owned; providers must validate it against the digest.
    pub provider_profile: Option<Arc<[u8]>>,
}

#[derive(Clone, Copy, Debug)]
pub struct SelectionRequest<'a> {
    pub request_id: RequestId,
    pub generation: AttemptGeneration,
    pub route_binding: ResolvedTargetBindingId,
    pub remaining_total: Duration,
    pub remaining_attempts: u32,
    pub realtime_facts: &'a RealtimeRoutingFacts,
    pub previous_attempts: &'a [CompletedAttemptObservation],
}

/// Linear per-request decision owner. Its mutable methods cannot be invoked in
/// parallel, and no request-id side map is needed to recover policy state.
pub trait DecisionSessionPort: Send {
    fn route_decision_id(&self) -> RouteDecisionId;

    /// Refreshes request-local routing observations immediately before one
    /// selection. The session owns the product-specific realtime seam and its
    /// finalized route context; core never recovers either from a global map.
    fn snapshot_realtime_facts(&mut self, now: Instant) -> Result<RealtimeRoutingFacts, Arc<str>>;

    fn select_next(
        &mut self,
        request: SelectionRequest<'_>,
    ) -> Result<Option<SelectedGatewayAttempt>, Arc<str>>;

    /// Optional product-owned response when selection ended before any
    /// upstream attempt. Core keeps its generic 502 when no exact response is
    /// available.
    fn exhaustion_local_reply(&mut self, _now: Instant) -> Result<Option<LocalReply>, Arc<str>> {
        Ok(None)
    }

    fn decide(
        &mut self,
        selected: &SelectedGatewayAttempt,
        facts: &ProviderClassificationFacts,
        transport: &AttemptTransportFacts,
    ) -> Result<Disposition, Arc<str>>;

    /// Owns the policy response to a mechanical attempt failure. Accept is
    /// invalid because no live provider readiness exists; core still owns the
    /// non-Accept gate, reset, and publication.
    fn decide_failure(
        &mut self,
        selected: &SelectedGatewayAttempt,
        failure: &AttemptFailureFacts,
    ) -> Result<Disposition, Arc<str>>;

    fn replace_blocked_accept(
        &mut self,
        selected: &SelectedGatewayAttempt,
        reason: AcceptBlockedReason,
    ) -> Result<Disposition, Arc<str>>;

    /// Best-effort ledger seam. Once a disposition is published, failures in
    /// this sink must never change the data-plane result.
    fn observe_published(&mut self, disposition: &PublishedDisposition) -> Result<(), Arc<str>>;

    /// Delivers the complete low-cardinality outcome after technical
    /// publication. This is the product-neutral handoff for request-local
    /// fallback state and an external trust ledger; core does not persist it.
    fn observe_completed(
        &mut self,
        observation: &CompletedAttemptObservation,
    ) -> Result<(), Arc<str>>;
}

/// Factory boundary. It creates one linear session from the finalized
/// provider-neutral route context; subsequent choices and realtime refreshes
/// stay inside that request-owned value.
pub trait SelectionPublicationPort<C>: Send + Sync + 'static {
    type Session: DecisionSessionPort;

    fn begin_session(&self, request: DecisionSessionRequest<C>) -> Result<Self::Session, Arc<str>>;

    /// Authorized extension used by production selectors that must grant an
    /// exact credential reference without learning secret material. Existing
    /// selectors keep their original contract through the default bridge.
    fn begin_authorized_session(
        &self,
        request: DecisionSessionRequest<C>,
        _candidates: Arc<[DecisionCandidateAuthority]>,
    ) -> Result<Self::Session, Arc<str>> {
        self.begin_session(request)
    }
}

/// Provider-owned, bounded representation of one classified SSE event. The
/// sequence is assigned by core and cannot be changed by the provider.
#[allow(clippy::too_many_arguments)]
pub(super) fn record_completed_attempt<D>(
    decision_session: &mut D,
    completed_attempts: &mut Vec<CompletedAttemptObservation>,
    selected: &SelectedGatewayAttempt,
    routing_facts: &RealtimeRoutingFacts,
    provider: Option<ProviderClassificationFacts>,
    failure: Option<AttemptFailureFacts>,
    completion: ProviderAttemptCompletion,
) where
    D: DecisionSessionPort,
{
    let observation = CompletedAttemptObservation {
        route_decision_id: selected.route_decision_id,
        attempt_id: selected.attempt_id,
        generation: selected.generation,
        binding: selected.binding,
        credential_ref: selected.credential_ref.clone(),
        budget: selected.budget,
        routing_facts: routing_facts.clone(),
        provider,
        failure,
        transport: completion.transport.clone(),
        disposition: completion.disposition,
        commits: completion.commits,
        stream: completion.stream,
        downstream: completion.downstream,
        cleanup: completion.cleanup,
        ended_at: completion.ended_at,
        termination_reason: completion.termination_reason,
    };
    // Both the request-local policy observation and any external ledger behind
    // it are explicitly best effort after core freezes true completion. A
    // fail-closed post-exchange path may complete without publishing.
    let _ = decision_session.observe_completed(&observation);
    completed_attempts.push(observation);
}

pub(super) fn validate_realtime_routing_facts(
    binding: &RequestExecutionBinding,
    candidates: &[ResolvedTargetBindingId],
    snapshot: &RealtimeRoutingFacts,
    now: Instant,
) -> Result<(), GatewayExecutionError> {
    if !snapshot.facts.is_empty() && snapshot.snapshot_id.is_none() {
        return Err(GatewayExecutionError::InvalidRoutingFacts);
    }
    for fact in snapshot.facts.iter() {
        if fact.subject.plan_revision != binding.plan_revision()
            || fact.subject.binding.plan_revision() != binding.plan_revision()
            || !candidates.contains(&fact.subject.binding)
            || fact.observed_at > now
            || fact.valid_until < fact.observed_at
        {
            return Err(GatewayExecutionError::InvalidRoutingFacts);
        }
        let attempt = binding
            .resolve_attempt(fact.subject.binding)
            .map_err(|_| GatewayExecutionError::InvalidRoutingFacts)?;
        if attempt.plan().stable_target_key.as_str() != fact.subject.stable_target.as_str() {
            return Err(GatewayExecutionError::InvalidRoutingFacts);
        }
        if fact
            .subject
            .credential
            .as_ref()
            .is_some_and(|credential| !attempt.plan().credential_refs.contains(credential))
        {
            return Err(GatewayExecutionError::InvalidRoutingFacts);
        }
        match fact.scope {
            FactScope::Candidate => {}
            FactScope::Provider if fact.subject.provider.is_some() => {}
            FactScope::Model if fact.subject.model.is_some() => {}
            FactScope::Entitlement if fact.subject.entitlement.is_some() => {}
            FactScope::Credential if fact.subject.credential.is_some() => {}
            _ => return Err(GatewayExecutionError::InvalidRoutingFacts),
        }
        match fact.state {
            RoutingFactState::Known(_) if now < fact.valid_until => {}
            RoutingFactState::Unknown => {}
            RoutingFactState::Stale { .. } if now >= fact.valid_until => {}
            _ => return Err(GatewayExecutionError::InvalidRoutingFacts),
        }
    }
    Ok(())
}

pub(super) fn validate_selection(
    request_id: RequestId,
    generation: AttemptGeneration,
    selected: &SelectedGatewayAttempt,
    plan_revision: crate::core::execution_plan::PlanRevision,
    route_decision_id: RouteDecisionId,
    overall_deadline: Instant,
) -> Result<(), GatewayExecutionError> {
    let now = Instant::now();
    let grant_deadline = selected
        .budget
        .issued_at
        .checked_add(selected.budget.allocated);
    if selected.request_id != request_id
        || selected.generation != generation
        || selected.binding.plan_revision() != plan_revision
        || selected.route_decision_id != route_decision_id
        || selected.budget.allocated.is_zero()
        || selected.budget.issued_at > now
        || selected.budget.deadline > overall_deadline
        || grant_deadline.is_none_or(|grant_deadline| {
            selected.budget.deadline > grant_deadline || grant_deadline > overall_deadline
        })
        || selected.budget.deadline <= now
    {
        return Err(GatewayExecutionError::InvalidSelection);
    }
    Ok(())
}

pub(super) fn attempt_failure_facts<T: AttemptTransport>(
    error: &GatewayExecutionError,
    exchange: &AttemptExchange<T>,
) -> Option<AttemptFailureFacts> {
    let (class, termination_reason) = match error {
        GatewayExecutionError::Attempt(AttemptError::ConnectFailed)
        | GatewayExecutionError::Attempt(AttemptError::ConnectTimeout) => {
            (AttemptFailureClass::Connect, "connect")
        }
        GatewayExecutionError::Attempt(AttemptError::RequestWriteTimeout) => {
            (AttemptFailureClass::RequestWrite, "request_write_timeout")
        }
        GatewayExecutionError::Attempt(AttemptError::FirstByteTimeout) => {
            (AttemptFailureClass::FirstByte, "first_byte_timeout")
        }
        GatewayExecutionError::Attempt(AttemptError::StreamIdleTimeout) => {
            (AttemptFailureClass::StreamIdle, "stream_idle_timeout")
        }
        GatewayExecutionError::Attempt(AttemptError::DeadlineExceeded) => {
            (AttemptFailureClass::AttemptDeadline, "attempt_deadline")
        }
        GatewayExecutionError::Attempt(AttemptError::Transport(_)) => {
            (AttemptFailureClass::Transport, "transport")
        }
        _ => return None,
    };
    let snapshot = exchange.snapshot();
    Some(AttemptFailureFacts {
        class,
        provider: None,
        transport: exchange.transport_facts(),
        request_committed: snapshot.upstream_request_fence != CommitFence::Clear,
        termination_reason: ObservationLabel::new(termination_reason)
            .expect("core-owned attempt failure reasons are safe static labels"),
    })
}

pub(super) fn materialization_failure_facts<P: ProviderRuntimePort>(
    error: &GatewayExecutionError,
    provider: &P,
    selected: &SelectedGatewayAttempt,
) -> Option<AttemptFailureFacts> {
    let (class, provider, timeout, termination_reason) = match error {
        GatewayExecutionError::Provider(error) => {
            let classified = provider.classify_materialization_failure(error);
            (
                classified.class.into(),
                Some(classified.provider),
                None,
                classified.termination_reason,
            )
        }
        GatewayExecutionError::PreexchangeAttemptDeadline => (
            AttemptFailureClass::AttemptDeadline,
            None,
            Some(AttemptTimeoutKind::AttemptDeadline),
            ObservationLabel::new("attempt_deadline")
                .expect("core-owned attempt failure reasons are safe static labels"),
        ),
        _ => return None,
    };
    Some(new_preexchange_failure_facts(
        selected,
        class,
        provider,
        timeout,
        termination_reason,
    ))
}

pub(super) fn attempt_request_preparation_failure_facts(
    error: &GatewayExecutionError,
    selected: &SelectedGatewayAttempt,
) -> Option<AttemptFailureFacts> {
    let (class, timeout, termination_reason) = match error {
        GatewayExecutionError::Filter(_)
        | GatewayExecutionError::OperationPanic
        | GatewayExecutionError::Body(_)
        | GatewayExecutionError::Attempt(_)
        | GatewayExecutionError::Plan(_)
        | GatewayExecutionError::Sse(_) => (
            AttemptFailureClass::AttemptRequestFilter,
            None,
            ObservationLabel::new("attempt_request_filter")
                .expect("core-owned attempt failure reasons are safe static labels"),
        ),
        GatewayExecutionError::PreexchangeAttemptDeadline => (
            AttemptFailureClass::AttemptDeadline,
            Some(AttemptTimeoutKind::AttemptDeadline),
            ObservationLabel::new("attempt_deadline")
                .expect("core-owned attempt failure reasons are safe static labels"),
        ),
        _ => return None,
    };
    Some(new_preexchange_failure_facts(
        selected,
        class,
        None,
        timeout,
        termination_reason,
    ))
}

pub(super) fn new_preexchange_failure_facts(
    selected: &SelectedGatewayAttempt,
    class: AttemptFailureClass,
    provider: Option<ProviderClassificationFacts>,
    timeout: Option<AttemptTimeoutKind>,
    termination_reason: ObservationLabel,
) -> AttemptFailureFacts {
    AttemptFailureFacts {
        class,
        provider,
        transport: preexchange_transport_facts(selected, timeout),
        request_committed: false,
        termination_reason,
    }
}

pub(super) fn preexchange_transport_facts(
    selected: &SelectedGatewayAttempt,
    timeout: Option<AttemptTimeoutKind>,
) -> AttemptTransportFacts {
    AttemptTransportFacts {
        started_at: selected.budget.issued_at,
        upstream_protocol: None,
        connect_elapsed: None,
        request_write_elapsed: None,
        upstream_ttfb: None,
        last_upstream_progress_at: None,
        local_read_suppressed: Duration::ZERO,
        upstream_body_bytes: 0,
        timeout,
    }
}

#[cfg(test)]
mod tests;
