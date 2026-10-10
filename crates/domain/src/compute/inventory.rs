use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::common::*;
use super::model_data::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedModelV1 {
    pub upstream_model_id: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

impl ObservedModelV1 {
    pub fn validate(&self) -> Result<(), ComputeContractError> {
        super::common::validate_upstream_model_id(&self.upstream_model_id)?;
        validate_inventory_metadata(&self.metadata)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryDisposition {
    /// The observed identifier matches a trusted ModelData capability. This is still only an
    /// inventory fact; Source, Binding, CredentialPool, and RuntimeState readiness are separate.
    CatalogMatched,
    InventoryOnly,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveInventoryModelV1 {
    pub upstream_model_id: String,
    pub disposition: InventoryDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_configuration_id: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

pub fn reconcile_inventory(
    endpoint_profile_id: &str,
    observed: impl IntoIterator<Item = ObservedModelV1>,
    model_data: &ModelDataBundleV1,
) -> Result<Vec<EffectiveInventoryModelV1>, ComputeContractError> {
    validate_identifier(endpoint_profile_id)?;
    let mut deduplicated = BTreeMap::<String, BTreeMap<String, String>>::new();
    for model in observed {
        model.validate()?;
        let metadata = deduplicated.entry(model.upstream_model_id).or_default();
        for (key, value) in model.metadata {
            if metadata
                .insert(key, value.clone())
                .is_some_and(|existing| existing != value)
            {
                return Err(ComputeContractError::ConflictingInventory);
            }
        }
    }
    Ok(deduplicated
        .into_iter()
        .map(|(upstream_model_id, metadata)| {
            let model_configuration_id = model_data
                .model_endpoint_capabilities
                .iter()
                .find(|capability| {
                    capability.endpoint_profile_id == endpoint_profile_id
                        && capability.upstream_model_id == upstream_model_id
                })
                .map(|capability| capability.model_configuration_id.clone());
            EffectiveInventoryModelV1 {
                upstream_model_id,
                disposition: if model_configuration_id.is_some() {
                    InventoryDisposition::CatalogMatched
                } else {
                    InventoryDisposition::InventoryOnly
                },
                model_configuration_id,
                metadata,
            }
        })
        .collect())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelQueryItemV1 {
    pub model_configuration_id: String,
    pub display_name: String,
    /// True only when provider inventory observed a matching trusted catalog model. It never
    /// claims that a materialized Binding is runnable.
    pub observed_in_inventory: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overall_score_tenths: Option<u8>,
}

pub fn list_models(
    model_data: &ModelDataBundleV1,
    inventory: &[EffectiveInventoryModelV1],
) -> Vec<ModelQueryItemV1> {
    let observed = inventory
        .iter()
        .filter(|item| item.disposition == InventoryDisposition::CatalogMatched)
        .filter_map(|item| item.model_configuration_id.as_deref())
        .collect::<BTreeSet<_>>();
    let ratings = model_data
        .ratings
        .iter()
        .map(|rating| {
            (
                rating.model_configuration_id.as_str(),
                rating.overall_score_tenths,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut values = model_data
        .models
        .iter()
        .map(|model| ModelQueryItemV1 {
            model_configuration_id: model.model_configuration_id.clone(),
            display_name: model.display_name.clone(),
            observed_in_inventory: observed.contains(model.model_configuration_id.as_str()),
            overall_score_tenths: ratings.get(model.model_configuration_id.as_str()).copied(),
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .observed_in_inventory
            .cmp(&left.observed_in_inventory)
            .then_with(|| right.overall_score_tenths.cmp(&left.overall_score_tenths))
            .then_with(|| {
                left.model_configuration_id
                    .cmp(&right.model_configuration_id)
            })
    });
    values
}

pub fn show_model<'a>(
    model_data: &'a ModelDataBundleV1,
    model_configuration_id: &str,
) -> Option<&'a ModelDefinitionV1> {
    model_data.model(model_configuration_id)
}

pub fn list_free_models(model_data: &ModelDataBundleV1) -> Vec<FreeOfferV1> {
    let mut values = model_data.free_offers.clone();
    values.sort_by(|left, right| left.free_offer_id.cmp(&right.free_offer_id));
    values
}

fn validate_inventory_metadata(
    metadata: &BTreeMap<String, String>,
) -> Result<(), ComputeContractError> {
    if metadata.len() > 32 {
        return Err(ComputeContractError::InvalidInventory);
    }
    for (key, value) in metadata {
        validate_identifier(key)?;
        if matches!(
            key.as_str(),
            "auth"
                | "authentication"
                | "base_url"
                | "billing_class"
                | "capabilities"
                | "connection_option_id"
                | "connector_id"
                | "endpoint_profile_id"
                | "endpoint_url"
                | "host"
                | "offer_ref"
                | "protocol"
                | "redirect"
                | "source_identity"
        ) || key.len() > 64
            || value.len() > 1_024
            || value.bytes().any(|byte| byte == 0 || !byte.is_ascii())
        {
            return Err(ComputeContractError::InvalidInventory);
        }
    }
    Ok(())
}
