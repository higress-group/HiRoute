use std::collections::{BTreeMap, BTreeSet};

use hiroute_domain::{
    AGENT_PROFILE_SCHEMA_V1, AgentAccessGrantMutationKindV1, AgentConfigDocumentV1,
    AgentConfigRestorePointV1, AgentIngressProtocolV1, AgentKindV1, AgentPlanAllowedScopeV1,
    AgentPlanDisplayName, AgentPlanId, AgentPlanPurpose, AgentProfileV1, CatalogDeliveryV1,
    ConfigLayerV1, FunctionToolV1, ManagedLaunchProfileV1, ModelAlias, OwnedConfigFieldV1,
    PublishedAgentPlanV1, RevisionSetV1, SpawnGuidanceProfileV1, SupportedAgentInstallationV1,
};
use serde_json::json;

use super::*;

const CODEX_VERIFIED_VERSION_V1: &str = "0.116.0";
const CLAUDE_CODE_VERIFIED_VERSION_V1: &str = "2.1.0";

#[test]
fn agent_connection_apply_requires_current_action_evidence_even_for_listed_version() {
    let mut input = facts(AgentKindV1::Codex);
    input.installation.capability_evidence.clear();
    assert!(
        matches!(AgentConnectionPlanner::preview_connect(spec(AgentKindV1::Codex, false, false), input),
        Err(AgentConnectionPlanningError::UnprovenCapabilities(blocks)) if blocks.len() == 3)
    );
    let mut stale = facts(AgentKindV1::Codex);
    stale.installation.observation_digest =
        hiroute_domain::CanonicalDigest::of_bytes(b"changed-binary-or-context");
    assert!(
        matches!(AgentConnectionPlanner::preview_connect(spec(AgentKindV1::Codex, false, false), stale),
        Err(AgentConnectionPlanningError::UnprovenCapabilities(blocks))
            if blocks.iter().all(|block| block.reason == hiroute_domain::CapabilityBlockReason::Stale))
    );
}

#[test]
fn agent_connection_current_proof_allows_unlisted_version_without_version_allowlist() {
    for kind in [AgentKindV1::Codex, AgentKindV1::ClaudeCode] {
        for version in ["0.0.1", "99.1.2", "", "unparseable version"] {
            let mut input = facts(kind);
            input.installation.version = version.into();
            // The request deliberately retains its earlier diagnostic version.
            assert!(
                AgentConnectionPlanner::preview_connect(spec(kind, false, false), input).is_ok()
            );
        }
    }
}

fn installation(kind: AgentKindV1) -> SupportedAgentInstallationV1 {
    let version = match kind {
        AgentKindV1::Codex => CODEX_VERIFIED_VERSION_V1,
        AgentKindV1::ClaudeCode => CLAUDE_CODE_VERIFIED_VERSION_V1,
    };
    SupportedAgentInstallationV1 {
        agent_id: format!("agent/{}", kind.as_str()),
        version: version.to_owned(),
        profile: profile(kind),
        effective_config: BTreeMap::new(),
        observation_digest: hiroute_domain::CanonicalDigest::of_bytes(
            format!("{kind:?}-{version}").as_bytes(),
        ),
        capability_evidence: [
            hiroute_domain::AgentCapability::EffectiveConfiguration,
            hiroute_domain::AgentCapability::AtomicManagedReplace,
            hiroute_domain::AgentCapability::IngressAuthentication,
        ]
        .into_iter()
        .map(|capability| hiroute_domain::CapabilityEvidence {
            capability,
            state: hiroute_domain::CapabilityState::Proven,
            adapter_contract: "application-fixture/v1".into(),
            observed_at_unix_ms: 1,
            dependency_digest: hiroute_domain::CanonicalDigest::of_bytes(
                format!("{kind:?}-{version}").as_bytes(),
            ),
            reason: None,
        })
        .collect(),
    }
}

