//! Offline Release-fact verification and registered compute adapter seams.

use std::collections::{BTreeMap, BTreeSet};

use hiroute_domain::{
    CanonicalDigest, ComputeContractError, ComputeSourceV1, ConnectorRegistryBundleV1,
    CredentialPoolV1, CredentialRefV1, EffectiveInventoryModelV1, ModelDataBundleV1,
    ObservedModelV1, OperationId, ResolvedConnectionOptionV1, reconcile_inventory,
};
use thiserror::Error;

#[path = "compute/control_plane.rs"]
mod control_plane;
mod defaults;
pub use control_plane::*;
pub use defaults::resolve_managed_defaults;

#[derive(Clone, Debug)]
pub struct TrustedReleaseCatalog {
    pub(crate) registry: ConnectorRegistryBundleV1,
    pub(crate) resolved_options: BTreeMap<String, hiroute_domain::ValidatedConnectionOptionV1>,
    pub(crate) model_data: ModelDataBundleV1,
    pub(crate) registry_catalog_id: String,
    pub(crate) model_data_catalog_id: String,
    pub(crate) registry_sequence: u64,
    pub(crate) model_data_sequence: u64,
    pub(crate) registry_digest: CanonicalDigest,
    pub(crate) model_data_digest: CanonicalDigest,
    pub(crate) release_facts: hiroute_domain::ReleaseFactsManifestV2,
    pub(crate) current_release_model_data: hiroute_domain::ReleaseModelDataBundleV2,
    pub(crate) feed_facts: Option<Box<crate::model_feed::FeedCatalogFacts>>,
    pub(crate) official_snapshot: Option<Box<hiroute_domain::OfficialModelSnapshotV1>>,
}

impl TrustedReleaseCatalog {
    pub fn complete_official_data(&self) -> Option<&hiroute_domain::OfficialModelDataV1> {
        self.official_snapshot.as_ref().map(|s| &s.data)
    }
    pub fn registry(&self) -> &ConnectorRegistryBundleV1 {
        &self.registry
    }

    pub fn model_data(&self) -> &ModelDataBundleV1 {
        &self.model_data
    }

    pub fn release_facts_manifest(&self) -> &hiroute_domain::ReleaseFactsManifestV2 {
        &self.release_facts
    }

    pub fn current_release_model_data(&self) -> &hiroute_domain::ReleaseModelDataBundleV2 {
        &self.current_release_model_data
    }

    /// Broad metadata is loaded from the one client-bundled Release Model Data catalog.
    pub fn model_metadata(&self) -> &hiroute_domain::ModelMetadataCatalogV1 {
        if let Some(snapshot) = &self.official_snapshot {
            return &snapshot.data.metadata_catalog;
        }
        &self.current_release_model_data.metadata_catalog
    }

    /// Unknown inventory IDs may use the conservative text fallback unless the bundled
    /// metadata has a determinate, exclusively non-text/internal outcome for that exact ID.
    /// This is only an eligibility guard; it does not assign canonical or provider identity.
    pub fn runtime_fallback_allows_observed_text(&self, upstream_model_id: &str) -> bool {
        let matching = self
            .model_metadata()
            .model_records
            .iter()
            .filter(|record| record.upstream_model_id == upstream_model_id)
            .collect::<Vec<_>>();
        matching.is_empty()
            || matching.iter().any(|record| {
                record.execution_fit.state
                    == hiroute_domain::MetadataExecutionFitStateV1::NativeTextRepresentable
            })
    }

    pub fn runtime_fallback_denied_model_ids(&self) -> BTreeSet<String> {
        self.model_metadata()
            .model_records
            .iter()
            .filter(|record| !self.runtime_fallback_allows_observed_text(&record.upstream_model_id))
            .map(|record| record.upstream_model_id.clone())
            .collect()
    }

    /// Exact-native references from digest-validated, client-bound bytes.
    pub fn rating_snapshot(&self) -> &hiroute_domain::RatingSnapshotV2 {
        if let Some(snapshot) = &self.official_snapshot {
            return &snapshot.data.rating_snapshot;
        }
        &self.current_release_model_data.rating_snapshot
    }

    pub fn native_reasoning(&self) -> &[hiroute_domain::ModelNativeReasoningV1] {
        if let Some(snapshot) = &self.official_snapshot {
            return &snapshot.data.rating_snapshot.models;
        }
        if let Some(feed) = &self.feed_facts {
            return &feed.native_reasoning;
        }
        self.current_release_model_data
            .rating_snapshot
            .models
            .as_slice()
    }

