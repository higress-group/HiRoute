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
