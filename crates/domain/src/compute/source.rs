use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{CanonicalDigest, CredentialRefV1};

use super::COMPUTE_STATE_SCHEMA_V1;
use super::common::*;
use super::model_data::*;
use super::registry::*;

mod pool_mutation;

pub use pool_mutation::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIdentityV1 {
    pub identity_revision: u64,
    pub provider_platform_id: String,
    pub service_offering_id: String,
    pub entitlement_id: String,
    pub usage_scope: String,
    pub endpoint_profile_id: String,
    pub endpoint_profile_revision: u64,
    pub region_id: String,
    /// Connector-owned opaque account reference; never an OAuth claim, email, or token.
    pub account_subject_ref: String,
    pub evidence_refs: Vec<CanonicalDigest>,
}

impl SourceIdentityV1 {
    pub fn validate(&self) -> Result<(), ComputeContractError> {
        for value in [
            &self.provider_platform_id,
            &self.service_offering_id,
            &self.entitlement_id,
            &self.usage_scope,
            &self.endpoint_profile_id,
            &self.region_id,
            &self.account_subject_ref,
        ] {
            validate_identifier(value)?;
        }
        if self.identity_revision == 0
            || self.endpoint_profile_revision == 0
            || self.evidence_refs.is_empty()
            || self
                .evidence_refs
                .iter()
                .any(|evidence| validate_nonempty_digest(evidence).is_err())
        {
            return Err(ComputeContractError::InvalidSource);
        }
        ensure_unique(self.evidence_refs.iter().map(CanonicalDigest::as_str))?;
        Ok(())
    }