fn profile(kind: AgentKindV1) -> AgentProfileV1 {
    let precedence = kind.config_precedence().to_vec();
    match kind {
        AgentKindV1::Codex => {
            let tool = codex_spawn_agent_tool_v1();
            AgentProfileV1 {
                schema: AGENT_PROFILE_SCHEMA_V1.to_owned(),
                profile_id: "codex-responses-v1".to_owned(),
                integration_profile_ref: "builtin/codex-responses/v1".to_owned(),
                kind,
                legacy_exact_versions: BTreeSet::from([CODEX_VERIFIED_VERSION_V1.to_owned()]),
                ingress_protocol: AgentIngressProtocolV1::Responses,
                config_precedence: precedence,
                owned_config_fields: vec![
                    field("provider", "model_provider"),
                    field("default_model", "model"),
                    field("base_endpoint", "model_providers.hiroute.base_url"),
                    field("wire_api", "model_providers.hiroute.wire_api"),
                    field("static_catalog", "model_catalog_json"),
                ],
                dynamic_catalog: true,
                static_catalog_fallback: true,
                native_subagent_routing: true,
                spawn_guidance: Some(SpawnGuidanceProfileV1 {
                    tool_name: "spawn_agent".to_owned(),
                    registration_id: tool.registration_id.clone(),
                    begin_marker:
                        "Available model overrides (optional; inherited parent model is preferred):"
                            .to_owned(),
                    end_marker: "Spawn arguments:".to_owned(),
                    expected_shape_digest: tool.shape_digest().unwrap(),
                }),
                managed_launch: None,
            }
        }
        AgentKindV1::ClaudeCode => AgentProfileV1 {
            schema: AGENT_PROFILE_SCHEMA_V1.to_owned(),
            profile_id: "claude-messages-v1".to_owned(),
            integration_profile_ref: "builtin/claude-messages/v1".to_owned(),
            kind,
            legacy_exact_versions: BTreeSet::from([CLAUDE_CODE_VERIFIED_VERSION_V1.to_owned()]),
            ingress_protocol: AgentIngressProtocolV1::Messages,
            config_precedence: precedence,
            owned_config_fields: vec![
                field("base_endpoint", "env.ANTHROPIC_BASE_URL"),
                field("default_model", "env.ANTHROPIC_MODEL"),
                field("default_opus_model", "env.ANTHROPIC_DEFAULT_OPUS_MODEL"),
                field("default_sonnet_model", "env.ANTHROPIC_DEFAULT_SONNET_MODEL"),
                field("default_haiku_model", "env.ANTHROPIC_DEFAULT_HAIKU_MODEL"),
                field("small_fast_model", "env.ANTHROPIC_SMALL_FAST_MODEL"),
                field("auth_helper", "apiKeyHelper"),
                field("auth_environment", "hiroute.auth_environment"),
            ],
            dynamic_catalog: false,
            static_catalog_fallback: false,
            native_subagent_routing: false,
            spawn_guidance: None,
            managed_launch: None,
        },
    }
}

fn field(id: &str, path: &str) -> OwnedConfigFieldV1 {
    OwnedConfigFieldV1::new(id, path, ConfigLayerV1::User).unwrap()
}

fn codex_spawn_agent_tool_v1() -> FunctionToolV1 {
    FunctionToolV1 {
        registration_id: "codex.builtin.spawn-agent.v1".to_owned(),
        name: "spawn_agent".to_owned(),
        kind: "function".to_owned(),
        description: concat!(
            "Delegate bounded work to a native subagent.\n\n",
            "Available model overrides (optional; inherited parent model is preferred):",
            "\nbuilt-in defaults\n\n",
            "Spawn arguments: task, fork_turns, and optional model."
        )
        .to_owned(),
        parameters: json!({
            "additionalProperties": false,
            "properties": {
                "fork_turns": {"type": "string"},
                "model": {"type": "string"},
                "task": {"type": "string"}
            },
            "required": ["task"],
            "type": "object"
        }),
    }
}

fn plans() -> Vec<PublishedAgentPlanV1> {
    vec![
        PublishedAgentPlanV1 {
            agent_plan_id: AgentPlanId::parse("plan/default").unwrap(),
            model_alias: ModelAlias::parse("hiroute/0011223344556677").unwrap(),
            display_name: AgentPlanDisplayName::parse("Default").unwrap(),
            purpose: AgentPlanPurpose::parse("Handle normal implementation work").unwrap(),
            agent_plan_revision: 3,
            active: true,
            supported_ingress: BTreeSet::from([
                AgentIngressProtocolV1::Responses,
                AgentIngressProtocolV1::Messages,
            ]),
        },
        PublishedAgentPlanV1 {
            agent_plan_id: AgentPlanId::parse("plan/review").unwrap(),
            model_alias: ModelAlias::parse("hiroute/8899aabbccddeeff").unwrap(),
            display_name: AgentPlanDisplayName::parse("Review").unwrap(),
            purpose: AgentPlanPurpose::parse("Review correctness and recovery risks").unwrap(),
            agent_plan_revision: 2,
            active: true,
            supported_ingress: BTreeSet::from([
                AgentIngressProtocolV1::Responses,
                AgentIngressProtocolV1::Messages,
            ]),
        },
    ]
}

