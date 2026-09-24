use std::collections::HashMap;
use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use http::Uri;
use http::header::CONTENT_LENGTH;
use thiserror::Error;

use crate::core::execution_plan::{
    AdapterId, AttemptBodyPlans, AttemptPlanIndex, AttemptTimeouts, AuthorityId, CaPolicy,
    CompiledAcceptedResponsePlan, CompiledAttemptPlan, CompiledIngressPlan,
    CompiledLocalResponsePlan, CompiledLogicalRequestPlan, CompiledRequestPlan, CompiledRoute,
    ConfigCellsHandle, ConfigRevision, CredentialRef, DEFAULT_OVERALL_REQUEST_TIMEOUT, PlanError,
    PlanRevision, PoolEpoch, ResolvedTargetBindingId, StableTargetKey, TransportReuseClassId,
    TransportScheme, TransportTarget, TransportTargetPolicy,
};
use crate::core::filter::{
    BodyRetentionPort, CompiledFilterDescriptor, FilterError, FramingLedgerPort, RetainedFrameId,
};
use crate::core::publication::CompiledGatewayPublicationEnvelope;
use crate::runtime::body::BodyPlan;
use crate::runtime::driver::path_prefix_matches;

pub mod contracts;
pub mod lifecycle;

/// Compiler-fixture input spanning all four directions. `build` deliberately
/// splits these values into the route-local request plan and attempt-local plan
/// so tests exercise the same ownership topology as production publications.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootstrapBodyPlans {
    pub logical_request: BodyPlan,
    pub attempt_request: BodyPlan,
    pub attempt_response_precommit: BodyPlan,
    pub accepted_response: BodyPlan,
}

impl BootstrapBodyPlans {
    pub fn validate(&self) -> Result<(), PlanError> {
        self.logical_request
            .validate()
            .map_err(|_| PlanError::InvalidBodyPlan)?;
        self.attempt_request
            .validate()
            .map_err(|_| PlanError::InvalidBodyPlan)?;
        self.attempt_response_precommit
            .validate()
            .map_err(|_| PlanError::InvalidBodyPlan)?;
        self.accepted_response
            .validate()
            .map_err(|_| PlanError::InvalidBodyPlan)
    }
}

#[derive(Clone, Debug)]
pub struct BootstrapPublicationBuilder {
    authority_id: AuthorityId,
    authority_epoch: u64,
    config_revision: ConfigRevision,
    plan_revision: PlanRevision,
    routes: Vec<CompiledRoute>,
    attempts: HashMap<ResolvedTargetBindingId, Arc<CompiledAttemptPlan>>,
    config_cells: ConfigCellsHandle,
    accepted_config_cell_ids: Arc<[crate::core::execution_plan::ConfigCellId]>,
    accepted_filters: Arc<[CompiledFilterDescriptor]>,
    full_snapshot: bool,
}

impl BootstrapPublicationBuilder {
    pub fn new(plan_revision: u64, config_revision: u64) -> Self {
        Self {
            authority_id: AuthorityId::new("bootstrap").expect("static authority"),
            authority_epoch: 1,
            config_revision: ConfigRevision(config_revision),
            plan_revision: PlanRevision(plan_revision),
            routes: Vec::new(),
            attempts: HashMap::new(),
            config_cells: Arc::new(HashMap::new()),
            accepted_config_cell_ids: Arc::new([]),
            accepted_filters: Arc::new([]),
            full_snapshot: true,
        }
    }

    pub fn authority(mut self, authority_id: &str, epoch: u64) -> Result<Self, PlanError> {
        self.authority_id = AuthorityId::new(authority_id)?;
        self.authority_epoch = epoch;
        Ok(self)
    }

    pub fn full_snapshot(mut self, full_snapshot: bool) -> Self {
        self.full_snapshot = full_snapshot;
        self
    }

    pub fn config_cells(mut self, config_cells: ConfigCellsHandle) -> Self {
        self.config_cells = config_cells;
        self
    }

