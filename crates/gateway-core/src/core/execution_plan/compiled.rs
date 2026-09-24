use super::*;

#[derive(Clone, Debug)]
pub struct CompiledIngressPlan {
    pub plan_revision: PlanRevision,
    pub routes: Arc<[CompiledRoute]>,
    /// Explicit plan for route misses and ingress-local failures. It is not an
    /// Endpoint/RouteProfile accepted-response plan and therefore cannot leak
    /// matched-route semantics into a local reply.
    pub local_response_plan: CompiledLocalResponsePlanHandle,
}

#[derive(Clone, Debug)]
pub struct CompiledRoute {
    pub normalized_host: Arc<str>,
    pub path_prefix: Arc<str>,
    pub binding: ResolvedTargetBindingId,
    pub request_plan: CompiledRequestPlanHandle,
}

/// The complete immutable scope of one matched Endpoint/RouteProfile. A
/// fallback replaces only an Attempt and never changes either downstream
/// direction stored here.
#[derive(Clone, Debug)]
pub struct CompiledRequestPlan {
    /// Complete candidate closure for this route. The primary route binding is
    /// included so request-pinned generations can be acquired once after route
    /// matching without retaining unrelated routes.
    pub candidate_bindings: Arc<[ResolvedTargetBindingId]>,
    pub logical_request: CompiledLogicalRequestPlanHandle,
    pub accepted_response: CompiledAcceptedResponsePlanHandle,
    pub overall_request_timeout: Duration,
    pub max_attempts: u32,
}

#[derive(Clone, Debug)]
pub struct CompiledLogicalRequestPlan {
    pub filters: Arc<[CompiledFilterDescriptor]>,
    pub body_plan: BodyPlan,
    pub config_cell_ids: Arc<[ConfigCellId]>,
    pub chunk_capacity: usize,
}

#[derive(Clone, Debug)]
pub struct CompiledAcceptedResponsePlan {
    pub filters: Arc<[CompiledFilterDescriptor]>,
    pub body_plan: BodyPlan,
    pub semantic_replacement_authorized: bool,
    pub config_cell_ids: Arc<[ConfigCellId]>,
}

