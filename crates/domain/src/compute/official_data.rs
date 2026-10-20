//! Replaceable public facts. Installed registry and free-offer authority are never serialized here.
use super::*;
use crate::CanonicalDigest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const OFFICIAL_MODEL_DATA_SCHEMA: &str = "hiroute.official-model-data/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialModelDataV1 {
    pub schema: String,
    pub models: Vec<ModelDefinitionV1>,
    pub model_endpoint_capabilities: Vec<ModelEndpointCapabilityV1>,
    pub offers: Vec<OfferV1>,
    pub price_rates: Vec<PriceRateV1>,
    pub rating_snapshot: RatingSnapshotV2,
    pub metadata_catalog: ModelMetadataCatalogV1,
    pub pricing_facts: Vec<DocumentedModelPriceV1>,
    pub connection_hints: Vec<OfficialConnectionHintV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialModelSnapshotV1 {
    pub data_revision: u64,
    pub data_digest: CanonicalDigest,
    pub data: OfficialModelDataV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentedModelPriceV1 {
    pub pricing_key: String,
    pub product_key: String,
    pub model_key: String,
    pub upstream_model_id: String,
    pub currency: String,
    pub unit: String,
    pub price_state: String,
    pub schedules: Vec<DocumentedPriceScheduleV1>,
    pub evidence_refs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentedPriceScheduleV1 {
    pub schedule_key: String,
    /// Conditions remain descriptive until the valuation engine proves it can evaluate them.
    pub condition: BTreeMap<String, Value>,
    pub application: String,
    pub rates: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialConnectionHintV1 {
    pub connection_option_id: String,
    pub provider_id: String,
    pub name: BTreeMap<String, String>,
    pub description: BTreeMap<String, String>,
    pub documentation_url: String,
}

impl OfficialModelSnapshotV1 {
    pub fn validate(&self) -> Result<(), ComputeContractError> {
        self.data.validate()?;
        if self.data_revision == 0
            || CanonicalDigest::of(&self.data)
                .map_err(|_| ComputeContractError::InvalidModelData)?
                != self.data_digest
        {
            return Err(ComputeContractError::InvalidModelData);
        }
        Ok(())
    }
}

fn unique<'a>(
    ids: impl Iterator<Item = &'a str>,
) -> Result<BTreeSet<&'a str>, ComputeContractError> {
    let mut result = BTreeSet::new();
    for id in ids {
        if id.is_empty() || id.len() > 256 || !result.insert(id) {
            return Err(ComputeContractError::InvalidModelData);
        }
    }
    Ok(result)
}

impl OfficialModelDataV1 {
    pub fn from_release(data: &ReleaseModelDataBundleV2) -> Self {
        Self {
            schema: OFFICIAL_MODEL_DATA_SCHEMA.into(),
            models: data.data.models.clone(),
            model_endpoint_capabilities: data.data.model_endpoint_capabilities.clone(),
            offers: data.data.offers.clone(),
            price_rates: data.data.price_rates.clone(),
            rating_snapshot: data.rating_snapshot.clone(),
            metadata_catalog: data.metadata_catalog.clone(),
            pricing_facts: vec![],
            connection_hints: vec![],
        }
    }

    /// Internal closure is independent of whether this client implements an external adapter.
    pub fn validate(&self) -> Result<(), ComputeContractError> {
        if self.schema != OFFICIAL_MODEL_DATA_SCHEMA
            || self.models.len() > 16_384
            || self.model_endpoint_capabilities.len() > 16_384
            || self.offers.len() > 16_384
            || self.price_rates.len() > 50_000
            || self.pricing_facts.len() > 50_000
            || self.connection_hints.len() > 1_000
        {
            return Err(ComputeContractError::InvalidModelData);
        }
        self.metadata_catalog.validate()?;
        self.rating_snapshot.validate()?;
        let models = unique(
            self.models
                .iter()
                .map(|m| m.model_configuration_id.as_str()),
        )?;
        unique(
            self.model_endpoint_capabilities
                .iter()
                .map(|c| c.capability_id.as_str()),
        )?;
        let offers = unique(self.offers.iter().map(|o| o.offer_id.as_str()))?;
        unique(self.price_rates.iter().map(|p| p.price_rate_id.as_str()))?;
        if self.models.iter().any(|m| {
            m.revision == 0
                || m.display_name.trim().is_empty()
                || m.capabilities.context_tokens == 0
                || m.capabilities.max_output_tokens == 0
        }) || self.model_endpoint_capabilities.iter().any(|c| {
            c.revision == 0
                || c.required_adapter_revision == 0
                || !models.contains(c.model_configuration_id.as_str())
        }) || self.offers.iter().any(|o| {
            o.revision == 0
                || o.model_configuration_ids.is_empty()
                || o.model_configuration_ids
                    .iter()
                    .any(|id| !models.contains(id.as_str()))
        }) || self.price_rates.iter().any(|p| {
            !models.contains(p.model_configuration_id.as_str())
                || !offers.contains(p.offer_ref.as_str())
        }) {
            return Err(ComputeContractError::CrossReference);
        }
        let native = unique(
            self.rating_snapshot
                .models
                .iter()
                .map(|m| m.model_configuration_id.as_str()),
        )?;
        if models != native
            || CanonicalDigest::of(&self.models)
                .map_err(|_| ComputeContractError::InvalidModelData)?
                != self.rating_snapshot.model_catalog_digest
        {
            return Err(ComputeContractError::CrossReference);
        }
        let metadata = &self.metadata_catalog;
        let evidence: BTreeSet<_> = metadata
            .evidence_sources
            .iter()
            .map(|v| v.source_key.as_str())
            .collect();
        let products: BTreeSet<_> = metadata
            .access_products
            .iter()
            .map(|v| v.product_key.as_str())
            .collect();
        let canonical: BTreeSet<_> = metadata
            .canonical_models
            .iter()
            .map(|v| v.model_key.as_str())
            .collect();
        unique(self.pricing_facts.iter().map(|p| p.pricing_key.as_str()))?;
        unique(
            self.connection_hints
                .iter()
                .map(|h| h.connection_option_id.as_str()),
        )?;
        let mut price_scopes = BTreeSet::new();
        for price in &self.pricing_facts {
            if !price_scopes.insert((
                &price.product_key,
                &price.model_key,
                &price.upstream_model_id,
                &price.currency,
            )) {
                return Err(ComputeContractError::DuplicateIdentity);
            }
            if !products.contains(price.product_key.as_str())
                || !canonical.contains(price.model_key.as_str())
                || price.currency.len() != 3
                || !price.currency.bytes().all(|c| c.is_ascii_uppercase())
                || price.unit != "per-1m-tokens"
                || price.schedules.len() > 64
                || price.evidence_refs.is_empty()
                || price
                    .evidence_refs
                    .iter()
                    .any(|e| !evidence.contains(e.as_str()))
            {
                return Err(ComputeContractError::CrossReference);
            }
            unique(price.schedules.iter().map(|s| s.schedule_key.as_str()))?;
            for schedule in &price.schedules {
                if schedule
                    .condition
                    .get("kind")
                    .and_then(Value::as_str)
                    .is_none()
                    || schedule.application.is_empty()
                    || schedule.rates.is_empty()
                    || schedule.rates.values().any(|rate| !valid_decimal(rate))
                {
                    return Err(ComputeContractError::InvalidModelData);
                }
            }
        }
        for hint in &self.connection_hints {
            if !hint.documentation_url.starts_with("https://")
                || hint.name.is_empty()
                || hint.provider_id.is_empty()
            {
                return Err(ComputeContractError::InvalidModelData);
            }
        }
        Ok(())
    }
}

fn valid_decimal(value: &str) -> bool {
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or("");
    let fraction = parts.next();
    !whole.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && fraction.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
        && parts.next().is_none()
        && value.len() <= 64
}
