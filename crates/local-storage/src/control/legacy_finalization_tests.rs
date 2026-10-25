//! Old-format discovery must distinguish a terminal journal from committed Plan content.
use super::*;
use hiroute_domain::{
    ChangeSpecV1, GatewayPublicationV1, OperationState, OperationStepStatus, PlanHeadV1,
    PlanLifecycleV1, PlanVersionV1, PublicationRecordV1,
};

fn content_operation() -> OperationV1 {
    let publication: GatewayPublicationV1 = serde_json::from_slice(include_bytes!(
        "../../../../e2e/product/fixtures/routing/current-publication.v3.json"
    ))
    .unwrap();
    let mut publication = publication.into_current().unwrap();
    publication.publication_revision = hiroute_domain::GatewayPublicationRevision::new(1).unwrap();
    let versions: Vec<_> = publication
        .plans
        .iter()
        .map(|compiled| {
            let legacy = PlanVersionV1::from_unversioned_compiled_recovery(
                publication.workspace_id.clone(),
                compiled.clone(),
            )
            .unwrap();
            PlanVersionV1::new(
                publication.workspace_id.clone(),
                legacy.configuration,
                legacy.compiled,
            )
            .unwrap()
        })
        .collect();
    publication.plan_heads = versions
        .iter()
        .map(|v| PlanHeadV1 {
            reference: v.reference.clone(),
            head_revision: v.reference.content_revision,
            model_alias: v.compiled.model_alias().clone(),
            status: PlanLifecycleV1::Enabled,
        })
        .collect();
    let version = versions[0].clone();
    let head = publication.plan_heads[0].clone();
    let spec = ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: "routing.apply".into(),
        resource_id: Some(format!("agent-plan/{}", version.reference.plan_id.as_str())),
        desired_state: json!({
            "schema":"hiroute.plan-content-change/v2",
            "target":{"intent":"create","creation_key":"legacy-finalization"},
            "editor":version.configuration.editor(Some(head.model_alias.as_str().into())).unwrap(),
            "consumed_draft":null
        }),
    };
    let record =
        PublicationRecordV1::from_publication(publication.workspace_id.clone(), &publication)
            .unwrap();
    let plan =
        TransactionPlanV1::from_plan_content_planner(spec, version, head, None, None, record, None)
            .unwrap();
    let workspace = WorkspaceId::default();
    let scope =
        IdempotencyScopeV1::new("interactive-user", "ApplyAgentPlanChange", "legacy-plan").unwrap();
    let request = CanonicalDigest::of_bytes(b"legacy-plan-request");
    OperationV1::new(
        OperationId::derive(&workspace, &scope, &request),
        workspace,
        scope,
        request,
        CanonicalDigest::of_bytes(b"legacy-plan-accepted"),
        RevisionSetV1 {
            target: 0,
            dependencies: BTreeMap::new(),
        },
        plan,
    )
    .unwrap()
}

#[test]
fn legacy_initializer_uses_committed_content_without_reopening_settled_finalization() {
    for scenario in [
        "committed",
        "missing_head",
        "prepared",
        "different_head",
        "foreign_owner",
        "malformed_published_version",
        "head_column_mismatch",
        "rolled_back",
    ] {
        let pending = scenario != "committed";
        let directory = tempdir().unwrap();
        let control = ControlStore::open(
            &crate::test_storage_authority(),
            directory.path().join("control.db"),
            directory.path().join("backups"),
        )
        .unwrap();
        let mut op = content_operation();
        let authorization = authorize(&control, &op, "legacy-plan-capability", i64::MAX);
        control.begin_operation(&op, &authorization).unwrap();
        let content = op.plan.plan_content_control().unwrap().unwrap();
        control
            .stage_plan_version(&op, &content.plan_version)
            .unwrap();
        // Storage-boundary fixture: actual provider execution is not claimed.
        op.state = if scenario == "rolled_back" {
            OperationState::RolledBack
        } else {
            OperationState::Succeeded
        };
        for step in &mut op.steps {
            step.status = if scenario == "rolled_back" {
                OperationStepStatus::Compensated
            } else {
                OperationStepStatus::Applied
            };
        }
        control.finish_operation(&mut op).unwrap();
        let db = control.connection.borrow();
        let encoded: String = db
            .query_row("SELECT operation_json FROM operations", [], |r| r.get(0))
            .unwrap();
        let inline = crate::control::payloads::expand(&db, &encoded).unwrap();
        db.execute("UPDATE operations SET operation_json=?1", [inline.as_str()])
            .unwrap();
        if scenario == "missing_head" {
            // The released writer could stop after terminal journal publication but before
            // committing this head. Preserve that unfinished local intent for real recovery.
            db.execute("DELETE FROM plan_heads", []).unwrap();
            db.execute("UPDATE plan_versions SET state='prepared'", [])
                .unwrap();
        } else if scenario == "prepared" {
            db.execute("UPDATE plan_versions SET state='prepared'", [])
                .unwrap();
        } else if scenario == "different_head" {
            let mut other = content.plan_head.clone();
            other.head_revision += 1;
            other.status = PlanLifecycleV1::Disabled;
            db.execute(
                "UPDATE plan_heads SET head_revision=?1,head_json=?2",
                rusqlite::params![other.head_revision, serde_json::to_string(&other).unwrap()],
            )
            .unwrap();
        } else if scenario == "foreign_owner" {
            db.execute(
                "UPDATE plan_versions SET owner_operation_id='unrelated-operation'",
                [],
            )
            .unwrap();
        } else if scenario == "malformed_published_version" {
            db.execute("UPDATE plan_versions SET version_json='{}'", [])
                .unwrap();
        } else if scenario == "head_column_mismatch" {
            db.execute("UPDATE plan_heads SET head_revision=head_revision+1", [])
                .unwrap();
        }
        db.execute_batch(
            "BEGIN;
            DROP TRIGGER operation_recovery_insert;
            DROP TRIGGER operation_recovery_guard;
            DROP TRIGGER operation_recovery_update;
            DELETE FROM operation_recovery_members;
            DELETE FROM operation_recovery_coverage;
            DELETE FROM artifact_head_transitions;
            DELETE FROM artifact_heads;
            DELETE FROM artifact_head_coverage;
            DELETE FROM agent_native_target_heads;
            DELETE FROM agent_native_target_coverage;
            DELETE FROM publication_grant_proofs;",
        )
        .unwrap();
        let initialized = crate::control::current_dependencies::initialize(&db);
        if scenario == "malformed_published_version" {
            assert!(
                initialized.is_err(),
                "a matching head cannot authenticate damaged content"
            );
            db.execute_batch("ROLLBACK").unwrap();
            continue;
        }
        initialized.unwrap();
        let actual: bool = db
            .query_row("SELECT finalization_pending FROM operations", [], |r| {
                r.get(0)
            })
            .unwrap();
        let members: u64 = db
            .query_row("SELECT count(*) FROM operation_recovery_members", [], |r| {
                r.get(0)
            })
            .unwrap();
        let after: String = db
            .query_row("SELECT operation_json FROM operations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            after,
            inline.as_str(),
            "discovery must preserve old journal bytes"
        );
        assert_eq!(
            actual, pending,
            "{scenario}: terminal label alone is not the completion proof"
        );
        assert_eq!(members, u64::from(pending));
        db.execute_batch("ROLLBACK").unwrap();
    }
}
