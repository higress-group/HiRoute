use super::*;

mod completion;
mod preexchange;
mod progress;
mod request;
mod run;
mod selection;

use preexchange::failure::{PreexchangeFailureContext, PreexchangeFailureOutcome};
use preexchange::{PreexchangeAttempt, PreexchangeContext, PreexchangeReady};
use progress::failure::{AttemptProgressFailureContext, AttemptProgressFailureOutcome};
use progress::{AttemptProgress, AttemptProgressContext, AttemptProgressReady};
use selection::{SelectedAttemptRun, SelectedAttemptSetup, SelectionState};
use std::any::Any;

// `Ready` is consumed immediately. Boxing it would add one allocation to every
// admitted request solely to equalize this short-lived enum's variant sizes.
#[allow(clippy::large_enum_variant)]
pub(super) enum RequestPreparation<L, D> {
    Completed(SessionReuse),
    Ready(RequestRun<L, D>),
}

pub(super) struct RequestRun<L, D> {
    cancellation: CancellationToken,
    request_id: RequestId,
    binding: RequestExecutionBinding,
    request_configs: ObservedConfigSnapshot<RequestConfigSnapshot>,
    driver: LogicalRequestDriver,
    budget: StreamBudget,
    final_writer: RequestFinalWriter,
    run_filter_callbacks: bool,
    downstream_method: Method,
    downstream_protocol: HttpProtocol,
    route_binding: ResolvedTargetBindingId,
    route_accepted_body_plan: BodyPlan,
    deadline: Instant,
    logical: Option<L>,
    decision_session: D,
    route_decision_id: RouteDecisionId,
    request_telemetry: Option<RequestTelemetry>,
    max_attempts: u32,
    candidate_bindings: Arc<[ResolvedTargetBindingId]>,
    leases: RequestLeaseBook,
    generation: AttemptGeneration,
    completed_attempts: Vec<CompletedAttemptObservation>,
    routing_snapshots: HashMap<RoutingFactsSnapshotId, Arc<[FreshRoutingFact]>>,
}

/// Request-owned admission produced by an authority layer that already bound
/// one immutable route. This is the sole bridge into the normal core
/// lifecycle: filters, attempt bindings, charged mailboxes, commit fences and
/// accepted-response ownership remain owned by `GatewayCoreLifecycle`.
pub struct BoundRequestAdmission {
    pub binding: RequestExecutionBinding,
    pub route_binding: ResolvedTargetBindingId,
    pub accepted_response_body_plan: BodyPlan,
    pub overall_deadline: Instant,
    /// Planner-produced subset and order for this logical request. This is
    /// never reconstructed from the publication candidate set.
    pub frozen_candidates: Arc<[DecisionCandidateAuthority]>,
    pub max_attempts: u32,
}

#[derive(Clone, Debug)]
pub struct GatewayCoreLifecycleLimits {
    pub max_request_body_bytes: usize,
    pub write_quantum: usize,
    /// Optional process/operator safety cap applied from request admission.
    /// The route-local compiled overall timeout remains the normal request
    /// budget. `None` ensures the default lifecycle never truncates it.
    pub bootstrap_hard_cap: Option<Duration>,
    /// Destructive reset/finalization may outlive the request deadline, but
    /// it is never allowed to detach or wait without a bound.
    pub cleanup_timeout: Duration,
    pub process_memory_bytes: usize,
    pub worker_memory_bytes: usize,
    pub stream_memory_bytes: usize,
    pub filter_sidecall_concurrency: usize,
    pub filter_sidecall_queue: usize,
    pub filter_compute_concurrency: usize,
    pub filter_compute_queue: usize,
    pub filter_blocking_concurrency: usize,
    pub filter_blocking_queue: usize,
}

impl GatewayCoreLifecycleLimits {
    fn request_transport_chunk_bytes(&self) -> usize {
        // Opaque codec backing and its exact charged copy coexist during admission.
        // Derive the workspace from its memory owner, not an extra frame quota.
        self.max_request_body_bytes
            .min((self.stream_memory_bytes / 2).max(1))
    }
}

impl Default for GatewayCoreLifecycleLimits {
    fn default() -> Self {
        Self {
            max_request_body_bytes: 1024 * 1024,
            write_quantum: 16 * 1024,
            bootstrap_hard_cap: None,
            cleanup_timeout: Duration::from_millis(500),
            process_memory_bytes: 256 * 1024 * 1024,
            worker_memory_bytes: 64 * 1024 * 1024,
            stream_memory_bytes: 8 * 1024 * 1024,
            filter_sidecall_concurrency: 64,
            filter_sidecall_queue: 256,
            filter_compute_concurrency: 8,
            filter_compute_queue: 64,
            filter_blocking_concurrency: 4,
            filter_blocking_queue: 32,
        }
    }
}