    pub fn attempt_config_cells(
        mut self,
        local_binding_id: u32,
        ids: impl Into<Arc<[crate::core::execution_plan::ConfigCellId]>>,
    ) -> Result<Self, PlanError> {
        let binding = ResolvedTargetBindingId::new(self.plan_revision, local_binding_id);
        let plan = self
            .attempts
            .get_mut(&binding)
            .ok_or(PlanError::UnknownBinding(binding))?;
        Arc::make_mut(plan).config_cell_ids = ids.into();
        Ok(self)
    }

    pub fn accepted_config_cells(
        mut self,
        ids: impl Into<Arc<[crate::core::execution_plan::ConfigCellId]>>,
    ) -> Self {
        self.accepted_config_cell_ids = ids.into();
        self
    }

    pub fn logical_config_cells(
        mut self,
        local_binding_id: u32,
        ids: impl Into<Arc<[crate::core::execution_plan::ConfigCellId]>>,
    ) -> Result<Self, PlanError> {
        let binding = ResolvedTargetBindingId::new(self.plan_revision, local_binding_id);
        let route = self
            .routes
            .iter_mut()
            .find(|route| route.binding == binding)
            .ok_or(PlanError::UnknownBinding(binding))?;
        Arc::make_mut(&mut Arc::make_mut(&mut route.request_plan).logical_request)
            .config_cell_ids = ids.into();
        Ok(self)
    }

    pub fn logical_filters(
        mut self,
        local_binding_id: u32,
        filters: impl Into<Arc<[CompiledFilterDescriptor]>>,
    ) -> Result<Self, PlanError> {
        let binding = ResolvedTargetBindingId::new(self.plan_revision, local_binding_id);
        let route = self
            .routes
            .iter_mut()
            .find(|route| route.binding == binding)
            .ok_or(PlanError::UnknownBinding(binding))?;
        Arc::make_mut(&mut Arc::make_mut(&mut route.request_plan).logical_request).filters =
            filters.into();
        Ok(self)
    }

    pub fn attempt_request_filters(
        mut self,
        local_binding_id: u32,
        filters: impl Into<Arc<[CompiledFilterDescriptor]>>,
    ) -> Result<Self, PlanError> {
        let binding = ResolvedTargetBindingId::new(self.plan_revision, local_binding_id);
        let plan = self
            .attempts
            .get_mut(&binding)
            .ok_or(PlanError::UnknownBinding(binding))?;
        Arc::make_mut(plan).attempt_request_filters = filters.into();
        Ok(self)
    }

    pub fn attempt_response_filters(
        mut self,
        local_binding_id: u32,
        filters: impl Into<Arc<[CompiledFilterDescriptor]>>,
    ) -> Result<Self, PlanError> {
        let binding = ResolvedTargetBindingId::new(self.plan_revision, local_binding_id);
        let plan = self
            .attempts
            .get_mut(&binding)
            .ok_or(PlanError::UnknownBinding(binding))?;
        Arc::make_mut(plan).attempt_response_filters = filters.into();
        Ok(self)
    }

    pub fn body_plans(
        mut self,
        local_binding_id: u32,
        body_plans: BootstrapBodyPlans,
    ) -> Result<Self, PlanError> {
        body_plans.validate()?;
        let binding = ResolvedTargetBindingId::new(self.plan_revision, local_binding_id);
        let plan = self
            .attempts
            .get_mut(&binding)
            .ok_or(PlanError::UnknownBinding(binding))?;
        let attempt = Arc::make_mut(plan);
        attempt.body_plans = AttemptBodyPlans {
            attempt_request: body_plans.attempt_request,
            attempt_response_precommit: body_plans.attempt_response_precommit,
        };
        let route = self
            .routes
            .iter_mut()
            .find(|route| route.binding == binding)
            .ok_or(PlanError::UnknownBinding(binding))?;
        let request = Arc::make_mut(&mut route.request_plan);
        Arc::make_mut(&mut request.logical_request).body_plan = body_plans.logical_request;
        Arc::make_mut(&mut request.accepted_response).body_plan = body_plans.accepted_response;
        Ok(self)
    }

