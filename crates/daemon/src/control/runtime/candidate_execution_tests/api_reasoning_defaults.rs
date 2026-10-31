use super::*;
use hiroute_domain::{
    ComputeManagementFactBasisV2, ComputeManagementFactValueV2, ModelDefaultFieldV1,
    ModelDefaultPreferenceV1,
};

fn observed_api(
    catalog: &TrustedReleaseCatalog,
    option: &str,
    protocol: UpstreamProtocol,
    model: &str,
) -> ComputeManagementCompilationFactV2 {
    let resolved = catalog.resolve_connection_option(option).unwrap();
    let endpoint = resolved
        .endpoint_profile
        .protocol_endpoints
        .iter()
        .find(|e| e.protocol == protocol)
        .unwrap();
    let authentication = endpoint.authentication_semantics.clone().unwrap();
    let mut fact = source_local_fact(authentication.clone());
    fact.provenance = hiroute_domain::ComputeManagementProvenanceV2::Registered {
        connection_option_id: option.into(),
        registry_version: catalog.registry().registry_version.clone(),
        catalog_digest: catalog
            .compute_catalog_provenance()
            .unwrap()
            .cross_reference_digest,
    };
    fact.eligibility = ComputeManagementEligibilityV2::RuntimeQualified;
    fact.upstream_model_id = model.into();
    fact.target = hiroute_domain::ComputeManagementTargetV2 {
        scheme: "https".into(),
        authority: endpoint.base_url.strip_prefix("https://").unwrap().into(),
        port: 443,
        request_path: endpoint.request_path.clone(),
        upstream_protocol: protocol,
        protocol_profile_id: endpoint.adapter_ref.clone(),
        protocol_profile_revision: endpoint.adapter_revision,
    };
    fact.capabilities.native_reasoning = ComputeManagementFactValueV2 {
        value: None,
        basis: ComputeManagementFactBasisV2::Unknown,
    };
    // The original generic fallback is intentionally retained in the raw fact:
    // materialization must resolve the actual current endpoint/model default.
    fact.native_reasoning = NativeReasoningCapabilityV1::Toggle {
        parameter: "enable_thinking".into(),
    };
    fact.credential = ComputeManagementCredentialCompilationV2::Native {
        ordered: vec![hiroute_domain::ComputeCredentialSelectionV2::Credential {
            credential_ref: hiroute_domain::CredentialRefV1::new(
                "credential/scoped-api",
                format!("source/{}", fact.source_id),
                "hirouted",
                "provider-auth",
                [fact.target.credential_destination().unwrap()],
                1,
            )
            .unwrap(),
        }],
    };
    fact
}

#[test]
fn old_api_imports_offer_effort_profiles_and_render_the_actual_native_parameter() {
    let catalog = current_catalog();
    for model in ["gpt-6.1-sol", "gpt-5.6-luna"] {
        for protocol in [
            UpstreamProtocol::Responses,
            UpstreamProtocol::ChatCompletions,
        ] {
            let fact = observed_api(&catalog, "openai.platform.global.v1", protocol, model);
            let before = serde_json::to_vec(&fact.capabilities).unwrap();
            let candidate = materialize_management_candidate(&fact, Some(&catalog)).unwrap();
            candidate.validate().unwrap();
            let expected_profiles = if model == "gpt-5.6-luna" {
                vec!["none", "low", "medium", "high", "xhigh", "max"]
            } else {
                vec!["low", "medium", "high", "xhigh", "max"]
            };
            assert!(
                matches!(&candidate.reasoning, NativeReasoningCapabilityV1::Discrete { profiles, .. }
            if profiles.iter().map(String::as_str).collect::<Vec<_>>() == expected_profiles)
            );
            let profile = candidate
                .protocol_profiles
                .iter()
                .find(|p| p.ingress_protocol == protocol)
                .unwrap();
            let high = profile
                .capability
                .reasoning_profiles
                .iter()
                .find(|r| r.profile_id == "high")
                .unwrap();
            assert_eq!(
                high.render,
                GatewayNativeReasoningRenderV1::ExactFields {
                    protocol,
                    fields: vec![field(
                        "reasoning_effort",
                        GatewayNativeReasoningValueV1::String("high".into()),
                        protocol
                    )],
                }
            );
            assert_eq!(serde_json::to_vec(&fact.capabilities).unwrap(), before);
        }
    }
}

#[test]
fn old_anthropic_api_imports_render_effort_and_adaptive_thinking_together() {
    let catalog = current_catalog();
    let fact = observed_api(
        &catalog,
        "anthropic.platform.global.v1",
        UpstreamProtocol::Messages,
        "claude-opus-5-5",
    );
    let candidate = materialize_management_candidate(&fact, Some(&catalog)).unwrap();
    candidate.validate().unwrap();
    let profile = candidate
        .protocol_profiles
        .iter()
        .find(|p| p.ingress_protocol == UpstreamProtocol::Messages)
        .unwrap();
    let high = profile
        .capability
        .reasoning_profiles
        .iter()
        .find(|r| r.profile_id == "high")
        .unwrap();
    assert_eq!(
        high.render,
        GatewayNativeReasoningRenderV1::ExactFields {
            protocol: UpstreamProtocol::Messages,
            fields: vec![
                field(
                    "output_config.effort",
                    GatewayNativeReasoningValueV1::String("high".into()),
                    UpstreamProtocol::Messages
                ),
                field(
                    "thinking.type",
                    GatewayNativeReasoningValueV1::String("adaptive".into()),
                    UpstreamProtocol::Messages
                ),
            ],
        }
    );
}

