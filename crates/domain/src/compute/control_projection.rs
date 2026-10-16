use serde::{Deserialize, Serialize};

use crate::CanonicalDigest;

use super::{
    ComputeContractError, ComputeSourceV1, CredentialPoolIdentityV1, ObservedModelV1,
    SourceBindingV1,
};

pub const COMPUTE_CONTROL_PROJECTION_SCHEMA_V1: &str = "hiroute.compute-control-projection/v1";

/// Exact client-bundled ReleaseFacts provenance used to derive one durable compute projection.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeCatalogProvenanceV1 {
    pub product_release: String,
    pub catalog_binding_id: String,
    pub release_sequence: u64,
    pub connector_registry_version: String,
    pub connector_registry_digest: CanonicalDigest,
    pub model_data_bundle_version: String,
    pub model_data_digest: CanonicalDigest,
    pub cross_reference_digest: CanonicalDigest,
}

/// Non-secret scanner identity. The opaque source ref is meaningful only to the exact scanner.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeScannerEvidenceV1 {
    pub scanner_id: String,
    pub scanner_version: String,
    pub discovered_source_ref: String,
    pub configuration_revision: u64,
    pub evidence_digest: CanonicalDigest,
}

/// Provider observations remain inventory only. They cannot introduce an endpoint, protocol,
/// authentication kind, billing class, or model capability.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeInventorySnapshotV1 {
    pub source_id: String,
    pub endpoint_profile_id: String,
    pub inventory_revision: u64,
    pub inventory_digest: CanonicalDigest,
    pub observed_models: Vec<ObservedModelV1>,
    pub captured_at: i64,
}

impl ComputeInventorySnapshotV1 {
    pub fn validate(&self) -> Result<(), ComputeContractError> {
        if !valid_id(&self.source_id)
            || !valid_id(&self.endpoint_profile_id)
            || self.inventory_revision == 0
            || self.captured_at <= 0
            || self.observed_models.is_empty()
            || self.observed_models.len() > 10_000
        {
            return Err(ComputeContractError::InvalidInventory);
        }
        for model in &self.observed_models {
            model.validate()?;
        }
        if CanonicalDigest::of(&self.observed_models)
            .map_err(|_| ComputeContractError::InvalidInventory)?
            != self.inventory_digest
        {
            return Err(ComputeContractError::InvalidInventory);
        }
        Ok(())
    }
}

/// Full non-secret projection activated with workspace desired state in one SQLite transaction.
/// A pool identity is durable even while its Secret-backed pool is intentionally absent.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeControlProjectionV1 {
    pub schema: String,
    pub source: ComputeSourceV1,
    pub binding: SourceBindingV1,
    pub inventory: ComputeInventorySnapshotV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_pool_identity: Option<CredentialPoolIdentityV1>,
    pub catalog: ComputeCatalogProvenanceV1,
    pub scanner: ComputeScannerEvidenceV1,
}

