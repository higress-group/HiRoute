use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    AgentPlanId, AgentPlanIdentityV1, BillingClass, CanonicalDigest, ConnectorRuntimeKind,
    ExactNativeReasoningV1, FreeAccess, GatewayAuthenticationSemanticsV1, GatewayCriticalFactV1,
    ModelAlias, UpstreamProtocol,
};

use super::gateway_execution::{GatewayCandidateProtocolProfileV1, GatewayOperationalTargetV1};
use super::{
    AGENT_PLAN_COMPILED_SCHEMA_V1, AGENT_PLAN_COMPILED_SCHEMA_V2, AGENT_PLAN_COMPILER_REVISION_V1,
    AGENT_PLAN_COMPILER_REVISION_V2, ComplexityClassifierV1, RoutingLimitsV1,
};

const LEGACY_MATERIALIZED_ROUTE_DIGEST_SCHEMA_V1: &str = "hiroute.materialized-route-digest/v1";
pub const MATERIALIZED_ROUTE_DIGEST_SCHEMA_V2: &str = "hiroute.materialized-route-digest/v2";

#[cfg(test)]
thread_local! {
    pub(crate) static CANDIDATE_VALIDATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterializedGroupId {
    Economy,
    Primary,
    Free,
    Custom,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "strategy", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequestOwnedRouteV1 {
    Classified {
        classifier: ComplexityClassifierV1,
        simple_groups: Vec<MaterializedGroupId>,
        complex_groups: Vec<MaterializedGroupId>,
    },
    Ordered {
        cost_policy: MaterializedCostPolicyV1,
        ordered_groups: Vec<MaterializedGroupId>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterializedCostPolicyV1 {
    ApiEquivalent,
    StrictFree,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaterializedOrderingV1 {
    QualityFirst,
    CheapestWithRatingGuard {
        quality_anchor_binding_id: String,
        maximum_score_gap_tenths: u8,
    },
    FreeScoreDescending,
    ExplicitOrder,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedModelGroupV1 {
    pub group_id: MaterializedGroupId,
    /// Evidence for how Application materialized this array. Runtime must not execute it as a
    /// sorting policy; `candidates` is already the final immutable order.
    pub ordering_evidence: MaterializedOrderingV1,
    /// Exact pinned ratings used for quality provenance and guard validation. Runtime consumes
    /// `candidates` as-is; it never folds these facts into a new score or re-sorts the array.
    #[serde(default)]
    pub pinned_ratings: BTreeMap<String, MaterializedRatingFactV1>,
    pub candidates: Vec<AttemptOwnedCandidateV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedRatingFactV1 {
    pub model_configuration_id: String,
    pub overall_score_tenths: u8,
    pub rating_count: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedFreeOfferRefV1 {
    pub free_offer_id: String,
    pub free_offer_revision: u64,
    pub offer_ref: String,
    pub access: FreeAccess,
    pub evidence_digest: CanonicalDigest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptOwnedCandidateV1 {
    pub binding_id: String,
    pub binding_revision: u64,
    pub binding_digest: CanonicalDigest,
    pub source_id: String,
    pub source_revision: u64,
    pub source_identity_digest: CanonicalDigest,
    pub connection_option_id: String,
    pub offer_ref: String,
    pub offer_revision: u64,
    pub offer_evidence_digest: CanonicalDigest,
    pub billing_class: BillingClass,
    pub model_configuration_id: String,
    pub model_configuration_revision: u64,
    pub upstream_model_id: String,
    /// Exact model identifier written to the operational connector request.
    /// Managed CPA targets may use an account-scoped alias that intentionally
    /// differs from the catalog-bound logical `upstream_model_id`.
    pub native_transport_model: String,
    pub capability_id: String,
    pub capability_revision: u64,
    pub capability_evidence_digest: CanonicalDigest,
    pub connector_id: String,
    pub connector_revision: u64,
    pub endpoint_profile_id: String,
    pub endpoint_profile_revision: u64,
    pub protocol_endpoint_id: String,
    /// Exact registered destination. Runtime may resolve DNS only after grant and alias checks;
    /// it must never reconstruct this value from an EndpointProfile or another store.
    pub endpoint: String,
    pub connector_runtime: ConnectorRuntimeKind,
    pub operational_target: GatewayOperationalTargetV1,
    pub operational_target_digest: CanonicalDigest,
    pub protocol_profiles: Vec<GatewayCandidateProtocolProfileV1>,
    pub protocol_profile_digest: CanonicalDigest,
    pub upstream_protocol: UpstreamProtocol,
    pub adapter_ref: String,
    pub adapter_revision: u64,
    /// Frozen order of stable, non-secret selectors understood by CredentialResolver. Credential
    /// material and store locators are intentionally absent from the publication.
    pub credential_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_destination_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_pool_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_offer: Option<MaterializedFreeOfferRefV1>,
    pub exact_reasoning: ExactNativeReasoningV1,
}

impl AttemptOwnedCandidateV1 {
    pub fn validate(&self) -> Result<(), CompiledPlanError> {
        #[cfg(test)]
        CANDIDATE_VALIDATIONS.with(|count| count.set(count.get() + 1));
        for value in [
            &self.binding_id,
            &self.source_id,
            &self.connection_option_id,
            &self.offer_ref,
            &self.model_configuration_id,
            &self.upstream_model_id,
            &self.native_transport_model,
            &self.capability_id,
            &self.connector_id,
            &self.endpoint_profile_id,
            &self.protocol_endpoint_id,
            &self.adapter_ref,
        ] {
            if !super::strategy::valid_reference(value) {
                return Err(CompiledPlanError::InvalidCandidate);
            }
        }
        if self.binding_revision == 0
            || self.source_revision == 0
            || self.offer_revision == 0
            || self.model_configuration_revision == 0
            || self.capability_revision == 0
            || self.connector_revision == 0
            || self.endpoint_profile_revision == 0
            || self.adapter_revision == 0
            || self.billing_class == BillingClass::Unknown
            || !self
                .operational_target
                .validate_for(self.connector_runtime, &self.endpoint)
            || !matches!(
                CanonicalDigest::of(&self.operational_target),
                Ok(digest) if digest == self.operational_target_digest
            )
            || !matches!(
                CanonicalDigest::of(&self.protocol_profiles),
                Ok(digest) if digest == self.protocol_profile_digest
            )
            || self.protocol_profiles.is_empty()
            || (self.connector_runtime == ConnectorRuntimeKind::BuiltinNative
                && self.native_transport_model != self.upstream_model_id)
            || invalid_digest(&self.binding_digest)
            || invalid_digest(&self.source_identity_digest)
            || invalid_digest(&self.offer_evidence_digest)
            || invalid_digest(&self.capability_evidence_digest)
            || self.credential_refs.is_empty()
            || self.credential_refs.len() > 64
            || self
                .credential_refs
                .iter()
                .any(|value| !super::strategy::valid_reference(value))
            || self.credential_refs.iter().collect::<BTreeSet<_>>().len()
                != self.credential_refs.len()
            || self
                .credential_destination_ref
                .as_deref()
                .is_some_and(|value| {
                    !value.starts_with("compute-target/")
                        || !super::strategy::valid_reference(value)
                })
            || self
                .credential_pool_id
                .as_deref()
                .is_some_and(|value| !super::strategy::valid_reference(value))
        {
            return Err(CompiledPlanError::InvalidCandidate);
        }
        let mut ingress_protocols = BTreeSet::new();
        for profile in &self.protocol_profiles {
            if !ingress_protocols.insert(profile.ingress_protocol) {
                return Err(CompiledPlanError::InvalidCandidate);
            }
            profile.validate_for_candidate(self)?;
        }
        let requires_credential = self.protocol_profiles.iter().any(|profile| {
            !matches!(
                profile.connector.authentication,
                GatewayCriticalFactV1::Exact(GatewayAuthenticationSemanticsV1::None)
            )
        });
        if requires_credential
            && self.billing_class != BillingClass::Free
            && self.credential_pool_id.is_none()
        {
            return Err(CompiledPlanError::InvalidCandidate);
        }
        match (&self.free_offer, self.billing_class) {
            (Some(free), BillingClass::Free) => {
                if !super::strategy::valid_reference(&free.free_offer_id)
                    || free.free_offer_revision == 0
                    || free.offer_ref != self.offer_ref
                    || invalid_digest(&free.evidence_digest)
                    || (free.access == FreeAccess::ApiKeyRequired
                        && self.credential_pool_id.is_none())
                {
                    return Err(CompiledPlanError::InvalidCandidate);
                }
            }
            (None, BillingClass::Free) | (Some(_), _) => {
                return Err(CompiledPlanError::InvalidCandidate);
            }
            (None, _) => {}
        }
        self.exact_reasoning
            .validate()
            .map_err(|_| CompiledPlanError::InvalidReasoning)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptOwnedRouteV1 {
    pub limits: RoutingLimitsV1,
    pub groups: Vec<MaterializedModelGroupV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceTrackingMode {
    FollowLatest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanFactRefsV1 {
    pub connector_registry_version: String,
    pub connector_registry_digest: CanonicalDigest,
    pub model_data_bundle_version: String,
    pub model_data_digest: CanonicalDigest,
    pub capability_slice_version: String,
    pub capability_slice_digest: CanonicalDigest,
    pub ratings_slice_version: String,
    pub ratings_slice_digest: CanonicalDigest,
    pub free_offers_slice_version: String,
    pub free_offers_slice_digest: CanonicalDigest,
    pub inventory_revision: u64,
    pub inventory_digest: CanonicalDigest,
    pub price_tracking: PriceTrackingMode,
}

impl AgentPlanFactRefsV1 {
    pub fn validate(&self) -> Result<(), CompiledPlanError> {
        for value in [
            &self.connector_registry_version,
            &self.model_data_bundle_version,
            &self.capability_slice_version,
            &self.ratings_slice_version,
            &self.free_offers_slice_version,
        ] {
            if !super::strategy::valid_reference(value) {
                return Err(CompiledPlanError::InvalidFactReference);
            }
        }
        if self.inventory_revision == 0
            || [
                &self.connector_registry_digest,
                &self.model_data_digest,
                &self.capability_slice_digest,
                &self.ratings_slice_digest,
                &self.free_offers_slice_digest,
                &self.inventory_digest,
            ]
            .into_iter()
            .any(invalid_digest)
        {
            Err(CompiledPlanError::InvalidFactReference)
        } else {
            Ok(())
        }
    }

    /// Price-only changes are consumed by the next Turn's value ledger. They do not mutate an
    /// ACTIVE AgentPlan or independently request a replan.
    pub fn replan_reasons(&self, newer: &Self) -> Vec<ReplanReason> {
        let mut reasons = BTreeSet::new();
        if self.connector_registry_digest != newer.connector_registry_digest {
            reasons.insert(ReplanReason::ConnectorRegistryChanged);
        }
        if self.model_data_digest != newer.model_data_digest {
            reasons.insert(ReplanReason::ModelDataChanged);
        }
        if self.capability_slice_digest != newer.capability_slice_digest {
            reasons.insert(ReplanReason::CapabilityChanged);
        }
        if self.ratings_slice_digest != newer.ratings_slice_digest {
            reasons.insert(ReplanReason::RatingChanged);
        }
        if self.free_offers_slice_digest != newer.free_offers_slice_digest {
            reasons.insert(ReplanReason::FreeOfferChanged);
        }
        if self.inventory_digest != newer.inventory_digest {
            reasons.insert(ReplanReason::InventoryChanged);
        }
        reasons.into_iter().collect()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplanReason {
    ConnectorRegistryChanged,
    ModelDataChanged,
    CapabilityChanged,
    RatingChanged,
    FreeOfferChanged,
    InventoryChanged,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedAgentPlanV1 {
    pub fact_refs: AgentPlanFactRefsV1,
    pub request_owned: RequestOwnedRouteV1,
    pub attempt_owned: AttemptOwnedRouteV1,
}

impl MaterializedAgentPlanV1 {
    pub fn validate(&self) -> Result<(), CompiledPlanError> {
        self.fact_refs.validate()?;
        self.attempt_owned
            .limits
            .validate()
            .map_err(|_| CompiledPlanError::InvalidLimits)?;
        if self.attempt_owned.groups.is_empty() || self.attempt_owned.groups.len() > 4 {
            return Err(CompiledPlanError::InvalidGroups);
        }
        let mut groups = BTreeMap::new();
        for group in &self.attempt_owned.groups {
            if group.candidates.is_empty()
                || group.candidates.len() > 128
                || groups.insert(group.group_id, group).is_some()
            {
                return Err(CompiledPlanError::InvalidGroups);
            }
            let mut bindings = BTreeSet::new();
            for candidate in &group.candidates {
                candidate.validate()?;
                if !bindings.insert(&candidate.binding_id) {
                    return Err(CompiledPlanError::DuplicateCandidate);
                }
            }
            super::ordering::validate_ordering_facts(group)?;
        }
        if self.attempt_owned.limits.context_window_tokens.is_some() {
            self.context_window_tokens()?;
        }
        validate_request_groups(&self.request_owned, &groups)
    }

    /// V2 freezes only selected executable inputs. Advisory catalog snapshots remain audit
    /// provenance, so a rating/price/new-offer refresh cannot silently change a saved route.
    pub fn route_digest(&self) -> Result<CanonicalDigest, CompiledPlanError> {
        self.validate()?;
        if self.attempt_owned.groups.iter().any(|group| {
            group.ordering_evidence != MaterializedOrderingV1::ExplicitOrder
                || !group.pinned_ratings.is_empty()
        }) {
            return Err(CompiledPlanError::InvalidOrderingFacts);
        }
        CanonicalDigest::of(&(
            MATERIALIZED_ROUTE_DIGEST_SCHEMA_V2,
            &self.request_owned,
            &self.attempt_owned,
        ))
        .map_err(|_| CompiledPlanError::Encoding)
    }

    /// Digests executable Plan semantics. Display metadata is excluded; the already-materialized
    /// candidate order and pinned rating/capability/free facts remain part of the digest. Exact
    /// price inputs stay compiler-only because runtime price tracking is follow-latest.
    pub fn legacy_route_digest(&self) -> Result<CanonicalDigest, CompiledPlanError> {
        self.validate()?;
        CanonicalDigest::of(&MaterializedRouteDigestInputV1 {
            schema: LEGACY_MATERIALIZED_ROUTE_DIGEST_SCHEMA_V1,
            connector_registry_version: &self.fact_refs.connector_registry_version,
            connector_registry_digest: &self.fact_refs.connector_registry_digest,
            model_data_bundle_version: &self.fact_refs.model_data_bundle_version,
            model_data_digest: &self.fact_refs.model_data_digest,
            capability_slice_version: &self.fact_refs.capability_slice_version,
            capability_slice_digest: &self.fact_refs.capability_slice_digest,
            ratings_slice_version: &self.fact_refs.ratings_slice_version,
            ratings_slice_digest: &self.fact_refs.ratings_slice_digest,
            free_offers_slice_version: &self.fact_refs.free_offers_slice_version,
            free_offers_slice_digest: &self.fact_refs.free_offers_slice_digest,
            inventory_revision: self.fact_refs.inventory_revision,
            inventory_digest: &self.fact_refs.inventory_digest,
            price_tracking: &self.fact_refs.price_tracking,
            request_owned: &self.request_owned,
            attempt_owned: &self.attempt_owned,
        })
        .map_err(|_| CompiledPlanError::Encoding)
    }
}

#[derive(Serialize)]
struct MaterializedRouteDigestInputV1<'a> {
    schema: &'static str,
    connector_registry_version: &'a str,
    connector_registry_digest: &'a CanonicalDigest,
    model_data_bundle_version: &'a str,
    model_data_digest: &'a CanonicalDigest,
    capability_slice_version: &'a str,
    capability_slice_digest: &'a CanonicalDigest,
    ratings_slice_version: &'a str,
    ratings_slice_digest: &'a CanonicalDigest,
    free_offers_slice_version: &'a str,
    free_offers_slice_digest: &'a CanonicalDigest,
    inventory_revision: u64,
    inventory_digest: &'a CanonicalDigest,
    price_tracking: &'a PriceTrackingMode,
    request_owned: &'a RequestOwnedRouteV1,
    attempt_owned: &'a AttemptOwnedRouteV1,
}

fn validate_request_groups(
    request: &RequestOwnedRouteV1,
    groups: &BTreeMap<MaterializedGroupId, &MaterializedModelGroupV1>,
) -> Result<(), CompiledPlanError> {
    let exact = |ids: &[MaterializedGroupId]| {
        ids.iter().all(|id| groups.contains_key(id))
            && ids.iter().copied().collect::<BTreeSet<_>>().len() == ids.len()
    };
    match request {
        RequestOwnedRouteV1::Classified {
            classifier,
            simple_groups,
            complex_groups,
        } => {
            classifier
                .validate()
                .map_err(|_| CompiledPlanError::InvalidClassifier)?;
            if (simple_groups.as_slice() != [MaterializedGroupId::Economy]
                && simple_groups.as_slice()
                    != [MaterializedGroupId::Economy, MaterializedGroupId::Primary])
                || complex_groups.as_slice() != [MaterializedGroupId::Primary]
                || groups.keys().copied().collect::<BTreeSet<_>>()
                    != BTreeSet::from([MaterializedGroupId::Economy, MaterializedGroupId::Primary])
            {
                return Err(CompiledPlanError::InvalidGroups);
            }
            Ok(())
        }
        RequestOwnedRouteV1::Ordered {
            cost_policy,
            ordered_groups,
        } => {
            if !exact(ordered_groups) || groups.len() != ordered_groups.len() {
                return Err(CompiledPlanError::InvalidGroups);
            }
            match (cost_policy, ordered_groups.as_slice()) {
                (MaterializedCostPolicyV1::StrictFree, [MaterializedGroupId::Free])
                | (
                    MaterializedCostPolicyV1::ApiEquivalent,
                    [MaterializedGroupId::Free, MaterializedGroupId::Primary],
                ) => {
                    if groups[&MaterializedGroupId::Free]
                        .candidates
                        .iter()
                        .any(|candidate| candidate.billing_class != BillingClass::Free)
                    {
                        Err(CompiledPlanError::StrictFreeViolation)
                    } else {
                        Ok(())
                    }
                }
                (MaterializedCostPolicyV1::ApiEquivalent, [MaterializedGroupId::Custom]) => Ok(()),
                _ => Err(CompiledPlanError::InvalidGroups),
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledAgentPlanBodyV1 {
    pub schema: String,
    pub compiler_revision: String,
    pub identity: AgentPlanIdentityV1,
    pub agent_plan_revision: u64,
    pub materialized_route_digest: CanonicalDigest,
    pub materialized: MaterializedAgentPlanV1,
}

/// A sealed body is shared immutably. Copy-on-write edits detach from the proof and must
/// pass full validation again; deserialization never imports an in-memory proof.
#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledAgentPlanV1 {
    #[serde(serialize_with = "crate::operation::shared_input::serialize")]
    pub body: Arc<CompiledAgentPlanBodyV1>,
    pub digest: CanonicalDigest,
    #[serde(skip)]
    verified_body: Arc<CompiledAgentPlanBodyV1>,
    #[serde(skip)]
    verified_digest: CanonicalDigest,
}

impl PartialEq for CompiledAgentPlanV1 {
    fn eq(&self, other: &Self) -> bool {
        self.body == other.body && self.digest == other.digest
    }
}
impl Eq for CompiledAgentPlanV1 {}

impl<'de> Deserialize<'de> for CompiledAgentPlanV1 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Encoded {
            body: CompiledAgentPlanBodyV1,
            digest: CanonicalDigest,
        }
        let encoded = Encoded::deserialize(deserializer)?;
        Self::authenticate_persisted(encoded.body, encoded.digest).map_err(serde::de::Error::custom)
    }
}

impl CompiledAgentPlanV1 {
    /// Seals the sole live compiled-plan contract. Historical bodies must enter through the
    /// authenticated persisted-record reader and are converted before any new publication write.
    pub fn seal_current(body: CompiledAgentPlanBodyV1) -> Result<Self, CompiledPlanError> {
        if body.schema != AGENT_PLAN_COMPILED_SCHEMA_V2 {
            return Err(CompiledPlanError::UnsupportedSchema);
        }
        Self::seal_compatible(body)
    }

    /// Authenticates an already-persisted record without minting a replacement legacy digest.
    pub fn authenticate_persisted(
        body: CompiledAgentPlanBodyV1,
        digest: CanonicalDigest,
    ) -> Result<Self, CompiledPlanError> {
        validate_body(&body)?;
        if CanonicalDigest::of(&body).map_err(|_| CompiledPlanError::Encoding)? != digest {
            return Err(CompiledPlanError::DigestMismatch);
        }
        Ok(Self::from_validated(body, digest))
    }

    fn seal_compatible(body: CompiledAgentPlanBodyV1) -> Result<Self, CompiledPlanError> {
        validate_body(&body)?;
        let digest = CanonicalDigest::of(&body).map_err(|_| CompiledPlanError::Encoding)?;
        Ok(Self::from_validated(body, digest))
    }

    fn from_validated(body: CompiledAgentPlanBodyV1, digest: CanonicalDigest) -> Self {
        let body = Arc::new(body);
        Self {
            verified_body: body.clone(),
            verified_digest: digest.clone(),
            body,
            digest,
        }
    }

    pub fn validate(&self) -> Result<(), CompiledPlanError> {
        if Arc::ptr_eq(&self.body, &self.verified_body) && self.digest == self.verified_digest {
            return Ok(());
        }
        validate_body(&self.body)?;
        let digest =
            CanonicalDigest::of(self.body.as_ref()).map_err(|_| CompiledPlanError::Encoding)?;
        if digest != self.digest {
            return Err(CompiledPlanError::DigestMismatch);
        }
        Ok(())
    }

    /// Converts an authenticated pre-v2 compiled plan without changing its executable arrays.
    /// The old ordering evidence is audit provenance; v2 records the already-frozen array as the
    /// explicit order and computes the one current route digest.
    pub fn into_current(mut self) -> Result<Self, CompiledPlanError> {
        self.validate()?;
        if self.body.schema == AGENT_PLAN_COMPILED_SCHEMA_V2 {
            return Ok(self);
        }
        let body = Arc::make_mut(&mut self.body);
        for group in &mut body.materialized.attempt_owned.groups {
            group.ordering_evidence = MaterializedOrderingV1::ExplicitOrder;
            group.pinned_ratings.clear();
        }
        body.schema = AGENT_PLAN_COMPILED_SCHEMA_V2.to_owned();
        body.compiler_revision = AGENT_PLAN_COMPILER_REVISION_V2.to_owned();
        body.materialized_route_digest = body.materialized.route_digest()?;
        Self::seal_current((*self.body).clone())
    }

    pub fn agent_plan_id(&self) -> &AgentPlanId {
        &self.body.identity.agent_plan_id
    }

    pub fn model_alias(&self) -> &ModelAlias {
        &self.body.identity.model_alias
    }
}

fn validate_body(body: &CompiledAgentPlanBodyV1) -> Result<(), CompiledPlanError> {
    let expected_revision = match body.schema.as_str() {
        AGENT_PLAN_COMPILED_SCHEMA_V1 => AGENT_PLAN_COMPILER_REVISION_V1,
        AGENT_PLAN_COMPILED_SCHEMA_V2 => AGENT_PLAN_COMPILER_REVISION_V2,
        _ => return Err(CompiledPlanError::UnsupportedSchema),
    };
    if body.compiler_revision != expected_revision {
        return Err(CompiledPlanError::UnsupportedCompilerRevision);
    }
    body.identity
        .validate()
        .map_err(|_| CompiledPlanError::InvalidIdentity)?;
    if body.agent_plan_revision == 0 {
        return Err(CompiledPlanError::InvalidIdentity);
    }
    // Both digest methods fully validate the borrowed materialized route.
    let route_digest = if body.schema == AGENT_PLAN_COMPILED_SCHEMA_V2 {
        body.materialized.route_digest()?
    } else {
        body.materialized.legacy_route_digest()?
    };
    if route_digest != body.materialized_route_digest {
        return Err(CompiledPlanError::MaterializedRouteDigestMismatch);
    }
    Ok(())
}

fn invalid_digest(value: &CanonicalDigest) -> bool {
    value == &CanonicalDigest::of_bytes(&[])
        || !matches!(CanonicalDigest::parse(value.as_str()), Ok(parsed) if &parsed == value)
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CompiledPlanError {
    #[error("compiled AgentPlan schema is unsupported")]
    UnsupportedSchema,
    #[error("compiled AgentPlan compiler revision is unsupported")]
    UnsupportedCompilerRevision,
    #[error("compiled AgentPlan identity or revision is invalid")]
    InvalidIdentity,
    #[error("compiled AgentPlan fact reference is invalid")]
    InvalidFactReference,
    #[error("compiled AgentPlan request groups are not closed")]
    InvalidGroups,
    #[error("compiled AgentPlan contains a duplicate candidate")]
    DuplicateCandidate,
    #[error("compiled AgentPlan candidate is not closed")]
    InvalidCandidate,
    #[error("compiled AgentPlan exact reasoning is invalid")]
    InvalidReasoning,
    #[error("compiled AgentPlan ordering facts do not justify the materialized order")]
    InvalidOrderingFacts,
    #[error("compiled AgentPlan limits are invalid")]
    InvalidLimits,
    #[error("compiled AgentPlan classifier is invalid")]
    InvalidClassifier,
    #[error("strict-free compiled route contains a non-free attempt")]
    StrictFreeViolation,
    #[error("compiled AgentPlan canonical encoding failed")]
    Encoding,
    #[error("compiled AgentPlan digest does not match its body")]
    DigestMismatch,
    #[error("compiled AgentPlan materialized route digest does not match executable semantics")]
    MaterializedRouteDigestMismatch,
}

#[cfg(test)]
mod routing_replan_tests {
    use super::*;

    fn refs() -> AgentPlanFactRefsV1 {
        AgentPlanFactRefsV1 {
            connector_registry_version: "registry.v1".into(),
            connector_registry_digest: CanonicalDigest::of_bytes(b"registry"),
            model_data_bundle_version: "model-data.v1".into(),
            model_data_digest: CanonicalDigest::of_bytes(b"model-data"),
            capability_slice_version: "capabilities.v1".into(),
            capability_slice_digest: CanonicalDigest::of_bytes(b"capabilities"),
            ratings_slice_version: "ratings.v1".into(),
            ratings_slice_digest: CanonicalDigest::of_bytes(b"ratings"),
            free_offers_slice_version: "free.v1".into(),
            free_offers_slice_digest: CanonicalDigest::of_bytes(b"free"),
            inventory_revision: 1,
            inventory_digest: CanonicalDigest::of_bytes(b"inventory"),
            price_tracking: PriceTrackingMode::FollowLatest,
        }
    }

    #[test]
    fn routing_price_only_change_does_not_request_replan() {
        let current = refs();
        let mut newer = current.clone();
        assert!(current.replan_reasons(&newer).is_empty());
        newer.ratings_slice_digest = CanonicalDigest::of_bytes(b"ratings-2");
        assert_eq!(
            current.replan_reasons(&newer),
            vec![ReplanReason::RatingChanged]
        );
    }
}
