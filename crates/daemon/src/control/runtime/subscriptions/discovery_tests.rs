//! Exercise exact discovery errors through the real RuntimeSet and daemon consumers.
use super::*;
use hiroute_application::control::{
    ComputeFactsPort, ComputeProjectionReadError, ControlReadError,
};
use hiroute_application::{ApplicationService, ConnectionOptionAuthorizationPort};
use hiroute_application_api::{
    COMPUTE_CONNECTION_CHANGE_SCHEMA_V1, ComputeConnectionChangeV1, LOCAL_CONTROL_SCHEMA_V2,
    LocalControlRequestV2, MachineStatus, PrincipalV1,
};
use hiroute_domain::{ComputeProjectionExpectationV1, PortErrorCode};
use hiroute_integrations::CpaRegisteredSourceV1;

fn isolated_discovery(name: &str) -> bool {
    crate::test_support::isolated_agent_home(&format!(
        "control::runtime::subscriptions::login_admission_tests::discovery_tests::{name}"
    ))
}

fn save_login(
    fixture: &Fixture,
    provider: SubscriptionLoginProviderV1,
) -> ComputeSubscriptionLoginSessionV1 {
    let login = fixture.authorize(provider);
    let checked = fixture.check(&login);
    fixture.save(fixture.candidate_request(&checked, &format!("save-{}", login.login_ref)));
    login
}

#[test]
fn regression_maintenance_keeps_cold_catalog_outage_available_for_fallback() {
    if isolated_discovery("regression_maintenance_keeps_cold_catalog_outage_available_for_fallback")
    {
        return;
    }
    use hiroute_cpa_bridge::{
        CpaAttemptError, CpaDownstreamCredentialPort, ExactCpaAttemptRequest,
        ExactCpaCredentialRequest,
    };
    use hiroute_integrations::CpaSupervisorPort;
    let mut fixture = Fixture::new();
    save_login(&fixture, SubscriptionLoginProviderV1::Codex);
    let before = fixture
        .adapter()
        .stores_lock()
        .unwrap()
        .control()
        .compute_management_snapshot(&WorkspaceId::default())
        .unwrap();
    let source = before
        .sources
        .iter()
        .find(|s| s.state == MaterializationState::Ready)
        .unwrap();
    let runtime = fixture
        .runtimes
        .runtime_for_candidate(&source.last_candidate_ref)
        .unwrap();
    let material = runtime
        .materialize_account("connector.cpa.codex", "endpoint.cpa.codex")
        .unwrap();
    let batch = runtime.begin_routing_batch().unwrap();
    let target = batch
        .prepare_target(ExactCpaAttemptRequest {
            credential_ref: &material.credential_ref,
            upstream_model_id: material.observed_model_ids.iter().next().unwrap(),
            protocol: hiroute_domain::UpstreamProtocol::Responses,
        })
        .unwrap();
    assert!(batch.finish().unwrap());
    let lease = || {
        fixture
            .runtimes
            .lease_downstream_capability(ExactCpaCredentialRequest {
                credential_id: target.credential_ref().credential_id(),
                connector_id: target.connector_id(),
                upstream_model_id: target.upstream_model_id(),
                protocol: target.protocol(),
                address: target.address(),
                request_path: target.request_path(),
                native_transport_model: target.native_transport_model(),
                runtime_epoch: target.runtime_epoch(),
                target_epoch: target.target_epoch(),
                excluded_key_ids: &[],
            })
    };
    assert!(lease().unwrap().is_some());
    // Both startup projection and the later maintenance cycle run while CPA's
    // model directory is unavailable, with the actual saved source and target.
    let fault = fixture.root.path().join("block-model-catalog");
    fs::write(&fault, b"blocked").unwrap();
    fixture.reopen(true);
    // This case owns the maintenance clock. Stop the background real-time
    // writer before advancing the two explicit scan timestamps below.
    drop(
        fixture
            .runtime
            .as_mut()
            .unwrap()
            ._observation_maintenance
            .take(),
    );
    fixture.adapter().enable_subscription_maintenance();
    fixture
        .adapter()
        .scan_subscription_maintenance(10_000)
        .unwrap();
    assert!(matches!(
        fixture
            .runtimes
            .lease_downstream_capability(ExactCpaCredentialRequest {
                credential_id: target.credential_ref().credential_id(),
                connector_id: target.connector_id(),
                upstream_model_id: target.upstream_model_id(),
                protocol: target.protocol(),
                address: target.address(),
                request_path: target.request_path(),
                native_transport_model: target.native_transport_model(),
                runtime_epoch: target.runtime_epoch(),
                target_epoch: target.target_epoch(),
                excluded_key_ids: &[],
            }),
        Err(CpaAttemptError::Unavailable)
    ));
    let after = fixture
        .adapter()
        .stores_lock()
        .unwrap()
        .control()
        .compute_management_snapshot(&WorkspaceId::default())
        .unwrap();
    assert_eq!(before, after);
    fs::remove_file(fault).unwrap();
    fixture
        .adapter()
        .scan_subscription_maintenance(20_000)
        .unwrap();
    assert!(
        fixture
            .adapter()
            .subscription_maintenance_presentation(&source.source_id)
            .is_none()
    );
}

