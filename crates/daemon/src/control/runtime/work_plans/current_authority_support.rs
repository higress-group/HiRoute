#![allow(dead_code)]
use hiroute_domain::*;
use hiroute_local_storage::{ApplyCapabilityRegistrationV1, LocalStorageSet};
use serde_json::json;

fn agent_transaction_plan(
    transaction: AgentConnectionTransactionKindV1,
    before: Option<CanonicalDigest>,
) -> TransactionPlanV1 {
    let spec = ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: transaction.command_id().to_owned(),
        resource_id: Some("agent-connection/codex-default".to_owned()),
        desired_state: match transaction {
            AgentConnectionTransactionKindV1::Settings => {
                panic!("legacy fixture does not model independent settings")
            }
            AgentConnectionTransactionKindV1::Apply => json!({
                "agent_id": "agent.codex",
                "profile_id": "default",
                "default_agent_plan_id": "plan.primary"
            }),
            AgentConnectionTransactionKindV1::Restore => json!({
                "agent_id": "agent.codex",
                "profile_id": "default",
                "restore_point_ref": "restore.codex.1"
            }),
        },
    };
    let subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
        "agent.codex",
        "default",
        "codex.profile.v1",
    )
    .unwrap();
    let control = AgentConnectionControlIntentV1::from_registered_planner(
        transaction,
        subject,
        &spec,
        &json!({"connection_revision": 4, "desired_digest": "connection.digest.v4"}),
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
            if role == AgentConnectionEffectRoleV1::ManagedConfiguration {
                before.clone()
            } else {
                None
            },
            &json!({"role": format!("{role:?}"), "content_digest": "sha256:fixture"}),
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

pub fn begin(stores: &LocalStorageSet, key: &str) -> OperationV1 {
    let workspace = WorkspaceId::default();
    let scope =
        IdempotencyScopeV1::new("interactive-user", "ApplyAgentConnectionChange", key).unwrap();
    let digest = CanonicalDigest::of_bytes(key.as_bytes());
    let revisions = stores.control().current_revisions(&workspace).unwrap();
    let operation = OperationV1::new(
        OperationId::derive(&workspace, &scope, &digest),
        workspace.clone(),
        scope,
        digest.clone(),
        digest.clone(),
        revisions.clone(),
        agent_transaction_plan(AgentConnectionTransactionKindV1::Apply, None),
    )
    .unwrap();
    let capability = format!("fixture-exact-apply-capability-{key}-0123456789");
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        + 120;
    stores
        .apply_capability_registrar()
        .register(
            ApplyCapabilityRegistrationV1::from_protected_launcher(
                capability.clone(),
                "interactive-user",
                workspace.clone(),
                "ApplyAgentConnectionChange",
                digest,
                revisions,
                expires,
            )
            .unwrap(),
        )
        .unwrap();
    let auth = stores
        .control()
        .verify_apply_authorization(
            &ProtectedApplyCapability::new(capability).unwrap(),
            &workspace,
            &operation.idempotency.principal,
            &operation.idempotency.operation_kind,
            &operation.accepted_digest,
            &operation.expected_revisions,
        )
        .unwrap();
    assert_eq!(
        stores.control().begin_operation(&operation, &auth).unwrap(),
        BeginOperationOutcome::Created
    );
    operation
}
pub fn finish(stores: &LocalStorageSet, mut operation: OperationV1) {
    operation.state = OperationState::Succeeded;
    stores.control().finish_operation(&mut operation).unwrap();
}