/// Production lifecycle owner. Pingora owns codecs, TLS, pooling and flow
/// control; this value owns publication binding, route matching, filters,
/// request/attempt scopes, disposition gates and accepted response emission.
pub struct GatewayCoreLifecycle<S, P, F, T> {
    publications: Option<Arc<PublicationInstaller>>,
    selection: S,
    provider: P,
    filters: F,
    transports: T,
    limits: GatewayCoreLifecycleLimits,
    budgets: BudgetTree,
    filter_executors: FilterExecutorPool,
    next_request_id: AtomicU64,
    next_attempt_id: AtomicU64,
    telemetry: Option<Arc<Telemetry>>,
}

impl<S, P, F, T> GatewayCoreLifecycle<S, P, F, T> {
    pub fn new(
        publications: Arc<PublicationInstaller>,
        selection: S,
        provider: P,
        filters: F,
        transports: T,
        limits: GatewayCoreLifecycleLimits,
    ) -> Result<Self, GatewayExecutionError> {
        Self::build(
            Some(publications),
            selection,
            provider,
            filters,
            transports,
            limits,
        )
    }

    /// Builds a lifecycle whose requests are admitted only through
    /// `process_bound`. It cannot accidentally rebind a product-authorized
    /// request through a second publication or route matcher.
    pub fn new_bound(
        selection: S,
        provider: P,
        filters: F,
        transports: T,
        limits: GatewayCoreLifecycleLimits,
    ) -> Result<Self, GatewayExecutionError> {
        Self::build(None, selection, provider, filters, transports, limits)
    }

    fn build(
        publications: Option<Arc<PublicationInstaller>>,
        selection: S,
        provider: P,
        filters: F,
        transports: T,
        limits: GatewayCoreLifecycleLimits,
    ) -> Result<Self, GatewayExecutionError> {
        if limits.max_request_body_bytes == 0
            || limits.write_quantum == 0
            || limits
                .bootstrap_hard_cap
                .is_some_and(|timeout| timeout.is_zero())
            || limits.cleanup_timeout.is_zero()
        {
            return Err(GatewayExecutionError::InvalidLimits);
        }
        let budgets = BudgetTree::new(limits.process_memory_bytes, limits.worker_memory_bytes)?;
        let filter_executors = FilterExecutorPool::new(&limits)?;
        if limits.stream_memory_bytes > limits.worker_memory_bytes {
            return Err(GatewayExecutionError::InvalidLimits);
        }
        Ok(Self {
            publications,
            selection,
            provider,
            filters,
            transports,
            limits,
            budgets,
            filter_executors,
            next_request_id: AtomicU64::new(0),
            next_attempt_id: AtomicU64::new(0),
            telemetry: None,
        })
    }

    pub fn with_telemetry(mut self, telemetry: Arc<Telemetry>) -> Self {
        self.telemetry = Some(telemetry);
        self
    }

    /// Owner-accounting snapshot used by health checks and dedicated
    /// lifecycle harnesses. This reports live core-owned bytes/streams rather
    /// than allocator RSS, which remains an external-runner measurement.
    pub fn budget_snapshot(&self) -> BudgetTreeSnapshot {
        self.budgets.snapshot()
    }
}

#[async_trait]
impl<S, P, F, T> GatewayLifecycle for GatewayCoreLifecycle<S, P, F, T>
where
    P: ProviderRuntimePort,
    S: SelectionPublicationPort<P::RouteRequestContext>,
    F: GatewayFilterManagerPort,
    T: AttemptTransportFactory,
{
    async fn process(
        &self,
        session: &mut dyn GatewaySession,
    ) -> Result<SessionReuse, TransportError> {
        let filters = self
            .filters
            .instantiate_request()
            .map_err(|error| TransportError::Io(error.to_string().into()))?;
        let mut request_filters = GatewayFilterRequestOwner::new(filters);
        let result = self.process_owned(session, &mut request_filters).await;
        let cleanup = request_filters
            .filters
            .finalize_bounded(self.limits.cleanup_timeout)
            .await
            .map_err(GatewayExecutionError::Filter);
        if cleanup.is_ok() {
            request_filters.finalized = true;
        }
        match result {
            Ok(reuse) => cleanup
                .map(|()| reuse)
                .map_err(|error| TransportError::Io(error.to_string().into())),
            Err(error) => {
                let _ = cleanup;
                Err(TransportError::Io(error.to_string().into()))
            }
        }
    }
}