#[test]
fn saved_reuse_resolves_original_owner_among_multiple_logins() {
    if isolated_discovery("saved_reuse_resolves_original_owner_among_multiple_logins") {
        return;
    }
    for provider in [
        SubscriptionLoginProviderV1::Codex,
        SubscriptionLoginProviderV1::Claude,
    ] {
        let fixture = Fixture::new();
        let original = save_login(&fixture, provider);
        let other = fixture.authorize(provider);
        assert_ne!(original.candidate, other.candidate);
        let snapshot = fixture.snapshot();
        let source = snapshot.sources.first().unwrap();
        let general = fixture.adapter().compute_subscriptions().unwrap();
        assert!(
            general
                .candidates
                .iter()
                .filter(
                    |candidate| candidate.existing_source_id.as_deref() == Some(&source.source_id)
                )
                .count()
                >= 2
        );
        let query = hiroute_application_api::ComputeSavedSubscriptionSourceV1 {
            source_id: source.source_id.clone(),
            expected_source_revision: source.revision,
        };
        let exact = fixture
            .adapter()
            .saved_subscription_candidate(query.clone())
            .unwrap();
        assert_eq!(exact.candidates.len(), 1);
        assert_eq!(
            exact.candidates[0].candidate.candidate_ref,
            source.last_candidate_ref
        );
        assert_eq!(
            exact.candidates[0].existing_source_id.as_deref(),
            Some(source.source_id.as_str())
        );
        let application = ApplicationService::new(fixture.runtime().application_ports());
        let request = |payload| LocalControlRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "saved-owner-query".into(),
            principal: PrincipalV1::interactive_user(),
            operation_id: "ListComputeSubscriptions".into(),
            payload,
            protected_grant: None,
        };
        let before_spawns = fixture.spawns();
        let response = application.dispatch(request(serde_json::json!({"source":query})));
        assert_eq!(response.status, MachineStatus::Succeeded, "{response:?}");
        let public: hiroute_application_api::ComputeSubscriptionCandidatesV2 =
            serde_json::from_value(response.data.unwrap()).unwrap();
        assert_eq!(
            public.candidates[0].candidate.candidate_ref,
            source.last_candidate_ref
        );
        assert_eq!(
            fixture.spawns(),
            before_spawns,
            "metadata lookup cannot check a login or start CPA"
        );
        assert_eq!(
            application
                .dispatch(request(
                    serde_json::json!({"source":{"source_id":source.source_id}})
                ))
                .status,
            MachineStatus::UsageError
        );
        assert_eq!(application.dispatch(request(serde_json::json!({"source":{"source_id":source.source_id,"expected_source_revision":0}}))).status, MachineStatus::UsageError);
        assert_eq!(
            fixture.snapshot(),
            snapshot,
            "metadata lookup must not rewrite saved credentials or models"
        );
        assert!(matches!(
            fixture.adapter().saved_subscription_candidate(
                hiroute_application_api::ComputeSavedSubscriptionSourceV1 {
                    expected_source_revision: source.revision + 1,
                    ..query
                }
            ),
            Err(hiroute_application::control::ComputeManagementControlError::Conflict)
        ));
    }
}

fn source_for(fixture: &Fixture, kind: CpaAccountKind) -> CpaRegisteredSourceV1 {
    // The old aggregate API is intentionally used only to locate both initially
    // healthy fixtures. The regression exercises unchanged public daemon ports.
    fixture
        .runtimes
        .discover_registered_sources()
        .unwrap()
        .into_iter()
        .find(|source| source.source.connector_id == kind.connector_id())
        .unwrap()
}