    pub fn registry_provenance(&self) -> (&str, u64, &CanonicalDigest) {
        (
            &self.registry_catalog_id,
            self.registry_sequence,
            &self.registry_digest,
        )
    }

    pub fn model_data_provenance(&self) -> (&str, u64, &CanonicalDigest) {
        (
            &self.model_data_catalog_id,
            self.model_data_sequence,
            &self.model_data_digest,
        )
    }

    pub fn resolve_connection_option(
        &self,
        option_id: &str,
    ) -> Result<ResolvedConnectionOptionV1, ReleaseVerificationError> {
        self.trusted_option(option_id)
            .map(|value| (**value).clone())
    }

    fn trusted_option(
        &self,
        option_id: &str,
    ) -> Result<&hiroute_domain::ValidatedConnectionOptionV1, ReleaseVerificationError> {
        self.resolved_options
            .get(option_id)
            .ok_or(ReleaseVerificationError::Contract(
                ComputeContractError::UnknownConnectionOption,
            ))
    }

    pub fn validate_source(
        &self,
        source: &ComputeSourceV1,
        explicit_materialization: bool,
    ) -> Result<(), ReleaseVerificationError> {
        let resolved = self.trusted_option(&source.connection_option_id)?;
        source
            .validate_resolved(resolved, explicit_materialization)
            .map_err(ReleaseVerificationError::Contract)
    }

    pub fn validate_pool(
        &self,
        pool: &CredentialPoolV1,
        source: &ComputeSourceV1,
    ) -> Result<(), ReleaseVerificationError> {
        let resolved = self.trusted_option(&source.connection_option_id)?;
        pool.validate_resolved_source(source, resolved, self.model_data())
            .map_err(ReleaseVerificationError::Contract)
    }

    pub fn reconcile_observed_inventory(
        &self,
        endpoint_profile_id: &str,
        observed: impl IntoIterator<Item = ObservedModelV1>,
    ) -> Result<Vec<EffectiveInventoryModelV1>, ReleaseVerificationError> {
        // Inventory receives only a profile reference; it has no field capable of returning a
        // destination, protocol, authentication scheme, or BillingClass.
        if self
            .registry
            .endpoint_profile(endpoint_profile_id)
            .is_none()
        {
            return Err(ReleaseVerificationError::Contract(
                ComputeContractError::CrossReference,
            ));
        }
        reconcile_inventory(endpoint_profile_id, observed, &self.model_data)
            .map_err(ReleaseVerificationError::Contract)
    }
}

fn valid_release_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("//")
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
}

#[derive(Debug, Error)]
pub enum ReleaseVerificationError {
    #[error("Release bundle exceeds the bounded size")]
    BundleTooLarge,
    #[error("Release bundle is malformed")]
    MalformedBundle,
    #[error("Release schema is unsupported")]
    UnsupportedSchema,
    #[error("Release manifest does not match the client-bundled manifest")]
    ManifestMismatch,
    #[error("Release payload digest does not match")]
    DigestMismatch,
    #[error("Release cross-reference digest does not match")]
    CrossReferenceDigestMismatch,
    #[error(transparent)]
    Contract(#[from] ComputeContractError),
}

/// Non-serializable receipt issued only inside the registered Connector adapter crate after the
/// exact candidate Secret has passed its command-specific safe probe. There is intentionally no
/// public constructor or field access that could let Application/wire code forge it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedCredentialProbeV1 {
    credential_id: String,
    next_generation: u64,
    candidate_fingerprint: CanonicalDigest,
    source_id: String,
    source_revision: u64,
    source_identity_digest: CanonicalDigest,
    connection_option_id: String,
    endpoint_profile_id: String,
    endpoint_profile_revision: u64,
    probe_operation_id: OperationId,
}

