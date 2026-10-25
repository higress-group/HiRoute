//! Real daemon artifact activation and reference persistence, without claiming Skill execution.
use super::*;
use hiroute_application::agent_connection::{
    CollaborationSkillTemplate, plan_skill_install, settings_skill_file_intent,
};
use hiroute_domain::{
    AgentConnectionControlIntentV1, AgentConnectionEffectRoleV1,
    AgentConnectionTransactionSubjectV1, NativeAgentArtifactPort, OperationState,
};

#[test]
fn settings_skill_activation_records_original_operation_and_native_target() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::effects::tests::skill_activation::settings_skill_activation_records_original_operation_and_native_target",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let runtime = super::super::super::ProductionControlRuntime::prepare_for_role_all(
        root.path(),
        crate::release_catalog::fixture_catalog(),
        None,
    )
    .unwrap();
    let adapter = &runtime.adapter;
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    for (agent, profile, integration, native) in [
        (
            "agent_codex_default",
            hiroute_integrations::CODEX_PROFILE_ID_V1,
            hiroute_integrations::CODEX_INTEGRATION_PROFILE_REF_V1,
            ".agents/skills/hiroute-collaboration/SKILL.md",
        ),
        (
            "agent_claude_default",
            hiroute_integrations::CLAUDE_PROFILE_ID_V1,
            hiroute_integrations::CLAUDE_INTEGRATION_PROFILE_REF_V1,
            ".claude/skills/hiroute-collaboration/SKILL.md",
        ),
    ] {
        let context = format!("context/{agent}");
        let root_ref = format!("skill-root/{agent}");
        let template = CollaborationSkillTemplate::bundled(
            "test-original",
            "---\nname: hiroute-collaboration\n---\nOriginal Skill content.\n",
        )
        .unwrap();
        let planned = plan_skill_install(&root_ref, &context, &template, None, None).unwrap();
        let spec = ChangeSpecV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            command_id: "agents.settings.apply".into(),
            resource_id: Some(context.clone()),
            desired_state: json!({"schema_version":{"major":2,"minor":0},"context_id":context,
                "collaboration":{"intent":"configure","settings":{"trigger_mode":"explicit"}}}),
        };
        let control = AgentConnectionControlIntentV1::from_settings_planner(
            AgentConnectionTransactionSubjectV1::from_registered_profile(
                agent,
                profile,
                integration,
            )
            .unwrap(),
            &spec,
            true,
            &json!({"revision":1}),
        )
        .unwrap();
        let target = AgentConnectionEffectRoleV1::RoutingSkill
            .target_for(control.subject())
            .unwrap();
        let intent =
            settings_skill_file_intent(&control, &context, None, &planned, Some(&template), None)
                .unwrap();
        let plan =
            TransactionPlanV1::from_agent_connection_planner(spec, control, vec![intent.clone()])
                .unwrap();
        let mut operation = admit(adapter, plan, agent);
        adapter.validate_external_admission(&intent).unwrap();
        let staged = adapter.apply_external(&operation, &intent).unwrap();
        operation
            .step_mut(OperationStepKind::ApplyAgentArtifacts)
            .effects = vec![staged.clone()];
        adapter
            .stores_lock()
            .unwrap()
            .control()
            .save_operation(&mut operation)
            .unwrap();
        let path = home.join(native);
        assert!(!path.exists(), "staging leaves native Skill absent");
        assert!(
            adapter
                .prepare_agent_artifact_activation(&operation, &intent)
                .is_err(),
            "wrong phase"
        );
        operation.state = OperationState::Activating;
        adapter
            .stores_lock()
            .unwrap()
            .control()
            .save_operation(&mut operation)
            .unwrap();
        let mut forged = operation.clone();
        forged.accepted_digest = CanonicalDigest::of_bytes(b"different confirmation");
        assert!(
            adapter
                .prepare_agent_artifact_activation(&forged, &intent)
                .is_err()
        );
        adapter
            .prepare_agent_artifact_activation(&operation, &intent)
            .unwrap();
        let record = adapter
            .stores_lock()
            .unwrap()
            .control()
            .skill_installation(&operation.workspace_id, &root_ref)
            .unwrap()
            .unwrap();
        assert_eq!(
            record.file_effect.as_ref().unwrap().operation_id,
            operation.operation_id
        );
        assert_eq!(record.revision, 1);
        assert!(
            !path.exists(),
            "reference is durable before file activation"
        );
        adapter.activate_external(&operation, &staged).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), template.content);
        assert_eq!(
            &**adapter
                .artifacts
                .read_native_target(&target)
                .unwrap()
                .unwrap(),
            template.content.as_bytes()
        );
        // The coordinator invokes the preparation hook even if observe_external reports Applied.
        // A lost response must replay the same reference, not advance its revision.
        adapter
            .prepare_agent_artifact_activation(&operation, &intent)
            .unwrap();
        assert_eq!(
            adapter
                .stores_lock()
                .unwrap()
                .control()
                .skill_installation(&operation.workspace_id, &root_ref)
                .unwrap(),
            Some(record)
        );
        fs::write(&path, b"User changed Skill\n").unwrap();
        assert!(
            adapter
                .prepare_agent_artifact_activation(&operation, &intent)
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"User changed Skill\n");
        // This component fixture activated the file above. Record that step before
        // settling its journal; a Succeeded label alone is not a file receipt proof.
        operation
            .step_mut(OperationStepKind::ApplyAgentArtifacts)
            .status = hiroute_domain::OperationStepStatus::Applied;
        operation.state = OperationState::Succeeded;
        adapter
            .stores_lock()
            .unwrap()
            .control()
            .finish_operation(&mut operation)
            .unwrap();
    }
}

fn admit(adapter: &LocalControlAdapter, plan: TransactionPlanV1, key: &str) -> OperationV1 {
    let stores = adapter.stores_lock().unwrap();
    let workspace = WorkspaceId::default();
    let scope =
        IdempotencyScopeV1::new("interactive-user", "ApplyAgentConnectionChange", key).unwrap();
    let digest = CanonicalDigest::of_bytes(key.as_bytes());
    let mut operation = OperationV1::new(
        hiroute_domain::OperationId::derive(&workspace, &scope, &digest),
        workspace,
        scope,
        digest.clone(),
        digest,
        stores
            .control()
            .current_revisions(&WorkspaceId::default())
            .unwrap(),
        plan,
    )
    .unwrap();
    let capability = format!("skill-activation-test-capability-{key}");
    stores
        .apply_capability_registrar()
        .register(
            ApplyCapabilityRegistrationV1::from_protected_launcher(
                capability.clone(),
                operation.idempotency.principal.clone(),
                operation.workspace_id.clone(),
                operation.idempotency.operation_kind.clone(),
                operation.accepted_digest.clone(),
                operation.expected_revisions.clone(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64
                    + 60,
            )
            .unwrap(),
        )
        .unwrap();
    let authorization = stores
        .control()
        .verify_apply_authorization(
            &ProtectedApplyCapability::new(capability).unwrap(),
            &operation.workspace_id,
            &operation.idempotency.principal,
            &operation.idempotency.operation_kind,
            &operation.accepted_digest,
            &operation.expected_revisions,
        )
        .unwrap();
    assert_eq!(
        stores
            .control()
            .begin_operation(&operation, &authorization)
            .unwrap(),
        BeginOperationOutcome::Created
    );
    operation.state = OperationState::ApplyingAgentArtifacts;
    stores.control().save_operation(&mut operation).unwrap();
    operation
}
