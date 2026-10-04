use super::*;

#[test]
fn existing_model_profiles_keep_their_serialized_bytes() {
    for original in [
        r#"{"schema":"hiroute.agent-profile/v1","profile_id":"codex.fixture","integration_profile_ref":"integration/codex/fixture","kind":"codex","exact_versions":["1.2.3"],"ingress_protocol":"responses","config_precedence":["process","launch","project","user","managed"],"owned_config_fields":[{"field_id":"model","path":"model","write_layer":"user"}],"dynamic_catalog":false,"static_catalog_fallback":false,"native_subagent_routing":false}"#,
        r#"{"schema":"hiroute.agent-profile/v1","profile_id":"claude_code.fixture","integration_profile_ref":"integration/claude_code/fixture","kind":"claude_code","exact_versions":["1.2.3"],"ingress_protocol":"messages","config_precedence":["managed","process","launch","project","user"],"owned_config_fields":[{"field_id":"model","path":"model","write_layer":"user"}],"dynamic_catalog":false,"static_catalog_fallback":false,"native_subagent_routing":false}"#,
    ] {
        let profile: AgentProfileV1 = serde_json::from_str(original).unwrap();
        profile.validate().unwrap();
        assert!(profile.model_connection().unwrap().is_some());
        assert_eq!(serde_json::to_string(&profile).unwrap(), original);
    }
}

#[test]
fn collaboration_only_profile_cannot_authorize_model_configuration_or_probe() {
    let json = r#"{"schema":"hiroute.agent-profile/v1","profile_id":"qoder.native.v1","integration_profile_ref":"integration/qoder/v1","kind":"qoder","config_precedence":[],"owned_config_fields":[],"dynamic_catalog":false,"static_catalog_fallback":false,"native_subagent_routing":false}"#;
    let profile: AgentProfileV1 = serde_json::from_str(json).unwrap();
    assert!(profile.model_connection().unwrap().is_none());
    assert_eq!(serde_json::to_string(&profile).unwrap(), json);
    assert!(
        BuiltInAgentProbeV1::for_profile(
            &profile,
            crate::ModelAlias::parse_custom("example").unwrap()
        )
        .is_err()
    );
    let mut forged = profile.clone();
    forged.ingress_protocol = Some(AgentIngressProtocolV1::Responses);
    assert!(forged.model_connection().is_err());
    forged = profile.clone();
    forged
        .owned_config_fields
        .push(OwnedConfigFieldV1::new("model", "model", ConfigLayerV1::User).unwrap());
    assert!(forged.validate().is_err());
    forged = profile;
    forged.kind = AgentKindV1::Codex;
    assert!(forged.validate().is_err());
}

#[test]
fn qoder_model_capability_does_not_claim_native_catalog_or_default_ownership() {
    let mut profile = AgentProfileV1 {
        schema: AGENT_PROFILE_SCHEMA_V1.into(),
        profile_id: "qoder-collaboration-v1".into(),
        integration_profile_ref: "builtin/qoder-collaboration/v1".into(),
        kind: AgentKindV1::Qoder,
        legacy_exact_versions: Default::default(),
        ingress_protocol: Some(AgentIngressProtocolV1::Responses),
        config_precedence: AgentKindV1::Qoder.config_precedence().to_vec(),
        owned_config_fields: vec![
            OwnedConfigFieldV1::new(
                "additional_provider",
                "hiroute.additional_provider",
                ConfigLayerV1::User,
            )
            .unwrap(),
        ],
        dynamic_catalog: false,
        static_catalog_fallback: false,
        native_subagent_routing: false,
        spawn_guidance: None,
        managed_launch: None,
    };
    assert!(profile.model_connection().unwrap().is_some());
    assert_eq!(profile.catalog_delivery(true), None);
    profile.dynamic_catalog = true;
    assert!(profile.validate().is_err());
    profile.dynamic_catalog = false;
    profile.ingress_protocol = Some(AgentIngressProtocolV1::Messages);
    assert!(profile.validate().is_err());
}