fn facts(kind: AgentKindV1) -> AgentConnectionPlanningFactsV1 {
    let installation = installation(kind);
    let activation_mode = if installation.profile.supports_managed_launch() {
        hiroute_domain::AgentActivationModeV1::ManagedLaunch
    } else {
        hiroute_domain::AgentActivationModeV1::ManagedConfiguration
    };
    AgentConnectionPlanningFactsV1 {
        installation,
        gateway: RegisteredGatewayEndpointV1 {
            access_point_ref: "access-point/local/default".to_owned(),
            base_endpoint: "http://127.0.0.1:5837/v1".to_owned(),
            trusted_cli_executable: "/opt/hiroute/bin/hiroute".to_owned(),
        },
        publication_digest: hiroute_domain::CanonicalDigest::of_bytes(b"publication-v7"),
        plans: plans(),
        plan_route_digests: plans()
            .into_iter()
            .map(|plan| {
                (
                    plan.agent_plan_id.clone(),
                    hiroute_domain::CanonicalDigest::of_bytes(
                        plan.agent_plan_id.as_str().as_bytes(),
                    ),
                )
            })
            .collect(),
        current_config: AgentConfigDocumentV1 {
            fields: BTreeMap::from([("user.theme".to_owned(), json!("dark"))]),
        },
        expected_revisions: RevisionSetV1 {
            target: 4,
            dependencies: BTreeMap::from([("publication".to_owned(), 7)]),
        },
        before_fingerprints: AgentConnectionBeforeFingerprintsV1::default(),
        registered_tools: if kind == AgentKindV1::Codex {
            vec![codex_spawn_agent_tool_v1()]
        } else {
            Vec::new()
        },
        activation_mode,
        expected_grant_generation: 0,
    }
}

fn spec(kind: AgentKindV1, native: bool, dynamic: bool) -> AgentConnectSpecV1 {
    let profile = match kind {
        AgentKindV1::Codex => profile(AgentKindV1::Codex),
        AgentKindV1::ClaudeCode => profile(AgentKindV1::ClaudeCode),
    };
    AgentConnectSpecV1 {
        schema_version: AGENT_CONNECT_SPEC_SCHEMA_V1,
        agent_id: format!("agent/{}", kind.as_str()),
        profile_id: profile.profile_id,
        installed_version: match kind {
            AgentKindV1::Codex => CODEX_VERIFIED_VERSION_V1,
            AgentKindV1::ClaudeCode => CLAUDE_CODE_VERIFIED_VERSION_V1,
        }
        .to_owned(),
        default_agent_plan_id: AgentPlanId::parse("plan/default").unwrap(),
        allowed_scope: AgentPlanAllowedScopeV1::Selected,
        allowed_agent_plan_ids: BTreeSet::from([
            AgentPlanId::parse("plan/default").unwrap(),
            AgentPlanId::parse("plan/review").unwrap(),
        ]),
        native_subagent_routing: native,
        dynamic_catalog_available: dynamic,
    }
}