impl ComputeControlProjectionV1 {
    pub fn validate_shape(&self) -> Result<(), ComputeContractError> {
        if self.schema != COMPUTE_CONTROL_PROJECTION_SCHEMA_V1 {
            return Err(ComputeContractError::UnsupportedSchema);
        }
        self.source.validate_shape()?;
        self.binding.validate_shape()?;
        self.inventory.validate()?;
        if self.binding.source_id != self.source.source_id
            || self.binding.source_revision != self.source.revision
            || self.binding.source_identity_digest != self.source.identity_digest
            || self.binding.billing_class != self.source.billing_class
            || self.binding.model_data_bundle_version != self.catalog.model_data_bundle_version
            || self.inventory.source_id != self.source.source_id
            || self.inventory.endpoint_profile_id != self.source.identity.endpoint_profile_id
            || !self
                .inventory
                .observed_models
                .iter()
                .any(|model| model.upstream_model_id == self.binding.upstream_model_id)
            || self.catalog.product_release.is_empty()
            || !valid_id(&self.catalog.catalog_binding_id)
            || self.catalog.release_sequence == 0
            || self.catalog.connector_registry_version.is_empty()
            || self.catalog.model_data_bundle_version.is_empty()
            || invalid_digest(&self.catalog.connector_registry_digest)
            || invalid_digest(&self.catalog.model_data_digest)
            || invalid_digest(&self.catalog.cross_reference_digest)
            || !valid_id(&self.scanner.scanner_id)
            || !valid_id(&self.scanner.scanner_version)
            || !valid_id(&self.scanner.discovered_source_ref)
            || self.scanner.configuration_revision == 0
            || invalid_digest(&self.scanner.evidence_digest)
            || !self
                .source
                .identity
                .evidence_refs
                .contains(&self.scanner.evidence_digest)
        {
            return Err(ComputeContractError::CrossReference);
        }
        match (
            self.binding.credential_pool_id.as_deref(),
            self.credential_pool_identity.as_ref(),
        ) {
            (None, None) => {}
            (Some(pool_id), Some(identity))
                if identity.pool_id == pool_id
                    && identity.validate_against_binding(&self.binding).is_ok() => {}
            _ => return Err(ComputeContractError::InvalidCredentialPool),
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<CanonicalDigest, ComputeContractError> {
        self.validate_shape()?;
        CanonicalDigest::of(self).map_err(|_| ComputeContractError::CrossReference)
    }
}

/// Exact pre-state bound into the sealed mutation and checked again by the storage CAS.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeProjectionExpectationV1 {
    pub source_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_digest: Option<CanonicalDigest>,
    pub binding_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_digest: Option<CanonicalDigest>,
    pub inventory_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_digest: Option<CanonicalDigest>,
}

impl ComputeProjectionExpectationV1 {
    pub fn validate_for(
        &self,
        desired: &ComputeControlProjectionV1,
    ) -> Result<(), ComputeContractError> {
        let source_present = self.source_revision > 0;
        let binding_present = self.binding_revision > 0;
        let inventory_present = self.inventory_revision > 0;
        if source_present != self.source_digest.is_some()
            || binding_present != self.binding_digest.is_some()
            || inventory_present != self.inventory_digest.is_some()
            || self.source_revision.checked_add(1) != Some(desired.source.revision)
            || self.binding_revision.checked_add(1) != Some(desired.binding.revision)
            || self.inventory_revision.checked_add(1) != Some(desired.inventory.inventory_revision)
            || self.source_digest.as_ref().is_some_and(invalid_digest)
            || self.binding_digest.as_ref().is_some_and(invalid_digest)
            || self.inventory_digest.as_ref().is_some_and(invalid_digest)
        {
            return Err(ComputeContractError::CrossReference);
        }
        Ok(())
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedComputeProjectionV1 {
    pub expected: ComputeProjectionExpectationV1,
    pub desired: ComputeControlProjectionV1,
}

impl PreparedComputeProjectionV1 {
    pub fn validate(&self) -> Result<(), ComputeContractError> {
        self.desired.validate_shape()?;
        self.expected.validate_for(&self.desired)
    }
}

fn invalid_digest(value: &CanonicalDigest) -> bool {
    value == &CanonicalDigest::of_bytes(&[])
        || !matches!(CanonicalDigest::parse(value.as_str()), Ok(parsed) if &parsed == value)
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains("..")
        && !value.contains("//")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b':' | b'-')
        })
}

#[cfg(test)]
mod compute_projection_tests {
    use super::*;
    use crate::{
        AuthenticationKind, BillingClass, COMPUTE_STATE_SCHEMA_V1, MaterializationState,
        SourceIdentityV1, SourceOrigin,
    };

