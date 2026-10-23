//! Client-bundled ReleaseFacts validator and deterministic built-in Agent profile projection.

use hiroute_domain::{
    AGENT_PROFILES_ARTIFACT_SCHEMA_V1, AgentProfilesArtifactV1, CanonicalDigest,
    ConnectorRegistryBundleV1, MAX_RELEASE_BUNDLE_BYTES, RELEASE_FACTS_TOOL_VERSION_V2,
    ReleaseFactsManifestV2,
};

use crate::{ReleaseVerificationError, TrustedReleaseCatalog, builtin_agent_profiles};

pub const MAX_RELEASE_FACTS_MANIFEST_BYTES: usize = 16 * 1024;

pub fn builtin_agent_profiles_artifact() -> AgentProfilesArtifactV1 {
    let mut profiles = builtin_agent_profiles();
    profiles.sort_by(|left, right| left.profile_id.cmp(&right.profile_id));
    AgentProfilesArtifactV1 {
        schema: AGENT_PROFILES_ARTIFACT_SCHEMA_V1.to_owned(),
        tool_version: RELEASE_FACTS_TOOL_VERSION_V2.to_owned(),
        profiles,
    }
}

impl TrustedReleaseCatalog {
    /// Accepts only the exact manifest compiled into the same client, then validates every
    /// referenced byte sequence before exposing registry authority or model metadata.
    pub fn load_bundled_release_facts(
        expected_manifest_bytes: &[u8],
        manifest_bytes: &[u8],
        registry_bytes: &[u8],
        model_data_bytes: &[u8],
    ) -> Result<Self, ReleaseVerificationError> {
        if manifest_bytes != expected_manifest_bytes {
            return Err(ReleaseVerificationError::ManifestMismatch);
        }
        Self::load_release_facts(manifest_bytes, registry_bytes, model_data_bytes)
    }

    /// Validates a complete current ReleaseFacts bundle. Product code must use
    /// [`Self::load_bundled_release_facts`] so the manifest is bound to the client revision.
    pub fn load_release_facts(
        manifest_bytes: &[u8],
        registry_bytes: &[u8],
        model_data_bytes: &[u8],
    ) -> Result<Self, ReleaseVerificationError> {
        if manifest_bytes.len() > MAX_RELEASE_FACTS_MANIFEST_BYTES
            || registry_bytes.len() > MAX_RELEASE_BUNDLE_BYTES
            || model_data_bytes.len() > MAX_RELEASE_BUNDLE_BYTES
        {
            return Err(ReleaseVerificationError::BundleTooLarge);
        }

        let manifest: ReleaseFactsManifestV2 = serde_json::from_slice(manifest_bytes)
            .map_err(|_| ReleaseVerificationError::MalformedBundle)?;
        manifest
            .validate_shape()
            .map_err(map_manifest_contract_error)?;

        let registry_digest = CanonicalDigest::of_bytes(registry_bytes);
        let model_data_digest = CanonicalDigest::of_bytes(model_data_bytes);
        if registry_digest != manifest.connector_registry_digest
            || model_data_digest != manifest.model_data_digest
        {
            return Err(ReleaseVerificationError::DigestMismatch);
        }

        let registry: ConnectorRegistryBundleV1 = serde_json::from_slice(registry_bytes)
            .map_err(|_| ReleaseVerificationError::MalformedBundle)?;
        let data: hiroute_domain::ReleaseModelDataBundleV2 =
            serde_json::from_slice(model_data_bytes)
                .map_err(|_| ReleaseVerificationError::MalformedBundle)?;
        let cross_reference_digest = data
            .cross_reference_digest(&registry)
            .map_err(ReleaseVerificationError::Contract)?;
        let model_data = data.data.clone();
        if manifest.product_release != registry.product_release
            || manifest.product_release != model_data.product_release
        {
            return Err(ReleaseVerificationError::Contract(
                hiroute_domain::ComputeContractError::MixedReleaseSlice,
            ));
        }
        if cross_reference_digest != manifest.cross_reference_digest {
            return Err(ReleaseVerificationError::CrossReferenceDigestMismatch);
        }

        let resolved_options = registry
            .validated_options()
            .map_err(ReleaseVerificationError::Contract)?;
        Ok(Self {
            registry,
            resolved_options,
            model_data,
            registry_catalog_id: manifest.catalog_id.clone(),
            model_data_catalog_id: manifest.catalog_id.clone(),
            registry_sequence: manifest.sequence,
            model_data_sequence: manifest.sequence,
            registry_digest,
            model_data_digest,
            release_facts: manifest,
            current_release_model_data: data,
            feed_facts: None,
            official_snapshot: None,
        })
    }
}

fn map_manifest_contract_error(
    error: hiroute_domain::ComputeContractError,
) -> ReleaseVerificationError {
    match error {
        hiroute_domain::ComputeContractError::UnsupportedSchema => {
            ReleaseVerificationError::UnsupportedSchema
        }
        _ => ReleaseVerificationError::Contract(error),
    }
}
