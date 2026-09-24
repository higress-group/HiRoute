use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::runtime::body::BodyPlan;
use crate::runtime::telemetry::{
    Correlation, LifecycleEvent, LifecycleKind, PublicationFact, PublicationResult,
    PublicationStage, Telemetry,
};

use super::execution_plan::{
    AtomicityGroupId, AttemptPlanIndexHandle, AuthorityId, CompiledAcceptedResponsePlan,
    CompiledIngressPlanHandle, CompiledLogicalRequestPlan, ConfigCellHandle, ConfigCellId,
    ConfigCellsHandle, ConfigRevision, ConnectionEpochFingerprint, PlanError, PlanRevision,
    PoolEpoch, RequestExecutionBinding, TransportReuseClassId,
};

pub const SUPPORTED_SCHEMA_VERSION: u32 = 2;
pub const SUPPORTED_COMPILER_VERSION: u32 = 2;

pub mod adapter;

#[derive(Clone, Debug)]
pub struct CompiledGatewayPublicationEnvelope {
    pub authority_id: AuthorityId,
    pub authority_epoch: u64,
    pub config_revision: ConfigRevision,
    pub plan_revision: PlanRevision,
    pub schema_version: u32,
    pub compiler_version: u32,
    pub payload_digest: [u8; 32],
    pub ingress_plan_handle: CompiledIngressPlanHandle,
    pub attempt_plan_index_handle: AttemptPlanIndexHandle,
    pub config_cells_handle: ConfigCellsHandle,
    pub connection_epoch_fingerprints: Arc<[ConnectionEpochFingerprint]>,
    /// A full authority snapshot may legitimately close a discovered revision
    /// gap. Incremental notifications must leave this false.
    pub full_snapshot: bool,
    /// Rollback is never inferred from an old value; the compiler/authority
    /// must opt in explicitly.
    pub rollback_authorized: bool,
}

impl CompiledGatewayPublicationEnvelope {
    fn validate_shape(
        &self,
        cancel: &CancellationToken,
        deadline: Instant,
    ) -> Result<(), InstallError> {
        let mut progress = ValidationProgress::new(cancel, deadline);
        progress.check_now()?;
        if self.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(InstallError::IncompatibleSchema(self.schema_version));
        }
        if self.compiler_version != SUPPORTED_COMPILER_VERSION {
            return Err(InstallError::IncompatibleCompiler(self.compiler_version));
        }
        if self.plan_revision != self.ingress_plan_handle.plan_revision
            || self.plan_revision != self.attempt_plan_index_handle.plan_revision
        {
            return Err(InstallError::MixedPlanRevision);
        }
        if self.payload_digest == [0; 32] {
            return Err(InstallError::InvalidPayloadDigest);
        }

        let mut config_groups: HashMap<AtomicityGroupId, &ConfigCellHandle> = HashMap::new();
        for (id, handle) in self.config_cells_handle.iter() {
            progress.checkpoint()?;
            if *id != handle.descriptor().id {
                return Err(InstallError::ConfigCatalogKeyMismatch {
                    key: *id,
                    descriptor: handle.descriptor().id,
                });
            }
            if let Some(first) = config_groups.get(&handle.descriptor().atomicity_group) {
                if !handle.shares_atomic_bundle_with(first) {
                    return Err(InstallError::SplitAtomicityGroup(
                        handle.descriptor().atomicity_group,
                    ));
                }
            } else {
                config_groups.insert(handle.descriptor().atomicity_group, handle);
            }
        }

        let mut route_keys = HashSet::new();
        for route in self.ingress_plan_handle.routes.iter() {
            progress.checkpoint()?;
            if !route_metadata_is_valid(&route.normalized_host, &route.path_prefix) {
                return Err(InstallError::InvalidRouteMetadata);
            }
            if !route_keys.insert((route.normalized_host.as_ref(), route.path_prefix.as_ref())) {
                return Err(InstallError::DuplicateRoute);
            }
            if route.binding.plan_revision() != self.plan_revision {
                return Err(InstallError::MixedPlanRevision);
            }
            let request = &route.request_plan;
            if request.candidate_bindings.is_empty()
                || !request.candidate_bindings.contains(&route.binding)
                || request.overall_request_timeout.is_zero()
                || request.max_attempts == 0
            {
                return Err(InstallError::InvalidCompiledPlan(
                    PlanError::InvalidRouteCandidateClosure,
                ));
            }
            let mut route_candidates = HashSet::new();
            for candidate in request.candidate_bindings.iter().copied() {
                progress.checkpoint()?;
                if candidate.plan_revision() != self.plan_revision
                    || !route_candidates.insert(candidate)
                {
                    return Err(InstallError::InvalidCompiledPlan(
                        PlanError::InvalidRouteCandidateClosure,
                    ));
                }
                self.attempt_plan_index_handle
                    .resolve(candidate)
                    .map_err(InstallError::InvalidCompiledPlan)?;
            }
            validate_logical_request_plan(
                &request.logical_request,
                &self.config_cells_handle,
                &mut progress,
            )?;
            validate_accepted_response_plan(
                &request.accepted_response,
                &self.config_cells_handle,
                &mut progress,
            )?;
        }