    /// Overrides the fixed queue capacities used by the production body
    /// owners. Long-run lifecycle fixtures use this to admit many small
    /// transport frames without weakening any byte limit in `BodyPlan`.
    pub fn body_queue_capacities(
        mut self,
        local_binding_id: u32,
        logical_request_chunks: usize,
        attempt_request_chunks: usize,
    ) -> Result<Self, PlanError> {
        if logical_request_chunks == 0 || attempt_request_chunks == 0 {
            return Err(PlanError::InvalidBodyPlan);
        }
        let binding = ResolvedTargetBindingId::new(self.plan_revision, local_binding_id);
        let plan = self
            .attempts
            .get_mut(&binding)
            .ok_or(PlanError::UnknownBinding(binding))?;
        Arc::make_mut(plan).attempt_request_chunk_capacity = attempt_request_chunks;
        let route = self
            .routes
            .iter_mut()
            .find(|route| route.binding == binding)
            .ok_or(PlanError::UnknownBinding(binding))?;
        Arc::make_mut(&mut Arc::make_mut(&mut route.request_plan).logical_request).chunk_capacity =
            logical_request_chunks;
        Ok(self)
    }

    pub fn precommit_event_capacity(
        mut self,
        local_binding_id: u32,
        capacity: usize,
    ) -> Result<Self, PlanError> {
        if capacity == 0 {
            return Err(PlanError::InvalidBodyPlan);
        }
        let binding = ResolvedTargetBindingId::new(self.plan_revision, local_binding_id);
        let plan = self
            .attempts
            .get_mut(&binding)
            .ok_or(PlanError::UnknownBinding(binding))?;
        Arc::make_mut(plan).precommit_event_capacity = capacity;
        Ok(self)
    }

    pub fn accepted_filters(mut self, filters: impl Into<Arc<[CompiledFilterDescriptor]>>) -> Self {
        self.accepted_filters = filters.into();
        self
    }

    pub fn route_accepted_filters(
        mut self,
        local_binding_id: u32,
        filters: impl Into<Arc<[CompiledFilterDescriptor]>>,
    ) -> Result<Self, PlanError> {
        let binding = ResolvedTargetBindingId::new(self.plan_revision, local_binding_id);
        let route = self
            .routes
            .iter_mut()
            .find(|route| route.binding == binding)
            .ok_or(PlanError::UnknownBinding(binding))?;
        Arc::make_mut(&mut Arc::make_mut(&mut route.request_plan).accepted_response).filters =
            filters.into();
        Ok(self)
    }

    pub fn overall_request_timeout(
        mut self,
        local_binding_id: u32,
        timeout: Duration,
    ) -> Result<Self, PlanError> {
        if timeout.is_zero() {
            return Err(PlanError::InvalidRequestTimeout);
        }
        let binding = ResolvedTargetBindingId::new(self.plan_revision, local_binding_id);
        let route = self
            .routes
            .iter_mut()
            .find(|route| route.binding == binding)
            .ok_or(PlanError::UnknownBinding(binding))?;
        Arc::make_mut(&mut route.request_plan).overall_request_timeout = timeout;
        Ok(self)
    }

    pub fn route_max_attempts(
        mut self,
        local_binding_id: u32,
        max_attempts: u32,
    ) -> Result<Self, PlanError> {
        if max_attempts == 0 {
            return Err(PlanError::InvalidRouteCandidateClosure);
        }
        let binding = ResolvedTargetBindingId::new(self.plan_revision, local_binding_id);
        let route = self
            .routes
            .iter_mut()
            .find(|route| route.binding == binding)
            .ok_or(PlanError::UnknownBinding(binding))?;
        Arc::make_mut(&mut route.request_plan).max_attempts = max_attempts;
        Ok(self)
    }

    pub fn attempt_timeouts(
        mut self,
        local_binding_id: u32,
        timeouts: AttemptTimeouts,
    ) -> Result<Self, PlanError> {
        timeouts.validate()?;
        let binding = ResolvedTargetBindingId::new(self.plan_revision, local_binding_id);
        let plan = self
            .attempts
            .get_mut(&binding)
            .ok_or(PlanError::UnknownBinding(binding))?;
        Arc::make_mut(plan).timeouts = timeouts;
        Ok(self)
    }

