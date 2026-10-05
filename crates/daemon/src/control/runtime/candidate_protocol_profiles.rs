use hiroute_domain::{
    ConnectorRuntimeKind, GatewayAuthenticationSemanticsV1, GatewayCandidateProtocolProfileV1,
    GatewayConnectorProfileV1, GatewayCriticalFactV1, GatewayErrorSemanticsV1,
    GatewayHeaderSemanticsV1, GatewayNativeReasoningFieldAssignmentV1,
    GatewayNativeReasoningRenderV1, GatewayNativeReasoningValueV1, GatewayReasoningAccountingV1,
    GatewayReasoningControlKindV1, GatewayReasoningProfileCapabilityV1, ModelDefinitionV1,
    ModelEndpointCapabilityV1, ModelNativeReasoningV1, NativeReasoningCapabilityV1,
    NativeReasoningRenderConventionV1, StoredModelCapabilitiesV1, StoredModelSupportV1,
    StoredNativeReasoningV1, UpstreamProtocol,
};

use super::{ProtocolConnectorFacts, ProtocolFace, protocol_label};

const MAX_MATERIALIZED_BUDGET_PROFILES: u64 = 64;

pub(super) fn protocol_profiles(
    connector: &ProtocolConnectorFacts,
    model: &ModelDefinitionV1,
    capability: &ModelEndpointCapabilityV1,
    reasoning: &ModelNativeReasoningV1,
    native_transport_model: &str,
    runtime_kind: ConnectorRuntimeKind,
    protocol_faces: &[ProtocolFace],
) -> Option<Vec<GatewayCandidateProtocolProfileV1>> {
    let mut profiles = Vec::new();
    for ingress in [
        UpstreamProtocol::Responses,
        UpstreamProtocol::ChatCompletions,
        UpstreamProtocol::Messages,
    ] {
        let face = if runtime_kind == ConnectorRuntimeKind::CpaBridge {
            protocol_faces
                .iter()
                .find(|face| face.protocol == ingress)
                .or_else(|| {
                    (ingress == UpstreamProtocol::Messages)
                        .then(|| {
                            protocol_faces
                                .iter()
                                .find(|face| face.protocol == UpstreamProtocol::Responses)
                        })
                        .flatten()
                })
        } else {
            protocol_faces
                .iter()
                .find(|face| face.protocol == ingress)
                .or_else(|| protocol_faces.first())
        };
        let Some(face) = face else {
            continue;
        };
        let upstream_protocol = face.protocol;
        let reasoning_profiles = if runtime_kind == ConnectorRuntimeKind::CpaBridge {
            cpa_reasoning_profiles(reasoning, capability.upstream_protocol, upstream_protocol)
        } else {
            reasoning_profiles(reasoning, upstream_protocol)
        };
        let Some(reasoning_profiles) = reasoning_profiles else {
            continue;
        };
        let preferred_profile = match &reasoning.capability {
            NativeReasoningCapabilityV1::Discrete {
                default_profile: Some(profile),
                ..
            } => Some(profile.as_str()),
            _ => None,
        };
        let Some(selected_reasoning_profile_id) = reasoning_profiles
            .iter()
            .find(|profile| Some(profile.profile_id.as_str()) == preferred_profile)
            .or_else(|| reasoning_profiles.first())
            .map(|profile| profile.profile_id.clone())
        else {
            continue;
        };
        profiles.push(GatewayCandidateProtocolProfileV1 {
            schema_version: "hiroute.candidate-protocol-profile/v1".into(),
            native_target: face.native_target.clone(),
            path_id: format!(
                "{}-to-{}-{}",
                protocol_label(ingress),
                protocol_label(upstream_protocol),
                capability.capability_id
            ),
            ingress_protocol: ingress,
            adapter_revision: format!(
                "{}@{}",
                face.adapter_ref
                    .as_deref()
                    .unwrap_or(&capability.required_adapter_ref),
                face.adapter_revision
                    .unwrap_or(capability.required_adapter_revision)
            ),
            serializer_revision: "hiroute-target-json/v1".into(),
            decoder_revision: "hiroute-native-response/v1".into(),
            capability: StoredModelCapabilitiesV1 {
                capability_id: capability.capability_id.clone(),
                capability_revision: capability.revision.to_string(),
                model_configuration_id: capability.model_configuration_id.clone(),
                native_model: native_transport_model.to_owned(),
                model_support: StoredModelSupportV1 {
                    tools: GatewayCriticalFactV1::Exact(model.capabilities.tool),
                    vision: GatewayCriticalFactV1::Exact(model.capabilities.vision),
                },
                max_input_tokens: GatewayCriticalFactV1::Exact(model.capabilities.context_tokens),
                max_output_tokens: GatewayCriticalFactV1::Exact(
                    model.capabilities.max_output_tokens,
                ),
                max_total_tokens: GatewayCriticalFactV1::Exact(
                    model
                        .capabilities
                        .context_tokens
                        .checked_add(model.capabilities.max_output_tokens),
                ),
                native_streaming: GatewayCriticalFactV1::Exact(model.capabilities.streaming),
                reasoning_profiles: reasoning_profiles
                    .iter()
                    .map(StoredNativeReasoningV1::freeze)
                    .collect(),
                selected_reasoning_profile_id,
            }
            .compile(upstream_protocol),
            connector: GatewayConnectorProfileV1 {
                schema_version: "hiroute.connector-profile/v1".into(),
                provider_id: connector.provider_id.clone(),
                endpoint_id: connector.endpoint_id.clone(),
                entitlement_id: connector.entitlement_id.clone(),
                connector_id: connector.connector_id.clone(),
                connector_revision: connector.connector_revision.clone(),
                upstream_protocol,
                request_path: face.request_path.clone(),
                authentication: GatewayCriticalFactV1::Exact(face.authentication.clone()),
                headers: GatewayCriticalFactV1::Exact(GatewayHeaderSemanticsV1 {
                    content_type: "application/json".into(),
                    required_headers: face.required_headers.clone(),
                    forbidden_forward_headers: forbidden_headers(&face.authentication),
                }),
                errors: GatewayCriticalFactV1::Exact(GatewayErrorSemanticsV1 {
                    http_status_typed: true,
                    sse_error_typed: true,
                    retry_after_header: Some("retry-after".into()),
                }),
            },
        });
    }
    (!profiles.is_empty()).then_some(profiles)
}