        if self
            .ingress_plan_handle
            .local_response_plan
            .overall_request_timeout
            .is_zero()
        {
            return Err(InstallError::InvalidCompiledPlan(
                PlanError::InvalidRequestTimeout,
            ));
        }
        validate_accepted_response_plan(
            &self
                .ingress_plan_handle
                .local_response_plan
                .accepted_response,
            &self.config_cells_handle,
            &mut progress,
        )?;

        let mut reuse_fingerprints: HashMap<TransportReuseClassId, ConnectionEpochFingerprint> =
            HashMap::new();
        for (binding, plan) in self.attempt_plan_index_handle.plans() {
            progress.checkpoint()?;
            if binding.plan_revision() != self.plan_revision || plan.binding != binding {
                return Err(InstallError::MixedPlanRevision);
            }
            plan.transport_target
                .validate()
                .map_err(InstallError::InvalidCompiledPlan)?;
            plan.timeouts
                .validate()
                .map_err(InstallError::InvalidCompiledPlan)?;
            plan.body_plans
                .validate()
                .map_err(InstallError::InvalidCompiledPlan)?;
            if plan.credential_refs.is_empty() {
                return Err(InstallError::InvalidCompiledPlan(
                    PlanError::EmptyCredentialClosure,
                ));
            }
            if plan.attempt_request_chunk_capacity == 0 || plan.precommit_event_capacity == 0 {
                return Err(InstallError::InvalidCompiledPlan(
                    PlanError::ZeroBodyQueueCapacity,
                ));
            }
            validate_config_references(
                &plan.config_cell_ids,
                &self.config_cells_handle,
                &mut progress,
            )?;
            validate_filter_descriptors(
                &plan.attempt_request_filters,
                &mut progress,
                InstallError::InvalidFilterMetadata,
            )?;
            validate_filter_body_plan(
                &plan.attempt_request_filters,
                &plan.body_plans.attempt_request,
            )?;
            validate_filter_config_dependencies(
                &plan.attempt_request_filters,
                &plan.config_cell_ids,
                &self.config_cells_handle,
                FilterValidationScope::Attempt,
            )?;
            validate_filter_descriptors(
                &plan.attempt_response_filters,
                &mut progress,
                InstallError::InvalidFilterMetadata,
            )?;
            validate_filter_body_plan(
                &plan.attempt_response_filters,
                &plan.body_plans.attempt_response_precommit,
            )?;
            if matches!(
                plan.body_plans.attempt_response_precommit,
                BodyPlan::SseFramedStreaming { .. }
            ) && plan
                .attempt_response_filters
                .iter()
                .any(|filter| filter.capabilities().expands_body())
            {
                // Precommit classification has one #15 decoded owner per
                // source sequence. One-to-many transforms are supported only
                // after Accept, where the final encoder owns a source ledger.
                return Err(InstallError::IncompatibleFilterBodyPlan);
            }
            validate_filter_config_dependencies(
                &plan.attempt_response_filters,
                &plan.config_cell_ids,
                &self.config_cells_handle,
                FilterValidationScope::Attempt,
            )?;
            for target in
                std::iter::once(&plan.transport_target).chain(plan.authorized_native_targets.iter())
            {
                let fingerprint = target.connection_epoch_fingerprint();
                match reuse_fingerprints.insert(target.reuse_class, fingerprint) {
                    Some(existing) if existing.pool_epoch != target.pool_epoch => {
                        return Err(InstallError::InconsistentPoolEpoch {
                            reuse_class: target.reuse_class,
                            first: existing.pool_epoch,
                            second: target.pool_epoch,
                        });
                    }
                    Some(existing) if existing != fingerprint => {
                        return Err(InstallError::InconsistentConnectionFingerprint(
                            target.reuse_class,
                        ));
                    }
                    _ => {}
                }
            }
        }