    fn projection() -> PreparedComputeProjectionV1 {
        let identity = SourceIdentityV1 {
            identity_revision: 1,
            provider_platform_id: "provider.fixture".into(),
            service_offering_id: "offering.fixture".into(),
            entitlement_id: "entitlement.fixture".into(),
            usage_scope: "account".into(),
            endpoint_profile_id: "endpoint.fixture".into(),
            endpoint_profile_revision: 1,
            region_id: "test".into(),
            account_subject_ref: "account.fixture".into(),
            evidence_refs: vec![CanonicalDigest::of_bytes(b"scanner")],
        };
        let source = ComputeSourceV1 {
            schema: COMPUTE_STATE_SCHEMA_V1.into(),
            source_id: "source.fixture".into(),
            revision: 1,
            connection_option_id: "option.fixture".into(),
            connector_id: "connector.fixture".into(),
            connector_revision: 1,
            origin: SourceOrigin::NativeApi,
            identity_digest: identity.digest().unwrap(),
            identity,
            billing_class: BillingClass::Paid,
            state: MaterializationState::NeedsCredential,
        };
        let binding = SourceBindingV1 {
            binding_id: "binding.fixture".into(),
            revision: 1,
            source_id: source.source_id.clone(),
            source_revision: 1,
            source_identity_digest: source.identity_digest.clone(),
            model_data_bundle_version: "models.fixture".into(),
            capability_slice_version: "capabilities.fixture".into(),
            offer_ref: "offer.fixture".into(),
            offer_evidence_digest: CanonicalDigest::of_bytes(b"offer"),
            billing_class: BillingClass::Paid,
            model_configuration_id: "model.fixture".into(),
            upstream_model_id: "upstream.fixture".into(),
            capability_id: "capability.fixture".into(),
            credential_pool_id: Some("pool.fixture".into()),
        };
        let pool = CredentialPoolIdentityV1 {
            pool_id: "pool.fixture".into(),
            binding_id: binding.binding_id.clone(),
            binding_revision: 1,
            binding_digest: CanonicalDigest::of(&binding).unwrap(),
            source_id: source.source_id.clone(),
            source_revision: 1,
            connection_option_id: source.connection_option_id.clone(),
            source_identity_digest: source.identity_digest.clone(),
            offer_ref: binding.offer_ref.clone(),
            offer_revision: 1,
            offer_evidence_digest: binding.offer_evidence_digest.clone(),
            billing_class: BillingClass::Paid,
            model_configuration_id: binding.model_configuration_id.clone(),
            authentication: AuthenticationKind::ProviderApiKey,
        };
        let observed_models = vec![ObservedModelV1 {
            upstream_model_id: binding.upstream_model_id.clone(),
            metadata: Default::default(),
        }];
        PreparedComputeProjectionV1 {
            expected: ComputeProjectionExpectationV1 {
                source_revision: 0,
                source_digest: None,
                binding_revision: 0,
                binding_digest: None,
                inventory_revision: 0,
                inventory_digest: None,
            },
            desired: ComputeControlProjectionV1 {
                schema: COMPUTE_CONTROL_PROJECTION_SCHEMA_V1.into(),
                source,
                binding,
                inventory: ComputeInventorySnapshotV1 {
                    source_id: "source.fixture".into(),
                    endpoint_profile_id: "endpoint.fixture".into(),
                    inventory_revision: 1,
                    inventory_digest: CanonicalDigest::of(&observed_models).unwrap(),
                    observed_models,
                    captured_at: 1,
                },
                credential_pool_identity: Some(pool),
                catalog: ComputeCatalogProvenanceV1 {
                    product_release: "release.fixture".into(),
                    catalog_binding_id: "fixture-catalog".into(),
                    release_sequence: 1,
                    connector_registry_version: "registry.fixture".into(),
                    connector_registry_digest: CanonicalDigest::of_bytes(b"registry"),
                    model_data_bundle_version: "models.fixture".into(),
                    model_data_digest: CanonicalDigest::of_bytes(b"models"),
                    cross_reference_digest: CanonicalDigest::of_bytes(b"cross"),
                },
                scanner: ComputeScannerEvidenceV1 {
                    scanner_id: "scanner.fixture".into(),
                    scanner_version: "1".into(),
                    discovered_source_ref: "source-ref.fixture".into(),
                    configuration_revision: 1,
                    evidence_digest: CanonicalDigest::of_bytes(b"scanner"),
                },
            },
        }
    }

    #[test]
    fn compute_projection_binds_every_non_secret_subresource() {
        let projection = projection();
        projection.validate().unwrap();
        let mut drift = projection;
        drift.desired.inventory.observed_models[0].upstream_model_id = "other".into();
        drift.desired.inventory.inventory_digest =
            CanonicalDigest::of(&drift.desired.inventory.observed_models).unwrap();
        assert!(drift.validate().is_err());
    }
}