    pub fn digest(&self) -> Result<CanonicalDigest, ComputeContractError> {
        self.validate()?;
        CanonicalDigest::of(&(
            "hiroute.compute-source-semantic-identity/v1",
            &self.provider_platform_id,
            &self.service_offering_id,
            &self.entitlement_id,
            &self.usage_scope,
            &self.endpoint_profile_id,
            &self.region_id,
            &self.account_subject_ref,
        ))
        .map_err(|_| ComputeContractError::InvalidSource)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceOrigin {
    NativeApi,
    Cpa,
    ReleaseFree,
}

#[cfg(test)]
#[path = "source/tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterializationState {
    NeedsCredential,
    NeedsAuthorization,
    Ready,
    Disabled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSourceV1 {
    pub schema: String,
    pub source_id: String,
    pub revision: u64,
    pub connection_option_id: String,
    pub connector_id: String,
    pub connector_revision: u64,
    pub origin: SourceOrigin,
    pub identity: SourceIdentityV1,
    pub identity_digest: CanonicalDigest,
    pub billing_class: BillingClass,
    pub state: MaterializationState,
}

impl ComputeSourceV1 {
    pub fn validate_shape(&self) -> Result<(), ComputeContractError> {
        if self.schema != COMPUTE_STATE_SCHEMA_V1 || self.revision == 0 {
            return Err(ComputeContractError::InvalidSource);
        }
        for value in [
            &self.source_id,
            &self.connection_option_id,
            &self.connector_id,
        ] {
            validate_identifier(value)?;
        }
        if self.connector_revision == 0 {
            return Err(ComputeContractError::InvalidSource);
        }
        self.identity.validate()?;
        if self.identity.digest()? != self.identity_digest || !self.billing_class.is_runnable() {
            return Err(ComputeContractError::InvalidSource);
        }
        Ok(())
    }

    pub fn validate(
        &self,
        registry: &ConnectorRegistryBundleV1,
        explicit_materialization: bool,
    ) -> Result<(), ComputeContractError> {
        let resolved = registry.resolve_validated_option(&self.connection_option_id)?;
        self.validate_resolved(&resolved, explicit_materialization)
    }

    /// Checks the source against one exact option from an already validated catalog.
    pub fn validate_resolved(
        &self,
        resolved: &ValidatedConnectionOptionV1,
        explicit_materialization: bool,
    ) -> Result<(), ComputeContractError> {
        self.validate_shape()?;
        if self.billing_class.requires_explicit_materialization() && !explicit_materialization {
            return Err(ComputeContractError::ExplicitMaterializationRequired);
        }
        if resolved.option.connection_option_id != self.connection_option_id {
            return Err(ComputeContractError::CrossReference);
        }
        let expected_origin = match self.origin {
            SourceOrigin::NativeApi => ConnectionOrigin::NativeApi,
            SourceOrigin::Cpa => ConnectionOrigin::AgentSubscription,
            SourceOrigin::ReleaseFree => ConnectionOrigin::FreeCatalog,
        };
        if resolved.option.connector_id != self.connector_id
            || resolved.option.connector_revision != self.connector_revision
            || resolved.option.billing_class != self.billing_class
            || resolved.option.origin != expected_origin
            || resolved.endpoint_profile.endpoint_profile_id != self.identity.endpoint_profile_id
            || resolved.endpoint_profile.revision != self.identity.endpoint_profile_revision
            || resolved.endpoint_profile.provider_platform_id != self.identity.provider_platform_id
            || resolved.endpoint_profile.service_offering_id != self.identity.service_offering_id
            || resolved.endpoint_profile.entitlement_id != self.identity.entitlement_id
            || resolved.endpoint_profile.usage_scope != self.identity.usage_scope
            || resolved.endpoint_profile.region_id != self.identity.region_id
        {
            return Err(ComputeContractError::CrossReference);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBindingV1 {
    pub binding_id: String,
    pub revision: u64,
    pub source_id: String,
    pub source_revision: u64,
    pub source_identity_digest: CanonicalDigest,
    pub model_data_bundle_version: String,
    pub capability_slice_version: String,
    pub offer_ref: String,
    pub offer_evidence_digest: CanonicalDigest,
    pub billing_class: BillingClass,
    pub model_configuration_id: String,
    pub upstream_model_id: String,
    pub capability_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_pool_id: Option<String>,
}

impl SourceBindingV1 {
    pub fn validate_shape(&self) -> Result<(), ComputeContractError> {
        super::common::validate_upstream_model_id(&self.upstream_model_id)?;
        for id in [
            &self.binding_id,
            &self.source_id,
            &self.model_data_bundle_version,
            &self.capability_slice_version,
            &self.offer_ref,
            &self.model_configuration_id,
            &self.capability_id,
        ] {
            validate_identifier(id)?;
        }
        if self.revision == 0
            || self.source_revision == 0
            || !self.billing_class.is_runnable()
            || validate_nonempty_digest(&self.offer_evidence_digest).is_err()
        {
            return Err(ComputeContractError::CrossReference);
        }
        if let Some(pool_id) = &self.credential_pool_id {
            validate_identifier(pool_id)?;
        }
        Ok(())
    }

    pub fn validate(
        &self,
        source: &ComputeSourceV1,
        model_data: &ModelDataBundleV1,
    ) -> Result<(), ComputeContractError> {
        self.validate_shape()?;
        source.validate_shape()?;
        let offer = model_data
            .offer(&self.offer_ref)
            .ok_or(ComputeContractError::CrossReference)?;
        let capability = model_data
            .model_endpoint_capabilities
            .iter()
            .find(|value| value.capability_id == self.capability_id)
            .ok_or(ComputeContractError::CrossReference)?;
        if self.revision == 0
            || self.source_id != source.source_id
            || self.source_revision != source.revision
            || self.source_identity_digest != source.identity_digest
            || self.model_data_bundle_version != model_data.bundle_version
            || self.capability_slice_version != model_data.capability_slice_version
            || self.billing_class == BillingClass::Unknown
            || self.billing_class != source.billing_class
            || self.billing_class != offer.billing_class
            || self.offer_evidence_digest != offer.evidence_digest
            || offer.endpoint_profile_id != source.identity.endpoint_profile_id
            || offer.endpoint_profile_revision != source.identity.endpoint_profile_revision
            || offer.service_offering_id != source.identity.service_offering_id
            || offer.entitlement_id != source.identity.entitlement_id
            || offer.usage_scope != source.identity.usage_scope
            || offer.region_id != source.identity.region_id
            || !offer
                .model_configuration_ids
                .contains(&self.model_configuration_id)
            || capability.model_configuration_id != self.model_configuration_id
            || capability.upstream_model_id != self.upstream_model_id
            || capability.connector_id != source.connector_id
            || capability.connector_revision != source.connector_revision
            || capability.endpoint_profile_id != source.identity.endpoint_profile_id
            || capability.endpoint_profile_revision != source.identity.endpoint_profile_revision
        {
            return Err(ComputeContractError::CrossReference);
        }
        Ok(())
    }

    pub const fn allowed_in_free_only(&self) -> bool {
        matches!(self.billing_class, BillingClass::Free)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialPoolIdentityV1 {
    pub pool_id: String,
    pub binding_id: String,
    pub binding_revision: u64,
    pub binding_digest: CanonicalDigest,
    pub source_id: String,
    pub source_revision: u64,
    pub connection_option_id: String,
    pub source_identity_digest: CanonicalDigest,
    pub offer_ref: String,
    pub offer_revision: u64,
    pub offer_evidence_digest: CanonicalDigest,
    pub billing_class: BillingClass,
    pub model_configuration_id: String,
    pub authentication: AuthenticationKind,
}

impl CredentialPoolIdentityV1 {
    /// Derives the only pool identity accepted for an exact, Release-validated Binding.
    pub fn for_registered_binding(
        binding: &SourceBindingV1,
        source: &ComputeSourceV1,
        registry: &ConnectorRegistryBundleV1,
        model_data: &ModelDataBundleV1,
    ) -> Result<Self, ComputeContractError> {
        let resolved = registry.resolve_validated_option(&source.connection_option_id)?;
        Self::for_resolved_binding(binding, source, &resolved, model_data)
    }

    /// Exact identity join for the immutable, validated release catalog.
    pub fn for_resolved_binding(
        binding: &SourceBindingV1,
        source: &ComputeSourceV1,
        resolved: &ValidatedConnectionOptionV1,
        model_data: &ModelDataBundleV1,
    ) -> Result<Self, ComputeContractError> {
        source.validate_resolved(resolved, true)?;
        // The release wrapper validates seed-rating provenance before exposing ModelData. Pool
        // identity depends only on this exact Source/Binding/Offer/Capability join, so repeating
        // the raw bundle validator here would incorrectly reject valid zero-sample seed ratings.
        // `binding.validate` below still fails closed on every identity-bearing cross-reference.
        binding.validate(source, model_data)?;
        let pool_id = binding
            .credential_pool_id
            .clone()
            .ok_or(ComputeContractError::InvalidCredentialPool)?;
        let offer = model_data
            .offer(&binding.offer_ref)
            .ok_or(ComputeContractError::InvalidCredentialPool)?;
        let identity = Self {
            pool_id,
            binding_id: binding.binding_id.clone(),
            binding_revision: binding.revision,
            binding_digest: CanonicalDigest::of(binding)
                .map_err(|_| ComputeContractError::InvalidCredentialPool)?,
            source_id: source.source_id.clone(),
            source_revision: source.revision,
            connection_option_id: source.connection_option_id.clone(),
            source_identity_digest: source.identity_digest.clone(),
            offer_ref: offer.offer_id.clone(),
            offer_revision: offer.revision,
            offer_evidence_digest: offer.evidence_digest.clone(),
            billing_class: offer.billing_class,
            model_configuration_id: binding.model_configuration_id.clone(),
            authentication: resolved.connector.authentication,
        };
        identity.validate_against_binding(binding)?;
        Ok(identity)
    }

    pub fn validate_shape(&self) -> Result<(), ComputeContractError> {
        for value in [
            &self.pool_id,
            &self.binding_id,
            &self.source_id,
            &self.connection_option_id,
            &self.offer_ref,
            &self.model_configuration_id,
        ] {
            validate_identifier(value)?;
        }
        if self.binding_revision == 0
            || self.source_revision == 0
            || self.offer_revision == 0
            || !self.billing_class.is_runnable()
            || self.authentication == AuthenticationKind::None
            || validate_nonempty_digest(&self.binding_digest).is_err()
            || validate_nonempty_digest(&self.source_identity_digest).is_err()
            || validate_nonempty_digest(&self.offer_evidence_digest).is_err()
        {
            return Err(ComputeContractError::InvalidCredentialPool);
        }
        Ok(())
    }

    pub fn validate_against_binding(
        &self,
        binding: &SourceBindingV1,
    ) -> Result<(), ComputeContractError> {
        self.validate_shape()?;
        binding.validate_shape()?;
        let digest = CanonicalDigest::of(binding)
            .map_err(|_| ComputeContractError::InvalidCredentialPool)?;
        if binding.credential_pool_id.as_deref() != Some(self.pool_id.as_str())
            || binding.binding_id != self.binding_id
            || binding.revision != self.binding_revision
            || digest != self.binding_digest
            || binding.source_id != self.source_id
            || binding.source_revision != self.source_revision
            || binding.source_identity_digest != self.source_identity_digest
            || binding.offer_ref != self.offer_ref
            || binding.offer_evidence_digest != self.offer_evidence_digest
            || binding.billing_class != self.billing_class
            || binding.model_configuration_id != self.model_configuration_id
        {
            return Err(ComputeContractError::InvalidCredentialPool);
        }
        Ok(())
    }

    pub fn materialize_first(
        &self,
        credential: CredentialRefV1,
        fingerprint: CanonicalDigest,
    ) -> Result<CredentialPoolV1, ComputeContractError> {
        let pool = CredentialPoolV1 {
            pool_id: self.pool_id.clone(),
            binding_id: self.binding_id.clone(),
            binding_revision: self.binding_revision,
            binding_digest: self.binding_digest.clone(),
            source_id: self.source_id.clone(),
            source_revision: self.source_revision,
            connection_option_id: self.connection_option_id.clone(),
            source_identity_digest: self.source_identity_digest.clone(),
            offer_ref: self.offer_ref.clone(),
            offer_revision: self.offer_revision,
            offer_evidence_digest: self.offer_evidence_digest.clone(),
            billing_class: self.billing_class,
            model_configuration_id: self.model_configuration_id.clone(),
            authentication: self.authentication,
            revision: 1,
            credentials: vec![PoolCredentialV1 {
                credential,
                fingerprint,
                ordinal: 0,
                enabled: true,
            }],
        };
        pool.validate()?;
        Ok(pool)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PoolCredentialV1 {
    pub credential: CredentialRefV1,
    pub fingerprint: CanonicalDigest,
    pub ordinal: u32,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialPoolV1 {
    pub pool_id: String,
    pub binding_id: String,
    pub binding_revision: u64,
    pub binding_digest: CanonicalDigest,
    pub source_id: String,
    pub source_revision: u64,
    pub connection_option_id: String,
    pub source_identity_digest: CanonicalDigest,
    pub offer_ref: String,
    pub offer_revision: u64,
    pub offer_evidence_digest: CanonicalDigest,
    pub billing_class: BillingClass,
    pub model_configuration_id: String,
    pub authentication: AuthenticationKind,
    pub revision: u64,
    pub credentials: Vec<PoolCredentialV1>,
}

impl CredentialPoolV1 {
    pub fn identity(&self) -> CredentialPoolIdentityV1 {
        CredentialPoolIdentityV1 {
            pool_id: self.pool_id.clone(),
            binding_id: self.binding_id.clone(),
            binding_revision: self.binding_revision,
            binding_digest: self.binding_digest.clone(),
            source_id: self.source_id.clone(),
            source_revision: self.source_revision,
            connection_option_id: self.connection_option_id.clone(),
            source_identity_digest: self.source_identity_digest.clone(),
            offer_ref: self.offer_ref.clone(),
            offer_revision: self.offer_revision,
            offer_evidence_digest: self.offer_evidence_digest.clone(),
            billing_class: self.billing_class,
            model_configuration_id: self.model_configuration_id.clone(),
            authentication: self.authentication,
        }
    }

    pub fn validate(&self) -> Result<(), ComputeContractError> {
        self.identity().validate_shape()?;
        if self.revision == 0
            || self.credentials.is_empty()
            || !self.credentials.iter().any(|entry| entry.enabled)
        {
            return Err(ComputeContractError::InvalidCredentialPool);
        }
        let mut ids = BTreeSet::new();
        let mut fingerprints = BTreeSet::new();
        for (index, entry) in self.credentials.iter().enumerate() {
            if entry.ordinal as usize != index
                || entry.credential.generation() == 0
                || !ids.insert(entry.credential.credential_id())
                || !fingerprints.insert(entry.fingerprint.as_str())
                || entry.credential.owner_scope() != format!("source/{}", self.source_id)
                || match self.authentication {
                    AuthenticationKind::ProviderApiKey => entry.credential.subject() != "hirouted",
                    AuthenticationKind::ConnectorOwnedOpaque => {
                        !entry.credential.subject().starts_with("connector/")
                    }
                    AuthenticationKind::None => true,
                }
                || entry.credential.purpose() != "provider-auth"
                || entry.credential.allowed_destinations()
                    != &BTreeSet::from([format!("connection-option/{}", self.connection_option_id)])
                || validate_nonempty_digest(&entry.fingerprint).is_err()
            {
                return Err(ComputeContractError::InvalidCredentialPool);
            }
        }
        Ok(())
    }

    pub fn validate_against_source(
        &self,
        source: &ComputeSourceV1,
        registry: &ConnectorRegistryBundleV1,
        model_data: &ModelDataBundleV1,
    ) -> Result<(), ComputeContractError> {
        model_data.validate_against(registry)?;
        self.validate_against_source_join(source, registry, model_data)
    }

    fn validate_against_source_join(
        &self,
        source: &ComputeSourceV1,
        registry: &ConnectorRegistryBundleV1,
        model_data: &ModelDataBundleV1,
    ) -> Result<(), ComputeContractError> {
        let resolved = registry.resolve_validated_option(&self.connection_option_id)?;
        self.validate_resolved_source(source, &resolved, model_data)
    }

    /// Validates the exact pool/source/offer join after the catalog loading boundary.
    pub fn validate_resolved_source(
        &self,
        source: &ComputeSourceV1,
        resolved: &ValidatedConnectionOptionV1,
        model_data: &ModelDataBundleV1,
    ) -> Result<(), ComputeContractError> {
        self.validate()?;
        source.validate_resolved(resolved, true)?;
        let offer = model_data
            .offer(&self.offer_ref)
            .ok_or(ComputeContractError::InvalidCredentialPool)?;
        if self.source_id != source.source_id
            || self.source_revision != source.revision
            || self.source_identity_digest != source.identity_digest
            || self.connection_option_id != source.connection_option_id
            || self.authentication != resolved.connector.authentication
            || self.billing_class != source.billing_class
            || self.billing_class != offer.billing_class
            || self.offer_revision != offer.revision
            || self.offer_evidence_digest != offer.evidence_digest
            || !offer
                .model_configuration_ids
                .contains(&self.model_configuration_id)
            || offer.endpoint_profile_id != source.identity.endpoint_profile_id
            || offer.endpoint_profile_revision != source.identity.endpoint_profile_revision
            || offer.service_offering_id != source.identity.service_offering_id
            || offer.entitlement_id != source.identity.entitlement_id
            || offer.usage_scope != source.identity.usage_scope
            || offer.region_id != source.identity.region_id
            || resolved.option.connector_id != source.connector_id
            || resolved.option.connector_revision != source.connector_revision
        {
            return Err(ComputeContractError::InvalidCredentialPool);
        }
        Ok(())
    }

    /// Closes the pool reference at the exact Binding/Offer identity. Two bindings for the same
    /// account and endpoint still cannot share a pool when their Offer semantics differ.
    pub fn validate_against_binding(
        &self,
        binding: &SourceBindingV1,
    ) -> Result<(), ComputeContractError> {
        self.validate()?;
        self.identity().validate_against_binding(binding)
    }

    /// Rebinds unchanged Secret references to a refreshed durable Source/Binding generation.
    /// Commercial identity fields cannot change through this path; a different model, Offer, or
    /// authentication contract requires a distinct pool instead.
    pub fn rebind_registered_identity(
        &self,
        identity: &CredentialPoolIdentityV1,
    ) -> Result<Self, ComputeContractError> {
        self.validate()?;
        identity.validate_shape()?;
        if self.pool_id != identity.pool_id
            || self.binding_id != identity.binding_id
            || self.source_id != identity.source_id
            || self.connection_option_id != identity.connection_option_id
            || self.source_identity_digest != identity.source_identity_digest
            || self.offer_ref != identity.offer_ref
            || self.billing_class != identity.billing_class
            || self.model_configuration_id != identity.model_configuration_id
            || self.authentication != identity.authentication
        {
            return Err(ComputeContractError::InvalidCredentialPool);
        }
        let mut rebound = self.clone();
        rebound.binding_revision = identity.binding_revision;
        rebound.binding_digest = identity.binding_digest.clone();
        rebound.source_revision = identity.source_revision;
        rebound.offer_revision = identity.offer_revision;
        rebound.offer_evidence_digest = identity.offer_evidence_digest.clone();
        rebound.revision = rebound
            .revision
            .checked_add(1)
            .ok_or(ComputeContractError::GenerationConflict)?;
        rebound.validate()?;
        Ok(rebound)
    }

    pub fn add(
        &self,
        expected_revision: u64,
        credential: CredentialRefV1,
        fingerprint: CanonicalDigest,
    ) -> Result<Self, ComputeContractError> {
        if self.revision != expected_revision
            || self.credentials.iter().any(|entry| {
                entry.credential.credential_id() == credential.credential_id()
                    || entry.fingerprint == fingerprint
            })
        {
            return Err(ComputeContractError::GenerationConflict);
        }
        let mut next = self.clone();
        next.credentials.push(PoolCredentialV1 {
            credential,
            fingerprint,
            ordinal: u32::try_from(next.credentials.len())
                .map_err(|_| ComputeContractError::InvalidCredentialPool)?,
            enabled: true,
        });
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or(ComputeContractError::GenerationConflict)?;
        next.validate()?;
        Ok(next)
    }

    pub fn replace(
        &self,
        expected_revision: u64,
        credential_id: &str,
        replacement: CredentialRefV1,
        fingerprint: CanonicalDigest,
    ) -> Result<Self, ComputeContractError> {
        if self.revision != expected_revision {
            return Err(ComputeContractError::GenerationConflict);
        }
        let mut next = self.clone();
        let entry = next
            .credentials
            .iter_mut()
            .find(|entry| entry.credential.credential_id() == credential_id)
            .ok_or(ComputeContractError::CredentialNotFound)?;
        if replacement.credential_id() != credential_id
            || entry.credential.generation().checked_add(1) != Some(replacement.generation())
        {
            return Err(ComputeContractError::InvalidCredentialPool);
        }
        entry.credential = replacement;
        entry.fingerprint = fingerprint;
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or(ComputeContractError::GenerationConflict)?;
        next.validate()?;
        Ok(next)
    }

    pub fn remove(
        &self,
        expected_revision: u64,
        credential_id: &str,
    ) -> Result<Self, ComputeContractError> {
        if self.revision != expected_revision {
            return Err(ComputeContractError::GenerationConflict);
        }
        let mut next = self.clone();
        let before = next.credentials.len();
        next.credentials
            .retain(|entry| entry.credential.credential_id() != credential_id);
        if before == next.credentials.len() {
            return Err(ComputeContractError::CredentialNotFound);
        }
        if !next.credentials.iter().any(|entry| entry.enabled) {
            return Err(ComputeContractError::LastUsableCredential);
        }
        for (ordinal, entry) in next.credentials.iter_mut().enumerate() {
            entry.ordinal = ordinal as u32;
        }
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or(ComputeContractError::GenerationConflict)?;
        next.validate()?;
        Ok(next)
    }

    pub fn reorder(
        &self,
        expected_revision: u64,
        ordered_ids: &[String],
    ) -> Result<Self, ComputeContractError> {
        if self.revision != expected_revision || ordered_ids.len() != self.credentials.len() {
            return Err(ComputeContractError::GenerationConflict);
        }
        ensure_unique(ordered_ids.iter().map(String::as_str))?;
        let by_id = self
            .credentials
            .iter()
            .map(|entry| (entry.credential.credential_id(), entry.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut credentials = Vec::with_capacity(ordered_ids.len());
        for (ordinal, id) in ordered_ids.iter().enumerate() {
            let mut entry = by_id
                .get(id.as_str())
                .cloned()
                .ok_or(ComputeContractError::CredentialNotFound)?;
            entry.ordinal = ordinal as u32;
            credentials.push(entry);
        }
        let mut next = self.clone();
        next.credentials = credentials;
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or(ComputeContractError::GenerationConflict)?;
        next.validate()?;
        Ok(next)
    }
}
