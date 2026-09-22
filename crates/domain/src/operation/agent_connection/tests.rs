use std::collections::BTreeMap;

use serde_json::json;

use super::*;
use crate::{CHANGE_SPEC_SCHEMA_V1, RevisionSetV1};

fn spec(transaction: AgentConnectionTransactionKindV1) -> ChangeSpecV1 {
    ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: transaction.command_id().to_owned(),
        resource_id: Some("agent-connection/codex-default".to_owned()),
        desired_state: match transaction {
            AgentConnectionTransactionKindV1::Settings => {
                panic!("legacy fixture does not model independent settings")
            }
            AgentConnectionTransactionKindV1::Apply => json!({
                "agent_id": "agent.codex",
                "profile_id": "default",
                "default_agent_plan_id": "plan.primary",
                "native_subagent_routing": "enabled"
            }),
            AgentConnectionTransactionKindV1::Restore => json!({
                "agent_id": "agent.codex",
                "profile_id": "default",
                "restore_point_ref": "restore.codex.1"
            }),
        },
    }
}

fn subject() -> AgentConnectionTransactionSubjectV1 {
    AgentConnectionTransactionSubjectV1::from_registered_profile(
        "agent.codex",
        "default",
        "codex.profile.v1",
    )
    .unwrap()
}

fn plan(
    transaction: AgentConnectionTransactionKindV1,
    with_native_routing: bool,
    with_spawn_guidance: bool,
) -> TransactionPlanV1 {
    let spec = spec(transaction);
    let control = AgentConnectionControlIntentV1::from_registered_planner(
        transaction,
        subject(),
        &spec,
        &json!({"connection_revision": 4, "desired_digest": "connection.digest.v4"}),
    )
    .unwrap();
    let mut roles = vec![
        AgentConnectionEffectRoleV1::GrantScopedPublication,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
    ];
    if with_native_routing {
        roles.extend([
            AgentConnectionEffectRoleV1::ModelCatalog,
            AgentConnectionEffectRoleV1::RoutingSkill,
            AgentConnectionEffectRoleV1::InstructionOverlay,
        ]);
    }
    if with_spawn_guidance {
        roles.push(AgentConnectionEffectRoleV1::SpawnGuidanceRewrite);
    }
    let external = roles
        .into_iter()
        .map(|role| {
            ExternalEffectIntentV1::from_agent_connection_planner(
                &control,
                role,
                None,
                &json!({"role": role.as_str(), "content_digest": "sha256:fixture"}),
                if role == AgentConnectionEffectRoleV1::ManagedConfiguration {
                    0o640
                } else {
                    0o644
                },
            )
            .unwrap()
        })
        .collect();
    TransactionPlanV1::from_agent_connection_planner(spec, control, external).unwrap()
}

#[test]
fn exact_apply_and_restore_variants_accept_complete_owned_effect_sets() {
    for transaction in [
        AgentConnectionTransactionKindV1::Apply,
        AgentConnectionTransactionKindV1::Restore,
    ] {
        assert_eq!(plan(transaction, false, false).external().len(), 2);
        assert_eq!(plan(transaction, true, false).external().len(), 5);
        assert_eq!(plan(transaction, true, true).external().len(), 6);
    }
}

#[test]
fn near_miss_effect_ids_and_incomplete_native_routing_sets_fail_closed() {
    let exact = plan(AgentConnectionTransactionKindV1::Apply, false, false);
    let effect = &exact.external()[0];
    assert!(matches!(
        ExternalEffectIntentV1::from_registered_adapter(
            "agent-connection-grant-publication-v2",
            effect.kind(),
            effect.target(),
            effect.before_fingerprint().cloned(),
            effect.desired().clone(),
            effect.desired_mode(),
            effect.sensitive(),
        ),
        Err(OperationValidationError::UnregisteredEffectPlan)
    ));

    let spec = spec(AgentConnectionTransactionKindV1::Apply);
    let control = AgentConnectionControlIntentV1::from_registered_planner(
        AgentConnectionTransactionKindV1::Apply,
        subject(),
        &spec,
        &json!({"connection_revision": 1}),
    )
    .unwrap();
    let external = [
        AgentConnectionEffectRoleV1::GrantScopedPublication,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
        AgentConnectionEffectRoleV1::ModelCatalog,
    ]
    .into_iter()
    .map(|role| {
        ExternalEffectIntentV1::from_agent_connection_planner(
            &control,
            role,
            None,
            &json!({"role": role.as_str()}),
            if role == AgentConnectionEffectRoleV1::ManagedConfiguration {
                0o640
            } else {
                0o644
            },
        )
        .unwrap()
    })
    .collect();
    assert!(matches!(
        TransactionPlanV1::from_agent_connection_planner(spec, control, external),
        Err(OperationValidationError::UnregisteredEffectPlan)
    ));
}

