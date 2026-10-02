//! Closed aggregate publication consumed by the Gateway boundary.
//!
//! It is the only persistent request authority; adapters may project it but may not join mutable
//! Registry, AgentConnection, or credential storage state.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    AGENT_PLAN_COMPILED_SCHEMA_V3, AGENT_PLAN_COMPILER_REVISION_V1,
    AGENT_PLAN_COMPILER_REVISION_V3, AgentIngressProtocolV1, AliasRegistryV1,
    AttemptOwnedCandidateV1, CanonicalDigest, CompiledAgentPlanV1, ConnectorRuntimeKind,
    GatewayAccessGrantV1, GatewayCandidateProtocolProfileV1, GatewayOperationalTargetV1,
    MaterializedGroupId, ModelAlias, PortResult, PublishedAgentPlanV1, RequestOwnedRouteV1,
    UpstreamProtocol, WorkspaceId,
};

mod plan_state;
mod projection;
mod record;
mod stored;
pub use record::PublicationRecordV1;
pub use stored::*;
mod snapshot;
#[cfg(test)]
mod validation_tests;
pub use plan_state::GATEWAY_PUBLICATION_SCHEMA_V3;
use projection::*;
pub use snapshot::{
    GatewayAdmissionStateV1, GatewayModelRouteV2, GatewayProjectedGrantV2,
    GatewayPublicationSnapshotProjectionV3,
};

pub(crate) const LEGACY_GATEWAY_PUBLICATION_SCHEMA_V2: &str = "hiroute.gateway-publication/v2";
#[cfg(test)]
const UNSUPPORTED_GATEWAY_PUBLICATION_SCHEMA_V1: &str = "hiroute.gateway-publication/v1";
pub const GATEWAY_SNAPSHOT_SCHEMA_V3: &str = "hiroute.gateway.publication-snapshot/v3";
pub const DEFAULT_CATALOG_RENDERER_REVISION: &str = "hiroute.gateway-catalog-renderer/v1";
const MAX_REQUEST_ATTEMPTS: u16 = 6;
const MAX_LOGICAL_REQUEST_DURATION_MS: u64 = 60 * 60 * 1_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct GatewayPublicationRevision(u64);