impl VerifiedCredentialProbeV1 {
    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)] // The production probe adapter is composed by the later Attempt owner.
    pub(crate) fn from_registered_probe(
        credential_id: impl Into<String>,
        next_generation: u64,
        candidate_fingerprint: CanonicalDigest,
        source: &ComputeSourceV1,
        option: &ResolvedConnectionOptionV1,
        probe_operation_id: OperationId,
    ) -> Result<Self, ComputeContractError> {
        let credential_id = credential_id.into();
        if !valid_release_id(&credential_id)
            || next_generation == 0
            || option.option.connection_option_id != source.connection_option_id
            || option.endpoint_profile.endpoint_profile_id != source.identity.endpoint_profile_id
            || option.endpoint_profile.revision != source.identity.endpoint_profile_revision
        {
            return Err(ComputeContractError::InvalidCredentialPool);
        }
        Ok(Self {
            credential_id,
            next_generation,
            candidate_fingerprint,
            source_id: source.source_id.clone(),
            source_revision: source.revision,
            source_identity_digest: source.identity_digest.clone(),
            connection_option_id: source.connection_option_id.clone(),
            endpoint_profile_id: source.identity.endpoint_profile_id.clone(),
            endpoint_profile_revision: source.identity.endpoint_profile_revision,
            probe_operation_id,
        })
    }

    fn matches(
        &self,
        replacement: &CredentialRefV1,
        fingerprint: &CanonicalDigest,
        source: &ComputeSourceV1,
        option: &ResolvedConnectionOptionV1,
        probe_operation_id: &OperationId,
    ) -> bool {
        self.credential_id == replacement.credential_id()
            && self.next_generation == replacement.generation()
            && self.candidate_fingerprint == *fingerprint
            && self.source_id == source.source_id
            && self.source_revision == source.revision
            && self.source_identity_digest == source.identity_digest
            && self.connection_option_id == source.connection_option_id
            && self.endpoint_profile_id == option.endpoint_profile.endpoint_profile_id
            && self.endpoint_profile_revision == option.endpoint_profile.revision
            && self.probe_operation_id == *probe_operation_id
    }
}

