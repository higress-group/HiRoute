use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::CanonicalDigest;

use super::ComputeContractError;
use super::model_metadata_records::{
    MetadataReasoningRenderingHintsV1, ModelCapabilityHintsV1, ModelMetadataRecordV1,
    ProviderMetadataRecordV1,
};

pub const MODEL_METADATA_CATALOG_SCHEMA_V1: &str = "hiroute.model-metadata-catalog/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataCompletenessV1 {
    Complete,
    Partial,
    Missing,
    /// The domain does not apply to this provider or model, which is a determinate outcome
    /// distinct from an uncollected `missing` field.
    NotApplicable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataCapabilityStateV1 {
    Supported,
    Unsupported,
    Conditional,
    Unknown,
    NotApplicable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataTokenStateV1 {
    Conflict,
    Known,
    Unknown,
    RuntimeRequired,
    SharedTotalBudget,
    EntitlementDependent,
    NotApplicable,
}

/// The explicit execution outcome of a provider-scoped model record against the native
/// text contract. It never grants runtime eligibility: endpoint, credential, and
/// protocol qualification stay separate.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataExecutionFitStateV1 {
    NativeTextRepresentable,
    Unsupported,
    NotApplicable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataExecutionFitV1 {
    pub state: MetadataExecutionFitStateV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataCostHintStateV1 {
    NotRecorded,
    Recorded,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataReasoningRenderingStateV1 {
    NotApplicable,
}

/// A single field that was closed by a reviewed inference rule instead of an observed
/// fact. `basis` is always `inferred`, so a rule-closed value can never be read as a
/// provider-verified assertion.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataFieldProvenanceV1 {
    pub basis: MetadataInferenceBasisV1,
    pub rule_key: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataInferenceBasisV1 {
    Inferred,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataInferenceRecordKindV1 {
    Model,
    Provider,
}

/// The audited shape of one inference rule that closed provider-scoped fields. The
/// deterministic match and assignment table stays in the maintained catalog; this record
/// keeps the rule identity, reason, and evidence inspectable next to the closed values.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataInferenceRuleV1 {
    pub rule_key: String,
    pub record_kind: MetadataInferenceRecordKindV1,
    pub basis: MetadataInferenceBasisV1,
    pub reason: String,
    pub evidence_refs: Vec<String>,
    pub collected_on: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataAuthenticationHintV1 {
    ApiKey,
    AwsSdk,
    Copilot,
    ExternalProcess,
    OauthDeviceCode,
    OauthExternal,
    Vertex,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MetadataUsageScenarioV1 {
    AdapterPlanning,
    ContextLimitPrefill,
    CostHintDisplay,
    CredentialSetupHint,
    CustomApiEndpointPrefill,
    CustomApiModelPrefill,
    LifecycleWarning,
    ModelDirectory,
    ModelDiscoverySetup,
    OutputLimitPrefill,
    ProviderDirectory,
    ReasoningCapabilityPrefill,
    ReasoningRenderingHint,
    RequestLimitPrefill,
    VisionCapabilityPrefill,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataTokenLimitV1 {
    pub state: MetadataTokenStateV1,
    pub value: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataProvenanceRefV1 {
    pub kind: String,
    pub source_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataEvidenceSourceV1 {
    pub source_key: String,
    pub authority: String,
    pub source_kind: String,
    pub locator: String,
    pub collected_on: String,
    pub bytes_digest: Option<CanonicalDigest>,
    pub digest_state: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataClientRunV1 {
    pub source_key: String,
    pub profile_digest: CanonicalDigest,
    pub repository_url: String,
    pub commit: String,
    pub tree: String,
    pub result_digest: CanonicalDigest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataProductInterfaceV1 {
    pub interface_key: String,
    pub protocol: String,
    pub base_url: Option<String>,
    pub base_url_template: Option<String>,
    pub request_path: Option<String>,
    pub request_path_template: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataAccessProductV1 {
    pub product_key: String,
    pub provider: String,
    pub product: String,
    pub product_kind: String,
    pub region_scope: String,
    pub billing: String,
    pub restriction: String,
    pub credential: String,
    pub interfaces: Vec<MetadataProductInterfaceV1>,
    pub documented_upstream_model_ids: Vec<String>,
    pub quota_facts: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub disposition: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataLimitVariantV1 {
    pub upstream_id: String,
    pub tokens: u64,
    pub condition: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataReasoningHintV1 {
    pub kind: String,
    pub profiles: Vec<String>,
    pub default: Option<String>,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataCanonicalModelV1 {
    pub model_key: String,
    pub publisher: String,
    pub display_name: String,
    pub canonical_identity: String,
    pub upstream_ids: Vec<String>,
    pub upstream_id_state: Option<String>,
    pub context_tokens: MetadataTokenLimitV1,
    pub context_variants: Vec<MetadataLimitVariantV1>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: MetadataTokenLimitV1,
    pub max_output_scope: Option<String>,
    pub max_output_note: Option<String>,
    pub modalities: BTreeMap<String, MetadataCapabilityStateV1>,
    pub capabilities: BTreeMap<String, MetadataCapabilityStateV1>,
    pub reasoning: MetadataReasoningHintV1,
    pub identity_stability: String,
    pub lifecycle: String,
    pub source_stability_note: String,
    pub evidence_refs: Vec<String>,
    pub data_note: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataAvailabilityV1 {
    pub state: String,
    pub condition: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataEndpointBindingV1 {
    pub binding_key: String,
    pub product_key: String,
    pub model_key: String,
    pub upstream_model_id: String,
    pub upstream_identity_role: Option<String>,
    pub lifecycle: Option<String>,
    pub replaced_by_upstream_id: Option<String>,
    pub interface_candidates: Vec<String>,
    pub availability: MetadataAvailabilityV1,
    pub protocol_qualification: String,
    #[serde(default)]
    pub capability_overrides: MetadataBindingCapabilityOverridesV1,
    pub evidence_refs: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataBindingCapabilityOverridesV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vision: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streaming: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataDynamicRouteV1 {
    pub route_key: String,
    pub product_key: String,
    pub upstream_route_ids: Vec<String>,
    pub identity_stability: String,
    pub rated_model_identity: Option<String>,
    pub require_actual_response_model: bool,
    pub require_actual_response_provider: bool,
    pub evidence_refs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelMetadataCatalogV1 {
    pub schema: String,
    pub as_of: String,
    pub source_catalog_digest: CanonicalDigest,
    pub evidence_sources: Vec<MetadataEvidenceSourceV1>,
    pub client_runs: Vec<MetadataClientRunV1>,
    pub access_products: Vec<MetadataAccessProductV1>,
    pub canonical_models: Vec<MetadataCanonicalModelV1>,
    pub endpoint_bindings: Vec<MetadataEndpointBindingV1>,
    pub dynamic_routes: Vec<MetadataDynamicRouteV1>,
    pub inference_rules: Vec<MetadataInferenceRuleV1>,
    pub provider_records: Vec<ProviderMetadataRecordV1>,
    pub model_records: Vec<ModelMetadataRecordV1>,
}

impl ModelMetadataCatalogV1 {
    pub fn validate(&self) -> Result<(), ComputeContractError> {
        if self.schema != MODEL_METADATA_CATALOG_SCHEMA_V1
            || !valid_date(&self.as_of)
            || self.source_catalog_digest == CanonicalDigest::of_bytes(&[])
            || self.evidence_sources.len() > 1_000
            || self.client_runs.len() > 64
            || self.access_products.len() > 1_000
            || self.canonical_models.len() > 10_000
            || self.endpoint_bindings.len() > 50_000
            || self.dynamic_routes.len() > 1_000
            || self.inference_rules.len() > 10_000
            || self.provider_records.len() > 10_000
            || self.model_records.len() > 100_000
        {
            return Err(ComputeContractError::InvalidModelData);
        }
        unique_sorted(self.evidence_sources.iter().map(|v| v.source_key.as_str()))?;
        unique_sorted(self.client_runs.iter().map(|v| v.source_key.as_str()))?;
        unique_sorted(self.access_products.iter().map(|v| v.product_key.as_str()))?;
        unique_sorted(self.canonical_models.iter().map(|v| v.model_key.as_str()))?;
        unique_sorted(
            self.endpoint_bindings
                .iter()
                .map(|v| v.binding_key.as_str()),
        )?;
        unique_sorted(self.dynamic_routes.iter().map(|v| v.route_key.as_str()))?;
        unique_sorted(self.inference_rules.iter().map(|v| v.rule_key.as_str()))?;
        unique_sorted(
            self.provider_records
                .iter()
                .map(|v| v.provider_record_key.as_str()),
        )?;
        unique_sorted(
            self.model_records
                .iter()
                .map(|v| v.model_record_key.as_str()),
        )?;

        let evidence = self
            .evidence_sources
            .iter()
            .map(|v| v.source_key.as_str())
            .collect::<BTreeSet<_>>();
        let runs = self
            .client_runs
            .iter()
            .map(|v| v.source_key.as_str())
            .collect::<BTreeSet<_>>();
        let products = self
            .access_products
            .iter()
            .map(|value| (value.product_key.as_str(), value))
            .collect::<BTreeMap<_, _>>();
        let models = self
            .canonical_models
            .iter()
            .map(|value| (value.model_key.as_str(), value))
            .collect::<BTreeMap<_, _>>();
        let providers = self
            .provider_records
            .iter()
            .map(|value| (value.provider_record_key.as_str(), value))
            .collect::<BTreeMap<_, _>>();
        let rules = self
            .inference_rules
            .iter()
            .map(|value| (value.rule_key.as_str(), value))
            .collect::<BTreeMap<_, _>>();

        for source in &self.evidence_sources {
            if !valid_text(&source.source_key, 256)
                || !valid_text(&source.authority, 256)
                || !valid_text(&source.source_kind, 128)
                || !valid_text(&source.locator, 2_048)
                || !source.locator.starts_with("https://")
                || !valid_date(&source.collected_on)
                || !valid_evidence_digest(&source.digest_state, &source.bytes_digest)
            {
                return Err(ComputeContractError::InvalidModelData);
            }
        }
        for run in &self.client_runs {
            if !valid_text(&run.source_key, 256)
                || !valid_text(&run.repository_url, 2_048)
                || !valid_hex(&run.commit, 40)
                || !valid_hex(&run.tree, 40)
                || run.profile_digest == CanonicalDigest::of_bytes(&[])
                || run.result_digest == CanonicalDigest::of_bytes(&[])
            {
                return Err(ComputeContractError::InvalidModelData);
            }
        }
        for product in &self.access_products {
            if !valid_product(product, &evidence) {
                return Err(ComputeContractError::InvalidModelData);
            }
        }
        for rule in &self.inference_rules {
            if !valid_text(&rule.rule_key, 256)
                || !valid_text(&rule.reason, 2_048)
                || !valid_date(&rule.collected_on)
                || rule.evidence_refs.is_empty()
                || !valid_strings(&rule.evidence_refs, 2_048)
                || !refs_exist(&rule.evidence_refs, &evidence)
            {
                return Err(ComputeContractError::InvalidModelData);
            }
        }
        for model in &self.canonical_models {
            if !valid_canonical_model(model, &evidence) {
                return Err(ComputeContractError::InvalidModelData);
            }
        }
        for binding in &self.endpoint_bindings {
            let Some(product) = products.get(binding.product_key.as_str()) else {
                return Err(ComputeContractError::CrossReference);
            };
            let Some(model) = models.get(binding.model_key.as_str()) else {
                return Err(ComputeContractError::CrossReference);
            };
            let product_interfaces = product
                .interfaces
                .iter()
                .map(|interface| interface.interface_key.as_str())
                .collect::<BTreeSet<_>>();
            if !valid_text(&binding.binding_key, 512)
                || !valid_text(&binding.upstream_model_id, 512)
                || !model.upstream_ids.contains(&binding.upstream_model_id)
                || binding.interface_candidates.is_empty()
                || binding
                    .interface_candidates
                    .iter()
                    .any(|value| !product_interfaces.contains(value.as_str()))
                || !valid_text(&binding.availability.state, 128)
                || !valid_text(&binding.availability.condition, 1_024)
                || !valid_text(&binding.protocol_qualification, 128)
                || !valid_optional_text(&binding.upstream_identity_role, 128)
                || !valid_optional_text(&binding.lifecycle, 128)
                || !valid_optional_text(&binding.replaced_by_upstream_id, 512)
                || binding.capability_overrides.context_tokens == Some(0)
                || binding.capability_overrides.max_output_tokens == Some(0)
                || !refs_exist(&binding.evidence_refs, &evidence)
            {
                return Err(ComputeContractError::CrossReference);
            }
        }
        for route in &self.dynamic_routes {
            if !valid_text(&route.route_key, 512)
                || !products.contains_key(route.product_key.as_str())
                || route.upstream_route_ids.is_empty()
                || !valid_strings(&route.upstream_route_ids, 512)
                || !valid_text(&route.identity_stability, 128)
                || !valid_optional_text(&route.rated_model_identity, 512)
                || !refs_exist(&route.evidence_refs, &evidence)
                || !route.require_actual_response_model
                || !route.require_actual_response_provider
            {
                return Err(ComputeContractError::InvalidModelData);
            }
        }
        for provider in &self.provider_records {
            if !valid_text(&provider.provider_record_key, 512)
                || !valid_text(&provider.provider_id, 256)
                || !valid_text(&provider.display_name, 256)
                || !valid_provenance(&provider.provenance_refs, &runs, &evidence)
                || !valid_strings(&provider.description_candidates, 2_048)
                || !valid_strings(&provider.aliases, 512)
                || !valid_strings(&provider.base_url_candidates, 2_048)
                || !valid_strings(&provider.unsupported_api_styles, 128)
                || !valid_strings(&provider.unsupported_transport_locators, 2_048)
                || !valid_strings(&provider.environment_variable_names, 256)
                || !valid_strings(&provider.models_url_candidates, 2_048)
                || !valid_strings(&provider.signup_url_candidates, 2_048)
                || !valid_strings(&provider.default_model_ids, 512)
                || !valid_strings(&provider.discovery_modes, 128)
                || !valid_usage_scenarios(&provider.usable_for)
                || !valid_field_provenance(
                    &provider.field_provenance,
                    &rules,
                    MetadataInferenceRecordKindV1::Provider,
                )
            {
                return Err(ComputeContractError::InvalidModelData);
            }
        }
        for model in &self.model_records {
            let Some(provider) = providers.get(model.provider_record_key.as_str()) else {
                return Err(ComputeContractError::CrossReference);
            };
            if !valid_text(&model.model_record_key, 1_024)
                || !valid_text(&model.provider_id, 256)
                || model.provider_id != provider.provider_id
                || !valid_text(&model.upstream_model_id, 512)
                || !valid_text(&model.display_name, 256)
                || !valid_provenance(&model.provenance_refs, &runs, &evidence)
                || !valid_limit(&model.context_tokens)
                || !valid_limit(&model.max_output_tokens)
                || !valid_determinate_limit(&model.context_tokens)
                || !valid_determinate_limit(&model.max_output_tokens)
                || !valid_strings(&model.input_modalities, 128)
                || !valid_determinate_capabilities(&model.capability_hints)
                || !valid_reasoning_rendering(&model.reasoning_rendering_hints)
                || !valid_rendering_outcome(model)
                || !valid_execution_fit(&model.execution_fit, &model.input_modalities)
                || model.cost_hint_state
                    != if has_executable_cost_hint(&model.cost_hints) {
                        MetadataCostHintStateV1::Recorded
                    } else {
                        MetadataCostHintStateV1::NotRecorded
                    }
                || !valid_field_provenance(
                    &model.field_provenance,
                    &rules,
                    MetadataInferenceRecordKindV1::Model,
                )
                || model
                    .normalized_model_matches
                    .iter()
                    .any(|value| !models.contains_key(value.as_str()))
                || !valid_text(&model.lifecycle, 128)
                || !valid_strings(&model.status_candidates, 128)
                || !valid_strings(&model.replacement_upstream_ids, 512)
                || !valid_strings(&model.roles, 128)
                || !valid_usage_scenarios(&model.usable_for)
                || !valid_cost_hints(&model.cost_hints)
            {
                return Err(ComputeContractError::CrossReference);
            }
        }
        Ok(())
    }
}

fn valid_product(product: &MetadataAccessProductV1, evidence: &BTreeSet<&str>) -> bool {
    valid_text(&product.product_key, 256)
        && valid_text(&product.provider, 256)
        && valid_text(&product.product, 256)
        && valid_text(&product.product_kind, 128)
        && valid_text(&product.region_scope, 1_024)
        && valid_text(&product.billing, 256)
        && valid_text(&product.restriction, 1_024)
        && valid_text(&product.credential, 512)
        && valid_strings(&product.documented_upstream_model_ids, 512)
        && valid_strings(&product.quota_facts, 1_024)
        && valid_strings(&product.disposition, 128)
        && refs_exist(&product.evidence_refs, evidence)
        && strictly_sorted_by_key(&product.interfaces, |interface| {
            interface.interface_key.as_str()
        })
        && product.interfaces.iter().all(|interface| {
            valid_text(&interface.interface_key, 512)
                && valid_text(&interface.protocol, 128)
                && exactly_one(&interface.base_url, &interface.base_url_template)
                && exactly_one(&interface.request_path, &interface.request_path_template)
                && interface
                    .base_url
                    .as_ref()
                    .or(interface.base_url_template.as_ref())
                    .is_some_and(|value| valid_text(value, 2_048))
                && interface
                    .request_path
                    .as_ref()
                    .or(interface.request_path_template.as_ref())
                    .is_some_and(|value| valid_text(value, 2_048))
        })
}

fn valid_canonical_model(model: &MetadataCanonicalModelV1, evidence: &BTreeSet<&str>) -> bool {
    valid_text(&model.model_key, 256)
        && valid_text(&model.publisher, 256)
        && valid_text(&model.display_name, 256)
        && valid_text(&model.canonical_identity, 512)
        && !model.upstream_ids.is_empty()
        && valid_strings(&model.upstream_ids, 512)
        && valid_limit(&model.context_tokens)
        && valid_limit(&model.max_output_tokens)
        && model.max_input_tokens.is_none_or(|value| value > 0)
        && model.context_variants.iter().all(|variant| {
            valid_text(&variant.upstream_id, 512)
                && variant.tokens > 0
                && valid_text(&variant.condition, 1_024)
        })
        && valid_text(&model.reasoning.kind, 128)
        && valid_strings(&model.reasoning.profiles, 128)
        && valid_optional_text(&model.reasoning.default, 128)
        && valid_optional_text(&model.reasoning.note, 1_024)
        && valid_optional_text(&model.upstream_id_state, 128)
        && valid_optional_text(&model.max_output_scope, 128)
        && valid_optional_text(&model.max_output_note, 1_024)
        && valid_optional_text(&model.data_note, 2_048)
        && valid_capability_map(&model.modalities)
        && valid_capability_map(&model.capabilities)
        && valid_text(&model.identity_stability, 128)
        && valid_text(&model.lifecycle, 128)
        && valid_text(&model.source_stability_note, 1_024)
        && refs_exist(&model.evidence_refs, evidence)
}

fn valid_limit(limit: &MetadataTokenLimitV1) -> bool {
    match (limit.state, limit.value, limit.candidates.as_slice()) {
        (MetadataTokenStateV1::Known, Some(value), []) => value > 0,
        (MetadataTokenStateV1::EntitlementDependent, Some(value), []) => value > 0,
        (MetadataTokenStateV1::Conflict, None, candidates) => {
            candidates.len() >= 2
                && candidates.iter().all(|value| *value > 0)
                && candidates.windows(2).all(|pair| pair[0] < pair[1])
        }
        (
            // An entitlement-dependent limit is a determinate outcome without a number: the
            // published contract states the cap depends on the plan, so no static value may
            // be invented for it.
            MetadataTokenStateV1::EntitlementDependent
            | MetadataTokenStateV1::Unknown
            | MetadataTokenStateV1::RuntimeRequired
            | MetadataTokenStateV1::SharedTotalBudget
            | MetadataTokenStateV1::NotApplicable,
            None,
            [],
        ) => true,
        _ => false,
    }
}

/// Provider-scoped records are a closed dataset: an uncollected `unknown` limit is not a
/// permitted outcome, because the closure must land on a determinate state.
fn valid_determinate_limit(limit: &MetadataTokenLimitV1) -> bool {
    limit.state != MetadataTokenStateV1::Unknown
}

fn valid_determinate_capabilities(hints: &ModelCapabilityHintsV1) -> bool {
    [hints.reasoning, hints.streaming, hints.tool, hints.vision]
        .iter()
        .all(|state| *state != MetadataCapabilityStateV1::Unknown)
}

fn valid_rendering_outcome(model: &ModelMetadataRecordV1) -> bool {
    let hints = &model.reasoning_rendering_hints;
    let has_hints = !hints.reasoning_effort_maps.is_empty()
        || !hints.supported_reasoning_efforts.is_empty()
        || !hints.thinking_level_maps.is_empty();
    match model.reasoning_rendering_state {
        Some(MetadataReasoningRenderingStateV1::NotApplicable) => !has_hints,
        None => has_hints,
    }
}

fn valid_execution_fit(fit: &MetadataExecutionFitV1, modalities: &[String]) -> bool {
    match fit.state {
        MetadataExecutionFitStateV1::NativeTextRepresentable => {
            fit.reason.is_none() && modalities.iter().any(|value| value == "text")
        }
        MetadataExecutionFitStateV1::Unsupported | MetadataExecutionFitStateV1::NotApplicable => {
            fit.reason
                .as_deref()
                .is_some_and(|value| valid_text(value, 1_024))
        }
    }
}

/// Cost hints stay display-only and the projection turns every numeric leaf into a lexical
/// string, so a recorded hint is any hint that still carries a price leaf: a non-empty string
/// anywhere in the hint map, or a numeric leaf that was never projected away.
fn has_executable_cost_hint(hints: &[BTreeMap<String, Value>]) -> bool {
    hints
        .iter()
        .any(|hint| hint.values().any(cost_hint_leaf_is_price))
}

fn cost_hint_leaf_is_price(value: &Value) -> bool {
    match value {
        Value::String(text) => !text.is_empty(),
        Value::Array(values) => values.iter().any(cost_hint_leaf_is_price),
        Value::Object(values) => values.values().any(cost_hint_leaf_is_price),
        Value::Number(_) => true,
        Value::Bool(_) | Value::Null => false,
    }
}

fn valid_field_provenance(
    values: &BTreeMap<String, MetadataFieldProvenanceV1>,
    rules: &BTreeMap<&str, &MetadataInferenceRuleV1>,
    kind: MetadataInferenceRecordKindV1,
) -> bool {
    values.len() <= 64
        && values.iter().all(|(field, provenance)| {
            valid_text(field, 128)
                && rules
                    .get(provenance.rule_key.as_str())
                    .is_some_and(|rule| rule.record_kind == kind)
        })
}

fn valid_evidence_digest(state: &str, digest: &Option<CanonicalDigest>) -> bool {
    match (state, digest) {
        ("captured", Some(digest)) => digest != &CanonicalDigest::of_bytes(&[]),
        ("not-captured", None) => true,
        _ => false,
    }
}

fn valid_provenance(
    values: &[MetadataProvenanceRefV1],
    runs: &BTreeSet<&str>,
    evidence: &BTreeSet<&str>,
) -> bool {
    !values.is_empty()
        && values.iter().all(|value| match value.kind.as_str() {
            "client-discovery-run" => runs.contains(value.source_key.as_str()),
            "evidence-source" => evidence.contains(value.source_key.as_str()),
            _ => false,
        })
}

fn valid_reasoning_rendering(value: &MetadataReasoningRenderingHintsV1) -> bool {
    value.reasoning_effort_maps.len() <= 16
        && value.thinking_level_maps.len() <= 16
        && valid_strings(&value.supported_reasoning_efforts, 128)
        && value
            .reasoning_effort_maps
            .iter()
            .chain(&value.thinking_level_maps)
            .all(|mapping| {
                mapping.len() <= 32
                    && mapping
                        .iter()
                        .all(|(key, value)| valid_text(key, 128) && valid_optional_text(value, 128))
            })
}

fn valid_capability_map(values: &BTreeMap<String, MetadataCapabilityStateV1>) -> bool {
    values.len() <= 64 && values.keys().all(|key| valid_text(key, 128))
}

fn valid_usage_scenarios(values: &[MetadataUsageScenarioV1]) -> bool {
    values.len() <= 32 && values.windows(2).all(|pair| pair[0] < pair[1])
}

fn refs_exist(values: &[String], evidence: &BTreeSet<&str>) -> bool {
    !values.is_empty() && values.iter().all(|value| evidence.contains(value.as_str()))
}

fn valid_cost_hints(values: &[BTreeMap<String, Value>]) -> bool {
    values.len() <= 16
        && values.iter().all(|value| {
            !value.is_empty()
                && value.len() <= 32
                && value
                    .iter()
                    .all(|(key, hint)| valid_text(key, 128) && valid_cost_hint_value(hint, 0))
        })
}

fn valid_cost_hint_value(value: &Value, depth: usize) -> bool {
    if depth > 4 {
        return false;
    }
    match value {
        Value::Null | Value::Bool(_) => true,
        Value::Number(_) => false,
        Value::String(value) => valid_text(value, 256),
        Value::Array(values) => {
            values.len() <= 32
                && values
                    .iter()
                    .all(|value| valid_cost_hint_value(value, depth + 1))
        }
        Value::Object(values) => {
            values.len() <= 32
                && values.iter().all(|(key, value)| {
                    valid_text(key, 128) && valid_cost_hint_value(value, depth + 1)
                })
        }
    }
}

fn exactly_one<T>(left: &Option<T>, right: &Option<T>) -> bool {
    left.is_some() != right.is_some()
}

fn valid_optional_text(value: &Option<String>, max_bytes: usize) -> bool {
    value
        .as_ref()
        .is_none_or(|value| valid_text(value, max_bytes))
}

fn strictly_sorted_by_key<T>(values: &[T], key: impl Fn(&T) -> &str) -> bool {
    values.windows(2).all(|pair| key(&pair[0]) < key(&pair[1]))
}

fn unique_sorted<'a>(
    values: impl IntoIterator<Item = &'a str>,
) -> Result<(), ComputeContractError> {
    let mut previous = None;
    for value in values {
        if previous.is_some_and(|previous| previous >= value) {
            return Err(ComputeContractError::DuplicateIdentity);
        }
        previous = Some(value);
    }
    Ok(())
}

fn valid_strings(values: &[String], max_bytes: usize) -> bool {
    values.len() <= 10_000 && values.iter().all(|value| valid_text(value, max_bytes))
}

fn valid_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value.trim() == value
        && value.chars().all(|character| !character.is_control())
}

fn valid_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_state_does_not_collapse_unknown_into_unsupported_or_zero() {
        assert!(valid_limit(&MetadataTokenLimitV1 {
            state: MetadataTokenStateV1::Unknown,
            value: None,
            candidates: Vec::new(),
        }));
        assert!(!valid_limit(&MetadataTokenLimitV1 {
            state: MetadataTokenStateV1::Unknown,
            value: Some(1),
            candidates: Vec::new(),
        }));
        assert!(!valid_limit(&MetadataTokenLimitV1 {
            state: MetadataTokenStateV1::Known,
            value: None,
            candidates: Vec::new(),
        }));
        assert!(valid_limit(&MetadataTokenLimitV1 {
            state: MetadataTokenStateV1::Conflict,
            value: None,
            candidates: vec![8_192, 16_384],
        }));
    }

    #[test]
    fn determinate_states_never_require_an_invented_number() {
        for state in [
            MetadataTokenStateV1::EntitlementDependent,
            MetadataTokenStateV1::RuntimeRequired,
            MetadataTokenStateV1::SharedTotalBudget,
            MetadataTokenStateV1::NotApplicable,
        ] {
            assert!(valid_limit(&MetadataTokenLimitV1 {
                state,
                value: None,
                candidates: Vec::new(),
            }));
        }
        assert!(!valid_limit(&MetadataTokenLimitV1 {
            state: MetadataTokenStateV1::NotApplicable,
            value: Some(1),
            candidates: Vec::new(),
        }));
        assert!(!valid_determinate_limit(&MetadataTokenLimitV1 {
            state: MetadataTokenStateV1::Unknown,
            value: None,
            candidates: Vec::new(),
        }));
    }
}