impl<S, P, F, T> GatewayCoreLifecycle<S, P, F, T>
where
    P: ProviderRuntimePort,
    S: SelectionPublicationPort<P::RouteRequestContext>,
    F: GatewayFilterManagerPort,
    T: AttemptTransportFactory,
{
    async fn process_owned(
        &self,
        session: &mut dyn GatewaySession,
        request_filters: &mut GatewayFilterRequestOwner<F::RequestFilters>,
    ) -> Result<SessionReuse, GatewayExecutionError> {
        let request = match self.prepare_request(session, request_filters).await? {
            RequestPreparation::Completed(reuse) => return Ok(reuse),
            RequestPreparation::Ready(request) => request,
        };
        Box::pin(self.run_request(session, request_filters, request)).await
    }

    /// Runs an already-authorized route through the same production execution
    /// owner as publication-bound requests. The admission is consumed exactly
    /// once and cannot be reused for a fallback or a second logical request.
    pub async fn process_bound(
        &self,
        session: &mut dyn GatewaySession,
        admission: BoundRequestAdmission,
    ) -> Result<SessionReuse, TransportError> {
        self.process_bound_inner(session, admission, None, None)
            .await
    }

    /// Allocates the exact stream budget that a bound authority and core will
    /// share for one request's replay owner and transient lifecycle units.
    pub fn allocate_bound_stream_budget(&self) -> Result<StreamBudget, GatewayExecutionError> {
        if self.publications.is_some() {
            return Err(GatewayExecutionError::InvalidBoundAdmission);
        }
        Ok(self.budgets.stream(self.limits.stream_memory_bytes)?)
    }

    /// Bound-only bridge for a request body already captured by one replay
    /// owner. The same budget is consumed by core, and the opaque context is
    /// delivered only to `ProviderRuntimePort::begin_request`.
    pub async fn process_bound_with_context(
        &self,
        session: &mut dyn GatewaySession,
        admission: BoundRequestAdmission,
        budget: StreamBudget,
        provider_context: Arc<dyn Any + Send + Sync>,
    ) -> Result<SessionReuse, TransportError> {
        if !self.budgets.owns_stream(&budget) {
            return Err(TransportError::Io(
                "bound request budget belongs to another lifecycle".into(),
            ));
        }
        self.process_bound_inner(session, admission, Some(budget), Some(provider_context))
            .await
    }

    async fn process_bound_inner(
        &self,
        session: &mut dyn GatewaySession,
        admission: BoundRequestAdmission,
        budget: Option<StreamBudget>,
        provider_context: Option<Arc<dyn Any + Send + Sync>>,
    ) -> Result<SessionReuse, TransportError> {
        if self.publications.is_some() {
            return Err(TransportError::Io(
                "bound admission requires a bound-only lifecycle".into(),
            ));
        }
        let filters = self
            .filters
            .instantiate_request()
            .map_err(|error| TransportError::Io(error.to_string().into()))?;
        let mut request_filters = GatewayFilterRequestOwner::new(filters);
        let result = async {
            let prepared = match (budget, provider_context) {
                (Some(budget), Some(provider_context)) => {
                    self.prepare_bound_request_with_context(
                        session,
                        &mut request_filters,
                        admission,
                        budget,
                        provider_context,
                    )
                    .await?
                }
                (None, None) => {
                    self.prepare_bound_request(session, &mut request_filters, admission)
                        .await?
                }
                _ => return Err(GatewayExecutionError::InvalidBoundAdmission),
            };
            let request = match prepared {
                RequestPreparation::Completed(reuse) => return Ok(reuse),
                RequestPreparation::Ready(request) => request,
            };
            // The execution future retains large provider and filter states.
            // Keep it on the heap so wrapper polls do not duplicate its size
            // on the listener thread's stack during a nested TLS handshake.
            Box::pin(self.run_request(session, &mut request_filters, request)).await
        }
        .await;
        let cleanup = request_filters
            .filters
            .finalize_bounded(self.limits.cleanup_timeout)
            .await
            .map_err(GatewayExecutionError::Filter);
        if cleanup.is_ok() {
            request_filters.finalized = true;
        }
        match result {
            Ok(reuse) => cleanup
                .map(|()| reuse)
                .map_err(|error| TransportError::Io(error.to_string().into())),
            Err(error) => {
                let _ = cleanup;
                Err(TransportError::Io(error.to_string().into()))
            }
        }
    }
}