    pub fn credential_refs(
        mut self,
        local_binding_id: u32,
        credential_refs: impl IntoIterator<Item = CredentialRef>,
    ) -> Result<Self, PlanError> {
        let credential_refs: Vec<_> = credential_refs.into_iter().collect();
        if credential_refs.is_empty() {
            return Err(PlanError::EmptyCredentialClosure);
        }
        let binding = ResolvedTargetBindingId::new(self.plan_revision, local_binding_id);
        let plan = self
            .attempts
            .get_mut(&binding)
            .ok_or(PlanError::UnknownBinding(binding))?;
        Arc::make_mut(plan).credential_refs = credential_refs.into();
        Ok(self)
    }

    pub fn route(
        mut self,
        host: &str,
        path_prefix: &str,
        local_binding_id: u32,
        transport_target: TransportTarget,
    ) -> Result<Self, PlanError> {
        transport_target.validate()?;
        let binding = ResolvedTargetBindingId::new(self.plan_revision, local_binding_id);
        self.routes.push(CompiledRoute {
            normalized_host: normalize_host(host)?.into(),
            path_prefix: normalize_path_prefix(path_prefix)?.into(),
            binding,
            request_plan: Arc::new(CompiledRequestPlan {
                candidate_bindings: Arc::from([binding]),
                logical_request: Arc::new(CompiledLogicalRequestPlan {
                    filters: Arc::new([]),
                    body_plan: BodyPlan::BufferedTransform {
                        max_body_bytes: 1024 * 1024,
                    },
                    config_cell_ids: Arc::new([]),
                    chunk_capacity: 64,
                }),
                accepted_response: Arc::new(CompiledAcceptedResponsePlan {
                    filters: Arc::new([]),
                    body_plan: BodyPlan::PassThrough {
                        max_chunk_bytes: 64 * 1024,
                    },
                    semantic_replacement_authorized: false,
                    config_cell_ids: Arc::new([]),
                }),
                overall_request_timeout: DEFAULT_OVERALL_REQUEST_TIMEOUT,
                max_attempts: 8,
            }),
        });
        self.attempts.insert(
            binding,
            Arc::new(CompiledAttemptPlan {
                binding,
                stable_target_key: StableTargetKey::new(format!("bootstrap-{local_binding_id}"))?,
                adapter_id: AdapterId::new("bootstrap-http")?,
                credential_refs: Arc::from([CredentialRef::new(format!(
                    "bootstrap-credential-{local_binding_id}"
                ))?]),
                transport_target,
                authorized_native_targets: Arc::from([]),
                transport_target_policy: TransportTargetPolicy::Exact,
                timeouts: AttemptTimeouts {
                    request_write: Duration::from_secs(30),
                    first_byte: Duration::from_secs(30),
                    stream_idle: Duration::from_secs(30),
                },
                config_cell_ids: Arc::new([]),
                attempt_request_filters: Arc::new([]),
                attempt_response_filters: Arc::new([]),
                body_plans: AttemptBodyPlans {
                    attempt_request: BodyPlan::StreamingReplay {
                        max_chunk_bytes: 16 * 1024,
                        max_replay_bytes: 1024 * 1024,
                    },
                    attempt_response_precommit: BodyPlan::PassThrough {
                        max_chunk_bytes: 64 * 1024,
                    },
                },
                attempt_request_chunk_capacity: 16 * 1024,
                precommit_event_capacity: 16,
            }),
        );
        Ok(self)
    }

    /// Seal the complete #13 fallback closure for one route. Every listed
    /// binding must already have a compiled attempt in this publication.
    pub fn route_candidates(
        mut self,
        route_local_binding_id: u32,
        candidate_local_binding_ids: impl IntoIterator<Item = u32>,
    ) -> Result<Self, PlanError> {
        let route_binding =
            ResolvedTargetBindingId::new(self.plan_revision, route_local_binding_id);
        let candidates: Vec<_> = candidate_local_binding_ids
            .into_iter()
            .map(|local_id| ResolvedTargetBindingId::new(self.plan_revision, local_id))
            .collect();
        if candidates.is_empty()
            || !candidates.contains(&route_binding)
            || candidates
                .iter()
                .any(|binding| !self.attempts.contains_key(binding))
        {
            return Err(PlanError::InvalidRouteCandidateClosure);
        }
        let mut unique = std::collections::HashSet::new();
        if candidates.iter().any(|binding| !unique.insert(*binding)) {
            return Err(PlanError::InvalidRouteCandidateClosure);
        }
        let route = self
            .routes
            .iter_mut()
            .find(|route| route.binding == route_binding)
            .ok_or(PlanError::UnknownBinding(route_binding))?;
        Arc::make_mut(&mut route.request_plan).candidate_bindings = candidates.into();
        Ok(self)
    }

