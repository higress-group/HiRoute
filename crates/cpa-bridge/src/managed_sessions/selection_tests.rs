//! Provider projection races exercise the real supervised process and exact routing lease.
use super::*;
use crate::{
    BorrowedCodexAuthSpec, CpaAttemptError, CpaDownstreamCredentialCapability,
    CpaDownstreamCredentialPort, CpaSourceManagementState, ExactCpaAttemptRequest,
    ExactCpaCredentialRequest, PreparedCpaTarget,
};
use hiroute_domain::UpstreamProtocol;
use hiroute_integrations::CpaSupervisorPort;
use std::sync::mpsc;

#[path = "catalog_readiness_tests.rs"]
mod catalog_readiness_tests;

const SOURCE: &str = "source/cpa/fixture-codex";
const MODEL: &str = "gpt-5.3-codex-spark";
const CONNECTOR: &str = "connector.cpa.codex";

fn selection(fixture: &Fixture, count: usize) -> (Arc<ManagedCpaRuntimeSet>, Vec<CpaLoginSession>) {
    let registry = fixture.registry();
    let sessions = (0..count)
        .map(|_| fixture.authorize(&registry, CpaAccountKind::Codex))
        .collect();
    drop(registry);
    let set = Arc::new(ManagedCpaRuntimeSet::new(fixture.templates.clone()).unwrap());
    (set, sessions)
}

fn project(
    set: &ManagedCpaRuntimeSet,
    session: &CpaLoginSession,
    revision: u64,
    state: CpaSourceManagementState,
) -> Result<(), CpaLifecycleError> {
    set.apply_saved_source(
        SOURCE,
        &session.candidate_ref(),
        session.account_ref.as_deref().unwrap(),
        revision,
        state,
    )
}

fn selected(set: &ManagedCpaRuntimeSet, session: &CpaLoginSession) -> Arc<ManagedCpaRuntime> {
    let runtime = set.for_connector(CONNECTOR).unwrap();
    assert!(Arc::ptr_eq(
        &runtime,
        &set.runtime_for_candidate(&session.candidate_ref()).unwrap()
    ));
    runtime
}

fn target(runtime: &ManagedCpaRuntime) -> PreparedCpaTarget {
    let account = runtime
        .materialize_account(CONNECTOR, "endpoint.cpa.codex")
        .unwrap();
    runtime
        .begin_routing_batch()
        .unwrap()
        .prepare_target(ExactCpaAttemptRequest {
            credential_ref: &account.credential_ref,
            upstream_model_id: MODEL,
            protocol: UpstreamProtocol::Responses,
        })
        .unwrap()
}

