use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::server::request_plan::IngressProtocol;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningControlKind {
    Fixed,
    Toggle,
    Discrete,
    Budget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningAccounting {
    WithinOutputCap,
    Additive,
}

pub use hiroute_domain::{
    GatewayNativeReasoningFieldAssignmentV1 as NativeReasoningFieldAssignment,
    GatewayNativeReasoningValueV1 as NativeReasoningValue,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeReasoningRender {
    /// A fixed model configuration with no request control field.
    NoControlParameter,
    /// One or more exact, protocol-owned assignments. This covers simple
    /// toggles and multi-field profiles such as Qwen and DeepSeek.
    ExactFields {
        protocol: IngressProtocol,
        fields: Vec<NativeReasoningFieldAssignment>,
    },
    /// Exact bounded budget selection plus any companion assignments.
    ExactBudget {
        protocol: IngressProtocol,
        fields: Vec<NativeReasoningFieldAssignment>,
        budget_path: Vec<String>,
        selected_tokens: u64,
        min_tokens: u64,
        max_tokens: u64,
        step_tokens: u64,
    },
}

impl NativeReasoningRender {
    pub fn protocol(&self) -> Option<IngressProtocol> {
        match self {
            Self::NoControlParameter => None,
            Self::ExactFields { protocol, .. } | Self::ExactBudget { protocol, .. } => {
                Some(*protocol)
            }
        }
    }

    pub fn fields(&self) -> &[NativeReasoningFieldAssignment] {
        match self {
            Self::NoControlParameter => &[],
            Self::ExactFields { fields, .. } | Self::ExactBudget { fields, .. } => fields,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReasoningProfileCapability {
    pub profile_id: String,
    pub control_kind: ReasoningControlKind,
    pub render: NativeReasoningRender,
    pub accounting: ReasoningAccounting,
    pub additional_reservation_tokens: u64,
}

impl ReasoningProfileCapability {
    pub fn validate_for(&self, protocol: IngressProtocol) -> bool {
        if self.profile_id.trim().is_empty()
            || (self.accounting == ReasoningAccounting::WithinOutputCap
                && self.additional_reservation_tokens != 0)
            || self
                .render
                .protocol()
                .is_some_and(|value| value != protocol)
            || !valid_assignments(self.render.fields())
        {
            return false;
        }

        match (&self.control_kind, &self.render) {
            (ReasoningControlKind::Fixed, NativeReasoningRender::NoControlParameter) => {
                self.additional_reservation_tokens == 0
                    || self.accounting == ReasoningAccounting::Additive
            }
            (ReasoningControlKind::Toggle, NativeReasoningRender::ExactFields { fields, .. }) => {
                let enabled = match self.profile_id.as_str() {
                    "enabled" => true,
                    "disabled" => false,
                    // Existing custom Boolean profile names retain their contract;
                    // protocol-owned strings require a precise binary identity.
                    _ => {
                        return fields
                            .iter()
                            .any(|field| matches!(field.value, NativeReasoningValue::Bool(_)));
                    }
                };
                let protocol = match protocol {
                    IngressProtocol::Responses => hiroute_domain::UpstreamProtocol::Responses,
                    IngressProtocol::Messages => hiroute_domain::UpstreamProtocol::Messages,
                    IngressProtocol::ChatCompletions => {
                        hiroute_domain::UpstreamProtocol::ChatCompletions
                    }
                };
                fields
                    .iter()
                    .any(|field| field.matches_toggle_wire(protocol, enabled))
            }
            (ReasoningControlKind::Discrete, NativeReasoningRender::ExactFields { fields, .. }) => {
                !fields.is_empty()
            }
            (
                ReasoningControlKind::Budget,
                NativeReasoningRender::ExactBudget {
                    fields,
                    budget_path,
                    selected_tokens,
                    min_tokens,
                    max_tokens,
                    step_tokens,
                    ..
                },
            ) => {
                *step_tokens > 0
                    && *min_tokens > 0
                    && min_tokens <= selected_tokens
                    && selected_tokens <= max_tokens
                    && (selected_tokens - min_tokens) % step_tokens == 0
                    && fields.iter().any(|field| {
                        field.path == *budget_path
                            && field.value == NativeReasoningValue::U64(*selected_tokens)
                    })
            }
            _ => false,
        }
    }
}

fn valid_assignments(fields: &[NativeReasoningFieldAssignment]) -> bool {
    let mut paths = BTreeSet::new();
    fields.iter().all(|field| {
        !field.path.is_empty()
            && field.path.iter().all(|part| !part.trim().is_empty())
            && !reserved_request_field(&field.path[0])
            && paths.insert(field.path.clone())
            && !matches!(&field.value, NativeReasoningValue::String(value) if value.is_empty())
    })
}

fn reserved_request_field(field: &str) -> bool {
    matches!(
        field,
        "model"
            | "stream"
            | "input"
            | "messages"
            | "instructions"
            | "system"
            | "tools"
            | "tool_choice"
            | "parallel_tool_calls"
            | "max_output_tokens"
            | "max_completion_tokens"
            | "max_tokens"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_toggle_profiles_validate_protocol_switch_and_keep_custom_boolean_names() {
        for (protocol, path, values) in [
            (
                IngressProtocol::Responses,
                "reasoning.effort",
                ["none", "high"],
            ),
            (
                IngressProtocol::Messages,
                "thinking.type",
                ["disabled", "enabled"],
            ),
            (
                IngressProtocol::ChatCompletions,
                "thinking.type",
                ["disabled", "enabled"],
            ),
        ] {
            for enabled in [false, true] {
                let mut profile = ReasoningProfileCapability {
                    profile_id: if enabled { "enabled" } else { "disabled" }.into(),
                    control_kind: ReasoningControlKind::Toggle,
                    render: NativeReasoningRender::ExactFields {
                        protocol,
                        fields: vec![NativeReasoningFieldAssignment {
                            path: path.split('.').map(str::to_owned).collect(),
                            value: NativeReasoningValue::String(
                                values[usize::from(enabled)].into(),
                            ),
                        }],
                    },
                    accounting: ReasoningAccounting::WithinOutputCap,
                    additional_reservation_tokens: 0,
                };
                assert!(profile.validate_for(protocol));
                profile.profile_id = if enabled { "disabled" } else { "enabled" }.into();
                assert!(!profile.validate_for(protocol));
                profile.profile_id = "custom-boolean-name".into();
                assert!(!profile.validate_for(protocol));
                let NativeReasoningRender::ExactFields { fields, .. } = &mut profile.render else {
                    unreachable!()
                };
                fields[0] = NativeReasoningFieldAssignment {
                    path: vec!["custom_toggle".into()],
                    value: NativeReasoningValue::Bool(enabled),
                };
                assert!(profile.validate_for(protocol));
                profile.profile_id = if enabled { "enabled" } else { "disabled" }.into();
                assert!(profile.validate_for(protocol));
                let NativeReasoningRender::ExactFields { fields, .. } = &mut profile.render else {
                    unreachable!()
                };
                fields[0].value = NativeReasoningValue::Bool(!enabled);
                assert!(!profile.validate_for(protocol));
            }
        }
    }
}
