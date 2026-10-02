//! Model evidence and exact native reasoning parameters, independent of execution features.
use super::*;
use crate::UpstreamProtocol;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredModelSupportV1 {
    pub tools: GatewayCriticalFactV1<bool>,
    pub vision: GatewayCriticalFactV1<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredModelCapabilitiesV1 {
    pub capability_id: String,
    pub capability_revision: String,
    pub model_configuration_id: String,
    pub native_model: String,
    pub model_support: StoredModelSupportV1,
    pub max_input_tokens: GatewayCriticalFactV1<u64>,
    pub max_output_tokens: GatewayCriticalFactV1<u64>,
    pub max_total_tokens: GatewayCriticalFactV1<Option<u64>>,
    pub native_streaming: GatewayCriticalFactV1<bool>,
    pub reasoning_profiles: Vec<StoredNativeReasoningV1>,
    pub selected_reasoning_profile_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredNativeReasoningV1 {
    pub profile_id: String,
    pub control_kind: GatewayReasoningControlKindV1,
    pub parameters: StoredReasoningParametersV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredNativeParameterV1 {
    pub path: Vec<String>,
    pub value: GatewayNativeReasoningValueV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoredReasoningParametersV1 {
    NoControlParameter,
    Fields {
        protocol: UpstreamProtocol,
        fields: Vec<StoredNativeParameterV1>,
    },
    Budget {
        protocol: UpstreamProtocol,
        fields: Vec<StoredNativeParameterV1>,
        budget_path: Vec<String>,
        selected_tokens: u64,
        min_tokens: u64,
        max_tokens: u64,
        step_tokens: u64,
    },
}

impl StoredModelCapabilitiesV1 {
    pub fn freeze(capability: &GatewayCandidateCapabilityProfileV1) -> Self {
        // Every new execution field must be classified; implementation fields stay out.
        let GatewayCandidateCapabilityProfileV1 {
            schema_version: _,
            capability_id,
            capability_revision,
            upstream_protocol: _,
            model_configuration_id,
            native_model,
            model_support,
            request: _,
            response: _,
            reasoning_profiles,
            selected_reasoning_profile_id,
            context,
            native_streaming,
            native_provider_state: _,
        } = capability;
        let GatewayContextLimitsV1 {
            max_input_tokens,
            max_output_tokens,
            max_total_tokens,
            estimator: _,
        } = context;
        Self {
            capability_id: capability_id.clone(),
            capability_revision: capability_revision.clone(),
            model_configuration_id: model_configuration_id.clone(),
            native_model: native_model.clone(),
            model_support: model_support.clone(),
            max_input_tokens: max_input_tokens.clone(),
            max_output_tokens: max_output_tokens.clone(),
            max_total_tokens: max_total_tokens.clone(),
            native_streaming: native_streaming.clone(),
            reasoning_profiles: reasoning_profiles
                .iter()
                .map(StoredNativeReasoningV1::freeze)
                .collect(),
            selected_reasoning_profile_id: selected_reasoning_profile_id.clone(),
        }
    }
}

impl StoredNativeReasoningV1 {
    pub fn freeze(profile: &GatewayReasoningProfileCapabilityV1) -> Self {
        let GatewayReasoningProfileCapabilityV1 {
            profile_id,
            control_kind,
            render,
            accounting: _,
            additional_reservation_tokens: _,
        } = profile;
        let fields = |fields: &[GatewayNativeReasoningFieldAssignmentV1]| {
            fields
                .iter()
                .map(|field| {
                    let GatewayNativeReasoningFieldAssignmentV1 { path, value } = field;
                    StoredNativeParameterV1 {
                        path: path.clone(),
                        value: value.clone(),
                    }
                })
                .collect()
        };
        let parameters = match render {
            GatewayNativeReasoningRenderV1::NoControlParameter => {
                StoredReasoningParametersV1::NoControlParameter
            }
            GatewayNativeReasoningRenderV1::ExactFields {
                protocol,
                fields: assignments,
            } => StoredReasoningParametersV1::Fields {
                protocol: *protocol,
                fields: fields(assignments),
            },
            GatewayNativeReasoningRenderV1::ExactBudget {
                protocol,
                fields: assignments,
                budget_path,
                selected_tokens,
                min_tokens,
                max_tokens,
                step_tokens,
            } => StoredReasoningParametersV1::Budget {
                protocol: *protocol,
                fields: fields(assignments),
                budget_path: budget_path.clone(),
                selected_tokens: *selected_tokens,
                min_tokens: *min_tokens,
                max_tokens: *max_tokens,
                step_tokens: *step_tokens,
            },
        };
        Self {
            profile_id: profile_id.clone(),
            control_kind: *control_kind,
            parameters,
        }
    }

    pub(super) fn compile(&self) -> GatewayReasoningProfileCapabilityV1 {
        let fields = |fields: &[StoredNativeParameterV1]| {
            fields
                .iter()
                .map(|field| GatewayNativeReasoningFieldAssignmentV1 {
                    path: field.path.clone(),
                    value: field.value.clone(),
                })
                .collect()
        };
        let render = match &self.parameters {
            StoredReasoningParametersV1::NoControlParameter => {
                GatewayNativeReasoningRenderV1::NoControlParameter
            }
            StoredReasoningParametersV1::Fields {
                protocol,
                fields: assignments,
            } => GatewayNativeReasoningRenderV1::ExactFields {
                protocol: *protocol,
                fields: fields(assignments),
            },
            StoredReasoningParametersV1::Budget {
                protocol,
                fields: assignments,
                budget_path,
                selected_tokens,
                min_tokens,
                max_tokens,
                step_tokens,
            } => GatewayNativeReasoningRenderV1::ExactBudget {
                protocol: *protocol,
                fields: fields(assignments),
                budget_path: budget_path.clone(),
                selected_tokens: *selected_tokens,
                min_tokens: *min_tokens,
                max_tokens: *max_tokens,
                step_tokens: *step_tokens,
            },
        };
        GatewayReasoningProfileCapabilityV1 {
            profile_id: self.profile_id.clone(),
            control_kind: self.control_kind,
            render,
            accounting: GatewayReasoningAccountingV1::WithinOutputCap,
            additional_reservation_tokens: 0,
        }
    }
}
