use hiroute_domain::{
    ConnectorRuntimeKind, GatewayAuthenticationSemanticsV1, GatewayCandidateCapabilityProfileV1,
    GatewayCandidateProtocolProfileV1, GatewayConnectorProfileV1, GatewayContextLimitsV1,
    GatewayCriticalFactV1, GatewayErrorSemanticsV1, GatewayFidelityV1, GatewayHeaderSemanticsV1,
    GatewayNativeProviderStateEmissionV1, GatewayNativeReasoningFieldAssignmentV1,
    GatewayNativeReasoningRenderV1, GatewayNativeReasoningValueV1, GatewayReasoningAccountingV1,
    GatewayReasoningControlKindV1, GatewayReasoningProfileCapabilityV1,
    GatewayRequestFeatureProfileV1, GatewayResponseFeatureProfileV1, GatewayStateAffinityV1,
    GatewayStreamingRefusalSemanticsV1, GatewayTokenEstimatorProfileV1, ModelDefinitionV1,
    ModelEndpointCapabilityV1, ModelNativeReasoningV1, NativeReasoningCapabilityV1,
    NativeReasoningRenderConventionV1, UpstreamProtocol,
};

use super::{ProtocolConnectorFacts, ProtocolFace, protocol_label};

const MAX_MATERIALIZED_BUDGET_PROFILES: u64 = 64;
const MAX_TERMINAL_REFUSAL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TERMINAL_REFUSAL_BLOCKS: u32 = 8;

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
        let Some(selected_reasoning_profile_id) = reasoning_profiles
            .first()
            .map(|profile| profile.profile_id.clone())
        else {
            continue;
        };
        let exact_provider_state = (runtime_kind == ConnectorRuntimeKind::CpaBridge
            && matches!(
                upstream_protocol,
                UpstreamProtocol::Responses | UpstreamProtocol::Messages
            ))
            || (runtime_kind == ConnectorRuntimeKind::BuiltinNative
                && upstream_protocol == UpstreamProtocol::Messages);
        let exact = GatewayFidelityV1::Exact;
        let unsupported = GatewayFidelityV1::Unsupported;
        let tool = if model.capabilities.tool {
            exact
        } else {
            unsupported
        };
        let vision = if model.capabilities.vision {
            exact
        } else {
            unsupported
        };
        profiles.push(GatewayCandidateProtocolProfileV1 {
            schema_version: "hiroute.candidate-protocol-profile/v1".into(),
            path_id: format!(
                "{}-to-{}-{}",
                protocol_label(ingress),
                protocol_label(upstream_protocol),
                capability.capability_id
            ),
            ingress_protocol: ingress,
            adapter_revision: format!(
                "{}@{}",
                capability.required_adapter_ref, capability.required_adapter_revision
            ),
            serializer_revision: "hiroute-target-json/v1".into(),
            decoder_revision: "hiroute-native-response/v1".into(),
            capability: GatewayCandidateCapabilityProfileV1 {
                schema_version: "hiroute.candidate-capability/v1".into(),
                capability_id: capability.capability_id.clone(),
                capability_revision: capability.revision.to_string(),
                upstream_protocol,
                model_configuration_id: capability.model_configuration_id.clone(),
                native_model: native_transport_model.to_owned(),
                request: GatewayRequestFeatureProfileV1 {
                    text: exact,
                    initial_instructions: exact,
                    mid_conversation_instructions: if upstream_protocol
                        == UpstreamProtocol::Messages
                    {
                        unsupported
                    } else {
                        exact
                    },
                    image_url: vision,
                    image_base64: vision,
                    image_base64_media_types: GatewayCriticalFactV1::Exact(
                        if model.capabilities.vision {
                            vec![
                                "image/gif".into(),
                                "image/jpeg".into(),
                                "image/png".into(),
                                "image/webp".into(),
                            ]
                        } else {
                            Vec::new()
                        },
                    ),
                    function_tools: tool,
                    strict_tools: if upstream_protocol == UpstreamProtocol::Messages {
                        unsupported
                    } else {
                        tool
                    },
                    tool_choice_none: if upstream_protocol == UpstreamProtocol::Messages {
                        unsupported
                    } else {
                        tool
                    },
                    tool_choice_auto: tool,
                    tool_choice_required_any: tool,
                    tool_choice_required_named: tool,
                    parallel_tools: tool,
                    tool_roundtrip: tool,
                    tool_result_text: tool,
                    tool_result_json: tool,
                    logical_tool_id_mapping: tool,
                    provider_state: if exact_provider_state {
                        exact
                    } else {
                        unsupported
                    },
                    state_affinity: if exact_provider_state {
                        GatewayStateAffinityV1::ExactOwner
                    } else {
                        GatewayStateAffinityV1::Unsupported
                    },
                },
                response: GatewayResponseFeatureProfileV1 {
                    text: exact,
                    reasoning: exact,
                    refusal: exact,
                    tool_calls: tool,
                    logical_tool_id_mapping: tool,
                    usage: exact,
                    finish_reason: exact,
                    typed_error: exact,
                    provider_state: if exact_provider_state {
                        exact
                    } else {
                        unsupported
                    },
                    state_affinity: if exact_provider_state {
                        GatewayStateAffinityV1::ExactOwner
                    } else {
                        GatewayStateAffinityV1::Unsupported
                    },
                    stream_refusal: if upstream_protocol == UpstreamProtocol::Messages {
                        GatewayStreamingRefusalSemanticsV1::TerminalClassified {
                            max_buffered_bytes: MAX_TERMINAL_REFUSAL_BYTES,
                            max_buffered_blocks: MAX_TERMINAL_REFUSAL_BLOCKS,
                        }
                    } else {
                        GatewayStreamingRefusalSemanticsV1::ExactDelta
                    },
                    stream_text_delta: exact,
                    stream_tool_argument_delta: tool,
                    stream_reasoning_delta: exact,
                    stream_usage: exact,
                },
                reasoning_profiles: reasoning_profiles.clone(),
                selected_reasoning_profile_id: selected_reasoning_profile_id.clone(),
                context: GatewayContextLimitsV1 {
                    max_input_tokens: GatewayCriticalFactV1::Exact(
                        model.capabilities.context_tokens,
                    ),
                    max_output_tokens: GatewayCriticalFactV1::Exact(
                        model.capabilities.max_output_tokens,
                    ),
                    max_total_tokens: GatewayCriticalFactV1::Exact(
                        model
                            .capabilities
                            .context_tokens
                            .checked_add(model.capabilities.max_output_tokens),
                    ),
                    estimator: GatewayCriticalFactV1::Exact(GatewayTokenEstimatorProfileV1 {
                        revision: "byte-upper-bound/v1".into(),
                        bytes_per_token: 1,
                        fixed_overhead_tokens: 0,
                    }),
                },
                native_streaming: GatewayCriticalFactV1::Exact(model.capabilities.streaming),
                native_provider_state: if exact_provider_state {
                    GatewayNativeProviderStateEmissionV1::ExactOwnerAffine
                } else {
                    GatewayNativeProviderStateEmissionV1::Never
                },
            },
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
                        fields: vec![field(
                            parameter,
                            GatewayNativeReasoningValueV1::Bool(enabled),
                            protocol,
                        )],
                    },
                )
            })
            .collect(),
        NativeReasoningCapabilityV1::Discrete {
            parameter,
            profiles,
        } => profiles
            .iter()
            .map(|profile| {
                make(
                    profile.clone(),
                    GatewayReasoningControlKindV1::Discrete,
                    GatewayNativeReasoningRenderV1::ExactFields {
                        protocol,
                        fields: vec![field(
                            parameter,
                            GatewayNativeReasoningValueV1::String(profile.clone()),
                            protocol,
                        )],
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
    // ModelData names the portable discrete control. The exact protocol
    // renderer owns its wire location: OpenAI Responses nests effort while
    // Chat Completions keeps the historical top-level field.
    if protocol == UpstreamProtocol::Responses && parameter == "reasoning_effort" {
        return vec!["reasoning".into(), "effort".into()];
    }
    parameter.split('.').map(str::to_owned).collect()
}