#[derive(Clone, Debug)]
pub struct CompiledLocalResponsePlan {
    pub accepted_response: CompiledAcceptedResponsePlanHandle,
    pub overall_request_timeout: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttemptTimeouts {
    pub request_write: Duration,
    pub first_byte: Duration,
    pub stream_idle: Duration,
}

impl AttemptTimeouts {
    pub fn validate(self) -> Result<(), PlanError> {
        if self.request_write.is_zero() || self.first_byte.is_zero() || self.stream_idle.is_zero() {
            return Err(PlanError::InvalidAttemptTimeouts);
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct CompiledAttemptPlan {
    pub binding: ResolvedTargetBindingId,
    pub stable_target_key: StableTargetKey,
    pub adapter_id: AdapterId,
    /// Compiler-sealed set from which a request-owned decision session may
    /// grant exactly one non-secret reference before materialization.
    pub credential_refs: Arc<[CredentialRef]>,
    pub transport_target: TransportTarget,
    /// Other immutable native targets selected by protocol profiles in this publication.
    pub authorized_native_targets: Arc<[TransportTarget]>,
    /// Defines whether request-scoped provider authority may replace only the
    /// numeric loopback socket of the compiled target. Durable CPA
    /// publications survive daemon restarts, while the supervised CPA process
    /// intentionally receives a newly reserved loopback port each lifetime.
    /// Native provider targets always remain exact.
    pub transport_target_policy: TransportTargetPolicy,
    pub timeouts: AttemptTimeouts,
    pub config_cell_ids: Arc<[ConfigCellId]>,
    /// Attempt request encoder chain. It runs after the provider materializes the
    /// provider request but before the semantic upstream exchange begins.
    pub attempt_request_filters: Arc<[CompiledFilterDescriptor]>,
    /// Attempt response decoder chain. It runs on the bounded precommit stream
    /// before the provider emits a structured classification.
    pub attempt_response_filters: Arc<[CompiledFilterDescriptor]>,
    pub body_plans: AttemptBodyPlans,
    pub attempt_request_chunk_capacity: usize,
    pub precommit_event_capacity: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportTargetPolicy {
    Exact,
    ManagedLoopback,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptBodyPlans {
    pub attempt_request: BodyPlan,
    pub attempt_response_precommit: BodyPlan,
}

impl AttemptBodyPlans {
    pub fn validate(&self) -> Result<(), PlanError> {
        self.attempt_request
            .validate()
            .map_err(|_| PlanError::InvalidBodyPlan)?;
        self.attempt_response_precommit
            .validate()
            .map_err(|_| PlanError::InvalidBodyPlan)?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct AttemptPlanIndex {
    pub plan_revision: PlanRevision,
    plans: Arc<HashMap<ResolvedTargetBindingId, Arc<CompiledAttemptPlan>>>,
}

impl AttemptPlanIndex {
    pub fn new(
        plan_revision: PlanRevision,
        plans: HashMap<ResolvedTargetBindingId, Arc<CompiledAttemptPlan>>,
    ) -> Result<Self, PlanError> {
        for (id, plan) in &plans {
            if id.plan_revision() != plan_revision || plan.binding != *id {
                return Err(PlanError::CrossRevisionBinding {
                    expected: plan_revision,
                    actual: id.plan_revision(),
                });
            }
            plan.transport_target.validate()?;
            if !plan.authorized_native_targets.is_empty()
                && plan.transport_target_policy != TransportTargetPolicy::Exact
            {
                return Err(PlanError::InvalidManagedLoopbackTarget);
            }
            for target in plan.authorized_native_targets.iter() {
                target.validate()?;
            }
            if plan.transport_target_policy == TransportTargetPolicy::ManagedLoopback
                && !plan.transport_target.is_numeric_loopback_http()
            {
                return Err(PlanError::InvalidManagedLoopbackTarget);
            }
            plan.timeouts.validate()?;
            plan.body_plans.validate()?;
            if plan.credential_refs.is_empty() {
                return Err(PlanError::EmptyCredentialClosure);
            }
            if plan.attempt_request_chunk_capacity == 0 || plan.precommit_event_capacity == 0 {
                return Err(PlanError::ZeroBodyQueueCapacity);
            }
        }
        Ok(Self {
            plan_revision,
            plans: Arc::new(plans),
        })
    }

    /// Resolution happens before DNS, credential access, or connect.
    pub fn resolve(
        &self,
        id: ResolvedTargetBindingId,
    ) -> Result<Arc<CompiledAttemptPlan>, PlanError> {
        if id.plan_revision() != self.plan_revision {
            return Err(PlanError::CrossRevisionBinding {
                expected: self.plan_revision,
                actual: id.plan_revision(),
            });
        }
        self.plans
            .get(&id)
            .cloned()
            .ok_or(PlanError::UnknownBinding(id))
    }

    pub fn plans(&self) -> impl Iterator<Item = (ResolvedTargetBindingId, &CompiledAttemptPlan)> {
        self.plans
            .iter()
            .map(|(binding, plan)| (*binding, plan.as_ref()))
    }
}

pub type CompiledIngressPlanHandle = Arc<CompiledIngressPlan>;
pub type AttemptPlanIndexHandle = Arc<AttemptPlanIndex>;
pub type CompiledRequestPlanHandle = Arc<CompiledRequestPlan>;
pub type CompiledLogicalRequestPlanHandle = Arc<CompiledLogicalRequestPlan>;
pub type CompiledAcceptedResponsePlanHandle = Arc<CompiledAcceptedResponsePlan>;
pub type CompiledLocalResponsePlanHandle = Arc<CompiledLocalResponsePlan>;
pub type ConfigCellsHandle = Arc<HashMap<ConfigCellId, ConfigCellHandle>>;