#[test]
fn agent_connection_codex_dynamic_preview_seals_exact_six_role_transaction() {
    let mut facts = facts(AgentKindV1::Codex);
    facts.current_config.fields.insert(
        "model_catalog_json".to_owned(),
        json!("stale-static-catalog"),
    );
    let original_config = facts.current_config.clone();
    let preview =
        AgentConnectionPlanner::preview_connect(spec(AgentKindV1::Codex, true, true), facts)
            .unwrap();
    assert_eq!(
        preview.catalog_delivery,
        Some(CatalogDeliveryV1::DynamicWithEtag)
    );
    assert_eq!(
        preview.effective_after,
        Some(CatalogEffectiveAfterV1::Immediate)
    );
    assert_eq!(
        preview.catalog_etag,
        preview
            .catalog
            .as_ref()
            .map(|catalog| catalog.digest.clone())
    );
    assert_eq!(preview.effects.len(), 6);
    assert!(
        preview
            .config_change
            .fields
            .iter()
            .any(|field| field.path == "model_catalog_json" && field.after.is_none())
    );
    assert_eq!(original_config.fields["user.theme"], "dark");
    let plan = AgentConnectionPlanner::seal_apply(
        &preview,
        &preview.change_digest,
        &preview.expected_revisions,
        &preview.expected_revisions,
    )
    .unwrap();
    assert_eq!(plan.external().len(), 6);
    assert_eq!(plan.spec().command_id, "agents.connect.apply");
    if std::env::var_os("HIROUTE_PRINT_AGENT_DIGESTS").is_some() {
        println!("grant_digest={}", preview.connection.grant.digest);
        println!(
            "catalog_digest={}",
            preview.catalog.as_ref().unwrap().digest
        );
        println!(
            "overlay_digest={}",
            preview.overlay.as_ref().unwrap().digest
        );
        println!("change_digest={}", preview.change_digest);
    }
}

#[test]
fn agent_connection_codex_static_fallback_is_single_source_and_requires_restart() {
    let preview = AgentConnectionPlanner::preview_connect(
        spec(AgentKindV1::Codex, true, false),
        facts(AgentKindV1::Codex),
    )
    .unwrap();
    assert_eq!(
        preview.catalog_delivery,
        Some(CatalogDeliveryV1::StaticRestartRequired)
    );
    assert_eq!(
        preview.effective_after,
        Some(CatalogEffectiveAfterV1::AgentRestart)
    );
    let catalog_change = preview
        .config_change
        .fields
        .iter()
        .find(|field| field.path == "model_catalog_json")
        .unwrap();
    assert_eq!(
        catalog_change.after.as_ref().unwrap().as_str().unwrap(),
        preview.catalog.as_ref().unwrap().canonical_json().unwrap()
    );
}

#[test]
fn agent_connection_marker_mismatch_is_typed_no_op_without_blocking_overlay() {
    let mut facts = facts(AgentKindV1::Codex);
    facts.registered_tools[0].description = "unexpected tool description".to_owned();
    let preview =
        AgentConnectionPlanner::preview_connect(spec(AgentKindV1::Codex, true, true), facts)
            .unwrap();
    assert_eq!(
        preview.rewrite.disposition,
        hiroute_domain::GuidanceRewriteDispositionV1::MarkerMismatch
    );
    assert!(preview.overlay.is_some());
    assert_eq!(preview.effects.len(), 5);
    let plan = AgentConnectionPlanner::seal_apply(
        &preview,
        &preview.change_digest,
        &preview.expected_revisions,
        &preview.expected_revisions,
    )
    .unwrap();
    assert_eq!(plan.external().len(), 5);
}

#[test]
fn agent_connection_claude_uses_messages_and_no_codex_routing_artifacts() {
    let preview = AgentConnectionPlanner::preview_connect(
        spec(AgentKindV1::ClaudeCode, false, false),
        facts(AgentKindV1::ClaudeCode),
    )
    .unwrap();
    assert_eq!(
        preview.connection.protocol,
        AgentIngressProtocolV1::Messages
    );
    assert_eq!(preview.effects.len(), 2);
    assert!(preview.catalog.is_none());
    assert!(preview.overlay.is_none());
    assert!(
        preview
            .config_change
            .fields
            .iter()
            .any(|field| field.path == "env.ANTHROPIC_BASE_URL")
    );
}

#[test]
fn agent_connection_exact_2_1_231_uses_managed_launch_without_persistent_config() {
    let mut facts = facts(AgentKindV1::ClaudeCode);
    facts.installation.version = "2.1.231".to_owned();
    facts
        .installation
        .profile
        .legacy_exact_versions
        .insert("2.1.231".to_owned());
    facts.installation.profile.managed_launch = Some(ManagedLaunchProfileV1::claude_code());
    facts.activation_mode = hiroute_domain::AgentActivationModeV1::ManagedLaunch;
    let mut spec = spec(AgentKindV1::ClaudeCode, false, false);
    spec.installed_version = "2.1.231".to_owned();

    let preview = AgentConnectionPlanner::preview_connect(spec, facts).unwrap();
    assert_eq!(
        preview.connection.activation_mode,
        hiroute_domain::AgentActivationModeV1::ManagedLaunch
    );
    assert!(preview.config_change.fields.is_empty());
    assert!(preview.effects.iter().any(|effect| {
        effect.role == "managed_configuration"
            && effect.state == PreviewEffectStateV1::NoFieldChange
    }));
}