        let expected_fingerprints = reuse_fingerprints.len();
        if self.connection_epoch_fingerprints.len() != expected_fingerprints {
            return Err(InstallError::ConnectionFingerprintCoverage {
                expected: expected_fingerprints,
                actual: self.connection_epoch_fingerprints.len(),
            });
        }
        let mut fingerprint_keys = HashSet::new();
        for fingerprint in self.connection_epoch_fingerprints.iter() {
            progress.checkpoint()?;
            if fingerprint.digest == [0; 32] || !fingerprint_keys.insert(fingerprint.reuse_class) {
                return Err(InstallError::InvalidConnectionFingerprint);
            }
            if reuse_fingerprints.get(&fingerprint.reuse_class) != Some(fingerprint) {
                return Err(InstallError::ConnectionFingerprintMismatch(
                    fingerprint.reuse_class,
                ));
            }
        }
        progress.check_now()
    }
}

fn validate_logical_request_plan(
    plan: &CompiledLogicalRequestPlan,
    catalog: &ConfigCellsHandle,
    progress: &mut ValidationProgress<'_>,
) -> Result<(), InstallError> {
    if plan.chunk_capacity == 0 {
        return Err(InstallError::InvalidCompiledPlan(
            PlanError::ZeroBodyQueueCapacity,
        ));
    }
    plan.body_plan
        .validate()
        .map_err(|_| InstallError::InvalidCompiledPlan(PlanError::InvalidBodyPlan))?;
    validate_config_references(&plan.config_cell_ids, catalog, progress)?;
    validate_filter_descriptors(&plan.filters, progress, InstallError::InvalidFilterMetadata)?;
    validate_filter_body_plan(&plan.filters, &plan.body_plan)?;
    validate_filter_config_dependencies(
        &plan.filters,
        &plan.config_cell_ids,
        catalog,
        FilterValidationScope::Logical,
    )
}

fn validate_accepted_response_plan(
    plan: &CompiledAcceptedResponsePlan,
    catalog: &ConfigCellsHandle,
    progress: &mut ValidationProgress<'_>,
) -> Result<(), InstallError> {
    plan.body_plan
        .validate()
        .map_err(|_| InstallError::InvalidCompiledPlan(PlanError::InvalidBodyPlan))?;
    validate_config_references(&plan.config_cell_ids, catalog, progress)?;
    validate_filter_descriptors(
        &plan.filters,
        progress,
        InstallError::InvalidAcceptedFilterMetadata,
    )?;
    validate_filter_body_plan(&plan.filters, &plan.body_plan)?;
    validate_filter_config_dependencies(
        &plan.filters,
        &plan.config_cell_ids,
        catalog,
        FilterValidationScope::Accepted,
    )
}

struct ValidationProgress<'a> {
    cancel: &'a CancellationToken,
    deadline: Instant,
    until_check: u8,
}

impl<'a> ValidationProgress<'a> {
    const CHECK_INTERVAL: u8 = 32;

    fn new(cancel: &'a CancellationToken, deadline: Instant) -> Self {
        Self {
            cancel,
            deadline,
            until_check: 0,
        }
    }

    fn checkpoint(&mut self) -> Result<(), InstallError> {
        if self.until_check == 0 {
            self.check_now()?;
            self.until_check = Self::CHECK_INTERVAL;
        }
        self.until_check -= 1;
        Ok(())
    }

