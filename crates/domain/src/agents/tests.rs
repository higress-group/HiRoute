use std::collections::BTreeSet;

use serde_json::json;

use super::*;
use crate::{AgentClaudePresetValuesV2, CanonicalDigest, ModelAlias};

fn profile(kind: AgentKindV1, protocol: AgentIngressProtocolV1) -> AgentProfileV1 {
    AgentProfileV1 {
        schema: AGENT_PROFILE_SCHEMA_V1.to_owned(),
        profile_id: format!("{}.verified.v1", kind.as_str()),
        integration_profile_ref: format!("integration/{}/v1", kind.as_str()),
        kind,
        legacy_exact_versions: BTreeSet::from(["1.2.3".to_owned()]),
        ingress_protocol: protocol,
        config_precedence: kind.config_precedence().to_vec(),
        owned_config_fields: vec![
            OwnedConfigFieldV1::new("provider", "model_provider", ConfigLayerV1::User).unwrap(),
        ],
        dynamic_catalog: kind == AgentKindV1::Codex,
        static_catalog_fallback: true,
        native_subagent_routing: true,
        spawn_guidance: (kind == AgentKindV1::Codex).then(|| SpawnGuidanceProfileV1 {
            tool_name: "spawn_agent".to_owned(),
            registration_id: "codex.builtin.spawn-agent.v1".to_owned(),
            begin_marker:
                "Available model overrides (optional; inherited parent model is preferred):"
                    .to_owned(),
            end_marker: "Spawn arguments:".to_owned(),
            expected_shape_digest: CanonicalDigest::of(&json!({"type": "object"})).unwrap(),
        }),
        managed_launch: None,
    }
}

#[test]
fn agents_profiles_bind_protocol_to_exact_kind_and_version() {
    let codex = profile(AgentKindV1::Codex, AgentIngressProtocolV1::Responses);
    assert!(codex.validate().is_ok());

    let mut invalid = codex;
    invalid.ingress_protocol = AgentIngressProtocolV1::Messages;
    assert_eq!(
        invalid.validate().unwrap_err(),
        AgentProfileError::ProtocolKindMismatch
    );
}

#[test]
fn agents_catalog_delivery_prefers_dynamic_then_static_restart() {
    let codex = profile(AgentKindV1::Codex, AgentIngressProtocolV1::Responses);
    assert_eq!(
        codex.catalog_delivery(true),
        Some(CatalogDeliveryV1::DynamicWithEtag)
    );
    assert_eq!(
        codex.catalog_delivery(false),
        Some(CatalogDeliveryV1::StaticRestartRequired)
    );
}

#[test]
fn agents_probe_has_no_caller_control_over_protocol_or_prompt() {
    let claude = profile(AgentKindV1::ClaudeCode, AgentIngressProtocolV1::Messages);
    let probe = BuiltInAgentProbeV1::for_profile(
        &claude,
        ModelAlias::parse("hiroute/0011223344556677").unwrap(),
    )
    .unwrap();
    assert_eq!(probe.protocol, AgentIngressProtocolV1::Messages);
    assert_eq!(probe.prompt, CONNECTIVITY_PROBE_PROMPT_V1);
    assert!(!probe.allow_tools);
    probe.validate_for(&claude).unwrap();
}

#[test]
fn managed_launch_profile_preserves_legacy_labels_without_admission() {
    let legacy = profile(AgentKindV1::ClaudeCode, AgentIngressProtocolV1::Messages);
    let encoded = serde_json::to_value(&legacy).unwrap();
    assert!(encoded.get("managed_launch").is_none());
    let decoded: AgentProfileV1 = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.managed_launch, None);

    let mut managed = legacy;
    managed.managed_launch = Some(ManagedLaunchProfileV1::claude_code());
    for label in ["2.1.0", "99.1.2", "", "unparseable version"] {
        managed.legacy_exact_versions = [label.to_owned()].into();
        managed
            .managed_launch
            .as_mut()
            .unwrap()
            .legacy_exact_version = label.into();
        assert!(managed.validate().is_ok());
        assert!(managed.supports_managed_launch());
    }
    managed.managed_launch.as_mut().unwrap().settings_argument = "--untrusted".into();
    assert!(managed.validate().is_err());
    assert!(!managed.supports_managed_launch());
}

#[test]
fn managed_launch_descriptor_freezes_helper_loopback_presets_and_environment() {
    let descriptor = ManagedClaudeLaunchDescriptorV2::trusted(
        "agent-connection/claude-default",
        "claude-messages-v1",
        "/opt/claude/bin/claude",
        CanonicalDigest::of_bytes(b"snapshot"),
        3,
        CanonicalDigest::of_bytes(b"publication"),
        "http://127.0.0.1:5837",
        AgentClaudePresetValuesV2 {
            opus: Some("hr-plan-opus".to_owned()),
            sonnet: None,
            haiku: Some("native-haiku".to_owned()),
        },
        "/opt/hiroute/bin/hiroute",
    )
    .unwrap();
    assert_eq!(
        descriptor.helper_argv,
        [
            HIDDEN_AGENT_GRANT_HELPER_VERB_V1,
            "agent-connection/claude-default"
        ]
    );
    assert_eq!(
        descriptor.environment_removals,
        MANAGED_CLAUDE_ENVIRONMENT_REMOVALS_V1
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    assert!(
        descriptor
            .validate_trusted_helper("/opt/hiroute/bin/hiroute")
            .is_ok()
    );

    let mut untrusted = descriptor.clone();
    untrusted.gateway_base_url = "https://127.0.0.1:5837".to_owned();
    assert_eq!(
        untrusted.validate().unwrap_err(),
        ManagedLaunchError::InvalidDescriptor
    );
    let mut untrusted = descriptor.clone();
    untrusted.presets.opus = Some("bad name\n".to_owned());
    assert_eq!(
        untrusted.validate().unwrap_err(),
        ManagedLaunchError::InvalidDescriptor
    );
    let mut untrusted = descriptor.clone();
    untrusted.helper_argv.push("extra".to_owned());
    assert_eq!(
        untrusted.validate().unwrap_err(),
        ManagedLaunchError::InvalidDescriptor
    );

    let mut contextual = descriptor;
    contextual.connection_id = format!(
        "agent-connection/agent-context/claude/{}",
        CanonicalDigest::of_bytes(b"claude-settings-path").as_str()
    );
    contextual.helper_argv[1] = contextual.connection_id.clone();
    assert!(contextual.validate().is_ok());
    for invalid in [
        "agent-connection/../context",
        "agent-connection/context\n",
        "agent-connection//context",
    ] {
        contextual.connection_id = invalid.to_owned();
        contextual.helper_argv[1] = invalid.to_owned();
        assert_eq!(
            contextual.validate(),
            Err(ManagedLaunchError::InvalidDescriptor)
        );
    }
}