fn lease(
    runtime: &impl CpaDownstreamCredentialPort,
    target: &PreparedCpaTarget,
) -> Result<Option<CpaDownstreamCredentialCapability>, CpaAttemptError> {
    runtime.lease_downstream_capability(ExactCpaCredentialRequest {
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
}

#[test]
fn stale_completion_and_maintenance_cannot_replace_or_stop_the_new_login() {
    let fixture = Fixture::new();
    let (set, sessions) = selection(&fixture, 2);
    assert_eq!(sessions[0].account_ref, sessions[1].account_ref);
    project(&set, &sessions[0], 1, CpaSourceManagementState::Enabled).unwrap();
    let old = selected(&set, &sessions[0]);
    let old_target = target(&old);
    let old_capability = lease(&*old, &old_target).unwrap().unwrap();
    project(&set, &sessions[1], 2, CpaSourceManagementState::Enabled).unwrap();
    let current = selected(&set, &sessions[1]);
    let current_target = target(&current);
    let before = fixture.spawns();
    assert!(matches!(
        project(&set, &sessions[0], 1, CpaSourceManagementState::Enabled),
        Err(CpaLifecycleError::StaleSourceManagement)
    ));
    set.suspend_saved_source(SOURCE, &sessions[0].candidate_ref(), 1);
    assert!(matches!(old.health(), Ok(CpaHealth::Stopped { .. })));
    assert!(matches!(
        lease(&*old, &old_target),
        Err(CpaAttemptError::RevokedCredential)
    ));
    assert_eq!(
        old_capability.apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
    lease(&*set, &current_target)
        .unwrap()
        .unwrap()
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    assert_eq!(fixture.spawns(), before);
    set.shutdown().unwrap();
}

#[test]
fn delayed_enabled_and_disabled_projections_cannot_reverse_newer_saved_state() {
    let fixture = Fixture::new();
    let (set, sessions) = selection(&fixture, 1);
    let session = &sessions[0];
    project(&set, session, 1, CpaSourceManagementState::Enabled).unwrap();
    project(&set, session, 2, CpaSourceManagementState::Disabled).unwrap();
    let before = fixture.spawns();
    assert!(matches!(
        project(&set, session, 1, CpaSourceManagementState::Enabled),
        Err(CpaLifecycleError::StaleSourceManagement)
    ));
    assert!(matches!(
        selected(&set, session).health(),
        Ok(CpaHealth::Stopped { .. })
    ));
    assert_eq!(fixture.spawns(), before);
    project(&set, session, 3, CpaSourceManagementState::Enabled).unwrap();
    let live = selected(&set, session);
    let current_target = target(&live);
    assert!(matches!(
        project(&set, session, 2, CpaSourceManagementState::Disabled),
        Err(CpaLifecycleError::StaleSourceManagement)
    ));
    lease(&*set, &current_target)
        .unwrap()
        .unwrap()
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    set.shutdown().unwrap();
}

#[test]
fn concurrent_projection_waits_for_old_writer_shutdown_before_replacing_it() {
    let fixture = Fixture::new();
    let (set, sessions) = selection(&fixture, 3);
    project(&set, &sessions[0], 1, CpaSourceManagementState::Enabled).unwrap();
    let old = selected(&set, &sessions[0]);
    let old_capability = lease(&*set, &target(&old)).unwrap().unwrap();
    old_capability
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    // Block the real old shutdown while the newer operation owns the provider lifecycle.
    let old_inner = old.inner.lock();
    let provider_lifecycle = Arc::clone(&set.selected[&CpaAccountKind::Codex].lock().lifecycle);
    let b_set = set.clone();
    let b_session = sessions[1].clone();
    let b = std::thread::spawn(move || {
        project(&b_set, &b_session, 2, CpaSourceManagementState::Enabled)
    });
    wait_until(|| provider_lifecycle.try_lock().is_none());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let c_set = set.clone();
    let c_session = sessions[2].clone();
    let c = std::thread::spawn(move || {
        entered_tx.send(()).unwrap();
        let result = project(&c_set, &c_session, 3, CpaSourceManagementState::Enabled);
        finished_tx.send(()).unwrap();
        result
    });
    entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    wait_until(|| {
        set.selected[&CpaAccountKind::Codex]
            .lock()
            .saved
            .as_ref()
            .is_some_and(|saved| saved.revision == 3)
    });
    let finished_while_old_writer_locked = finished_rx.try_recv().is_ok();
    let spawns_while_old_writer_locked = fixture.spawns();
    drop(old_inner);
    assert!(matches!(
        b.join().unwrap(),
        Err(CpaLifecycleError::OperationCancelled)
    ));
    c.join().unwrap().unwrap();
    assert!(!finished_while_old_writer_locked);
    assert_eq!(spawns_while_old_writer_locked, 4); // three OAuth writers, then saved A.
    assert_eq!(
        fixture.spawns(),
        5,
        "superseded revision 2 started a writer"
    );
    let current = selected(&set, &sessions[2]);
    assert!(matches!(old.health(), Ok(CpaHealth::Stopped { .. })));
    assert!(matches!(
        set.runtime_for_candidate(&sessions[1].candidate_ref())
            .unwrap()
            .health(),
        Ok(CpaHealth::Stopped { .. })
    ));
    assert_eq!(
        old_capability.apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
    lease(&*set, &target(&current))
        .unwrap()
        .unwrap()
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    set.shutdown().unwrap();
}

#[test]
fn housekeeping_retries_both_old_and_disabled_writers_after_either_stop_fails() {
    for failing_index in 0..2 {
        let fixture = Fixture::new();
        let (set, sessions) = selection(&fixture, 2);
        project(&set, &sessions[0], 1, CpaSourceManagementState::Enabled).unwrap();
        let old = selected(&set, &sessions[0]);
        let old_target = target(&old);
        let old_batch = old.begin_routing_batch().unwrap();
        let current = set
            .runtime_for_candidate(&sessions[1].candidate_ref())
            .unwrap();
        current.start().unwrap();
        let failing = if failing_index == 0 { &old } else { &current };
        let blocker = failing
            .spec
            .state_root
            .join(&failing.spec.instance_id)
            .join("owner.lock/fixture-blocks-release");
        fs::write(&blocker, b"owned stop failure injection").unwrap();
        let before = fixture.spawns();
        assert!(project(&set, &sessions[1], 2, CpaSourceManagementState::Disabled).is_err());
        let old_request_denied = lease(&*old, &old_target).is_err();
        let spawns_after_old_request = fixture.spawns();
        let selected_disabled_request_denied = lease(&*set, &old_target).is_err();
        let spawns_after_selected_disabled_request = fixture.spawns();
        let old_batch_denied = old_batch.finish().is_err();
        let old_materialization_denied = old
            .materialize_account(CONNECTOR, "endpoint.cpa.codex")
            .is_err();
        assert!(set.discover_registered_sources().unwrap().is_empty());
        let spawns_after_passive_reads = fixture.spawns();
        assert_eq!(set.retry_saved_shutdowns(), vec![SOURCE]);
        fs::remove_file(blocker).unwrap();
        assert!(set.retry_saved_shutdowns().is_empty());
        assert!(matches!(old.health(), Ok(CpaHealth::Stopped { .. })));
        assert!(matches!(current.health(), Ok(CpaHealth::Stopped { .. })));
        let spawns_after_stop_retry = fixture.spawns();
        set.shutdown().unwrap();
        assert!(old_request_denied, "old writer retained routing admission");
        assert!(
            selected_disabled_request_denied,
            "disabled selection retained routing admission"
        );
        assert!(old_batch_denied, "old routing batch survived withdrawal");
        assert!(old_materialization_denied);
        assert_eq!(
            spawns_after_old_request, before,
            "a revoked old-runtime request restarted a suspended refresh writer"
        );
        assert_eq!(
            spawns_after_selected_disabled_request, before,
            "a request restarted the disabled provider selection"
        );
        assert_eq!(
            spawns_after_passive_reads, before,
            "a passive read restarted a revoked refresh writer"
        );
        assert_eq!(
            spawns_after_stop_retry, before,
            "stop retry created a new refresh writer"
        );
    }
}

#[test]
fn saved_provider_recovery_restores_exact_lease_after_half_written_token() {
    let fixture = Fixture::new();
    let (set, sessions) = selection(&fixture, 1);
    let session = &sessions[0];
    project(&set, session, 1, CpaSourceManagementState::Enabled).unwrap();
    let live = selected(&set, session);
    let saved_target = target(&live);
    let old_capability = lease(&*set, &saved_target).unwrap().unwrap();
    let auth = fixture
        .session_dir(&session.login_ref)
        .join("auth/credential.json");
    let original = fs::read(&auth).unwrap();
    fs::write(&auth, b"{").unwrap();
    assert!(project(&set, session, 1, CpaSourceManagementState::Enabled).is_err());
    assert_eq!(
        old_capability.apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
    fs::write(&auth, original).unwrap();
    let before = fixture.spawns();
    project(&set, session, 1, CpaSourceManagementState::Enabled).unwrap();
    // Do not rebuild the target or ask discovery to repair the account before this lease.
    lease(&*set, &saved_target)
        .unwrap()
        .unwrap()
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    assert_eq!(
        old_capability.apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
    assert_eq!(fixture.spawns(), before);
    set.shutdown().unwrap();
}

#[test]
fn saved_enabled_child_recovers_on_projection_without_client_or_inference_request() {
    let fixture = Fixture::new();
    let (set, sessions) = selection(&fixture, 1);
    let session = &sessions[0];
    project(&set, session, 1, CpaSourceManagementState::Enabled).unwrap();
    let live = selected(&set, session);
    let saved_target = target(&live);
    kill_owned_pending(&live);
    let before = fixture.spawns();
    project(&set, session, 1, CpaSourceManagementState::Enabled).unwrap();
    assert_eq!(fixture.spawns(), before + 1);
    lease(&*set, &saved_target)
        .unwrap()
        .unwrap()
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    set.shutdown().unwrap();
}

#[test]
fn same_account_native_and_managed_switch_preserves_source_and_closes_previous_writer() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    let session = fixture.authorize(&registry, CpaAccountKind::Codex);
    drop(registry);
    let native_source = fixture.root.path().join("native-fixture-auth.json");
    let native_bytes = serde_json::to_vec(&serde_json::json!({
        "auth_mode": "chatgpt", "OPENAI_API_KEY": null, "last_refresh": "fixture",
        "tokens": {"access_token": "fixture-native-access", "id_token": "fixture.id.token",
            "refresh_token": "fixture-native-refresh-never-import", "account_id": "codex-fixture-account"}
    })).unwrap();
    fs::write(&native_source, &native_bytes).unwrap();
    fs::set_permissions(&native_source, fs::Permissions::from_mode(0o600)).unwrap();
    let executable = fixture.root.path().join("codex-fixture");
    fs::write(&executable, "#!/bin/sh\nprintf 'codex-cli 0.116.0\\n'\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let mut spec = fixture.templates[0].spec.clone();
    spec.instance_id = "native-codex-fixture".into();
    spec.auth_dir = fixture.root.path().join("cpa/native-fixture-auth");
    spec.managed_oauth = None;
    let borrowed = BorrowedCodexAuthSpec::new(&native_source).with_executable(executable);
    assert_eq!(
        borrowed.inspect().unwrap().account_ref(),
        session.account_ref.clone().unwrap()
    );
    spec.borrowed_codex_auth = Some(borrowed);
    let binary = fixture.root.path().join("cpa-fixture");
    let locator = Arc::new(PinnedCpaBinaryLocator::new(PinnedCpaArtifact::new(
        fixture.root.path(),
        "cpa-fixture",
        MANAGED_CPA_ARTIFACT_VERSION.parse().unwrap(),
        format!("{:x}", Sha256::digest(fs::read(binary).unwrap())),
    )));
    let native = Arc::new(
        ManagedCpaRuntime::new(spec, fixture.templates[0].catalog.clone(), locator).unwrap(),
    );
    let set =
        ManagedCpaRuntimeSet::new(vec![native.clone(), fixture.templates[1].clone()]).unwrap();
    let native_candidate = "candidate/cpa/codex/current";
    let account = session.account_ref.as_deref().unwrap();
    set.apply_saved_source(
        SOURCE,
        native_candidate,
        account,
        1,
        CpaSourceManagementState::Enabled,
    )
    .unwrap();
    let native_target = target(&native);
    let native_capability = lease(&set, &native_target).unwrap().unwrap();
    project(&set, &session, 2, CpaSourceManagementState::Enabled).unwrap();
    let managed = selected(&set, &session);
    let managed_target = target(&managed);
    assert_eq!(
        native_target.upstream_model_id(),
        managed_target.upstream_model_id()
    );
    assert_eq!(
        native_target.credential_ref().subject(),
        managed_target.credential_ref().subject()
    );
    assert!(matches!(native.health(), Ok(CpaHealth::Stopped { .. })));
    assert_eq!(
        native_capability.apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
    set.apply_saved_source(
        SOURCE,
        native_candidate,
        account,
        3,
        CpaSourceManagementState::Enabled,
    )
    .unwrap();
    assert!(matches!(managed.health(), Ok(CpaHealth::Stopped { .. })));
    assert!(Arc::ptr_eq(&set.for_connector(CONNECTOR).unwrap(), &native));
    lease(&set, &target(&native))
        .unwrap()
        .unwrap()
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    assert_eq!(fs::read(native_source).unwrap(), native_bytes);
    let flat = fs::read_to_string(native.spec.auth_dir.join("hiroute-managed-codex.json")).unwrap();
    assert!(!flat.contains("fixture-native-refresh-never-import"));
    set.shutdown().unwrap();
}
