//! The real journal must recover an accepted V3 snapshot without its resolver.
use super::lifecycle::saved;
use super::*;
use hiroute_application_api::{ApplyRequestV1, PreviewRequestV1};
use hiroute_domain::*;
use std::cell::Cell;

thread_local! {
    static REFUSE_RESOLUTION: Cell<bool> = const { Cell::new(false) };
    static RESOLUTION_CALLS: Cell<u32> = const { Cell::new(0) };
}
struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        REFUSE_RESOLUTION.set(false);
        RESOLUTION_CALLS.set(0);
    }
}

pub(super) fn resolve(
    endpoint: &str,
    protocol: DecisionWireProtocolV2,
    _model: &str,
    policy: &DecisionContextPolicyV1,
) -> PortResult<ResolvedDecisionContextV1> {
    RESOLUTION_CALLS.set(RESOLUTION_CALLS.get() + 1);
    if REFUSE_RESOLUTION.get() {
        return Err(unexpected("test.context.resolver_unavailable"));
    }
    let window = policy.window_tokens.unwrap_or(32_000);
    let source = if policy.window_tokens.is_some() {
        DecisionContextSourceV1::User
    } else {
        DecisionContextSourceV1::Fallback
    };
    let mut resolved = ResolvedDecisionContextV1 {
        request_window_tokens: window,
        per_question_window_tokens: policy.per_question_tokens.unwrap_or(window),
        request_source: source,
        per_question_source: source,
        catalog_revision: 1,
        catalog_digest: CanonicalDigest::of_bytes(b"test-context-catalog"),
        metadata: None,
        typesafe_long_context_header: policy.typesafe_long_context
            == TypeSafeLongContextV1::Automatic
            && official_typesafe_long_context_destination(endpoint, protocol),
        accounting_algorithm: DECISION_CONTEXT_ALGORITHM_V1.into(),
        reserve_policy: DECISION_CONTEXT_RESERVE_V1.into(),
        context_digest: CanonicalDigest::of_bytes(&[]),
    };
    resolved.context_digest = resolved.calculate_digest().unwrap();
    Ok(resolved)
}

#[test]
fn accepted_v3_context_recovers_once_when_metadata_resolution_is_unavailable() {
    let _reset = Reset;
    let root = tempdir().unwrap();
    populate_saved_source(root.path(), &WorkspaceId::default());
    let service;
    let operation_id;
    let calls_at_acceptance;
    {
        let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
        let source = saved(&stores);
        let policy = DecisionContextPolicyV1 {
            mode: DecisionContextModeV1::Custom,
            window_tokens: Some(65_536),
            ..Default::default()
        };
        let endpoint = "https://decision.example.test/v1/decisions";
        let resolved = resolve(
            endpoint,
            DecisionWireProtocolV2::OpenaiDecisions,
            "fixture",
            &policy,
        )
        .unwrap();
        service = DecisionServiceV3 {
            schema: DecisionServiceSchemaV3::V3,
            id: "decision/context-recovery".into(),
            revision: 1,
            name: "Context recovery".into(),
            context: Some(Box::new(DecisionContextConfigurationV1 {
                policy,
                resolved,
            })),
            connection: DecisionConnectionV2::Native {
                provider: "fixture".into(),
                wire_protocol: DecisionWireProtocolV2::OpenaiDecisions,
                model: "fixture".into(),
                endpoint: endpoint.into(),
                timeout_ms: 3000,
                authentication: DecisionAuthenticationV2::SharedSource {
                    source_id: source.source_id.clone(),
                    source_lineage_digest: source.lineage_digest.clone(),
                },
            },
        };
        let input = DecisionServiceOperationInputV3 {
            schema: DECISION_SERVICE_OPERATION_INPUT_SCHEMA_V3.into(),
            change: DecisionServiceChangeV3 {
                id: service.id.clone(),
                expected_revision: 0,
                service: Some(service.clone()),
                expected_source_revisions: vec![DecisionExpectedSourceRevisionV1 {
                    source_id: source.source_id.clone(),
                    revision: source.revision,
                }],
            },
            input_slot: None,
        };
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
            command_id: "decision.connections.apply".into(),
            resource_id: Some(service.id.clone()),
            desired_state: serde_json::to_value(input).unwrap(),
        };
        let preview = coordinator
            .preview(&WorkspaceId::default(), PreviewRequestV1::new(spec))
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
                    idempotency_key: "context-recovery".into(),
                    apply_capability: None,
                },
            )
            .unwrap();
        operation_id = accepted.operation().operation_id.clone();
        calls_at_acceptance = RESOLUTION_CALLS.get();
        REFUSE_RESOLUTION.set(true);
        let fault = crate::control::terminal_fault::inject(OperationState::Succeeded);
        assert!(coordinator.run_accepted(accepted).is_err());
        drop(fault);
        assert_eq!(RESOLUTION_CALLS.get(), calls_at_acceptance);
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
        assert_eq!(RESOLUTION_CALLS.get(), calls_at_acceptance);
        assert_eq!(
            stores
                .control()
                .load_operation(&operation_id)
                .unwrap()
                .unwrap()
                .state,
            OperationState::Succeeded
        );
        assert_eq!(
            stores
                .control()
                .decision_service_record(&WorkspaceId::default(), &service.id, 1)
                .unwrap(),
            Some(DecisionServiceRecord::V3(service.clone()))
        );
        let binding = DecisionCredentialBindingV1::for_connection(
            &service.id,
            service.revision,
            &service.connection,
        )
        .unwrap()
        .unwrap();
        assert!(
            !stores
                .admit_decision_credentials(&binding)
                .unwrap()
                .keys
                .is_empty()
        );
    }
}