    fn check_now(&self) -> Result<(), InstallError> {
        if self.cancel.is_cancelled() {
            return Err(InstallError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(InstallError::DeadlineExceeded);
        }
        Ok(())
    }
}

fn validate_filter_descriptors(
    filters: &[crate::core::filter::CompiledFilterDescriptor],
    progress: &mut ValidationProgress<'_>,
    invalid: InstallError,
) -> Result<(), InstallError> {
    let mut ids = HashSet::new();
    for filter in filters {
        progress.checkpoint()?;
        if filter.id.trim().is_empty()
            || filter.max_pending_frames == 0
            || !ids.insert(filter.id.as_ref())
        {
            return Err(invalid);
        }
    }
    Ok(())
}

fn validate_filter_body_plan(
    filters: &[crate::core::filter::CompiledFilterDescriptor],
    plan: &BodyPlan,
) -> Result<(), InstallError> {
    for filter in filters {
        let capabilities = filter.capabilities();
        if capabilities.requires_semantic_provenance()
            && !matches!(plan, BodyPlan::SseFramedStreaming { .. })
        {
            return Err(InstallError::IncompatibleFilterBodyPlan);
        }
        if !capabilities.affects_body_commit() {
            continue;
        }
        let compatible = match plan {
            BodyPlan::BufferedTransform { .. } => true,
            BodyPlan::StreamingReplay { .. } => {
                !capabilities.mutates_headers_during_body() && !capabilities.may_local_reply()
            }
            BodyPlan::SseFramedStreaming { .. } => {
                !capabilities.mutates_headers_during_body()
                    && !capabilities.may_local_reply()
                    && (!capabilities.mutates_body() && !capabilities.drops_body()
                        || capabilities.requires_semantic_provenance())
            }
            BodyPlan::PassThrough { .. } => false,
        };
        if !compatible {
            return Err(InstallError::IncompatibleFilterBodyPlan);
        }
    }
    Ok(())
}

fn validate_filter_config_dependencies(
    filters: &[crate::core::filter::CompiledFilterDescriptor],
    referenced_ids: &[ConfigCellId],
    catalog: &HashMap<ConfigCellId, ConfigCellHandle>,
    scope: FilterValidationScope,
) -> Result<(), InstallError> {
    for filter in filters {
        for dependency in filter.config_dependencies() {
            let Some(handle) = catalog.get(&dependency.id) else {
                return Err(InstallError::InvalidFilterConfigDependency(dependency.id));
            };
            if !referenced_ids.contains(&dependency.id)
                || handle.descriptor().binding_policy != dependency.policy
                || (matches!(
                    scope,
                    FilterValidationScope::Logical | FilterValidationScope::Accepted
                ) && matches!(
                    dependency.policy,
                    crate::core::execution_plan::ConfigBindingPolicy::ConnectionPinned
                        | crate::core::execution_plan::ConfigBindingPolicy::AttemptPinned
                ))
            {
                return Err(InstallError::InvalidFilterConfigDependency(dependency.id));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum FilterValidationScope {
    Logical,
    Attempt,
    Accepted,
}

fn route_metadata_is_valid(host: &str, path_prefix: &str) -> bool {
    let Ok(authority) = host.parse::<http::uri::Authority>() else {
        return false;
    };
    !host.is_empty()
        && host == authority.host()
        && authority.port_u16().is_none()
        && host == host.to_ascii_lowercase()
        && !host.ends_with('.')
        && path_prefix.starts_with('/')
        && !path_prefix.contains(['?', '#'])
}

fn validate_config_references(
    ids: &[ConfigCellId],
    catalog: &ConfigCellsHandle,
    progress: &mut ValidationProgress<'_>,
) -> Result<(), InstallError> {
    let mut seen = HashSet::new();
    for id in ids {
        progress.checkpoint()?;
        if !seen.insert(*id) {
            return Err(InstallError::DuplicateConfigReference(*id));
        }
        if !catalog.contains_key(id) {
            return Err(InstallError::MissingReferencedConfig(*id));
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct ActivePublication {
    pub authority_id: AuthorityId,
    pub authority_epoch: u64,
    pub config_revision: ConfigRevision,
    pub plan_revision: PlanRevision,
    pub payload_digest: [u8; 32],
    ingress_plan: CompiledIngressPlanHandle,
    attempt_plan_index: AttemptPlanIndexHandle,
    config_cells: ConfigCellsHandle,
    pub connection_epoch_fingerprints: Arc<[ConnectionEpochFingerprint]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicationIdentity {
    pub authority_id: AuthorityId,
    pub authority_epoch: u64,
    pub config_revision: ConfigRevision,
    pub plan_revision: PlanRevision,
}

impl ActivePublication {
    fn from_envelope(envelope: CompiledGatewayPublicationEnvelope) -> Self {
        Self {
            authority_id: envelope.authority_id,
            authority_epoch: envelope.authority_epoch,
            config_revision: envelope.config_revision,
            plan_revision: envelope.plan_revision,
            payload_digest: envelope.payload_digest,
            ingress_plan: envelope.ingress_plan_handle,
            attempt_plan_index: envelope.attempt_plan_index_handle,
            config_cells: envelope.config_cells_handle,
            connection_epoch_fingerprints: envelope.connection_epoch_fingerprints,
        }
    }

    pub fn bind_request(&self) -> Result<RequestExecutionBinding, PlanError> {
        RequestExecutionBinding::new(
            self.plan_revision,
            Arc::clone(&self.ingress_plan),
            Arc::clone(&self.attempt_plan_index),
            Arc::clone(&self.config_cells),
        )
    }

    fn config_bundle_runtime_facts(
        &self,
    ) -> HashMap<usize, super::execution_plan::ConfigBundleRuntimeFact> {
        self.config_cells
            .values()
            .map(ConfigCellHandle::active_bundle_runtime_fact)
            .map(|fact| (fact.identity, fact))
            .collect()
    }

    fn retirement_facts_against(&self, next: &Self) -> PublicationRetirementFacts {
        let next_identities = next
            .config_bundle_runtime_facts()
            .into_keys()
            .collect::<HashSet<_>>();
        let retired = self
            .config_bundle_runtime_facts()
            .into_values()
            .filter(|fact| !next_identities.contains(&fact.identity) && fact.external_leases > 0)
            .collect::<Vec<_>>();
        PublicationRetirementFacts {
            bytes: retired.iter().map(|fact| fact.bytes).sum(),
            generations: retired
                .iter()
                .map(|fact| fact.generation)
                .collect::<HashSet<_>>()
                .len(),
            oldest_lease_age: retired
                .iter()
                .map(|fact| fact.age)
                .max()
                .unwrap_or_default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PublicationRetirementFacts {
    bytes: usize,
    generations: usize,
    oldest_lease_age: Duration,
}

#[derive(Debug)]
struct InstallerState {
    phase: InstallerPhase,
    generation: u64,
    /// Only the operation that owns this ticket may advance or reject the
    /// shared phase. Validation failures without a ticket remain local facts.
    prepared_generation: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallerPhase {
    Idle,
    Preparing,
    Prepared,
    Publishing,
    Published,
    Rejected,
    Cancelled,
}

#[derive(Debug)]
pub struct PreparedPublication {
    generation: u64,
    publication: Arc<ActivePublication>,
    baseline: Option<Arc<ActivePublication>>,
    retirement_facts: PublicationRetirementFacts,
    prepared_at: Instant,
}

impl PreparedPublication {
    pub fn plan_revision(&self) -> PlanRevision {
        self.publication.plan_revision
    }
}

#[derive(Debug)]
pub enum PrepareOutcome {
    Prepared(PreparedPublication),
    Duplicate(Arc<ActivePublication>),
}

#[derive(Debug)]
pub struct PublicationInstaller {
    active: ArcSwapOption<ActivePublication>,
    state: Mutex<InstallerState>,
    telemetry: Option<Arc<Telemetry>>,
}

impl Default for PublicationInstaller {
    fn default() -> Self {
        Self::new()
    }
}

impl PublicationInstaller {
    pub fn new() -> Self {
        Self {
            active: ArcSwapOption::empty(),
            state: Mutex::new(InstallerState {
                phase: InstallerPhase::Idle,
                generation: 0,
                prepared_generation: None,
            }),
            telemetry: None,
        }
    }

    pub fn with_telemetry(mut self, telemetry: Arc<Telemetry>) -> Self {
        self.telemetry = Some(telemetry);
        self
    }

    pub fn phase(&self) -> InstallerPhase {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .phase
    }

    pub fn active(&self) -> Option<Arc<ActivePublication>> {
        self.active.load_full()
    }

    /// The root Arc exists only within this method. Returned segments have no
    /// backlink, which makes root release independent from stream duration.
    pub fn bind_request(&self) -> Result<RequestExecutionBinding, InstallError> {
        let root = self.active().ok_or(InstallError::NoActivePublication)?;
        root.bind_request()
            .map_err(InstallError::InvalidCompiledPlan)
    }

    pub fn bind_request_with_identity(
        &self,
    ) -> Result<(RequestExecutionBinding, PublicationIdentity), InstallError> {
        let root = self.active().ok_or(InstallError::NoActivePublication)?;
        let identity = PublicationIdentity {
            authority_id: root.authority_id.clone(),
            authority_epoch: root.authority_epoch,
            config_revision: root.config_revision,
            plan_revision: root.plan_revision,
        };
        Ok((
            root.bind_request()
                .map_err(InstallError::InvalidCompiledPlan)?,
            identity,
        ))
    }

    pub fn prepare(
        &self,
        candidate: CompiledGatewayPublicationEnvelope,
        cancel: &CancellationToken,
        deadline: Instant,
    ) -> Result<PrepareOutcome, InstallError> {
        let operation_started = Instant::now();
        let correlation = publication_correlation(
            candidate.authority_id.clone(),
            candidate.authority_epoch,
            candidate.config_revision,
            candidate.plan_revision,
        );
        // Shape validation is pure and may be proportional to a large route,
        // config, and attempt closure. Keep it outside the installer mutex so
        // request binding and another installer operation are never blocked by
        // compiler-owned input traversal. Periodic checkpoints make that
        // traversal itself deadline/cancellation responsive.
        if let Err(error) = candidate.validate_shape(cancel, deadline) {
            let phase = if matches!(
                error,
                InstallError::Cancelled | InstallError::DeadlineExceeded
            ) {
                InstallerPhase::Cancelled
            } else {
                InstallerPhase::Rejected
            };
            // This operation never acquired an installer ticket. Its failure
            // is observable telemetry, but cannot rewrite the shared phase of
            // a different prepared candidate.
            self.observe_publication(
                correlation,
                PublicationStage::Prepared,
                publication_result_for_error(&error),
                phase,
                self.active().is_some(),
                operation_started.elapsed(),
                PublicationRetirementFacts::default(),
            );
            return Err(error);
        }
        let mut state = match self.try_enter(deadline) {
            Ok(state) => state,
            Err(error) => {
                self.observe_publication(
                    correlation,
                    PublicationStage::Prepared,
                    publication_result_for_error(&error),
                    InstallerPhase::Rejected,
                    self.active().is_some(),
                    operation_started.elapsed(),
                    PublicationRetirementFacts::default(),
                );
                return Err(error);
            }
        };
        if cancel.is_cancelled() {
            drop(state);
            self.observe_publication(
                correlation,
                PublicationStage::Prepared,
                PublicationResult::Cancelled,
                InstallerPhase::Cancelled,
                self.active().is_some(),
                operation_started.elapsed(),
                PublicationRetirementFacts::default(),
            );
            return Err(InstallError::Cancelled);
        }
        let baseline = self.active();

        if let Some(active) = baseline.as_ref() {
            match compare_candidate(active, &candidate) {
                CandidateRelation::Duplicate => {
                    drop(state);
                    self.observe_publication(
                        correlation,
                        PublicationStage::Published,
                        PublicationResult::Duplicate,
                        InstallerPhase::Published,
                        true,
                        operation_started.elapsed(),
                        PublicationRetirementFacts::default(),
                    );
                    return Ok(PrepareOutcome::Duplicate(Arc::clone(active)));
                }
                CandidateRelation::Accept => {}
                CandidateRelation::Reject(error) => {
                    drop(state);
                    self.observe_publication(
                        correlation,
                        PublicationStage::Prepared,
                        publication_result_for_error(&error),
                        InstallerPhase::Rejected,
                        true,
                        operation_started.elapsed(),
                        PublicationRetirementFacts::default(),
                    );
                    return Err(error);
                }
            }
        }

        if cancel.is_cancelled() {
            drop(state);
            self.observe_publication(
                correlation,
                PublicationStage::Prepared,
                PublicationResult::Cancelled,
                InstallerPhase::Cancelled,
                self.active().is_some(),
                operation_started.elapsed(),
                PublicationRetirementFacts::default(),
            );
            return Err(InstallError::Cancelled);
        }
        if Instant::now() >= deadline {
            drop(state);
            self.observe_publication(
                correlation,
                PublicationStage::Prepared,
                PublicationResult::Cancelled,
                InstallerPhase::Cancelled,
                self.active().is_some(),
                operation_started.elapsed(),
                PublicationRetirementFacts::default(),
            );
            return Err(InstallError::DeadlineExceeded);
        }

        state.generation = state.generation.wrapping_add(1);
        state.phase = InstallerPhase::Prepared;
        let generation = state.generation;
        state.prepared_generation = Some(generation);
        let publication = Arc::new(ActivePublication::from_envelope(candidate));
        let last_good_preserved = baseline.is_some();
        drop(state);
        let retirement_facts = baseline
            .as_ref()
            .map_or_else(PublicationRetirementFacts::default, |active| {
                active.retirement_facts_against(&publication)
            });
        self.observe_publication(
            correlation,
            PublicationStage::Prepared,
            PublicationResult::Applied,
            InstallerPhase::Prepared,
            last_good_preserved,
            operation_started.elapsed(),
            PublicationRetirementFacts::default(),
        );
        Ok(PrepareOutcome::Prepared(PreparedPublication {
            generation,
            publication,
            baseline,
            retirement_facts,
            prepared_at: operation_started,
        }))
    }

    pub fn publish(
        &self,
        prepared: PreparedPublication,
        cancel: &CancellationToken,
        deadline: Instant,
    ) -> Result<Arc<ActivePublication>, InstallError> {
        let PreparedPublication {
            generation,
            publication,
            baseline,
            retirement_facts,
            prepared_at,
        } = prepared;
        let publication_lag = prepared_at.elapsed();
        let mut state = self.try_enter(deadline)?;
        if state.prepared_generation != Some(generation) {
            // A stale ticket has no authority to rewrite the phase owned by a
            // newer prepared operation.
            return Err(InstallError::StalePreparedPublication);
        }
        if cancel.is_cancelled() {
            state.phase = InstallerPhase::Cancelled;
            state.prepared_generation = None;
            return Err(InstallError::Cancelled);
        }
        let current = self.active();
        let baseline_is_current = match (&baseline, &current) {
            (None, None) => true,
            (Some(baseline), Some(current)) => Arc::ptr_eq(baseline, current),
            _ => false,
        };
        if !baseline_is_current {
            state.phase = InstallerPhase::Rejected;
            state.prepared_generation = None;
            return Err(InstallError::StalePreparedPublication);
        }

        state.phase = InstallerPhase::Publishing;
        // This is the final cancellation/deadline observation immediately
        // before the O(1) ArcSwap linearization point. Cancellation after it
        // races after publication and therefore cannot revoke the new root.
        if cancel.is_cancelled() || Instant::now() >= deadline {
            state.phase = InstallerPhase::Cancelled;
            state.prepared_generation = None;
            return Err(if cancel.is_cancelled() {
                InstallError::Cancelled
            } else {
                InstallError::DeadlineExceeded
            });
        }
        self.active.store(Some(Arc::clone(&publication)));
        state.phase = InstallerPhase::Published;
        state.prepared_generation = None;
        drop(state);
        self.observe_publication(
            publication_correlation(
                publication.authority_id.clone(),
                publication.authority_epoch,
                publication.config_revision,
                publication.plan_revision,
            ),
            PublicationStage::Published,
            PublicationResult::Applied,
            InstallerPhase::Published,
            true,
            publication_lag,
            retirement_facts,
        );
        Ok(publication)
    }

    fn try_enter(&self, deadline: Instant) -> Result<MutexGuard<'_, InstallerState>, InstallError> {
        if Instant::now() >= deadline {
            return Err(InstallError::DeadlineExceeded);
        }
        match self.state.try_lock() {
            Ok(state) => Ok(state),
            Err(TryLockError::WouldBlock) => Err(InstallError::Busy),
            Err(TryLockError::Poisoned(poisoned)) => Ok(poisoned.into_inner()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn observe_publication(
        &self,
        correlation: Correlation,
        stage: PublicationStage,
        result: PublicationResult,
        installer_phase: InstallerPhase,
        last_good_preserved: bool,
        lag: Duration,
        retired: PublicationRetirementFacts,
    ) {
        if let Some(telemetry) = &self.telemetry {
            telemetry.emit(LifecycleEvent {
                correlation,
                monotonic_nanos: 0,
                kind: LifecycleKind::Publication(PublicationFact {
                    stage,
                    result,
                    installer_phase,
                    lag_micros: lag.as_micros().min(u128::from(u64::MAX)) as u64,
                    last_good_preserved,
                    retired_config_bytes: retired.bytes,
                    retired_generation_count: retired.generations,
                    oldest_lease_age_micros: retired
                        .oldest_lease_age
                        .as_micros()
                        .min(u128::from(u64::MAX))
                        as u64,
                }),
            });
        }
    }
}

fn publication_correlation(
    authority_id: AuthorityId,
    authority_epoch: u64,
    config_revision: ConfigRevision,
    plan_revision: PlanRevision,
) -> Correlation {
    Correlation {
        authority_id,
        authority_epoch,
        config_revision,
        plan_revision,
        config_generations: Arc::new([]),
        stable_target_key: None,
        binding_local_id: None,
        request_id: None,
        decision_id: None,
        attempt_id: None,
        attempt_generation: None,
    }
}

fn publication_result_for_error(error: &InstallError) -> PublicationResult {
    match error {
        InstallError::Cancelled | InstallError::DeadlineExceeded => PublicationResult::Cancelled,
        InstallError::StaleAuthorityEpoch
        | InstallError::RollbackNotAuthorized
        | InstallError::StalePreparedPublication => PublicationResult::Stale,
        InstallError::AuthorityConflict | InstallError::RevisionDigestConflict => {
            PublicationResult::Conflict
        }
        InstallError::ResyncRequired { .. } => PublicationResult::ResyncRequired,
        _ => PublicationResult::Rejected,
    }
}

enum CandidateRelation {
    Accept,
    Duplicate,
    Reject(InstallError),
}

fn compare_candidate(
    active: &ActivePublication,
    candidate: &CompiledGatewayPublicationEnvelope,
) -> CandidateRelation {
    if candidate.authority_id != active.authority_id {
        return CandidateRelation::Reject(InstallError::AuthorityConflict);
    }
    if candidate.authority_epoch < active.authority_epoch {
        return CandidateRelation::Reject(InstallError::StaleAuthorityEpoch);
    }
    if candidate.authority_epoch > active.authority_epoch {
        return CandidateRelation::Accept;
    }

    if candidate.config_revision == active.config_revision {
        return if candidate.payload_digest == active.payload_digest {
            CandidateRelation::Duplicate
        } else {
            CandidateRelation::Reject(InstallError::RevisionDigestConflict)
        };
    }

    if candidate.config_revision.0 < active.config_revision.0 && !candidate.rollback_authorized {
        return CandidateRelation::Reject(InstallError::RollbackNotAuthorized);
    }
    if candidate.config_revision.0 > active.config_revision.0.saturating_add(1)
        && !candidate.full_snapshot
    {
        return CandidateRelation::Reject(InstallError::ResyncRequired {
            active: active.config_revision,
            candidate: candidate.config_revision,
        });
    }
    CandidateRelation::Accept
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum InstallError {
    #[error("publication installer is busy")]
    Busy,
    #[error("publication operation was cancelled")]
    Cancelled,
    #[error("publication deadline exceeded")]
    DeadlineExceeded,
    #[error("schema version {0} is incompatible")]
    IncompatibleSchema(u32),
    #[error("compiler version {0} is incompatible")]
    IncompatibleCompiler(u32),
    #[error("publication mixes plan revisions")]
    MixedPlanRevision,
    #[error("publication payload digest is the invalid all-zero sentinel")]
    InvalidPayloadDigest,
    #[error("publication route metadata is not canonical")]
    InvalidRouteMetadata,
    #[error("publication contains a duplicate normalized host/path route")]
    DuplicateRoute,
    #[error("publication contains an invalid compiled plan: {0}")]
    InvalidCompiledPlan(PlanError),
    #[error("compiled plan references missing config cell {0}")]
    MissingReferencedConfig(ConfigCellId),
    #[error("compiled plan references config cell {0} more than once")]
    DuplicateConfigReference(ConfigCellId),
    #[error("config catalog key {key} does not match descriptor {descriptor}")]
    ConfigCatalogKeyMismatch {
        key: ConfigCellId,
        descriptor: ConfigCellId,
    },
    #[error("config atomicity group {0} is split across independent bundle pointers")]
    SplitAtomicityGroup(AtomicityGroupId),
    #[error("logical/attempt filter metadata is empty, duplicated, or unbounded")]
    InvalidFilterMetadata,
    #[error("accepted-response filter metadata is empty or duplicated")]
    InvalidAcceptedFilterMetadata,
    #[error("compiled filter capabilities are incompatible with the directional body plan")]
    IncompatibleFilterBodyPlan,
    #[error("compiled filter config dependency {0} is missing, mis-scoped, or unreachable")]
    InvalidFilterConfigDependency(ConfigCellId),
    #[error(
        "transport reuse class {reuse_class} has inconsistent pool epochs {first} and {second}"
    )]
    InconsistentPoolEpoch {
        reuse_class: TransportReuseClassId,
        first: PoolEpoch,
        second: PoolEpoch,
    },
    #[error("transport reuse class {0} has incompatible connection fingerprints")]
    InconsistentConnectionFingerprint(TransportReuseClassId),
    #[error("connection fingerprint coverage mismatch: expected {expected}, got {actual}")]
    ConnectionFingerprintCoverage { expected: usize, actual: usize },
    #[error("connection fingerprint is zero or duplicated")]
    InvalidConnectionFingerprint,
    #[error("connection fingerprint for transport reuse class {0} does not match its plan")]
    ConnectionFingerprintMismatch(TransportReuseClassId),
    #[error("a different authority attempted to overwrite active state")]
    AuthorityConflict,
    #[error("authority epoch is stale")]
    StaleAuthorityEpoch,
    #[error("same revision carries a different digest")]
    RevisionDigestConflict,
    #[error("rollback requires explicit authority authorization")]
    RollbackNotAuthorized,
    #[error("revision gap from {active} to {candidate} requires full resync")]
    ResyncRequired {
        active: ConfigRevision,
        candidate: ConfigRevision,
    },
    #[error("prepared publication is stale")]
    StalePreparedPublication,
    #[error("no active publication is installed")]
    NoActivePublication,
}