#[test]
fn unknown_operations_and_public_effect_dsl_fields_fail_closed() {
    let mut unknown = spec(AgentConnectionTransactionKindV1::Apply);
    unknown.command_id = "agents.connect.apply-near-miss".to_owned();
    assert!(matches!(
        AgentConnectionControlIntentV1::from_registered_planner(
            AgentConnectionTransactionKindV1::Apply,
            subject(),
            &unknown,
            &json!({"connection_revision": 1}),
        ),
        Err(OperationValidationError::UnregisteredEffectPlan)
    ));

    let mut injected = spec(AgentConnectionTransactionKindV1::Apply);
    injected.desired_state["effect_id"] = json!("agent-connection-managed-configuration");
    assert!(matches!(
        AgentConnectionControlIntentV1::from_registered_planner(
            AgentConnectionTransactionKindV1::Apply,
            subject(),
            &injected,
            &json!({"connection_revision": 1}),
        ),
        Err(OperationValidationError::UnregisteredEffectPlan)
    ));
}

#[test]
fn secret_bearing_payload_keys_are_rejected_before_journaling() {
    let spec = spec(AgentConnectionTransactionKindV1::Apply);
    assert!(matches!(
        AgentConnectionControlIntentV1::from_registered_planner(
            AgentConnectionTransactionKindV1::Apply,
            subject(),
            &spec,
            &json!({"authorization": "random-sentinel"}),
        ),
        Err(OperationValidationError::UnregisteredEffectPlan)
    ));
}

#[test]
fn registered_agent_effects_have_deterministic_fingerprints_and_step_digests() {
    let first = plan(AgentConnectionTransactionKindV1::Apply, true, true);
    let second = plan(AgentConnectionTransactionKindV1::Apply, true, true);
    assert_eq!(
        CanonicalDigest::of(&first).unwrap(),
        CanonicalDigest::of(&second).unwrap()
    );

    let workspace = WorkspaceId::default();
    let request_digest = CanonicalDigest::of_bytes(b"agent-connection-request");
    let accepted_digest = CanonicalDigest::of_bytes(b"agent-connection-change");
    let scope = IdempotencyScopeV1::new(
        "interactive-user",
        "ApplyAgentConnectionChange",
        "agent-connection-idem",
    )
    .unwrap();
    let operation = |plan| {
        OperationV1::new(
            OperationId::derive(&workspace, &scope, &request_digest),
            workspace.clone(),
            scope.clone(),
            request_digest.clone(),
            accepted_digest.clone(),
            RevisionSetV1 {
                target: 0,
                dependencies: BTreeMap::new(),
            },
            plan,
        )
        .unwrap()
    };
    assert_eq!(operation(first).steps, operation(second).steps);
}

#[test]
fn dedicated_grant_mutation_is_authenticated_but_plaintext_never_enters_plan() {
    let spec = spec(AgentConnectionTransactionKindV1::Apply);
    let control = AgentConnectionControlIntentV1::from_registered_planner(
        AgentConnectionTransactionKindV1::Apply,
        subject(),
        &spec,
        &json!({"connection_revision": 5}),
    )
    .unwrap();
    let external = [
        AgentConnectionEffectRoleV1::GrantScopedPublication,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
    ]
    .into_iter()
    .map(|role| {
        ExternalEffectIntentV1::from_agent_connection_planner(
            &control,
            role,
            None,
            &json!({"role": role.as_str(), "content_digest": "sha256:fixture"}),
            if role == AgentConnectionEffectRoleV1::ManagedConfiguration {
                0o640
            } else {
                0o644
            },
        )
        .unwrap()
    })
    .collect();
    let grant_scope = AgentAccessGrantScopeV1::new(
        spec.resource_id.clone().unwrap(),
        crate::AgentModelGrantV2::seal(
            crate::AgentIngressProtocolV1::Messages,
            ["hiroute/0123456789abcdef", "hiroute/fedcba9876543210"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| {
                    (
                        name.to_owned(),
                        crate::AgentModelRouteV2::Plan {
                            plan_id: crate::AgentPlanId::parse(format!("plan/{index}")).unwrap(),
                            alias: crate::ModelAlias::parse(name).unwrap(),
                            revision: 1,
                            semantic_digest: CanonicalDigest::of_bytes(name.as_bytes()),
                        },
                    )
                })
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    let mutation =
        AgentAccessGrantMutationV1::ensure("principal/local-owner", grant_scope, 0).unwrap();
    let plan = TransactionPlanV1::from_agent_connection_planner_with_agent_access_grants(
        spec,
        control,
        vec![mutation.clone()],
        external,
    )
    .unwrap();

    assert_eq!(plan.agent_access_grants(), [mutation]);
    let encoded = serde_json::to_value(&plan).unwrap();
    assert!(encoded.get("agent_access_grants").is_none());
    assert_eq!(
        encoded["control"]["agent_access_grants_digest"]
            .as_str()
            .unwrap()
            .len(),
        71
    );
    let material = AgentAccessGrantMaterial::from_csprng_entropy([0x42; 32]);
    let bytes = serde_json::to_vec(&plan).unwrap();
    assert!(
        !bytes
            .windows(material.expose().len())
            .any(|part| part == material.expose())
    );
    let serialized = String::from_utf8(bytes).unwrap();
    assert!(!serialized.contains("\"material\":"));
    assert!(!serialized.contains("\"bearer_token\":"));
}