fn forbidden_headers(authentication: &GatewayAuthenticationSemanticsV1) -> Vec<String> {
    let mut headers = vec!["authorization".into(), "x-api-key".into()];
    if let GatewayAuthenticationSemanticsV1::ApiKeyHeader { header } = authentication
        && !headers.iter().any(|current| current == header)
    {
        headers.push(header.clone());
    }
    headers
}

pub(super) fn reasoning_profiles(
    native: &ModelNativeReasoningV1,
    protocol: UpstreamProtocol,
) -> Option<Vec<GatewayReasoningProfileCapabilityV1>> {
    native.validate_for_protocol(protocol).ok()?;
    let capability = &native.capability;
    if matches!(capability, NativeReasoningCapabilityV1::Discrete { parameter, .. }
        if parameter == "claude_adaptive_effort" && protocol != UpstreamProtocol::Messages)
    {
        return None;
    }
    let make = |profile_id: String,
                control_kind: GatewayReasoningControlKindV1,
                render: GatewayNativeReasoningRenderV1| {
        GatewayReasoningProfileCapabilityV1 {
            profile_id,
            control_kind,
            render,
            accounting: GatewayReasoningAccountingV1::WithinOutputCap,
            additional_reservation_tokens: 0,
        }
    };
    let mut values = match capability {
        NativeReasoningCapabilityV1::Fixed { profile } => vec![make(
            profile.clone(),
            GatewayReasoningControlKindV1::Fixed,
            GatewayNativeReasoningRenderV1::NoControlParameter,
        )],
        NativeReasoningCapabilityV1::Toggle { parameter } => [false, true]
            .into_iter()
            .map(|enabled| {
                make(
                    if enabled { "enabled" } else { "disabled" }.into(),
                    GatewayReasoningControlKindV1::Toggle,
                    GatewayNativeReasoningRenderV1::ExactFields {
                        protocol,
                        fields: if parameter == "deepseek_thinking" {
                            match protocol {
                                UpstreamProtocol::Responses => vec![field(
                                    "reasoning.effort",
                                    GatewayNativeReasoningValueV1::String(
                                        if enabled { "high" } else { "none" }.into(),
                                    ),
                                    protocol,
                                )],
                                UpstreamProtocol::ChatCompletions | UpstreamProtocol::Messages => {
                                    vec![field(
                                        "thinking.type",
                                        GatewayNativeReasoningValueV1::String(
                                            if enabled { "enabled" } else { "disabled" }.into(),
                                        ),
                                        protocol,
                                    )]
                                }
                            }
                        } else if parameter == "enable_thinking" {
                            match protocol {
                                UpstreamProtocol::ChatCompletions => vec![field(
                                    "enable_thinking",
                                    GatewayNativeReasoningValueV1::Bool(enabled),
                                    protocol,
                                )],
                                UpstreamProtocol::Responses => vec![field(
                                    "reasoning.effort",
                                    GatewayNativeReasoningValueV1::String(
                                        if enabled { "high" } else { "none" }.into(),
                                    ),
                                    protocol,
                                )],
                                UpstreamProtocol::Messages => {
                                    vec![field(
                                        "thinking.type",
                                        GatewayNativeReasoningValueV1::String(
                                            if enabled { "enabled" } else { "disabled" }.into(),
                                        ),
                                        protocol,
                                    )]
                                }
                            }
                        } else {
                            vec![field(
                                parameter,
                                GatewayNativeReasoningValueV1::Bool(enabled),
                                protocol,
                            )]
                        },
                    },
                )
            })
            .collect(),
        NativeReasoningCapabilityV1::Discrete {
            parameter,
            profiles,
            ..
        } => profiles
            .iter()
            .map(|profile| {
                make(
                    profile.clone(),
                    GatewayReasoningControlKindV1::Discrete,
                    GatewayNativeReasoningRenderV1::ExactFields {
                        protocol,
                        fields: if parameter == "claude_adaptive_effort"
                            && protocol == UpstreamProtocol::Messages
                        {
                            vec![
                                field(
                                    "output_config.effort",
                                    GatewayNativeReasoningValueV1::String(profile.clone()),
                                    protocol,
                                ),
                                field(
                                    "thinking.type",
                                    GatewayNativeReasoningValueV1::String("adaptive".into()),
                                    protocol,
                                ),
                            ]
                        } else if parameter == "reasoning_effort"
                            && protocol == UpstreamProtocol::Messages
                        {
                            vec![
                                field(
                                    "output_config.effort",
                                    GatewayNativeReasoningValueV1::String(profile.clone()),
                                    protocol,
                                ),
                                field(
                                    "thinking.type",
                                    GatewayNativeReasoningValueV1::String("enabled".into()),
                                    protocol,
                                ),
                            ]
                        } else {
                            vec![field(
                                parameter,
                                GatewayNativeReasoningValueV1::String(profile.clone()),
                                protocol,
                            )]
                        },
                    },
                )
            })
            .collect(),
        NativeReasoningCapabilityV1::Budget {
            parameter,
            minimum_tokens,
            maximum_tokens,
            step_tokens,
        } => {
            let count = u64::from((maximum_tokens - minimum_tokens) / step_tokens) + 1;
            if count > MAX_MATERIALIZED_BUDGET_PROFILES {
                return None;
            }
            (*minimum_tokens..=*maximum_tokens)
                .step_by(*step_tokens as usize)
                .map(|tokens| {
                    let value = u64::from(tokens);
                    make(
                        format!("budget-{tokens}"),
                        GatewayReasoningControlKindV1::Budget,
                        GatewayNativeReasoningRenderV1::ExactBudget {
                            protocol,
                            fields: vec![field(
                                parameter,
                                GatewayNativeReasoningValueV1::U64(value),
                                protocol,
                            )],
                            budget_path: parameter_path(parameter, protocol),
                            selected_tokens: value,
                            min_tokens: u64::from(*minimum_tokens),
                            max_tokens: u64::from(*maximum_tokens),
                            step_tokens: u64::from(*step_tokens),
                        },
                    )
                })
                .collect()
        }
    };
    if native.native_render_convention
        == Some(NativeReasoningRenderConventionV1::ClaudeAdaptiveEffortMessages)
    {
        for profile in &mut values {
            let GatewayNativeReasoningRenderV1::ExactFields { fields, .. } = &mut profile.render
            else {
                return None;
            };
            fields.push(field(
                "thinking.type",
                GatewayNativeReasoningValueV1::String("adaptive".into()),
                protocol,
            ));
        }
    }
    Some(values)
}