    pub fn build(mut self) -> Result<CompiledGatewayPublicationEnvelope, PlanError> {
        let digest = deterministic_digest(
            self.authority_id.as_str(),
            self.authority_epoch,
            self.config_revision.0,
            self.plan_revision.0,
        );
        let mut connection_epoch_fingerprints: Vec<_> = self
            .attempts
            .values()
            .map(|plan| plan.transport_target.connection_epoch_fingerprint())
            .collect();
        connection_epoch_fingerprints.sort_unstable();
        connection_epoch_fingerprints.dedup();
        let connection_epoch_fingerprints = connection_epoch_fingerprints.into();
        for route in &mut self.routes {
            let accepted = &mut Arc::make_mut(&mut route.request_plan).accepted_response;
            let accepted = Arc::make_mut(accepted);
            if accepted.filters.is_empty() {
                accepted.filters = Arc::clone(&self.accepted_filters);
            }
            if accepted.config_cell_ids.is_empty() {
                accepted.config_cell_ids = Arc::clone(&self.accepted_config_cell_ids);
            }
        }
        let local_accepted = Arc::new(CompiledAcceptedResponsePlan {
            filters: Arc::clone(&self.accepted_filters),
            body_plan: BodyPlan::PassThrough {
                max_chunk_bytes: 64 * 1024,
            },
            semantic_replacement_authorized: false,
            config_cell_ids: Arc::clone(&self.accepted_config_cell_ids),
        });
        let attempt_index = AttemptPlanIndex::new(self.plan_revision, self.attempts)?;
        Ok(CompiledGatewayPublicationEnvelope {
            authority_id: self.authority_id,
            authority_epoch: self.authority_epoch,
            config_revision: self.config_revision,
            plan_revision: self.plan_revision,
            schema_version: 2,
            compiler_version: 2,
            payload_digest: digest,
            ingress_plan_handle: Arc::new(CompiledIngressPlan {
                plan_revision: self.plan_revision,
                routes: self.routes.into(),
                local_response_plan: Arc::new(CompiledLocalResponsePlan {
                    accepted_response: local_accepted,
                    overall_request_timeout: DEFAULT_OVERALL_REQUEST_TIMEOUT,
                }),
            }),
            attempt_plan_index_handle: Arc::new(attempt_index),
            config_cells_handle: self.config_cells,
            connection_epoch_fingerprints,
            full_snapshot: self.full_snapshot,
            rollback_authorized: false,
        })
    }
}

pub fn plain_target(address: SocketAddr, reuse_class: u64) -> TransportTarget {
    TransportTarget {
        scheme: TransportScheme::Http,
        authority: format!("{}:{}", address.ip(), address.port()).into(),
        addresses: Arc::new([address]),
        sni: None,
        ca: CaPolicy::System,
        alpn: Arc::new([Arc::from("http/1.1")]),
        connect_timeout: Duration::from_secs(2),
        transport_read_buffer_bytes: 64 * 1024,
        h2_stream_window_bytes: 64 * 1024,
        h2_connection_window_bytes: 256 * 1024,
        h2_max_concurrent_streams: 16,
        reuse_class: TransportReuseClassId(reuse_class),
        pool_epoch: PoolEpoch(1),
        connection_fingerprint: [0; 32],
    }
    .with_derived_connection_fingerprint()
}