pub(super) fn observe_runtime_error(telemetry: Option<&RequestTelemetry>, class: ErrorClass) {
    if let Some(telemetry) = telemetry {
        telemetry.error(class);
    }
}

pub(super) struct ConfigLeaseObservation {
    telemetry: RequestTelemetry,
    scope: ConfigAcquireScope,
    acquired_at: Instant,
    generations: Vec<(ConfigCellId, crate::core::execution_plan::ConfigGeneration)>,
}

impl ConfigLeaseObservation {
    fn acquire(
        telemetry: &RequestTelemetry,
        scope: ConfigAcquireScope,
        generations: impl Iterator<Item = (ConfigCellId, crate::core::execution_plan::ConfigGeneration)>,
    ) -> Self {
        let generations = generations.collect::<Vec<_>>();
        for (id, generation) in &generations {
            telemetry.config_lease(id.0, *generation, scope);
        }
        Self {
            telemetry: telemetry.clone(),
            scope,
            acquired_at: Instant::now(),
            generations,
        }
    }

    fn retain_only(&mut self, ids: &[ConfigCellId]) {
        let retained: std::collections::HashSet<_> = ids.iter().copied().collect();
        let latency = self.acquired_at.elapsed();
        self.generations.retain(|(id, generation)| {
            if retained.contains(id) {
                true
            } else {
                self.telemetry
                    .config_lease_released(id.0, *generation, self.scope, latency);
                false
            }
        });
    }
}

impl Drop for ConfigLeaseObservation {
    fn drop(&mut self) {
        let latency = self.acquired_at.elapsed();
        for (id, generation) in &self.generations {
            self.telemetry
                .config_lease_released(id.0, *generation, self.scope, latency);
        }
    }
}

pub(super) struct ObservedConfigSnapshot<T> {
    snapshot: Option<T>,
    observation: Option<ConfigLeaseObservation>,
}

impl<T> ObservedConfigSnapshot<T> {
    fn new(snapshot: T, observation: Option<ConfigLeaseObservation>) -> Self {
        Self {
            snapshot: Some(snapshot),
            observation,
        }
    }
}

impl ObservedConfigSnapshot<RequestConfigSnapshot> {
    fn retain_only(&mut self, ids: &[ConfigCellId]) {
        self.snapshot
            .as_mut()
            .expect("observed request config snapshot exists until drop")
            .retain_only(ids);
        if let Some(observation) = self.observation.as_mut() {
            observation.retain_only(ids);
        }
    }
}

impl<T> std::ops::Deref for ObservedConfigSnapshot<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.snapshot
            .as_ref()
            .expect("observed config snapshot exists until drop")
    }
}

impl<T> Drop for ObservedConfigSnapshot<T> {
    fn drop(&mut self) {
        // Release the real generation guard first; the following event then
        // measures the actual time for which the lease pinned its bundle.
        drop(self.snapshot.take());
        drop(self.observation.take());
    }
}

pub(super) async fn abort_attempt_bounded<T: AttemptTransport>(
    exchange: &mut AttemptExchange<T>,
    driver: &mut LogicalRequestDriver,
    cleanup_timeout: Duration,
) -> Result<(), GatewayExecutionError> {
    exchange.cancellation_token().cancel();
    let cleanup = finish_attempt_bounded(exchange, cleanup_timeout).await;
    driver.complete();
    cleanup
}

pub(super) async fn finish_attempt_bounded<T: AttemptTransport>(
    exchange: &mut AttemptExchange<T>,
    cleanup_timeout: Duration,
) -> Result<(), GatewayExecutionError> {
    exchange
        .finish_or_abort_bounded(cleanup_timeout)
        .await
        .map_err(|error| match error {
            AttemptError::CleanupTimeout => GatewayExecutionError::CleanupTimeout,
            error => GatewayExecutionError::Attempt(error),
        })
}

/// Releases an accepted upstream after the downstream protocol terminal has
/// already been written. This is cleanup, not part of the delivered response:
/// a late downstream disconnect must not cancel it, and the cleanup timeout is
/// deliberately independent from the request deadline.
pub(super) async fn finish_accepted_response_bounded<T: AttemptTransport>(
    exchange: &mut AttemptExchange<T>,
    reusable: bool,
    cleanup_timeout: Duration,
) -> Result<(), GatewayExecutionError> {
    if cleanup_timeout.is_zero() {
        return Err(GatewayExecutionError::CleanupTimeout);
    }
    match tokio::time::timeout(cleanup_timeout, exchange.finish_accepted_response(reusable)).await {
        Ok(result) => result.map_err(GatewayExecutionError::Attempt),
        Err(_) => Err(GatewayExecutionError::CleanupTimeout),
    }
}

