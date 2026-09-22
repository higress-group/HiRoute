use hiroute_domain::{AgentIngressProtocolV1, AgentKindV1, AgentProfilesArtifactV1, ConfigLayerV1};
use hiroute_integrations::{
    AGENT_SCAN_OBSERVATION_SCHEMA_V1, AgentDiscoveryOutcomeV1, AgentScanObservationV1,
    builtin_agent_profiles, builtin_agent_profiles_artifact, resolve_agent_observation,
};

const GENERATED_PROFILES: &str =
    include_str!("../../../assets/release-facts/current/bundle/agent-profiles.json");

#[test]
fn agent_profiles_generated_artifact_has_builtin_registry_as_its_only_source() {
    let generated: AgentProfilesArtifactV1 = serde_json::from_str(GENERATED_PROFILES).unwrap();
    generated.validate().unwrap();
    assert_eq!(generated, builtin_agent_profiles_artifact());
    let mut builtin = builtin_agent_profiles();
    builtin.sort_by(|left, right| left.profile_id.cmp(&right.profile_id));
    assert_eq!(generated.profiles, builtin);
}

#[test]
fn agent_profiles_freeze_protocol_precedence_and_owned_fields_without_version_gates() {
    let artifact = builtin_agent_profiles_artifact();
    let codex = artifact
        .profiles
        .iter()
        .find(|profile| profile.kind == AgentKindV1::Codex)
        .unwrap();
    assert!(codex.legacy_exact_versions.is_empty());
    assert_eq!(codex.ingress_protocol, AgentIngressProtocolV1::Responses);
    assert_eq!(
        codex.config_precedence,
        [
            ConfigLayerV1::Process,
            ConfigLayerV1::Launch,
            ConfigLayerV1::Project,
            ConfigLayerV1::User,
            ConfigLayerV1::Managed,
        ]
    );
    assert!(codex.field("base_endpoint").is_some());
    assert!(codex.field("static_catalog").is_some());

    let claude = artifact
        .profiles
        .iter()
        .find(|profile| profile.kind == AgentKindV1::ClaudeCode)
        .unwrap();
    assert!(claude.legacy_exact_versions.is_empty());
    assert_eq!(claude.ingress_protocol, AgentIngressProtocolV1::Messages);
    assert!(claude.field("base_endpoint").is_some());
    assert!(claude.supports_managed_launch());
    assert_eq!(
        claude.managed_launch.as_ref().unwrap(),
        &hiroute_domain::ManagedLaunchProfileV1::claude_code()
    );

    for version in ["2.1.0", "2.1.231"] {
        let observation = AgentScanObservationV1 {
            schema: AGENT_SCAN_OBSERVATION_SCHEMA_V1.into(),
            agent_id: format!("agent/claude-{version}"),
            kind: AgentKindV1::ClaudeCode,
            version: version.into(),
            config: vec![],
        };
        assert!(matches!(
            resolve_agent_observation(observation),
            AgentDiscoveryOutcomeV1::Supported { .. }
        ));
    }
}

#[test]
fn agent_profiles_unknown_version_requires_action_evidence_instead_of_an_allowlist() {
    for (agent_id, kind, version) in [
        ("agent/codex-patch", AgentKindV1::Codex, "0.116.1"),
        ("agent/codex-0-147-0", AgentKindV1::Codex, "0.147.0"),
        ("agent/claude-patch", AgentKindV1::ClaudeCode, "2.1.232"),
    ] {
        let observation = AgentScanObservationV1 {
            schema: AGENT_SCAN_OBSERVATION_SCHEMA_V1.into(),
            agent_id: agent_id.into(),
            kind,
            version: version.into(),
            config: vec![],
        };
        let AgentDiscoveryOutcomeV1::Supported { installation } =
            resolve_agent_observation(observation)
        else {
            panic!(
                "a known adapter can describe an unlisted installation without claiming its actions"
            );
        };
        assert_eq!(installation.version, version);
        assert!(
            installation
                .require_action(hiroute_domain::AgentAction::ConfigureModel)
                .is_err()
        );
    }
}