pub fn tls_target(
    address: SocketAddr,
    authority: &str,
    sni: &str,
    reuse_class: u64,
) -> TransportTarget {
    TransportTarget {
        scheme: TransportScheme::Https,
        authority: authority.into(),
        addresses: Arc::new([address]),
        sni: Some(sni.into()),
        ca: CaPolicy::System,
        alpn: Arc::new([Arc::from("h2"), Arc::from("http/1.1")]),
        connect_timeout: Duration::from_secs(2),
        transport_read_buffer_bytes: 64 * 1024,
        h2_stream_window_bytes: 64 * 1024,
        h2_connection_window_bytes: 256 * 1024,
        h2_max_concurrent_streams: 16,
        reuse_class: TransportReuseClassId(reuse_class),
        pool_epoch: PoolEpoch(1),
        connection_fingerprint: [0; 32],
    }
    .with_derived_connection_fingerprint()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatchedBootstrapRequest {
    pub binding: ResolvedTargetBindingId,
    pub original_path_and_query: Arc<str>,
}

pub fn match_bootstrap_request(
    plan: &CompiledIngressPlan,
    uri: &Uri,
    host_header: Option<&str>,
) -> Result<Option<MatchedBootstrapRequest>, BootstrapError> {
    let authority = uri
        .authority()
        .map(|value| value.as_str())
        .or(host_header)
        .ok_or(BootstrapError::MissingAuthority)?;
    let normalized_host = normalize_host(authority)?;
    let path = uri.path();
    let Some(route) = plan
        .routes
        .iter()
        .filter(|route| {
            route.normalized_host.as_ref() == normalized_host
                && path_prefix_matches(path, &route.path_prefix)
        })
        .max_by_key(|route| route.path_prefix.len())
    else {
        return Ok(None);
    };
    Ok(Some(MatchedBootstrapRequest {
        binding: route.binding,
        original_path_and_query: uri
            .path_and_query()
            .map_or_else(|| Arc::from("/"), |value| Arc::from(value.as_str())),
    }))
}

fn normalize_host(host: &str) -> Result<String, BootstrapError> {
    let host = host.trim();
    if host.is_empty() {
        return Err(BootstrapError::MissingAuthority);
    }
    if let Ok(authority) = host.parse::<http::uri::Authority>() {
        return Ok(authority.host().trim_end_matches('.').to_ascii_lowercase());
    }
    Err(BootstrapError::InvalidAuthority)
}

fn normalize_path_prefix(path: &str) -> Result<String, BootstrapError> {
    if !path.starts_with('/') {
        return Err(BootstrapError::InvalidPathPrefix);
    }
    Ok(path.to_owned())
}

fn deterministic_digest(authority: &str, epoch: u64, config: u64, plan: u64) -> [u8; 32] {
    // This fixture digest is deliberately non-cryptographic. The production
    // compiler (#11) owns authentication and digest construction; #9 treats it
    // as opaque version-fence input.
    let mut digest = [0_u8; 32];
    for (index, byte) in authority
        .bytes()
        .chain(epoch.to_le_bytes())
        .chain(config.to_le_bytes())
        .chain(plan.to_le_bytes())
        .enumerate()
    {
        let slot = index % digest.len();
        digest[slot] = digest[slot].wrapping_mul(31).wrapping_add(byte);
    }
    digest
}

#[derive(Debug)]
pub struct BootstrapListenerConfig {
    pub bind: SocketAddr,
    pub allow_non_loopback: bool,
    pub downstream_tls: Option<Arc<[u8]>>,
}

impl Default for BootstrapListenerConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            allow_non_loopback: false,
            downstream_tls: None,
        }
    }
}