fn change_for(fixture: &Fixture, source: &CpaRegisteredSourceV1) -> ComputeConnectionChangeV1 {
    let model = source
        .inventory
        .iter()
        .find_map(|model| model.model_configuration_id.clone())
        .unwrap();
    let catalog = fixture.adapter().current_catalog().unwrap();
    let ids = catalog.cpa_compute_projection_ids(source, &model).unwrap();
    let expected = fixture
        .adapter()
        .stores_lock()
        .unwrap()
        .control()
        .compute_projection_expectation_with_legacy_lineage(
            ids.source_id(),
            ids.binding_id(),
            ids.endpoint_profile_id(),
            ids.legacy_v7_source_id(),
        )
        .unwrap();
    make_change(
        source.source.source_id.clone(),
        &source.source.connection_option_id,
        model,
        expected,
    )
}

fn make_change(
    discovered_source_ref: String,
    option: &str,
    model_configuration_id: String,
    expected: ComputeProjectionExpectationV1,
) -> ComputeConnectionChangeV1 {
    ComputeConnectionChangeV1 {
        schema: COMPUTE_CONNECTION_CHANGE_SCHEMA_V1.into(),
        discovered_source_ref,
        connection_option_id: option.into(),
        model_configuration_id,
        expected_source_revision: expected.source_revision,
        expected_binding_revision: expected.binding_revision,
        expected_inventory_revision: expected.inventory_revision,
        explicit_materialization: true,
    }
}

fn unknown_change(option: &str) -> ComputeConnectionChangeV1 {
    ComputeConnectionChangeV1 {
        schema: COMPUTE_CONNECTION_CHANGE_SCHEMA_V1.into(),
        discovered_source_ref: "discovery/unregistered-fixture".into(),
        connection_option_id: option.into(),
        model_configuration_id: "model.openai.gpt-5.5".into(),
        expected_source_revision: 0,
        expected_binding_revision: 0,
        expected_inventory_revision: 0,
        explicit_materialization: true,
    }
}

fn break_login_control(fixture: &Fixture, login: &ComputeSubscriptionLoginSessionV1) {
    let session_dir = fixture
        .credential_path(&login.login_ref)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    fs::write(
        session_dir.join("catalog-auth-unavailable"),
        b"fixture-only",
    )
    .unwrap();
}