#[allow(clippy::too_many_arguments)]
pub fn rotate_credential_after_verified_probe(
    pool: &CredentialPoolV1,
    expected_pool_revision: u64,
    replacement: CredentialRefV1,
    candidate_fingerprint: CanonicalDigest,
    source: &ComputeSourceV1,
    option: &ResolvedConnectionOptionV1,
    probe_operation_id: &OperationId,
    receipt: &VerifiedCredentialProbeV1,
) -> Result<CredentialPoolV1, ComputeContractError> {
    if pool.source_id != source.source_id
        || pool.source_revision != source.revision
        || pool.source_identity_digest != source.identity_digest
        || pool.connection_option_id != option.option.connection_option_id
        || !receipt.matches(
            &replacement,
            &candidate_fingerprint,
            source,
            option,
            probe_operation_id,
        )
    {
        return Err(ComputeContractError::InvalidCredentialPool);
    }
    let credential_id = replacement.credential_id().to_owned();
    pool.replace(
        expected_pool_revision,
        &credential_id,
        replacement,
        candidate_fingerprint,
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CpaAccountMaterializationV1 {
    pub connector_id: String,
    pub connection_option_id: String,
    pub endpoint_profile_id: String,
    pub source_id: String,
    pub account_subject: String,
    pub credential_ref: CredentialRefV1,
    pub observed_model_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CpaRegisteredSourceV1 {
    pub source: ComputeSourceV1,
    pub credential_ref: CredentialRefV1,
    pub inventory: Vec<EffectiveInventoryModelV1>,
}

/// Converts a connector-owned CPA account attestation into registered domain facts. Logical
/// identity comes exclusively from the current connection option; the CPA loopback transport and
/// stock account locator are intentionally absent from the result.
pub fn register_cpa_account(
    catalog: &TrustedReleaseCatalog,
    materialization: &CpaAccountMaterializationV1,
) -> Result<CpaRegisteredSourceV1, CpaSupervisorError> {
    materialization.validate_against(catalog)?;
    let resolved = catalog
        .resolve_connection_option(&materialization.connection_option_id)
        .map_err(|_| CpaSupervisorError::ConnectorNotRegistered)?;
    if resolved.option.origin != hiroute_domain::ConnectionOrigin::AgentSubscription
        || resolved.option.billing_class != hiroute_domain::BillingClass::Subscription
    {
        return Err(CpaSupervisorError::InvalidOpaqueMaterialization);
    }
    let evidence = CanonicalDigest::of(&(
        "hiroute.cpa-account-attestation/v1",
        materialization.connector_id.as_str(),
        materialization.connection_option_id.as_str(),
        materialization.endpoint_profile_id.as_str(),
        materialization.account_subject.as_str(),
        resolved.endpoint_profile.verification_evidence.as_str(),
    ))
    .map_err(|_| CpaSupervisorError::InvalidOpaqueMaterialization)?;
    let identity = hiroute_domain::SourceIdentityV1 {
        identity_revision: 1,
        provider_platform_id: resolved.endpoint_profile.provider_platform_id.clone(),
        service_offering_id: resolved.endpoint_profile.service_offering_id.clone(),
        entitlement_id: resolved.endpoint_profile.entitlement_id.clone(),
        usage_scope: resolved.endpoint_profile.usage_scope.clone(),
        endpoint_profile_id: resolved.endpoint_profile.endpoint_profile_id.clone(),
        endpoint_profile_revision: resolved.endpoint_profile.revision,
        region_id: resolved.endpoint_profile.region_id.clone(),
        account_subject_ref: materialization.account_subject.clone(),
        evidence_refs: vec![evidence],
    };
    let identity_digest = identity
        .digest()
        .map_err(|_| CpaSupervisorError::InvalidOpaqueMaterialization)?;
    let source = ComputeSourceV1 {
        schema: hiroute_domain::COMPUTE_STATE_SCHEMA_V1.to_owned(),
        source_id: materialization.source_id.clone(),
        revision: 1,
        connection_option_id: materialization.connection_option_id.clone(),
        connector_id: materialization.connector_id.clone(),
        connector_revision: resolved.connector.revision,
        origin: hiroute_domain::SourceOrigin::Cpa,
        identity,
        identity_digest,
        billing_class: resolved.option.billing_class,
        state: hiroute_domain::MaterializationState::Ready,
    };
    catalog
        .validate_source(&source, true)
        .map_err(|_| CpaSupervisorError::InvalidOpaqueMaterialization)?;
    let observed = materialization
        .observed_model_ids
        .iter()
        .cloned()
        .map(|upstream_model_id| ObservedModelV1 {
            upstream_model_id,
            metadata: BTreeMap::new(),
        });
    let inventory = catalog
        .reconcile_observed_inventory(&materialization.endpoint_profile_id, observed)
        .map_err(|_| CpaSupervisorError::InvalidOpaqueMaterialization)?;
    Ok(CpaRegisteredSourceV1 {
        source,
        credential_ref: materialization.credential_ref.clone(),
        inventory,
    })
}

impl CpaAccountMaterializationV1 {
    pub fn validate_against(
        &self,
        catalog: &TrustedReleaseCatalog,
    ) -> Result<(), CpaSupervisorError> {
        let resolved = catalog
            .resolve_connection_option(&self.connection_option_id)
            .map_err(|_| CpaSupervisorError::ConnectorNotRegistered)?;
        let destination =
            BTreeSet::from([format!("connection-option/{}", self.connection_option_id)]);
        if !valid_release_id(&self.account_subject)
            || !valid_release_id(&self.source_id)
            || resolved.connector.connector_id != self.connector_id
            || resolved.connector.runtime_kind != hiroute_domain::ConnectorRuntimeKind::CpaBridge
            || resolved.connector.authentication
                != hiroute_domain::AuthenticationKind::ConnectorOwnedOpaque
            || resolved.endpoint_profile.endpoint_profile_id != self.endpoint_profile_id
            || self.credential_ref.subject() != format!("connector/{}", self.connector_id)
            || self.credential_ref.owner_scope() != format!("source/{}", self.source_id)
            || self.credential_ref.purpose() != "provider-auth"
            || self.credential_ref.allowed_destinations() != &destination
            || self.credential_ref.generation() == 0
            || self.observed_model_ids.len() > 10_000
            || self
                .observed_model_ids
                .iter()
                .any(|model_id| !valid_release_id(model_id))
        {
            return Err(CpaSupervisorError::InvalidOpaqueMaterialization);
        }
        Ok(())
    }
}

/// Replaceable CPA boundary. The returned CredentialRef is connector-owned and opaque; there is
/// intentionally no token accessor or byte field in this contract.
pub trait CpaSupervisorPort {
    fn materialize_account(
        &self,
        connector_id: &str,
        endpoint_profile_id: &str,
    ) -> Result<CpaAccountMaterializationV1, CpaSupervisorError>;
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CpaSupervisorError {
    #[error("CPA connector is not registered")]
    ConnectorNotRegistered,
    #[error("CPA authorization requires user action")]
    ActionRequired,
    #[error("CPA supervisor is unavailable")]
    Unavailable,
    #[error("CPA returned an invalid or non-opaque account materialization")]
    InvalidOpaqueMaterialization,
}

#[cfg(test)]
#[path = "compute/tests.rs"]
mod tests;
