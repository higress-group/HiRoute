use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::CanonicalDigest;

use super::MODEL_DATA_SCHEMA_V1;
use super::common::*;
use super::price::{PriceRateV1, validate_rate_sets, validate_schedule};
use super::registry::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDefinitionV1 {
    pub model_configuration_id: String,
    pub revision: u64,
    pub display_name: String,
    pub publisher_id: String,
    pub capabilities: CapabilityFactsV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityFactsV1 {
    pub tool: bool,
    pub vision: bool,
    pub streaming: bool,
    pub context_tokens: u64,
    pub max_output_tokens: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelEndpointCapabilityV1 {
    pub capability_id: String,
    pub revision: u64,
    pub model_configuration_id: String,
    pub connector_id: String,
    pub connector_revision: u64,
    pub endpoint_profile_id: String,
    pub endpoint_profile_revision: u64,
    pub protocol_endpoint_id: String,
    pub upstream_protocol: UpstreamProtocol,
    pub upstream_model_id: String,
    pub required_adapter_ref: String,
    pub required_adapter_revision: u64,
    pub evidence_digest: CanonicalDigest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RatingV1 {
    pub model_configuration_id: String,
    pub overall_score_tenths: u8,
    pub rating_count: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OfferV1 {
    pub offer_id: String,
    pub revision: u64,
    pub endpoint_profile_id: String,
    pub endpoint_profile_revision: u64,
    pub service_offering_id: String,
    pub entitlement_id: String,
    pub usage_scope: String,
    pub region_id: String,
    pub model_configuration_ids: Vec<String>,
    pub billing_class: BillingClass,
    pub evidence_digest: CanonicalDigest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FreeOfferV1 {
    pub free_offer_id: String,
    pub revision: u64,
    pub connection_option_id: String,
    pub offer_ref: String,
    pub model_configuration_ids: Vec<String>,
    pub access: FreeAccess,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct_verification_evidence: Option<String>,
    pub last_verified_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDataBundleV1 {
    pub schema: String,
    pub bundle_version: String,
    pub product_release: String,
    pub connector_registry_version: String,
    pub models_slice_version: String,
    pub capability_slice_version: String,
    pub ratings_slice_version: String,
    pub prices_slice_version: String,
    pub free_offers_slice_version: String,
    pub models: Vec<ModelDefinitionV1>,
    pub model_endpoint_capabilities: Vec<ModelEndpointCapabilityV1>,
    pub ratings: Vec<RatingV1>,
    pub offers: Vec<OfferV1>,
    pub free_offers: Vec<FreeOfferV1>,
    pub price_rates: Vec<PriceRateV1>,
}

impl ModelDataBundleV1 {
    pub fn validate_against(
        &self,
        registry: &ConnectorRegistryBundleV1,
    ) -> Result<(), ComputeContractError> {
        self.validate_against_with_seed_ratings(registry, &BTreeSet::new())
    }

    pub(super) fn validate_against_with_seed_ratings(
        &self,
        registry: &ConnectorRegistryBundleV1,
        seed_rating_ids: &BTreeSet<&str>,
    ) -> Result<(), ComputeContractError> {
        registry.validate()?;
        if self.schema != MODEL_DATA_SCHEMA_V1 {
            return Err(ComputeContractError::UnsupportedSchema);
        }
        if self.product_release != registry.product_release
            || self.connector_registry_version != registry.registry_version
        {
            return Err(ComputeContractError::MixedReleaseSlice);
        }
        for id in [
            &self.bundle_version,
            &self.product_release,
            &self.connector_registry_version,
            &self.models_slice_version,
            &self.capability_slice_version,
            &self.ratings_slice_version,
            &self.prices_slice_version,
            &self.free_offers_slice_version,
        ] {
            validate_identifier(id)?;
        }
        ensure_unique(
            self.models
                .iter()
                .map(|value| value.model_configuration_id.as_str()),
        )?;
        ensure_unique(
            self.model_endpoint_capabilities
                .iter()
                .map(|value| value.capability_id.as_str()),
        )?;
        ensure_unique(self.offers.iter().map(|value| value.offer_id.as_str()))?;
        ensure_unique(
            self.free_offers
                .iter()
                .map(|value| value.free_offer_id.as_str()),
        )?;
        ensure_unique(
            self.price_rates
                .iter()
                .map(|value| value.price_rate_id.as_str()),
        )?;
        ensure_unique(
            self.ratings
                .iter()
                .map(|value| value.model_configuration_id.as_str()),
        )?;
        for model in &self.models {
            validate_identifier(&model.model_configuration_id)?;
            validate_identifier(&model.publisher_id)?;
            if model.revision == 0
                || !valid_display_name(&model.display_name)
                || model.capabilities.context_tokens == 0
                || model.capabilities.max_output_tokens == 0
            {
                return Err(ComputeContractError::InvalidModelData);
            }
        }
        for capability in &self.model_endpoint_capabilities {
            validate_identifier(&capability.capability_id)?;
            let connector = registry
                .connector(&capability.connector_id)
                .ok_or(ComputeContractError::CrossReference)?;
            let profile = registry
                .endpoint_profile(&capability.endpoint_profile_id)
                .ok_or(ComputeContractError::CrossReference)?;
            let endpoint = profile
                .protocol_endpoints
                .iter()
                .find(|endpoint| endpoint.protocol_endpoint_id == capability.protocol_endpoint_id);
            if capability.revision == 0
                || capability.required_adapter_revision == 0
                || self.model(&capability.model_configuration_id).is_none()
                || connector.revision != capability.connector_revision
                || profile.revision != capability.endpoint_profile_revision
                || !connector
                    .endpoint_profile_refs
                    .contains(&capability.endpoint_profile_id)
                || endpoint.is_none_or(|endpoint| {
                    endpoint.protocol != capability.upstream_protocol
                        || endpoint.adapter_ref != capability.required_adapter_ref
                        || endpoint.adapter_revision != capability.required_adapter_revision
                })
            {
                return Err(ComputeContractError::CrossReference);
            }
            super::common::validate_upstream_model_id(&capability.upstream_model_id)?;
            validate_identifier(&capability.required_adapter_ref)?;
            validate_nonempty_digest(&capability.evidence_digest)?;
        }
        let mut inventory_identity = BTreeMap::<(&str, &str), &str>::new();
        for capability in &self.model_endpoint_capabilities {
            let key = (
                capability.endpoint_profile_id.as_str(),
                capability.upstream_model_id.as_str(),
            );
            if inventory_identity
                .insert(key, capability.model_configuration_id.as_str())
                .is_some_and(|existing| existing != capability.model_configuration_id)
            {
                return Err(ComputeContractError::InvalidModelData);
            }
        }
        for rating in &self.ratings {
            if self.model(&rating.model_configuration_id).is_none()
                || !(5..=50).contains(&rating.overall_score_tenths)
                || (rating.rating_count == 0
                    && !seed_rating_ids.contains(rating.model_configuration_id.as_str()))
            {
                return Err(ComputeContractError::InvalidModelData);
            }
        }
        for offer in &self.offers {
            validate_identifier(&offer.offer_id)?;
            validate_identifier(&offer.endpoint_profile_id)?;
            for id in [
                &offer.service_offering_id,
                &offer.entitlement_id,
                &offer.usage_scope,
                &offer.region_id,
            ] {
                validate_identifier(id)?;
            }
            let profile = registry.endpoint_profile(&offer.endpoint_profile_id);
            if offer.revision == 0
                || !offer.billing_class.is_runnable()
                || profile.is_none_or(|profile| {
                    profile.revision != offer.endpoint_profile_revision
                        || profile.service_offering_id != offer.service_offering_id
                        || profile.entitlement_id != offer.entitlement_id
                        || profile.usage_scope != offer.usage_scope
                        || profile.region_id != offer.region_id
                })
                || offer.model_configuration_ids.is_empty()
                || ensure_unique(offer.model_configuration_ids.iter().map(String::as_str)).is_err()
                || offer
                    .model_configuration_ids
                    .iter()
                    .any(|id| self.model(id).is_none())
            {
                return Err(ComputeContractError::CrossReference);
            }
            validate_nonempty_digest(&offer.evidence_digest)?;
        }
        for free in &self.free_offers {
            validate_identifier(&free.free_offer_id)?;
            validate_identifier(&free.connection_option_id)?;
            validate_identifier(&free.offer_ref)?;
            let offer = self
                .offer(&free.offer_ref)
                .ok_or(ComputeContractError::CrossReference)?;
            if free.revision == 0
                || free.last_verified_at <= 0
                || offer.billing_class != BillingClass::Free
                || free.model_configuration_ids.is_empty()
                || ensure_unique(free.model_configuration_ids.iter().map(String::as_str)).is_err()
                || free
                    .model_configuration_ids
                    .iter()
                    .any(|id| !offer.model_configuration_ids.contains(id))
                || free.model_configuration_ids.iter().any(|id| {
                    !self.model_endpoint_capabilities.iter().any(|capability| {
                        capability.model_configuration_id == *id
                            && capability.endpoint_profile_id == offer.endpoint_profile_id
                    })
                })
            {
                return Err(ComputeContractError::CrossReference);
            }
            match (&free.access, &free.direct_verification_evidence) {
                (FreeAccess::Direct, Some(evidence)) => validate_evidence(evidence)?,
                (FreeAccess::Direct, None) | (FreeAccess::ApiKeyRequired, Some(_)) => {
                    return Err(ComputeContractError::UnverifiedDirectOffer);
                }
                _ => {}
            }
            let registry_option = registry.connection_option(&free.connection_option_id);
            if registry_option.is_none_or(|option| {
                let authentication = registry
                    .connector(&option.connector_id)
                    .map(|connector| connector.authentication);
                option.billing_class != BillingClass::Free
                    || option.free_offer_ref.as_deref() != Some(free.free_offer_id.as_str())
                    || option.endpoint_profile_id != offer.endpoint_profile_id
                    || match free.access {
                        FreeAccess::Direct => authentication != Some(AuthenticationKind::None),
                        FreeAccess::ApiKeyRequired => {
                            authentication != Some(AuthenticationKind::ProviderApiKey)
                        }
                    }
                    || (free.access == FreeAccess::Direct
                        && option.direct_verification_evidence != free.direct_verification_evidence)
                    || (free.access == FreeAccess::ApiKeyRequired
                        && option.direct_verification_evidence.is_some())
            }) {
                return Err(ComputeContractError::CrossReference);
            }
        }
        for option in registry
            .connection_options
            .iter()
            .filter(|option| option.billing_class == BillingClass::Free)
        {
            if option
                .free_offer_ref
                .as_deref()
                .is_none_or(|free_offer_ref| {
                    !self
                        .free_offers
                        .iter()
                        .any(|free| free.free_offer_id == free_offer_ref)
                })
            {
                return Err(ComputeContractError::CrossReference);
            }
        }
        for rate in &self.price_rates {
            validate_identifier(&rate.price_rate_id)?;
            validate_identifier(&rate.offer_ref)?;
            validate_identifier(&rate.model_configuration_id)?;
            let offer = self
                .offer(&rate.offer_ref)
                .ok_or(ComputeContractError::CrossReference)?;
            if rate.revision == 0
                || !offer
                    .model_configuration_ids
                    .contains(&rate.model_configuration_id)
                || rate.currency.len() != 3
                || !rate.currency.bytes().all(|byte| byte.is_ascii_uppercase())
            {
                return Err(ComputeContractError::InvalidPrice);
            }
            if let Some(schedule) = &rate.schedule {
                validate_schedule(schedule)?;
            }
        }
        // Freeze a single reference base and non-overlapping scheduled segments for each exact
        // Offer/ModelConfiguration/currency tuple before the bundle can become trusted data.
        validate_rate_sets(&self.price_rates)?;
        Ok(())
    }

    pub fn model(&self, id: &str) -> Option<&ModelDefinitionV1> {
        self.models
            .iter()
            .find(|value| value.model_configuration_id == id)
    }

    pub fn offer(&self, id: &str) -> Option<&OfferV1> {
        self.offers.iter().find(|value| value.offer_id == id)
    }
}
