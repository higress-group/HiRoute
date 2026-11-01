use crate::compiler::test_fixtures::{compiled_publication, custom_desired};
use hiroute_domain::delegation::WorkerHarnessV1;
use hiroute_domain::*;

fn version() -> PlanVersionV1 {
    let compiled = compiled_publication(1)
        .plans
        .into_iter()
        .find(|p| p.agent_plan_id().as_str() == "plan/custom")
        .unwrap();
    let desired = custom_desired();
    let AgentPlanStrategyV1::Custom { candidates } = desired.strategy else {
        panic!("custom fixture");
    };
    PlanVersionV1::new(
        WorkspaceId::default(),
        AgentPlanAuthoringV2 {
            follow_request_reasoning: false,
            schema: PLAN_AUTHORING_SCHEMA_V2.into(),
            display_name: desired.display_name,
            purpose: desired.purpose,
            mode: PlanEditorMode::FixedModel,
            requirements: desired.requirements,
            limits: desired.limits,
            strategy: AgentPlanStrategyV2::Custom { candidates },
            delegation_enabled: false,
            work: None,
        },
        compiled,
    )
    .unwrap()
}

#[test]
fn complete_version_binds_configuration_compiled_content_and_workspace() {
    let version = version();
    version.validate().unwrap();
    let mut changed = version.clone();
    changed.configuration.purpose = AgentPlanPurpose::parse("different task").unwrap();
    assert_eq!(changed.validate(), Err(PlanVersionError::Invalid));
    let mut changed = version.clone();
    changed.reference.workspace_id = WorkspaceId::parse("workspace/other").unwrap();
    assert_eq!(changed.validate(), Err(PlanVersionError::Invalid));
    let mut changed = version.clone();
    if let AgentPlanStrategyV2::Custom { candidates } = &mut changed.configuration.strategy {
        candidates.reverse();
    }
    assert!(
        PlanVersionV1::new(
            WorkspaceId::default(),
            changed.configuration,
            changed.compiled
        )
        .is_err()
    );
}

#[test]
fn work_configuration_changes_full_identity_without_changing_route() {
    let before = version();
    let mut configured = before.configuration.clone();
    configured.delegation_enabled = true;
    configured.work = Some(WorkerPlanV1 {
        harness: WorkerHarnessV1::CodexCli,
        protocol: AgentIngressProtocolV1::Responses,
    });
    let with_worker =
        PlanVersionV1::new(WorkspaceId::default(), configured, before.compiled.clone()).unwrap();
    assert_ne!(
        before.reference.content_digest,
        with_worker.reference.content_digest
    );
    assert_eq!(
        before.compiled.body.materialized_route_digest,
        with_worker.compiled.body.materialized_route_digest
    );
    assert_eq!(
        before.compiled.model_alias(),
        with_worker.compiled.model_alias()
    );
}