impl GatewayPublicationRevision {
    pub fn new(value: u64) -> Result<Self, PublicationError> {
        if value == 0 {
            Err(PublicationError::InvalidRevision)
        } else {
            Ok(Self(value))
        }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Exact G0 candidate fields with aggregate-global deterministic `local_id` allocation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayExecutableCandidateV2 {
    pub local_id: u32,
    pub stable_target_key: String,
    pub adapter_id: String,
    pub credential_refs: Vec<String>,
    /// Exact Secret authority destination derived from the current connection option.
    pub credential_destination_ref: String,
    /// Current Registry model identity.
    pub upstream_model_id: String,
    /// Exact model identifier rendered on the operational transport.
    pub native_transport_model: String,
    /// Current Registry identity; never rewritten to the loopback transport target.
    /// Its Provider path may differ from a managed bridge's local request path.
    pub endpoint: String,
    pub connector_runtime: ConnectorRuntimeKind,
    pub operational_target: GatewayOperationalTargetV1,
    pub operational_target_digest: CanonicalDigest,
    pub protocol_profiles: Vec<GatewayCandidateProtocolProfileV1>,
    pub protocol_profile_digest: CanonicalDigest,
    /// Non-secret actual-price lookup identity. Product publications keep the
    /// full source facts in their compiled Plans; `gateway_snapshot` adds this
    /// narrow projection without changing the durable Product record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_identity: Option<GatewayCandidatePricingIdentityV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayCandidatePricingIdentityV1 {
    pub source_id: String,
    pub source_identity_digest: CanonicalDigest,
    pub model_configuration_id: String,
    pub actual_offer_ref: String,
}

/// Exact request-owned route and group membership projected to the Gateway.
/// Candidate ordering is already frozen by the Application compiler; the
/// Gateway may filter for request capability but must not reconstruct groups.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayExecutableGroupV2 {
    pub group_id: MaterializedGroupId,
    pub candidate_local_ids: Vec<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayExecutableRoutingV2 {
    pub agent_plan_id: String,
    /// Safe plan display text frozen into new publications; absent on legacy snapshots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_display_name: Option<String>,
    pub request_owned: RequestOwnedRouteV1,
    pub groups: Vec<GatewayExecutableGroupV2>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayExecutableAliasV2 {
    pub served_model_id: ModelAlias,
    pub purpose: String,
    pub agent_plan_revision: u64,
    pub protocols: Vec<AgentIngressProtocolV1>,
    pub overall_timeout_ms: u64,
    pub max_attempts: u32,
    /// Request-owned routing metadata was added after the first G0 publication revision. `None`
    /// remains readable only so an already-durable publication can be upgraded by the next
    /// Application-owned publish; newly materialized aliases always carry the closed projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<GatewayExecutableRoutingV2>,
    pub candidates: Vec<GatewayExecutableCandidateV2>,
}

/// G0-compatible, non-secret grant projection derived from an exact AgentConnection Plan grant.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayExecutableGrantV2 {
    pub grant_id: String,
    pub generation: u64,
    pub bearer_token_sha256: CanonicalDigest,
    pub model_grant: crate::AgentModelGrantV2,
}

impl GatewayExecutableGrantV2 {
    pub fn permits_plan(&self, alias: &ModelAlias) -> bool {
        matches!(self.model_grant.routes.get(alias.as_str()), Some(crate::AgentModelRouteV2::Plan { alias: permitted, .. }) if permitted == alias)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayPublicationV1 {
    pub schema: String,
    pub compiler_revision: String,
    pub workspace_id: WorkspaceId,
    pub authority_id: String,
    pub authority_epoch: u64,
    pub publication_revision: GatewayPublicationRevision,
    pub catalog_renderer_revision: String,
    pub alias_registry: AliasRegistryV1,
    pub plans: Vec<CompiledAgentPlanV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan_heads: Vec<crate::PlanHeadV1>,
    pub aliases: Vec<GatewayExecutableAliasV2>,
    pub grants: Vec<GatewayExecutableGrantV2>,
}

impl GatewayPublicationV1 {
    /// Creates an empty durable bootstrap record. Executable publications must use `seal`.
    pub fn new(
        workspace_id: WorkspaceId,
        publication_revision: GatewayPublicationRevision,
        alias_registry: AliasRegistryV1,
        plans: Vec<CompiledAgentPlanV1>,
    ) -> Result<Self, PublicationError> {
        if !plans.is_empty() {
            return Err(PublicationError::PublicationNotClosed);
        }
        let authority_id = workspace_id.as_str().to_owned();
        Self::from_parts(
            workspace_id,
            authority_id,
            1,
            publication_revision,
            DEFAULT_CATALOG_RENDERER_REVISION.to_owned(),
            alias_registry,
            plans,
            Vec::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn seal(
        workspace_id: WorkspaceId,
        authority_id: impl Into<String>,
        authority_epoch: u64,
        publication_revision: GatewayPublicationRevision,
        catalog_renderer_revision: impl Into<String>,
        alias_registry: AliasRegistryV1,
        mut plans: Vec<CompiledAgentPlanV1>,
        access_grants: Vec<GatewayAccessGrantV1>,
    ) -> Result<Self, PublicationError> {
        plans.sort_by(|left, right| left.agent_plan_id().cmp(right.agent_plan_id()));
        let grants = materialize_grants(access_grants)?;
        Self::from_parts(
            workspace_id,
            authority_id.into(),
            authority_epoch,
            publication_revision,
            catalog_renderer_revision.into(),
            alias_registry,
            plans,
            grants,
        )
    }

    /// Produces the next publication after creating or revising one independent AgentPlan.
    ///
    /// A Plan may exist before any AgentConnection grants it. Such a catalog-only publication
    /// is durable but deliberately cannot be projected into a Gateway request authority. Adding
    /// a matching grant later makes only the granted aliases executable.
    pub fn next_with_plan(
        &self,
        publication_revision: GatewayPublicationRevision,
        alias_registry: AliasRegistryV1,
        plan: CompiledAgentPlanV1,
    ) -> Result<Self, PublicationError> {
        self.validate()?;
        let mut plans = self
            .plans
            .iter()
            .filter(|current| current.agent_plan_id() != plan.agent_plan_id())
            .cloned()
            .collect::<Vec<_>>();
        plans.push(
            plan.into_current()
                .map_err(|_| PublicationError::InvalidPlan)?,
        );
        plans.sort_by(|left, right| left.agent_plan_id().cmp(right.agent_plan_id()));
        Self::from_parts(
            self.workspace_id.clone(),
            self.authority_id.clone(),
            self.authority_epoch,
            publication_revision,
            self.catalog_renderer_revision.clone(),
            alias_registry,
            plans,
            self.grants.clone(),
        )?
        .preserve_heads(self)
    }

    /// Produces the next closed request authority after ensuring one AgentConnection grant.
    pub fn next_with_access_grant(
        &self,
        publication_revision: GatewayPublicationRevision,
        access_grant: GatewayAccessGrantV1,
    ) -> Result<Self, PublicationError> {
        self.validate()?;
        let replacement = materialize_grants(vec![access_grant])?
            .pop()
            .ok_or(PublicationError::InvalidGrant)?;
        let mut grants = self
            .grants
            .iter()
            .filter(|current| current.grant_id != replacement.grant_id)
            .cloned()
            .collect::<Vec<_>>();
        grants.push(replacement);
        grants.sort_by(|left, right| left.grant_id.cmp(&right.grant_id));
        Self::from_parts(
            self.workspace_id.clone(),
            self.authority_id.clone(),
            self.authority_epoch,
            publication_revision,
            self.catalog_renderer_revision.clone(),
            self.alias_registry.clone(),
            self.plans.clone(),
            grants,
        )?
        .preserve_heads(self)
    }

    /// Produces the next closed publication after revoking one exact AgentConnection grant.
    /// Plans remain independently published; removing the last grant therefore yields a durable,
    /// deliberately non-executable plan-only publication.
    pub fn next_without_access_grant(
        &self,
        publication_revision: GatewayPublicationRevision,
        grant_id: &str,
    ) -> Result<Self, PublicationError> {
        self.validate()?;
        let grants = self
            .grants
            .iter()
            .filter(|current| current.grant_id != grant_id)
            .cloned()
            .collect::<Vec<_>>();
        if grants.len() == self.grants.len() {
            return Err(PublicationError::InvalidGrant);
        }
        Self::from_parts(
            self.workspace_id.clone(),
            self.authority_id.clone(),
            self.authority_epoch,
            publication_revision,
            self.catalog_renderer_revision.clone(),
            self.alias_registry.clone(),
            self.plans.clone(),
            grants,
        )?
        .preserve_heads(self)
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        workspace_id: WorkspaceId,
        authority_id: String,
        authority_epoch: u64,
        publication_revision: GatewayPublicationRevision,
        catalog_renderer_revision: String,
        alias_registry: AliasRegistryV1,
        plans: Vec<CompiledAgentPlanV1>,
        grants: Vec<GatewayExecutableGrantV2>,
    ) -> Result<Self, PublicationError> {
        let plans = plans
            .into_iter()
            .map(|plan| {
                plan.into_current()
                    .map_err(|_| PublicationError::InvalidPlan)
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Sealing upgrades pre-v2 Plans to the current compiler contract; every grant route
        // bound to such a Plan follows the upgraded digest so a sealed publication is always
        // internally consistent.
        let grants = grants
            .into_iter()
            .map(|grant| rebind_current_grant_routes(grant, &plans))
            .collect::<Result<Vec<_>, _>>()?;
        let aliases = materialize_aliases(&plans, &grants)?;
        let value = Self {
            schema: GATEWAY_PUBLICATION_SCHEMA_V3.to_owned(),
            compiler_revision: AGENT_PLAN_COMPILER_REVISION_V3.to_owned(),
            workspace_id,
            authority_id,
            authority_epoch,
            publication_revision,
            catalog_renderer_revision,
            alias_registry,
            plans,
            plan_heads: Vec::new(),
            aliases,
            grants,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), PublicationError> {
        if self.schema != LEGACY_GATEWAY_PUBLICATION_SCHEMA_V2
            && self.schema != GATEWAY_PUBLICATION_SCHEMA_V3
        {
            return Err(PublicationError::UnsupportedSchema);
        }
        let current_contract = self.schema == GATEWAY_PUBLICATION_SCHEMA_V3
            && self.compiler_revision == AGENT_PLAN_COMPILER_REVISION_V3
            && self
                .plans
                .iter()
                .all(|plan| plan.body.schema == AGENT_PLAN_COMPILED_SCHEMA_V3);
        // Preserve the separately registered unversioned publication recovery boundary.
        let persisted_compatibility = matches!(
            self.compiler_revision.as_str(),
            AGENT_PLAN_COMPILER_REVISION_V1
        ) && self.plans.iter().all(|plan| {
            matches!(
                plan.body.schema.as_str(),
                crate::AGENT_PLAN_COMPILED_SCHEMA_V1 | AGENT_PLAN_COMPILED_SCHEMA_V3
            )
        });
        if !current_contract && !persisted_compatibility {
            return Err(PublicationError::UnsupportedCompilerRevision);
        }
        WorkspaceId::parse(self.workspace_id.as_str())
            .map_err(|_| PublicationError::InvalidWorkspace)?;
        GatewayPublicationRevision::new(self.publication_revision.get())?;
        if !valid_reference(&self.authority_id)
            || self.authority_epoch == 0
            || !valid_reference(&self.catalog_renderer_revision)
        {
            return Err(PublicationError::InvalidAuthority);
        }
        self.alias_registry
            .validate()
            .map_err(|_| PublicationError::InvalidAliasRegistry)?;

        let mut plan_ids = BTreeSet::new();
        let mut aliases = BTreeSet::new();
        let mut previous = None;
        for plan in &self.plans {
            plan.validate().map_err(|_| PublicationError::InvalidPlan)?;
            if previous.is_some_and(|value| value >= plan.agent_plan_id())
                || !plan_ids.insert(plan.agent_plan_id())
                || !aliases.insert(plan.model_alias())
                || self.alias_registry.alias_for(plan.agent_plan_id()) != Some(plan.model_alias())
            {
                return Err(PublicationError::InvalidPlanSet);
            }
            previous = Some(plan.agent_plan_id());
        }
        if self.alias_registry.active.len() != self.plans.len()
            || self
                .alias_registry
                .active
                .keys()
                .any(|plan_id| !plan_ids.contains(plan_id))
        {
            return Err(PublicationError::PublicationNotClosed);
        }

        self.validate_plan_heads()?;
        validate_grants(&self.grants, &aliases)?;
        // Independent Plans may be saved before an Agent grants them. Only aliases reached by at
        // least one grant are executable; the Gateway projection remains closed over that subset.
        let enabled_grants = self.enabled_grants()?;
        let expected_aliases = materialize_aliases(&self.plans, &enabled_grants)?;
        if expected_aliases != self.aliases {
            // Publications written after request-owned routing but before the safe Plan display
            // name was frozen remain exact legacy records. The next Application-owned publish
            // deterministically upgrades every active alias.
            let mut legacy_named_aliases = expected_aliases.clone();
            for alias in &mut legacy_named_aliases {
                if let Some(routing) = &mut alias.routing {
                    routing.plan_display_name = None;
                }
            }
            if legacy_named_aliases == self.aliases {
                return Ok(());
            }
            // Publications emitted before request-owned routing became part of the executable
            // alias remain valid exact records. Accept only the complete legacy projection (all
            // routing fields absent and every other field identical); the next publication
            // deterministically upgrades it to the closed routing projection.
            let mut legacy_aliases = legacy_named_aliases;
            for alias in &mut legacy_aliases {
                alias.routing = None;
            }
            if self.aliases.iter().any(|alias| alias.routing.is_some())
                || legacy_aliases != self.aliases
            {
                return Err(PublicationError::InvalidExecutableProjection);
            }
        }
        Ok(())
    }

    pub fn validate_current_contract(&self) -> Result<(), PublicationError> {
        self.validate()?;
        self.check_current_schema()
    }

    fn check_current_schema(&self) -> Result<(), PublicationError> {
        if self.schema != GATEWAY_PUBLICATION_SCHEMA_V3
            || self.compiler_revision != AGENT_PLAN_COMPILER_REVISION_V3
            || self
                .plans
                .iter()
                .any(|plan| plan.body.schema != AGENT_PLAN_COMPILED_SCHEMA_V3)
        {
            return Err(PublicationError::UnsupportedSchema);
        }
        Ok(())
    }

    /// Authenticates a persisted publication and deterministically upgrades its complete
    /// Product-owned projection to the sole current contract before live use.
    pub fn into_current(mut self) -> Result<Self, PublicationError> {
        self.validate()?;
        // Every plan route binds the compiled plan it names in its persisted form; the plan
        // upgrade below re-derives the V2 route digest, so authenticate the persisted binding
        // first and rebind it afterwards instead of accepting a stale provenance digest.
        for grant in &self.grants {
            for route in grant.model_grant.routes.values() {
                if let crate::AgentModelRouteV2::Plan {
                    plan_id,
                    alias,
                    revision,
                    semantic_digest,
                } = route
                {
                    let plan = self
                        .plans
                        .iter()
                        .find(|plan| plan.agent_plan_id() == plan_id && plan.model_alias() == alias)
                        .ok_or(PublicationError::InvalidGrant)?;
                    if *revision > plan.body.agent_plan_revision
                        || (*revision == plan.body.agent_plan_revision
                            && semantic_digest != &plan.body.materialized_route_digest)
                    {
                        return Err(PublicationError::InvalidGrant);
                    }
                }
            }
        }
        if self.check_current_schema().is_ok() {
            // The complete aggregate and exact grant bindings were checked above. Current
            // Plans need no migration or resealing; only upgrade any persisted alias projection.
            self.aliases = materialize_aliases(&self.plans, &self.enabled_grants()?)?;
            return Ok(self);
        }
        self.schema = GATEWAY_PUBLICATION_SCHEMA_V3.to_owned();
        self.compiler_revision = AGENT_PLAN_COMPILER_REVISION_V3.to_owned();
        self.plans = self
            .plans
            .into_iter()
            .map(|plan| {
                plan.into_current()
                    .map_err(|_| PublicationError::InvalidPlan)
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.grants = self
            .grants
            .into_iter()
            .map(|grant| rebind_current_grant_routes(grant, &self.plans))
            .collect::<Result<Vec<_>, _>>()?;
        self.aliases = materialize_aliases(&self.plans, &self.enabled_grants()?)?;
        self.validate_current_contract()?;
        Ok(self)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PublicationError> {
        self.validate()?;
        serde_json::to_vec(&StoredPublicationV1::freeze(self)?)
            .map_err(|_| PublicationError::Encoding)
    }

    /// Validates publication, authority, grant-generation, and per-Plan revision monotonicity
    /// against the exact active aggregate.
    pub fn validate_transition_from(&self, active: &Self) -> Result<(), PublicationError> {
        self.validate()?;
        active.validate()?;
        self.validate_head_transition(active)?;
        if self.workspace_id != active.workspace_id
            || self.authority_id != active.authority_id
            || self.authority_epoch < active.authority_epoch
            || self.publication_revision.get() <= active.publication_revision.get()
            || self.alias_registry.next_sequence < active.alias_registry.next_sequence
            || !self
                .alias_registry
                .tombstones
                .is_superset(&active.alias_registry.tombstones)
            || !self
                .alias_registry
                .retired_plan_ids
                .is_superset(&active.alias_registry.retired_plan_ids)
        {
            return Err(PublicationError::InvalidTransition);
        }
        for (plan_id, alias) in &active.alias_registry.active {
            match self.alias_registry.active.get(plan_id) {
                Some(current) if current == alias => {}
                Some(_) => return Err(PublicationError::AliasLifecycleConflict),
                None if self.alias_registry.tombstones.contains(alias)
                    && self.alias_registry.retired_plan_ids.contains(plan_id) => {}
                None => return Err(PublicationError::AliasLifecycleConflict),
            }
        }
        for plan in &self.plans {
            if let Some(previous) = active
                .plans
                .iter()
                .find(|previous| previous.agent_plan_id() == plan.agent_plan_id())
            {
                if plan.body.agent_plan_revision < previous.body.agent_plan_revision {
                    return Err(PublicationError::InvalidTransition);
                }
                if plan.body.agent_plan_revision == previous.body.agent_plan_revision
                    && crate::StoredPlanV1::freeze(plan).ok()
                        != previous
                            .clone()
                            .into_current()
                            .ok()
                            .and_then(|p| crate::StoredPlanV1::freeze(&p).ok())
                {
                    return Err(PublicationError::ImmutablePlanRevisionConflict);
                }
            }
        }
        // A same-generation Grant may differ from its persisted form only by the deterministic
        // current-contract upgrade of the Plan routes it binds; any other change requires a new
        // generation.
        let mut active_current: Option<Option<Self>> = None;
        for grant in &self.grants {
            let Some(previous) = active
                .grants
                .iter()
                .find(|previous| previous.grant_id == grant.grant_id)
            else {
                continue;
            };
            if grant.generation < previous.generation {
                return Err(PublicationError::ImmutableGrantGenerationConflict);
            }
            if grant.generation == previous.generation && grant != previous {
                let current = match active_current
                    .get_or_insert_with(|| active.clone().into_current().ok())
                {
                    Some(publication) => publication
                        .grants
                        .iter()
                        .find(|candidate| candidate.grant_id == grant.grant_id),
                    None => None,
                };
                if !current.is_some_and(|candidate| candidate == grant) {
                    return Err(PublicationError::ImmutableGrantGenerationConflict);
                }
            }
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<CanonicalDigest, PublicationError> {
        Ok(CanonicalDigest::of_bytes(&self.canonical_bytes()?))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PublicationError> {
        let stored: StoredPublicationV1 =
            serde_json::from_slice(bytes).map_err(|_| PublicationError::Decoding)?;
        let publication = stored.build()?;
        if publication.canonical_bytes()?.as_slice() != bytes {
            return Err(PublicationError::NonCanonicalEncoding);
        }
        Ok(publication)
    }

    /// Recovery reader for a persisted publication aggregate. Current bodies use the strict
    /// decoder; persisted legacy aggregates keep their name-set grants, which are
    /// authenticated here and re-derived into the sole current model-grant route mapping
    /// before any live use. New writes never emit the legacy shape.
    pub fn decode_persisted(bytes: &[u8]) -> Result<Self, PublicationError> {
        if let Ok(current) = Self::decode(bytes) {
            return Ok(current);
        }
        if let Ok(persisted) = serde_json::from_slice::<Self>(bytes) {
            persisted.validate()?;
            if serde_json::to_vec(&persisted)
                .map_err(|_| PublicationError::Encoding)?
                .as_slice()
                != bytes
            {
                return Err(PublicationError::NonCanonicalEncoding);
            }
            return Ok(persisted);
        }
        let legacy: LegacyGatewayPublicationV2 =
            serde_json::from_slice(bytes).map_err(|_| PublicationError::Decoding)?;
        let plans = legacy.plans;
        let grants = legacy
            .grants
            .into_iter()
            .map(|grant| recover_legacy_grant(grant, &plans))
            .collect::<Result<Vec<_>, PublicationError>>()?;
        let publication = Self {
            schema: legacy.schema,
            compiler_revision: legacy.compiler_revision,
            workspace_id: legacy.workspace_id,
            authority_id: legacy.authority_id,
            authority_epoch: legacy.authority_epoch,
            publication_revision: legacy.publication_revision,
            catalog_renderer_revision: legacy.catalog_renderer_revision,
            alias_registry: legacy.alias_registry,
            plans,
            plan_heads: legacy.plan_heads,
            aliases: legacy.aliases,
            grants,
        };
        publication.validate()?;
        Ok(publication)
    }

    /// Produces the frozen Gateway DTO solely from the verified product publication.
    pub fn gateway_snapshot(
        &self,
    ) -> Result<GatewayPublicationSnapshotProjectionV3, PublicationError> {
        let current = self.clone().into_current()?;
        let enabled_grants = current.enabled_grants()?;
        if enabled_grants.is_empty() {
            return GatewayPublicationSnapshotProjectionV3::no_new_calls(
                current.workspace_id.as_str().into(),
                current.authority_id,
                current.authority_epoch,
                current.publication_revision.get(),
                current.catalog_renderer_revision,
            );
        }
        let mut aliases = current.aliases;
        snapshot::project_pricing_identities(&current.plans, &mut aliases)?;
        let enabled_grants = snapshot::project_grants(enabled_grants, &aliases, &current.plans)?;
        GatewayPublicationSnapshotProjectionV3::seal(
            current.workspace_id.as_str().to_owned(),
            current.authority_id,
            current.authority_epoch,
            current.publication_revision.get(),
            current.catalog_renderer_revision,
            aliases,
            enabled_grants,
        )
    }

    /// Projects the independent, durable AgentPlan catalog without requiring an
    /// AgentConnection grant. A protocol is advertised only when every ordered candidate has
    /// one exact renderer for that ingress; grant material never participates in this catalog.
    pub fn published_agent_plans(&self) -> Result<Vec<PublishedAgentPlanV1>, PublicationError> {
        self.validate()?;
        self.plans
            .iter()
            .filter_map(|plan| {
                let candidates = match ordered_candidates(plan) {
                    Ok(candidates) => candidates,
                    Err(error) => return Some(Err(error)),
                };
                let supported_ingress = [
                    (
                        AgentIngressProtocolV1::Responses,
                        UpstreamProtocol::Responses,
                    ),
                    (AgentIngressProtocolV1::Messages, UpstreamProtocol::Messages),
                ]
                .into_iter()
                .filter(|(_, ingress)| {
                    candidates.iter().all(|candidate| {
                        candidate
                            .protocol_profiles
                            .iter()
                            .filter(|profile| profile.ingress_protocol == *ingress)
                            .count()
                            == 1
                    })
                })
                .map(|(protocol, _)| protocol)
                .collect::<BTreeSet<_>>();
                if supported_ingress.is_empty() {
                    return None;
                }
                Some(Ok(PublishedAgentPlanV1 {
                    agent_plan_id: plan.body.identity.agent_plan_id.clone(),
                    model_alias: plan.body.identity.model_alias.clone(),
                    display_name: plan.body.identity.display_name.clone(),
                    purpose: plan.body.identity.purpose.clone(),
                    agent_plan_revision: plan.body.agent_plan_revision,
                    active: self
                        .plan_heads
                        .iter()
                        .find(|h| h.reference.plan_id == *plan.agent_plan_id())
                        .is_none_or(|h| h.status == crate::PlanLifecycleV1::Enabled),
                    supported_ingress,
                }))
            })
            .collect()
    }
}

/// Recovery-only mirror of a persisted legacy `hiroute.gateway-publication/v2` aggregate.
/// The V2 writer sealed exactly one ingress protocol and a set of published aliases per
/// grant; recovery re-derives the current route mapping from those facts and never
/// re-persists the legacy shape.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyGatewayPublicationV2 {
    schema: String,
    compiler_revision: String,
    workspace_id: WorkspaceId,
    authority_id: String,
    authority_epoch: u64,
    publication_revision: GatewayPublicationRevision,
    catalog_renderer_revision: String,
    alias_registry: AliasRegistryV1,
    plans: Vec<CompiledAgentPlanV1>,
    #[serde(default)]
    plan_heads: Vec<crate::PlanHeadV1>,
    aliases: Vec<GatewayExecutableAliasV2>,
    grants: Vec<LegacyGatewayExecutableGrantV2>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyGatewayExecutableGrantV2 {
    grant_id: String,
    generation: u64,
    bearer_token_sha256: CanonicalDigest,
    allowed_protocols: Vec<AgentIngressProtocolV1>,
    allowed_aliases: Vec<ModelAlias>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreparePublicationOutcome {
    Created,
    ExistingSame,
}

/// Durable state: prepare is transactional and activation atomically retains the prior LKG.
pub trait PublicationRepositoryPort {
    fn prepare_publication(
        &self,
        record: &PublicationRecordV1,
        expected_active_revision: Option<GatewayPublicationRevision>,
    ) -> PortResult<PreparePublicationOutcome>;

    fn prepared_publication(
        &self,
        workspace: &WorkspaceId,
    ) -> PortResult<Option<PublicationRecordV1>>;

    fn active_publication(
        &self,
        workspace: &WorkspaceId,
    ) -> PortResult<Option<PublicationRecordV1>>;

    fn last_known_good_publication(
        &self,
        workspace: &WorkspaceId,
    ) -> PortResult<Option<PublicationRecordV1>>;

    fn mark_publication_active(
        &self,
        workspace: &WorkspaceId,
        publication_revision: GatewayPublicationRevision,
        digest: &CanonicalDigest,
    ) -> PortResult<()>;
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PublicationError {
    #[error("Gateway publication schema is unsupported")]
    UnsupportedSchema,
    #[error("AgentPlan compiler revision is unsupported")]
    UnsupportedCompilerRevision,
    #[error("Gateway publication revision must be non-zero")]
    InvalidRevision,
    #[error("Gateway publication workspace is invalid or does not match its record")]
    InvalidWorkspace,
    #[error("Gateway authority or catalog renderer identity is invalid")]
    InvalidAuthority,
    #[error("Gateway publication alias registry is invalid")]
    InvalidAliasRegistry,
    #[error("Gateway publication contains an invalid compiled AgentPlan")]
    InvalidPlan,
    #[error("Gateway publication AgentPlans are duplicated, unsorted, or alias-inconsistent")]
    InvalidPlanSet,
    #[error("Gateway publication grant is invalid or does not close over published aliases")]
    InvalidGrant,
    #[error("Gateway publication does not close every active alias")]
    PublicationNotClosed,
    #[error("Gateway publication executable alias projection does not match compiled Plans")]
    InvalidExecutableProjection,
    #[error("Gateway publication execution limits exceed the frozen G0 contract")]
    InvalidExecutionLimits,
    #[error("Gateway publication executable candidate IDs collide")]
    CandidateIdCollision,
    #[error("empty publication cannot be projected as executable Gateway authority")]
    PublicationNotExecutable,
    #[error("Gateway publication canonical encoding failed")]
    Encoding,
    #[error("Gateway publication could not be decoded")]
    Decoding,
    #[error("Gateway publication bytes are not the canonical encoding")]
    NonCanonicalEncoding,
    #[error("Gateway publication digest does not match its bytes")]
    DigestMismatch,
    #[error("Gateway publication transition is stale or crosses authorities/workspaces")]
    InvalidTransition,
    #[error("an immutable AgentPlan revision was reused for different compiled bytes")]
    ImmutablePlanRevisionConflict,
    #[error("an immutable grant generation was reused for different verifier or scope")]
    ImmutableGrantGenerationConflict,
    #[error("an active alias was renamed, removed without a tombstone, or lifecycle state rewound")]
    AliasLifecycleConflict,
}

#[cfg(test)]
mod routing_publication_tests;
