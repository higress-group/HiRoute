use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::UpstreamProtocol;
use super::model_metadata::{
    MetadataAuthenticationHintV1, MetadataCapabilityStateV1, MetadataCompletenessV1,
    MetadataCostHintStateV1, MetadataExecutionFitV1, MetadataFieldProvenanceV1,
    MetadataProvenanceRefV1, MetadataReasoningRenderingStateV1, MetadataTokenLimitV1,
    MetadataUsageScenarioV1,
};

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderMetadataCompletenessV1 {
    pub authentication: MetadataCompletenessV1,
    pub discovery: MetadataCompletenessV1,
    pub endpoint: MetadataCompletenessV1,
    pub identity: MetadataCompletenessV1,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderMetadataRecordV1 {
    pub provider_record_key: String,
    pub provenance_refs: Vec<MetadataProvenanceRefV1>,
    pub provider_id: String,
    pub display_name: String,
    pub description_candidates: Vec<String>,
    pub aliases: Vec<String>,
    pub base_url_candidates: Vec<String>,
    pub protocol_candidates: Vec<UpstreamProtocol>,
    pub unsupported_api_styles: Vec<String>,
    pub unsupported_transport_locators: Vec<String>,
    pub authentication_candidates: Vec<MetadataAuthenticationHintV1>,
    pub environment_variable_names: Vec<String>,
    pub default_model_ids: Vec<String>,
    pub discovery_modes: Vec<String>,
    pub models_url_candidates: Vec<String>,
    pub signup_url_candidates: Vec<String>,
    pub metadata_completeness: ProviderMetadataCompletenessV1,
    pub usable_for: Vec<MetadataUsageScenarioV1>,
    /// Present only for completeness domains that were closed by an inference rule.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub field_provenance: BTreeMap<String, MetadataFieldProvenanceV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCapabilityHintsV1 {
    pub reasoning: MetadataCapabilityStateV1,
    pub streaming: MetadataCapabilityStateV1,
    pub tool: MetadataCapabilityStateV1,
    pub vision: MetadataCapabilityStateV1,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataReasoningRenderingHintsV1 {
    pub reasoning_effort_maps: Vec<BTreeMap<String, Option<String>>>,
    pub supported_reasoning_efforts: Vec<String>,
    pub thinking_level_maps: Vec<BTreeMap<String, Option<String>>>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelMetadataCompletenessV1 {
    pub capabilities: MetadataCompletenessV1,
    pub cost: MetadataCompletenessV1,
    pub identity: MetadataCompletenessV1,
    pub lifecycle: MetadataCompletenessV1,
    pub limits: MetadataCompletenessV1,
    pub modalities: MetadataCompletenessV1,
    pub reasoning_rendering: MetadataCompletenessV1,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelMetadataRecordV1 {
    pub model_record_key: String,
    pub provider_record_key: String,
    pub provenance_refs: Vec<MetadataProvenanceRefV1>,
    pub provider_id: String,
    pub upstream_model_id: String,
    pub display_name: String,
    pub context_tokens: MetadataTokenLimitV1,
    pub max_output_tokens: MetadataTokenLimitV1,
    pub input_modalities: Vec<String>,
    pub capability_hints: ModelCapabilityHintsV1,
    pub reasoning_rendering_hints: MetadataReasoningRenderingHintsV1,
    /// Display-only, source-shaped hints. Nested tier schedules stay nested and numeric leaves are
    /// lexical strings: no currency, unit, product, region, schedule, or numeric price semantics
    /// are inferred, so these are never PriceRate facts.
    pub cost_hints: Vec<BTreeMap<String, Value>>,
    pub lifecycle: String,
    pub status_candidates: Vec<String>,
    pub replacement_upstream_ids: Vec<String>,
    pub roles: Vec<String>,
    pub normalized_model_matches: Vec<String>,
    pub metadata_completeness: ModelMetadataCompletenessV1,
    pub usable_for: Vec<MetadataUsageScenarioV1>,
    /// The determinate outcome of this record against the native text contract.
    pub execution_fit: MetadataExecutionFitV1,
    /// Derived from the recorded cost hints; never rule-assigned.
    pub cost_hint_state: MetadataCostHintStateV1,
    /// Present only when the reasoning rendering domain does not apply to this model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_rendering_state: Option<MetadataReasoningRenderingStateV1>,
    /// Present only for fields that were closed by an inference rule.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub field_provenance: BTreeMap<String, MetadataFieldProvenanceV1>,
}