#[test]
fn manual_api_controls_and_saved_default_preferences_keep_their_original_meaning() {
    let catalog = current_catalog();
    let mut fact = observed_api(
        &catalog,
        "openai.platform.global.v1",
        UpstreamProtocol::Responses,
        "gpt-6.1-sol",
    );
    let manual = NativeReasoningCapabilityV1::Discrete {
        parameter: "reasoning_effort".into(),
        profiles: vec!["low".into(), "high".into()],
        default_profile: Some("low".into()),
    };
    fact.native_reasoning = manual.clone();
    fact.capabilities.native_reasoning = user_fact(manual.clone());
    assert_eq!(
        materialize_management_candidate(&fact, Some(&catalog))
            .unwrap()
            .reasoning,
        manual
    );
    fact = observed_api(
        &catalog,
        "openai.platform.global.v1",
        UpstreamProtocol::Responses,
        "gpt-6.1-sol",
    );
    fact.default_preferences.insert(
        ModelDefaultFieldV1::NativeReasoning,
        ModelDefaultPreferenceV1::PreserveSaved,
    );
    assert_eq!(
        materialize_management_candidate(&fact, Some(&catalog))
            .unwrap()
            .reasoning,
        NativeReasoningCapabilityV1::Discrete {
            parameter: "reasoning_effort".into(),
            profiles: vec![
                "low".into(),
                "medium".into(),
                "high".into(),
                "xhigh".into(),
                "max".into()
            ],
            default_profile: Some("medium".into()),
        }
    );
}

#[test]
fn custom_anthropic_efforts_keep_the_required_adaptive_wire_companion() {
    let catalog = current_catalog();
    let mut fact = observed_api(
        &catalog,
        "anthropic.platform.global.v1",
        UpstreamProtocol::Messages,
        "claude-opus-5-5",
    );
    let custom = NativeReasoningCapabilityV1::Discrete {
        parameter: "output_config.effort".into(),
        profiles: vec!["low".into(), "high".into()],
        default_profile: Some("low".into()),
    };
    fact.default_preferences.insert(
        ModelDefaultFieldV1::NativeReasoning,
        ModelDefaultPreferenceV1::Custom {
            value: hiroute_domain::ModelDefaultValueV1::Reasoning(custom.clone()),
        },
    );
    let candidate = materialize_management_candidate(&fact, Some(&catalog)).unwrap();
    candidate.validate().unwrap();
    assert_eq!(candidate.reasoning, custom);
    let profile = candidate
        .protocol_profiles
        .iter()
        .find(|p| p.ingress_protocol == UpstreamProtocol::Messages)
        .unwrap()
        .capability
        .reasoning_profiles
        .iter()
        .find(|r| r.profile_id == "high")
        .unwrap();
    assert_eq!(
        profile.render,
        GatewayNativeReasoningRenderV1::ExactFields {
            protocol: UpstreamProtocol::Messages,
            fields: vec![
                field(
                    "output_config.effort",
                    GatewayNativeReasoningValueV1::String("high".into()),
                    UpstreamProtocol::Messages
                ),
                field(
                    "thinking.type",
                    GatewayNativeReasoningValueV1::String("adaptive".into()),
                    UpstreamProtocol::Messages
                )
            ]
        }
    );
}

#[test]
fn custom_unknown_and_following_missing_metadata_never_resurrect_saved_controls() {
    let catalog = current_catalog();
    for preference in [
        ModelDefaultPreferenceV1::Custom {
            value: hiroute_domain::ModelDefaultValueV1::Unknown(()),
        },
        ModelDefaultPreferenceV1::FollowMetadata,
    ] {
        let mut fact = observed_api(
            &catalog,
            "openai.platform.global.v1",
            UpstreamProtocol::Responses,
            "gpt-reserve",
        );
        let old = NativeReasoningCapabilityV1::Discrete {
            parameter: "reasoning_effort".into(),
            profiles: vec!["low".into(), "high".into()],
            default_profile: Some("high".into()),
        };
        fact.native_reasoning = old.clone();
        fact.capabilities.native_reasoning = user_fact(old);
        fact.default_preferences
            .insert(ModelDefaultFieldV1::NativeReasoning, preference);
        let candidate = materialize_management_candidate(&fact, Some(&catalog)).unwrap();
        candidate.validate().unwrap();
        assert_eq!(
            candidate.reasoning,
            NativeReasoningCapabilityV1::Fixed {
                profile: "provider-default".into()
            }
        );
    }
}