fn control_calls(fixture: &Fixture, login: &ComputeSubscriptionLoginSessionV1) -> usize {
    // Check/Save can replace the login's initial OAuth process. Count all owned
    // process generations, so a later provider probe cannot escape this oracle.
    let pids = fs::read_to_string(fixture.root.path().join("spawns.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|row| row["login_ref"] == login.login_ref)
        .map(|row| row["pid"].as_u64().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(!pids.is_empty(), "a checked login has no owned process");
    fs::read_to_string(fixture.root.path().join("catalog.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|row| row["pid"].as_u64().is_some_and(|pid| pids.contains(&pid)))
        .count()
}

#[test]
fn requested_provider_projection_propagates_real_set_discovery_failure() {
    if isolated_discovery("requested_provider_projection_propagates_real_set_discovery_failure") {
        return;
    }
    for (provider, kind) in [
        (SubscriptionLoginProviderV1::Claude, CpaAccountKind::Claude),
        (SubscriptionLoginProviderV1::Codex, CpaAccountKind::Codex),
    ] {
        let fixture = Fixture::new();
        let login = save_login(&fixture, provider);
        let source = source_for(&fixture, kind);
        let change = change_for(&fixture, &source);
        assert!(
            fixture
                .adapter()
                .prepare_compute_projection(&change)
                .is_ok()
        );
        break_login_control(&fixture, &login);
        assert!(
            matches!(
                fixture.adapter().prepare_compute_projection(&change),
                Err(ComputeProjectionReadError::Control(
                    ControlReadError::Unavailable
                ))
            ),
            "a real requested-provider failure became InvalidSelection"
        );
    }
}

#[test]
fn requested_provider_materialization_propagates_real_set_discovery_failure() {
    if isolated_discovery(
        "requested_provider_materialization_propagates_real_set_discovery_failure",
    ) {
        return;
    }
    for (provider, kind) in [
        (SubscriptionLoginProviderV1::Claude, CpaAccountKind::Claude),
        (SubscriptionLoginProviderV1::Codex, CpaAccountKind::Codex),
    ] {
        let fixture = Fixture::new();
        let login = save_login(&fixture, provider);
        let source = source_for(&fixture, kind);
        let change = change_for(&fixture, &source);
        assert!(
            fixture
                .adapter()
                .compute_source_materialization(
                    &change.connection_option_id,
                    &source.source.source_id,
                    change.expected_source_revision,
                    true,
                )
                .unwrap()
                .is_some()
        );
        break_login_control(&fixture, &login);
        let error = fixture
            .adapter()
            .compute_source_materialization(
                &change.connection_option_id,
                &source.source.source_id,
                change.expected_source_revision,
                true,
            )
            .err()
            .expect("a real requested-provider failure became a successful None");
        assert_eq!(error.code, PortErrorCode::Unavailable);
    }
}

#[test]
fn healthy_sibling_and_unknown_candidates_do_not_probe_the_failed_provider() {
    if isolated_discovery("healthy_sibling_and_unknown_candidates_do_not_probe_the_failed_provider")
    {
        return;
    }
    let fixture = Fixture::new();
    fs::write(
        fixture.root.path().join("record-model-catalog"),
        b"fixture-only",
    )
    .unwrap();
    let claude_login = save_login(&fixture, SubscriptionLoginProviderV1::Claude);
    let _codex_login = save_login(&fixture, SubscriptionLoginProviderV1::Codex);
    let codex = source_for(&fixture, CpaAccountKind::Codex);
    let codex_change = change_for(&fixture, &codex);
    break_login_control(&fixture, &claude_login);
    let claude_change = unknown_change("claude.subscription.global.v1");
    assert!(matches!(
        fixture.adapter().prepare_compute_projection(&claude_change),
        Err(ComputeProjectionReadError::Control(
            ControlReadError::Unavailable
        ))
    ));
    let failed_calls = control_calls(&fixture, &claude_login);
    let prepared = fixture
        .adapter()
        .prepare_compute_projection(&codex_change)
        .unwrap();
    assert_eq!(prepared.desired.source.source_id, codex.source.source_id);
    assert!(
        fixture
            .adapter()
            .compute_source_materialization(
                &codex_change.connection_option_id,
                &codex.source.source_id,
                codex_change.expected_source_revision,
                true,
            )
            .unwrap()
            .is_some()
    );

    for option in [
        "codex.subscription.global.v1",
        "zhipu.coding-plan.cn.v1",
        "unknown.fixture.option",
    ] {
        let unknown = unknown_change(option);
        assert!(matches!(
            fixture.adapter().prepare_compute_projection(&unknown),
            Err(ComputeProjectionReadError::InvalidSelection)
        ));
        assert!(
            fixture
                .adapter()
                .compute_source_materialization(option, &unknown.discovered_source_ref, 0, true,)
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(
        control_calls(&fixture, &claude_login),
        failed_calls,
        "an exact healthy/non-CPA query retried the failed provider"
    );
}

#[test]
fn a_known_cpa_option_without_a_runtime_is_unavailable_at_both_consumers() {
    if isolated_discovery("a_known_cpa_option_without_a_runtime_is_unavailable_at_both_consumers") {
        return;
    }
    let fixture = Fixture::new();
    for option in [
        "codex.subscription.global.v1",
        "claude.subscription.global.v1",
    ] {
        let change = unknown_change(option);
        let projection = fixture.adapter().prepare_compute_projection(&change);
        let materialization = fixture.adapter().compute_source_materialization(
            option,
            &change.discovered_source_ref,
            0,
            true,
        );
        assert!(matches!(
            projection,
            Err(ComputeProjectionReadError::Control(
                ControlReadError::Unavailable
            ))
        ));
        assert_eq!(
            materialization.err().unwrap().code,
            PortErrorCode::Unavailable
        );
    }
    assert!(!fixture.root.path().join("spawns.jsonl").exists());
}

#[test]
fn saved_subscription_without_runtime_does_not_become_missing_authentication() {
    if isolated_discovery(
        "saved_subscription_without_runtime_does_not_become_missing_authentication",
    ) {
        return;
    }
    for provider in [
        SubscriptionLoginProviderV1::Codex,
        SubscriptionLoginProviderV1::Claude,
    ] {
        let mut fixture = Fixture::new();
        save_login(&fixture, provider);
        // Independent OAuth is healthy even when no borrowed native runtime exists.
        fixture.adapter().enable_subscription_maintenance();
        fixture
            .adapter()
            .scan_subscription_maintenance(10_000)
            .unwrap();
        let before = fixture
            .adapter()
            .stores_lock()
            .unwrap()
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .unwrap();
        let source = before
            .sources
            .iter()
            .find(|source| source.state == MaterializationState::Ready)
            .unwrap()
            .clone();
        assert!(
            fixture
                .adapter()
                .subscription_maintenance_presentation(&source.source_id)
                .is_none()
        );
        fixture.reopen(false);
        fixture.adapter().enable_subscription_maintenance();
        for now in [10_000, 20_000] {
            fixture
                .adapter()
                .scan_subscription_maintenance(now)
                .unwrap();
            assert!(matches!(
                fixture.adapter().subscription_maintenance_presentation(&source.source_id),
                Some(super::super::maintenance::SubscriptionMaintenancePresentation::RuntimeUnavailable)
            ));
            assert_eq!(
                fixture
                    .adapter()
                    .subscription_routing_unavailability(&source),
                hiroute_application_api::PlanCandidateUnavailableReasonV1::RuntimeUnavailable
            );
        }
        let after = fixture
            .adapter()
            .stores_lock()
            .unwrap()
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .unwrap();
        assert_eq!(
            before, after,
            "runtime loss must not rewrite saved source or models"
        );
        fixture.reopen(true);
        fixture.adapter().enable_subscription_maintenance();
        fixture
            .adapter()
            .scan_subscription_maintenance(30_000)
            .unwrap();
        assert!(
            fixture
                .adapter()
                .subscription_maintenance_presentation(&source.source_id)
                .is_none()
        );
        assert!(
            !fixture
                .runtimes
                .discover_registered_sources()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn public_decision_service_list_reads_saved_state_without_contacting_failed_cpa() {
    if isolated_discovery(
        "public_decision_service_list_reads_saved_state_without_contacting_failed_cpa",
    ) {
        return;
    }
    let fixture = Fixture::new();
    fs::write(
        fixture.root.path().join("record-model-catalog"),
        b"fixture-only",
    )
    .unwrap();
    let claude = save_login(&fixture, SubscriptionLoginProviderV1::Claude);
    let codex = save_login(&fixture, SubscriptionLoginProviderV1::Codex);
    let saved = fixture.snapshot();
    assert_eq!(saved.sources.len(), 2);
    assert!(
        saved
            .sources
            .iter()
            .all(|source| source.state == MaterializationState::Ready)
    );
    let expected_services = fixture
        .adapter()
        .stores_lock()
        .unwrap()
        .control()
        .decision_services(&WorkspaceId::default())
        .unwrap();
    break_login_control(&fixture, &claude);
    break_login_control(&fixture, &codex);
    let before_control = [
        control_calls(&fixture, &claude),
        control_calls(&fixture, &codex),
    ];
    let before_spawns = fixture.spawns();
    let application = ApplicationService::new(fixture.runtime().application_ports());
    let request = |operation: &str| LocalControlRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: format!("health-read-{operation}"),
        principal: PrincipalV1::interactive_user(),
        operation_id: operation.into(),
        payload: serde_json::json!({}),
        protected_grant: None,
    };

    let response = application.dispatch(request("ListDecisionServices"));
    assert_eq!(response.status, MachineStatus::Succeeded, "{response:?}");
    assert_eq!(
        response.data,
        Some(serde_json::json!({"services": expected_services}))
    );
    assert_eq!(
        fixture.snapshot(),
        saved,
        "a health read changed saved CPA state"
    );
    assert_eq!(fixture.spawns(), before_spawns);
    assert_eq!(
        [
            control_calls(&fixture, &claude),
            control_calls(&fixture, &codex)
        ],
        before_control,
        "the public local read contacted a configured CPA provider"
    );

    // The former health operation performs actual routing discovery. This proves
    // these real processes and counters can detect the I/O excluded above.
    // Managed-only fixtures have no native Claude profile reader configured.
    let _ = application.dispatch(request("GetSystemStatus"));
    assert!(control_calls(&fixture, &claude) > before_control[0]);
    assert!(control_calls(&fixture, &codex) > before_control[1]);
}
