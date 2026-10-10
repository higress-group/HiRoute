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
    let catalog = fixture.adapter().release_catalog.as_ref().unwrap();
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
    let application = ApplicationService::new(fixture.runtime.application_ports());
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