#[test]
fn agent_connection_same_independent_plan_can_be_granted_to_two_agents() {
    let codex = AgentConnectionPlanner::preview_connect(
        spec(AgentKindV1::Codex, false, false),
        facts(AgentKindV1::Codex),
    )
    .unwrap();
    let claude = AgentConnectionPlanner::preview_connect(
        spec(AgentKindV1::ClaudeCode, false, false),
        facts(AgentKindV1::ClaudeCode),
    )
    .unwrap();
    assert_eq!(
        codex.connection.grant.aliases,
        claude.connection.grant.aliases
    );
    assert_ne!(codex.connection.agent_id, claude.connection.agent_id);
}

#[test]
fn agent_connection_all_published_scope_is_derived_from_active_protocol_eligible_plans() {
    let mut request = spec(AgentKindV1::Codex, false, false);
    request.allowed_scope = AgentPlanAllowedScopeV1::AllPublished;
    request.allowed_agent_plan_ids.clear();
    let preview =
        AgentConnectionPlanner::preview_connect(request, facts(AgentKindV1::Codex)).unwrap();
    assert_eq!(
        preview.connection.grant.allowed_scope,
        AgentPlanAllowedScopeV1::AllPublished
    );
    assert_eq!(preview.connection.grant.allowed_agent_plan_ids.len(), 2);
}

#[test]
fn agent_connection_stale_preview_never_enters_transaction_engine() {
    let preview = AgentConnectionPlanner::preview_connect(
        spec(AgentKindV1::Codex, false, false),
        facts(AgentKindV1::Codex),
    )
    .unwrap();
    let mut current = preview.expected_revisions.clone();
    current.target += 1;
    assert!(matches!(
        AgentConnectionPlanner::seal_apply(
            &preview,
            &preview.change_digest,
            &preview.expected_revisions,
            &current
        ),
        Err(AgentConnectionPlanningError::PreviewStale)
    ));
}

#[test]
fn agent_connection_wire_dto_denies_unknown_fields() {
    let mut value = serde_json::to_value(spec(AgentKindV1::Codex, false, false)).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("protocol".to_owned(), json!("messages"));
    assert!(serde_json::from_value::<AgentConnectSpecV1>(value).is_err());
}

#[test]
fn agent_connection_accepts_only_canonical_registered_loopback_endpoints() {
    for endpoint in ["http://127.0.0.1:5837/v1", "http://[::1]:5837/v1"] {
        let mut planning_facts = facts(AgentKindV1::Codex);
        planning_facts.gateway.base_endpoint = endpoint.to_owned();
        let preview = AgentConnectionPlanner::preview_connect(
            spec(AgentKindV1::Codex, false, false),
            planning_facts,
        )
        .unwrap();
        let base_endpoint = preview
            .config_change
            .fields
            .iter()
            .find(|field| field.path == "model_providers.hiroute.base_url")
            .and_then(|field| field.after.as_ref())
            .and_then(serde_json::Value::as_str);
        assert_eq!(base_endpoint, Some(endpoint));
    }
}