#[test]
fn referenced_v3_deletion_is_refused_before_journaling_or_rebinding_shared_credentials() {
    let _reset = Reset;
    let root = tempdir().unwrap();
    populate_saved_source(root.path(), &WorkspaceId::default());
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let before = saved(&stores);
    let policy = DecisionContextPolicyV1::default();
    let endpoint = "https://decision.example.test/v1/decisions";
    let service = DecisionServiceV3 {
        schema: DecisionServiceSchemaV3::V3,
        id: "decision/referenced-context".into(),
        revision: 1,
        name: "Referenced context".into(),
        context: Some(Box::new(DecisionContextConfigurationV1 {
            resolved: resolve(
                endpoint,
                DecisionWireProtocolV2::OpenaiDecisions,
                "fixture",
                &policy,
            )
            .unwrap(),
            policy,
        })),
        connection: DecisionConnectionV2::Native {
            provider: "fixture".into(),
            wire_protocol: DecisionWireProtocolV2::OpenaiDecisions,
            model: "fixture".into(),
            endpoint: endpoint.into(),
            timeout_ms: 3000,
            authentication: DecisionAuthenticationV2::SharedSource {
                source_id: before.source_id.clone(),
                source_lineage_digest: before.lineage_digest.clone(),
            },
        },
    };
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
    let accept = |input: DecisionServiceOperationInputV3, key: &str| {
        let preview = coordinator.preview(
            &WorkspaceId::default(),
            PreviewRequestV1::new(ChangeSpecV1 {
                schema_version: CHANGE_SPEC_SCHEMA_V1,
                command_id: "decision.connections.apply".into(),
                resource_id: Some(input.change.id.clone()),
                desired_state: serde_json::to_value(input).unwrap(),
            }),
        )?;
        coordinator.accept(
            &WorkspaceId::default(),
            &VerifiedPrincipal::for_local_control(),
            ApplyRequestV1 {
                schema_version: CHANGE_SPEC_SCHEMA_V1,
                spec: preview.normalized_spec,
                accept_digest: preview.change_digest,
                expected_revisions: preview.expected_revisions,
                idempotency_key: key.into(),
                apply_capability: None,
            },
        )
    };
    let input = |value, expected_revision, source_revision| DecisionServiceOperationInputV3 {
        schema: DECISION_SERVICE_OPERATION_INPUT_SCHEMA_V3.into(),
        change: DecisionServiceChangeV3 {
            id: service.id.clone(),
            expected_revision,
            service: value,
            expected_source_revisions: vec![DecisionExpectedSourceRevisionV1 {
                source_id: before.source_id.clone(),
                revision: source_revision,
            }],
        },
        input_slot: None,
    };
    assert_eq!(
        coordinator
            .run_accepted(
                accept(
                    input(Some(service.clone()), 0, before.revision),
                    "context-create"
                )
                .unwrap()
            )
            .unwrap()
            .state,
        OperationState::Succeeded
    );
    let current = saved(&stores);
    let owner = serde_json::json!({"classifier":{"kind":"decision_service_v3","service":service}})
        .to_string();
    stores.control().with_connection(|db| db.execute("INSERT INTO plan_drafts(workspace_id,draft_id,revision,draft_json) VALUES(?1,'draft/pin',1,?2)", rusqlite::params![WorkspaceId::default().as_str(), owner]).unwrap());
    let count = || {
        stores.control().with_connection(|db| {
            db.query_row("SELECT count(*) FROM operations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
        })
    };
    let count_before = count();
    let error = accept(input(None, 1, current.revision), "context-delete")
        .err()
        .expect("referenced deletion must not be accepted");
    assert!(
        matches!(&error, TransactionError::Port(port) if port.code == PortErrorCode::Conflict && port.context == "decision_service.in_use")
    );
    assert_eq!(
        error.error_code(),
        hiroute_application_api::ErrorCode::RevisionConflict
    );
    assert_eq!(
        count(),
        count_before,
        "refusal must not create a recovery dependency"
    );
    assert!(stores.control().writer_claim_operation().unwrap().is_none());
    assert_eq!(saved(&stores), current);
    let binding = DecisionCredentialBindingV1::for_connection(
        &service.id,
        service.revision,
        &service.connection,
    )
    .unwrap()
    .unwrap();
    assert!(
        !stores
            .admit_decision_credentials(&binding)
            .unwrap()
            .keys
            .is_empty()
    );
    stores.control().with_connection(|db| {
        db.execute("DELETE FROM plan_drafts WHERE draft_id='draft/pin'", [])
            .unwrap()
    });
    // The refused attempt consumed neither its idempotency key nor any credentials.
    assert_eq!(
        coordinator
            .run_accepted(accept(input(None, 1, current.revision), "context-delete").unwrap())
            .unwrap()
            .state,
        OperationState::Succeeded
    );
    assert_eq!(count(), count_before + 1);
    assert!(
        stores
            .control()
            .decision_service_versions(&WorkspaceId::default(), &service.id)
            .unwrap()
            .is_empty()
    );
}