impl BootstrapListenerConfig {
    pub fn validate(&self) -> Result<(), BootstrapError> {
        if !self.bind.ip().is_loopback() && !self.allow_non_loopback {
            return Err(BootstrapError::NonLoopbackRequiresOptIn);
        }
        if self.downstream_tls.is_some() {
            return Err(BootstrapError::DownstreamTlsUnsupported);
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct NetworkUseCounter {
    connections: AtomicUsize,
}

impl NetworkUseCounter {
    pub fn record_connect(&self) {
        self.connections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum BootstrapError {
    #[error("request authority is missing")]
    MissingAuthority,
    #[error("request authority is invalid")]
    InvalidAuthority,
    #[error("route path prefix must start with '/'")]
    InvalidPathPrefix,
    #[error("non-loopback listener requires explicit opt-in")]
    NonLoopbackRequiresOptIn,
    #[error("downstream TLS termination is unsupported in P1")]
    DownstreamTlsUnsupported,
}

impl From<BootstrapError> for PlanError {
    fn from(value: BootstrapError) -> Self {
        match value {
            BootstrapError::MissingAuthority | BootstrapError::InvalidAuthority => {
                PlanError::EmptyTransportAuthority
            }
            BootstrapError::InvalidPathPrefix
            | BootstrapError::NonLoopbackRequiresOptIn
            | BootstrapError::DownstreamTlsUnsupported => PlanError::EmptyTransportAuthority,
        }
    }
}

#[derive(Debug)]
pub struct BoundedBodyRetention {
    max_bytes: usize,
    live_bytes: usize,
    next_id: u64,
    frames: VecDeque<(RetainedFrameId, Option<bytes::Bytes>, usize)>,
    pub read_paused: bool,
}

impl BoundedBodyRetention {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            live_bytes: 0,
            next_id: 0,
            frames: VecDeque::new(),
            read_paused: false,
        }
    }
}

impl BodyRetentionPort for BoundedBodyRetention {
    fn retain(&mut self, bytes: bytes::Bytes) -> Result<RetainedFrameId, FilterError> {
        let next = self
            .live_bytes
            .checked_add(bytes.len())
            .ok_or(FilterError::RetentionLimit)?;
        if next > self.max_bytes {
            return Err(FilterError::RetentionLimit);
        }
        self.next_id = self.next_id.wrapping_add(1);
        let id = RetainedFrameId(self.next_id);
        self.live_bytes = next;
        let retained = bytes.len();
        self.frames.push_back((id, Some(bytes), retained));
        Ok(id)
    }

    fn take(&mut self, id: RetainedFrameId) -> Result<bytes::Bytes, FilterError> {
        let position = self
            .frames
            .iter()
            .position(|(candidate, _, _)| *candidate == id)
            .ok_or(FilterError::UnknownRetainedFrame)?;
        let (_, bytes, retained) = self
            .frames
            .remove(position)
            .ok_or(FilterError::UnknownRetainedFrame)?;
        let bytes = bytes.ok_or(FilterError::UnknownRetainedFrame)?;
        self.live_bytes -= retained;
        Ok(bytes)
    }

    fn retain_charged(&mut self, bytes: usize) -> Result<RetainedFrameId, FilterError> {
        let next = self
            .live_bytes
            .checked_add(bytes)
            .ok_or(FilterError::RetentionLimit)?;
        if next > self.max_bytes {
            return Err(FilterError::RetentionLimit);
        }
        self.next_id = self.next_id.wrapping_add(1);
        let id = RetainedFrameId(self.next_id);
        self.live_bytes = next;
        self.frames.push_back((id, None, bytes));
        Ok(id)
    }

    fn release_charged(&mut self, id: RetainedFrameId) -> Result<(), FilterError> {
        let position = self
            .frames
            .iter()
            .position(|(candidate, _, _)| *candidate == id)
            .ok_or(FilterError::UnknownRetainedFrame)?;
        let (_, bytes, retained) = self
            .frames
            .remove(position)
            .ok_or(FilterError::UnknownRetainedFrame)?;
        if bytes.is_some() {
            return Err(FilterError::UnknownRetainedFrame);
        }
        self.live_bytes -= retained;
        Ok(())
    }

    fn set_read_paused(&mut self, paused: bool) {
        self.read_paused = paused;
    }
}

#[derive(Debug, Default)]
pub struct FakeFramingLedger {
    pub content_length_touched: bool,
    pub transformed: bool,
}

impl FramingLedgerPort for FakeFramingLedger {
    fn header_mutated(&mut self, name: &http::HeaderName) {
        self.content_length_touched |= name == CONTENT_LENGTH;
    }

    fn body_transformed(&mut self) {
        self.transformed = true;
    }
}