pub(super) async fn finish_attempt_filter_scope_bounded<R: GatewayRequestFilterPort>(
    filters: &mut R,
    cleanup_timeout: Duration,
) -> Result<(), GatewayExecutionError> {
    filters
        .finish_attempt_bounded(cleanup_timeout)
        .await
        .map_err(GatewayExecutionError::Filter)
}

pub(super) async fn finish_accepted_filter_scope_bounded<R: GatewayRequestFilterPort>(
    filters: &mut R,
    cleanup_timeout: Duration,
) -> Result<(), GatewayExecutionError> {
    filters
        .finish_accepted_response_bounded(cleanup_timeout)
        .await
        .map_err(GatewayExecutionError::Filter)
}

pub(super) fn combine_cleanup_results(
    first: Result<(), GatewayExecutionError>,
    second: Result<(), GatewayExecutionError>,
) -> Result<(), GatewayExecutionError> {
    match first {
        Ok(()) => second,
        Err(error) => Err(error),
    }
}

pub(super) fn attempt_commit_facts<T: AttemptTransport>(
    exchange: &AttemptExchange<T>,
    final_writer: &RequestFinalWriter,
) -> AttemptCommitFacts {
    let snapshot = exchange.snapshot();
    AttemptCommitFacts {
        upstream_request: snapshot.upstream_request_fence,
        downstream_headers: final_writer.header_fence(),
        downstream_semantic: final_writer.semantic_fence(),
    }
}

pub(super) fn attempt_completion_from_result<T>(
    disposition: Disposition,
    commits: AttemptCommitFacts,
    transport: AttemptTransportFacts,
    cleanup: AttemptCleanupOutcome,
    result: &Result<T, GatewayExecutionError>,
) -> ProviderAttemptCompletion {
    let cancelled = matches!(
        result,
        Err(GatewayExecutionError::Cancelled)
            | Err(GatewayExecutionError::Attempt(AttemptError::Cancelled))
    );
    let semantic_started = commits.downstream_semantic != CommitFence::Clear;
    let stream = if semantic_started && result.is_err() {
        AttemptStreamOutcome::StreamStartedNoRetry
    } else if result.is_ok() && disposition != Disposition::Continue {
        AttemptStreamOutcome::CompletedEos
    } else if commits.upstream_request == CommitFence::Clear {
        AttemptStreamOutcome::NotStarted
    } else {
        AttemptStreamOutcome::AbortedBeforeSemanticCommit
    };
    let downstream = match result {
        Ok(_) if disposition == Disposition::Continue => AttemptDownstreamOutcome::NotStarted,
        Ok(_) => AttemptDownstreamOutcome::Completed,
        Err(_) if cancelled => AttemptDownstreamOutcome::Cancelled,
        Err(_) if commits.downstream_headers != CommitFence::Clear => {
            AttemptDownstreamOutcome::Failed
        }
        Err(_) => AttemptDownstreamOutcome::NotStarted,
    };
    let termination_reason = if cleanup == AttemptCleanupOutcome::Failed {
        AttemptTerminationReason::CleanupFailure
    } else if semantic_started && result.is_err() {
        AttemptTerminationReason::StreamStartedNoRetry
    } else {
        match result {
            Ok(_) => match disposition {
                Disposition::Accept => AttemptTerminationReason::AcceptedEos,
                Disposition::Continue => AttemptTerminationReason::FallbackComplete,
                Disposition::Terminate => AttemptTerminationReason::TerminatedResponseComplete,
            },
            Err(_) if cancelled => AttemptTerminationReason::Cancelled,
            Err(GatewayExecutionError::Transport(_)) => AttemptTerminationReason::DownstreamFailure,
            Err(GatewayExecutionError::ProviderTerminalRelease(_)) => {
                AttemptTerminationReason::ProviderReleaseFailure
            }
            Err(GatewayExecutionError::Provider(_))
            | Err(GatewayExecutionError::TerminalEncoderDidNotComplete) => {
                AttemptTerminationReason::ProviderEncoderFailure
            }
            Err(GatewayExecutionError::Filter(_)) => AttemptTerminationReason::FilterFailure,
            Err(_) => AttemptTerminationReason::AttemptFailure,
        }
    };
    ProviderAttemptCompletion {
        disposition,
        commits,
        stream,
        downstream,
        cleanup,
        transport,
        ended_at: Instant::now(),
        termination_reason,
    }
}
use completion::{AttemptCompletionOutcome, PublishedAttempt};