pub(super) fn cpa_reasoning_profiles(
    native: &ModelNativeReasoningV1,
    registered_protocol: UpstreamProtocol,
    face_protocol: UpstreamProtocol,
) -> Option<Vec<GatewayReasoningProfileCapabilityV1>> {
    native.validate_for_protocol(registered_protocol).ok()?;
    if face_protocol != UpstreamProtocol::Messages
        || registered_protocol == UpstreamProtocol::Messages
    {
        return reasoning_profiles(native, face_protocol);
    }
    let NativeReasoningCapabilityV1::Discrete { profiles, .. } = &native.capability else {
        return None;
    };
    Some(
        profiles
            .iter()
            .map(|profile| GatewayReasoningProfileCapabilityV1 {
                profile_id: profile.clone(),
                control_kind: GatewayReasoningControlKindV1::Discrete,
                render: GatewayNativeReasoningRenderV1::ExactFields {
                    protocol: UpstreamProtocol::Messages,
                    fields: vec![
                        field(
                            "output_config.effort",
                            GatewayNativeReasoningValueV1::String(profile.clone()),
                            UpstreamProtocol::Messages,
                        ),
                        field(
                            "thinking.type",
                            GatewayNativeReasoningValueV1::String("adaptive".into()),
                            UpstreamProtocol::Messages,
                        ),
                    ],
                },
                accounting: GatewayReasoningAccountingV1::WithinOutputCap,
                additional_reservation_tokens: 0,
            })
            .collect(),
    )
}

pub(super) fn field(
    parameter: &str,
    value: GatewayNativeReasoningValueV1,
    protocol: UpstreamProtocol,
) -> GatewayNativeReasoningFieldAssignmentV1 {
    GatewayNativeReasoningFieldAssignmentV1 {
        path: parameter_path(parameter, protocol),
        value,
    }
}

pub(super) fn parameter_path(parameter: &str, protocol: UpstreamProtocol) -> Vec<String> {
    GatewayNativeReasoningFieldAssignmentV1::parameter_path(parameter, protocol)
}