#[test]
fn agent_connection_rejects_noncanonical_or_ambiguous_endpoints_before_agent_effects() {
    let invalid_endpoints = [
        (
            "remote userinfo target",
            "http://127.0.0.1:80@evil.example/v1",
        ),
        ("local userinfo", "http://user:password@127.0.0.1:5837/v1"),
        ("ipv4 suffix host", "http://127.0.0.1.evil.example:5837/v1"),
        ("remote host", "http://evil.example:5837/v1"),
        ("localhost alias", "http://localhost:5837/v1"),
        ("other ipv4 loopback", "http://127.0.0.2:5837/v1"),
        ("noncanonical ipv4", "http://127.1:5837/v1"),
        ("noncanonical ipv6", "http://[0:0:0:0:0:0:0:1]:5837/v1"),
        ("missing port", "http://127.0.0.1/v1"),
        ("empty port", "http://127.0.0.1:/v1"),
        ("zero port", "http://127.0.0.1:0/v1"),
        ("noncanonical port", "http://127.0.0.1:05837/v1"),
        ("invalid port", "http://127.0.0.1:65536/v1"),
        ("https scheme", "https://127.0.0.1:5837/v1"),
        ("query", "http://127.0.0.1:5837/v1?target=evil"),
        ("empty query", "http://127.0.0.1:5837/v1?"),
        ("fragment", "http://127.0.0.1:5837/v1#evil"),
        ("path suffix", "http://127.0.0.1:5837/v1/extra"),
        ("trailing slash", "http://127.0.0.1:5837/v1/"),
        ("dot traversal", "http://127.0.0.1:5837/allowed/../v1"),
        ("encoded traversal", "http://127.0.0.1:5837/%2e%2e/v1"),
        ("encoded path", "http://127.0.0.1:5837/v%31"),
    ];

    for (case, endpoint) in invalid_endpoints {
        for kind in [AgentKindV1::Codex, AgentKindV1::ClaudeCode] {
            let mut planning_facts = facts(kind);
            planning_facts.gateway.base_endpoint = endpoint.to_owned();
            let result =
                AgentConnectionPlanner::preview_connect(spec(kind, false, false), planning_facts);
            assert!(
                matches!(&result, Err(AgentConnectionPlanningError::ProfileMismatch)),
                "{case} must fail for {kind:?} before producing a Preview or Agent file effect"
            );
            let planned_agent_file_effects = result.as_ref().map_or(0, |preview| {
                preview
                    .effects
                    .iter()
                    .filter(|effect| effect.role == "managed_configuration")
                    .count()
            });
            assert_eq!(
                planned_agent_file_effects, 0,
                "{case} must plan zero {kind:?} Agent file effects"
            );
        }
    }
}

#[test]
fn agent_connection_restore_reuses_typed_transaction_and_exact_restore_point() {
    let mut connect_facts = facts(AgentKindV1::Codex);
    connect_facts.before_fingerprints.grant_publication =
        Some(connect_facts.publication_digest.clone());
    let connect = AgentConnectionPlanner::preview_connect(
        spec(AgentKindV1::Codex, true, true),
        connect_facts,
    )
    .unwrap();
    let config_restore = AgentConfigRestorePointV1::from_change(
        connect.connection.profile_id.clone(),
        connect.config_change.clone(),
    )
    .unwrap();
    let restore_point = AgentConnectionRestorePointV1::from_preview(
        "restore/codex/1",
        CODEX_VERIFIED_VERSION_V1,
        &connect,
        config_restore,
    )
    .unwrap();
    restore_point.validate().unwrap();
    if std::env::var_os("HIROUTE_PRINT_AGENT_DIGESTS").is_some() {
        println!(
            "config_restore_digest={}",
            restore_point.configuration.digest
        );
        println!("connection_restore_digest={}", restore_point.digest);
    }
    let restore_spec = AgentRestoreSpecV1 {
        schema_version: AGENT_CONNECT_SPEC_SCHEMA_V1,
        agent_id: "agent/codex".to_owned(),
        profile_id: profile(AgentKindV1::Codex).profile_id,
        installed_version: CODEX_VERIFIED_VERSION_V1.to_owned(),
        restore_point_ref: "restore/codex/1".to_owned(),
    };
    for observed_version in ["0.0.1", "99.1.2", "", "unparseable version"] {
        let mut current_installation = installation(AgentKindV1::Codex);
        current_installation.version = observed_version.into();
        current_installation.capability_evidence.retain(|proof| {
            proof.capability != hiroute_domain::AgentCapability::IngressAuthentication
        });
        let mut current_spec = restore_spec.clone();
        current_spec.installed_version = observed_version.into();
        let outcome = preview_agent_connection_restore(
            current_spec,
            restore_point.clone(),
            AgentConnectionRestorePlanningFactsV1 {
                installation: current_installation,
                current_revisions: RevisionSetV1 {
                    target: 5,
                    dependencies: BTreeMap::from([("publication".to_owned(), 8)]),
                },
                current_fingerprints: AgentConnectionBeforeFingerprintsV1::default(),
                expected_grant_generation: 4,
            },
        )
        .unwrap();
        let AgentConnectionRestorePreviewOutcomeV1::Ready(restore) = outcome else {
            panic!("exact restore point must be ready");
        };
        let plan = seal_agent_connection_restore(
            &restore,
            &restore.change_digest,
            &restore.expected_revisions,
            &restore.expected_revisions,
        )
        .unwrap();
        assert_eq!(plan.spec().command_id, "agents.restore.apply");
        assert_eq!(plan.external().len(), 6);
        let [revoke] = plan.agent_access_grants() else {
            panic!("restore must carry one exact grant revocation");
        };
        assert_eq!(revoke.kind(), AgentAccessGrantMutationKindV1::Revoke);
        assert_eq!(revoke.expected_generation(), 4);
    }
}

