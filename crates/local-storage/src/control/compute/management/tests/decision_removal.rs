use super::*;
use hiroute_application_api::{ApplyRequestV1, PreviewRequestV1};
use hiroute_domain::*;
use serde_json::{Value, json};

fn execute(stores: &LocalStorageSet, desired: Value, version: u8, key: &str) -> OperationV1 {
    let external = NoExternal;
    let protected = ProtectedInput;
    let runtime = TransactionRuntime::default();
    let coordinator = TransactionCoordinator::new(
        stores.control(),
        stores.secrets(),
        stores.runtime(),
        &external,
        &protected,
        &runtime,
    );
    coordinator.reconcile_startup_and_open().unwrap();
    let spec = ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: if version == 1 {
            "decision.services.apply"
        } else {
            "decision.connections.apply"
        }
        .into(),
        resource_id: Some("decision/removal".into()),
        desired_state: desired,
    };
    let preview = coordinator
        .preview(&WorkspaceId::default(), PreviewRequestV1::new(spec))
        .unwrap();
    let input = ApplyRequestV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        spec: preview.normalized_spec,
        accept_digest: preview.change_digest,
        expected_revisions: preview.expected_revisions,
        idempotency_key: key.into(),
        apply_capability: None,
    };
    let accepted = coordinator
        .accept(
            &WorkspaceId::default(),
            &VerifiedPrincipal::for_local_control(),
            input.clone(),
        )
        .unwrap();
    let completed = coordinator.run_accepted(accepted).unwrap();
    let repeated = coordinator
        .accept(
            &WorkspaceId::default(),
            &VerifiedPrincipal::for_local_control(),
            input,
        )
        .unwrap();
    assert_eq!(
        coordinator.run_accepted(repeated).unwrap().operation_id,
        completed.operation_id
    );
    completed
}

fn desired(version: u8, revision: u64, service: Option<Value>) -> Value {
    if version == 1 {
        json!({"id":"decision/removal","expected_revision":revision,"service":service,"input_slot":null})
    } else {
        let mut value = json!({"change":{"id":"decision/removal","expected_revision":revision,"service":service,"expected_source_revisions":[]},"input_slot":null});
        if version == 3 {
            value["schema"] = json!(DECISION_SERVICE_OPERATION_INPUT_SCHEMA_V3);
        }
        value
    }
}

fn service(version: u8) -> Value {
    let mut value = json!({"id":"decision/removal","revision":1,"name":"Removal fixture","connection":{"kind":"custom","endpoint":"https://decision.example.test/decide","timeout_ms":3000,"auth_header":null}});
    if version > 1 {
        value["schema"] = json!(format!("hiroute.decision-service/v{version}"));
    }
    value
}