#[test]
fn agent_connection_unknown_restore_version_is_report_only() {
    let mut connect_facts = facts(AgentKindV1::Codex);
    connect_facts.before_fingerprints.grant_publication =
        Some(connect_facts.publication_digest.clone());
    let connect = AgentConnectionPlanner::preview_connect(
        spec(AgentKindV1::Codex, false, false),
        connect_facts,
    )
    .unwrap();
    let config_restore = AgentConfigRestorePointV1::from_change(
        connect.connection.profile_id.clone(),
        connect.config_change.clone(),
    )
    .unwrap();
    let mut restore_point = AgentConnectionRestorePointV1::from_preview(
        "restore/codex/2",
        CODEX_VERIFIED_VERSION_V1,
        &connect,
        config_restore,
    )
    .unwrap();
    restore_point.schema = "hiroute.agent-connection-restore-point/v2".to_owned();
    let outcome = preview_agent_connection_restore(
        AgentRestoreSpecV1 {
            schema_version: AGENT_CONNECT_SPEC_SCHEMA_V1,
            agent_id: "agent/codex".to_owned(),
            profile_id: profile(AgentKindV1::Codex).profile_id,
            installed_version: CODEX_VERIFIED_VERSION_V1.to_owned(),
            restore_point_ref: "restore/codex/2".to_owned(),
        },
        restore_point,
        AgentConnectionRestorePlanningFactsV1 {
            installation: installation(AgentKindV1::Codex),
            current_revisions: RevisionSetV1 {
                target: 5,
                dependencies: BTreeMap::new(),
            },
            current_fingerprints: AgentConnectionBeforeFingerprintsV1::default(),
            expected_grant_generation: 4,
        },
    )
    .unwrap();
    assert!(matches!(
        outcome,
        AgentConnectionRestorePreviewOutcomeV1::ReportOnlyUnknownVersion
    ));
}

#[test]
fn repeated_planning_snapshot_ignores_probe_time_but_not_proof_or_native_changes() {
    let first = facts(AgentKindV1::Codex);
    let mut later = first.clone();
    for proof in &mut later.installation.capability_evidence {
        proof.observed_at_unix_ms += 100;
    }
    assert_ne!(first, later);
    assert!(first.same_snapshot(&later));
    let mut changed = later.clone();
    changed.installation.capability_evidence[0].state = hiroute_domain::CapabilityState::Unknown;
    assert!(!first.same_snapshot(&changed));
    changed = later.clone();
    changed.installation.capability_evidence[0].dependency_digest =
        hiroute_domain::CanonicalDigest::of_bytes(b"different-proof-input");
    assert!(!first.same_snapshot(&changed));
    changed = later;
    changed.installation.version = "different diagnostic label".into();
    assert!(first.same_snapshot(&changed));
}

#[test]
fn client_version_changes_do_not_invalidate_configuration_snapshot() {
    for kind in [AgentKindV1::Codex, AgentKindV1::ClaudeCode] {
        let original = facts(kind);
        for version in ["0.0.1", "99.1.2", "", "unparseable version"] {
            let mut changed = original.clone();
            changed.installation.version = version.into();
            changed.installation.profile.legacy_exact_versions.clear();
            assert!(original.same_snapshot(&changed));
            let request = spec(kind, kind == AgentKindV1::Codex, false);
            let before =
                AgentConnectionPlanner::preview_connect(request.clone(), original.clone()).unwrap();
            let after = AgentConnectionPlanner::preview_connect(request, changed.clone()).unwrap();
            assert_eq!(before.change_digest, after.change_digest);
            assert_eq!(before.effects, after.effects);
            changed.installation.observation_digest =
                hiroute_domain::CanonicalDigest::of_bytes(b"different executable or configuration");
            assert!(!original.same_snapshot(&changed));
        }
    }
}