#[test]
fn decision_removal_legacy_operations_preserve_history_tombstones_replay_and_full_audit() {
    for version in [1, 2, 3] {
        for deleted in [false, true] {
            let root = tempdir().unwrap();
            let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
            let service = service(version);
            let saved = execute(
                &stores,
                desired(version, 0, Some(service.clone())),
                version,
                "removal-save",
            );
            assert_eq!(saved.state, OperationState::Succeeded);
            let record: DecisionServiceRecord = serde_json::from_value(service).unwrap();
            let (_, history) = crate::control::plans::tests::decision_removal::fixture_for(record);
            let encoded = serde_json::to_string(&history).unwrap();
            stores.control().connection.borrow().execute(
                "INSERT INTO plan_versions(workspace_id,plan_id,content_revision,content_digest,version_json,state,owner_operation_id) VALUES(?1,?2,?3,?4,?5,'published',?6)",
                rusqlite::params![WorkspaceId::DEFAULT,history.reference.plan_id.as_str(),history.reference.content_revision,history.reference.content_digest.as_str(),encoded,saved.operation_id.as_str()],
            ).unwrap();
            if deleted {
                let head = PlanHeadV1 {
                    reference: history.reference.clone(),
                    head_revision: history.reference.content_revision + 1,
                    model_alias: history.compiled.model_alias().clone(),
                    status: PlanLifecycleV1::Deleted,
                };
                head.validate().unwrap();
                stores.control().connection.borrow().execute(
                    "INSERT INTO plan_heads(workspace_id,plan_id,head_revision,head_json) VALUES(?1,?2,?3,?4)",
                    rusqlite::params![WorkspaceId::DEFAULT,history.reference.plan_id.as_str(),head.head_revision,serde_json::to_string(&head).unwrap()],
                ).unwrap();
            }
            stores
                .control()
                .reconcile_plan_versions(&WorkspaceId::default(), &[], 1)
                .unwrap();
            assert!(
                stores
                    .control()
                    .decision_removal_preview(&WorkspaceId::default(), "decision/removal", 1)
                    .unwrap()
                    .1
                    .is_empty()
            );
            let removed = execute(
                &stores,
                desired(version, 1, None),
                version,
                "removal-delete",
            );
            assert_eq!(removed.state, OperationState::Succeeded);
            assert!(
                stores
                    .control()
                    .decision_service_records(&WorkspaceId::default())
                    .unwrap()
                    .is_empty()
            );
            assert!(stores.control().writer_claim_operation().unwrap().is_none());
            let stored: String = stores
                .control()
                .connection
                .borrow()
                .query_row("SELECT version_json FROM plan_versions", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(stored, encoded);
            let count: u64 = stores.control().connection.borrow().query_row("SELECT COUNT(*) FROM decision_plan_references WHERE service_id='decision/removal'", [], |row| row.get(0)).unwrap();
            assert_eq!(count, 1);
            crate::migrations::audit_current_storage(
                stores.control(),
                stores.runtime(),
                stores.secrets(),
            )
            .unwrap();
            drop(stores);
            let reopened = LocalStorageSet::open_for_daemon_startup(root.path())
                .unwrap_or_else(|error| panic!("V{version}, deleted={deleted}: {error:?}"));
            assert_eq!(
                reopened
                    .control()
                    .published_plan_versions(&WorkspaceId::default())
                    .unwrap(),
                vec![history]
            );
            assert!(
                reopened
                    .control()
                    .decision_service_records(&WorkspaceId::default())
                    .unwrap()
                    .is_empty()
            );
            crate::migrations::audit_current_storage(
                reopened.control(),
                reopened.runtime(),
                reopened.secrets(),
            )
            .unwrap();
        }
    }
}

#[test]
fn decision_removal_terminal_recovery_preserves_the_writer_fence_and_history_for_v1_v2_v3() {
    for version in [1, 2, 3] {
        let root = tempdir().unwrap();
        let operation_id;
        let history;
        {
            let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
            let service = service(version);
            let saved = execute(
                &stores,
                desired(version, 0, Some(service.clone())),
                version,
                "recovery-save",
            );
            let record: DecisionServiceRecord = serde_json::from_value(service).unwrap();
            history = crate::control::plans::tests::decision_removal::fixture_for(record).1;
            stores.control().connection.borrow().execute(
                "INSERT INTO plan_versions(workspace_id,plan_id,content_revision,content_digest,version_json,state,owner_operation_id) VALUES(?1,?2,?3,?4,?5,'published',?6)",
                rusqlite::params![WorkspaceId::DEFAULT,history.reference.plan_id.as_str(),history.reference.content_revision,history.reference.content_digest.as_str(),serde_json::to_string(&history).unwrap(),saved.operation_id.as_str()],
            ).unwrap();
            stores
                .control()
                .reconcile_plan_versions(&WorkspaceId::default(), &[], 1)
                .unwrap();
            let external = NoExternal;
            let protected = ProtectedInput;
            let runtime = TransactionRuntime::default();
            let coordinator = TransactionCoordinator::new(
                stores.control(),
                stores.secrets(),
                stores.runtime(),
                &external,
                &protected,
                &runtime,
            );
            coordinator.reconcile_startup_and_open().unwrap();
            let preview = coordinator
                .preview(
                    &WorkspaceId::default(),
                    PreviewRequestV1::new(ChangeSpecV1 {
                        schema_version: CHANGE_SPEC_SCHEMA_V1,
                        command_id: if version == 1 {
                            "decision.services.apply"
                        } else {
                            "decision.connections.apply"
                        }
                        .into(),
                        resource_id: Some("decision/removal".into()),
                        desired_state: desired(version, 1, None),
                    }),
                )
                .unwrap();
            let accepted = coordinator
                .accept(
                    &WorkspaceId::default(),
                    &VerifiedPrincipal::for_local_control(),
                    ApplyRequestV1 {
                        schema_version: CHANGE_SPEC_SCHEMA_V1,
                        spec: preview.normalized_spec,
                        accept_digest: preview.change_digest,
                        expected_revisions: preview.expected_revisions,
                        idempotency_key: "recovery-remove".into(),
                        apply_capability: None,
                    },
                )
                .unwrap();
            operation_id = accepted.operation().operation_id.clone();
            let fault = crate::control::terminal_fault::inject(OperationState::Succeeded);
            let completion = coordinator.run_accepted(accepted);
            assert!(completion.is_err(), "V{version}: {completion:?}");
            drop(fault);
            assert_eq!(
                stores
                    .control()
                    .writer_claim_operation()
                    .unwrap()
                    .unwrap()
                    .operation_id,
                operation_id
            );
            let blocked_preview = coordinator
                .preview(
                    &WorkspaceId::default(),
                    PreviewRequestV1::new(ChangeSpecV1 {
                        schema_version: CHANGE_SPEC_SCHEMA_V1,
                        command_id: if version == 1 {
                            "decision.services.apply"
                        } else {
                            "decision.connections.apply"
                        }
                        .into(),
                        resource_id: Some("decision/removal".into()),
                        desired_state: desired(version, 1, None),
                    }),
                )
                .unwrap();
            assert!(
                coordinator
                    .accept(
                        &WorkspaceId::default(),
                        &VerifiedPrincipal::for_local_control(),
                        ApplyRequestV1 {
                            schema_version: CHANGE_SPEC_SCHEMA_V1,
                            spec: blocked_preview.normalized_spec,
                            accept_digest: blocked_preview.change_digest,
                            expected_revisions: blocked_preview.expected_revisions,
                            idempotency_key: "blocked-by-removal-recovery".into(),
                            apply_capability: None,
                        },
                    )
                    .is_err()
            );
            assert!(
                !stores
                    .control()
                    .decision_service_records(&WorkspaceId::default())
                    .unwrap()
                    .is_empty()
            );
        }
        let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
        let external = NoExternal;
        let protected = ProtectedInput;
        let runtime = TransactionRuntime::default();
        let coordinator = TransactionCoordinator::new(
            stores.control(),
            stores.secrets(),
            stores.runtime(),
            &external,
            &protected,
            &runtime,
        );
        for _ in 0..2 {
            coordinator.reconcile_startup_and_open().unwrap();
            assert_eq!(
                stores
                    .control()
                    .load_operation(&operation_id)
                    .unwrap()
                    .unwrap()
                    .state,
                OperationState::Succeeded
            );
            assert!(stores.control().writer_claim_operation().unwrap().is_none());
            assert!(
                stores
                    .control()
                    .decision_service_records(&WorkspaceId::default())
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                stores
                    .control()
                    .published_plan_versions(&WorkspaceId::default())
                    .unwrap(),
                vec![history.clone()]
            );
            crate::migrations::audit_current_storage(
                stores.control(),
                stores.runtime(),
                stores.secrets(),
            )
            .unwrap();
        }
    }
}
