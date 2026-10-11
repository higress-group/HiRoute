//! Retained publications must validate the catalog of the actual supervised child.
use super::*;

fn arm_catalog(fixture: &Fixture, block: bool) {
    fs::write(fixture.root.path().join("record-model-catalog"), b"").unwrap();
    if block {
        fs::write(fixture.root.path().join("block-model-catalog"), b"").unwrap();
    }
}

fn catalog_events(fixture: &Fixture) -> Vec<serde_json::Value> {
    fs::read_to_string(fixture.root.path().join("catalog.jsonl"))
        .unwrap_or_default()
        .lines()
        // A concurrently appended final line is not yet a completed fixture event.
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn account_inventory_count(fixture: &Fixture) -> usize {
    catalog_events(fixture)
        .iter()
        .filter(|event| {
            event["event"] == "inventory"
                || (event["event"] == "models"
                    && event["route"] == "/v0/management/auth-files/models")
        })
        .count()
}

fn await_blocked_catalog<T>(fixture: &Fixture, completed: &mpsc::Receiver<T>) -> (bool, Option<T>) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match completed.try_recv() {
            Ok(result) => return (false, Some(result)),
            Err(mpsc::TryRecvError::Disconnected) => {
                panic!("catalog worker exited without a result")
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if catalog_events(fixture).iter().any(|event| {
            event["event"] == "models"
                && event["route"] == "/v0/management/auth-files/models"
                && event["blocked"] == true
        }) {
            return (true, completed.try_recv().ok());
        }
        assert!(
            Instant::now() < deadline,
            "child never attempted its catalog"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn retained_request_waits_for_the_restarted_child_catalog_before_issuing_authority() {
    let fixture = Fixture::new();
    let (set, sessions) = selection(&fixture, 1);
    project(&set, &sessions[0], 1, CpaSourceManagementState::Enabled).unwrap();
    let live = selected(&set, &sessions[0]);
    let saved_target = target(&live);
    arm_catalog(&fixture, true);
    kill_owned_pending(&live);
    let before = fixture.spawns();
    let request_set = set.clone();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        assert!(tx.send(lease(&*request_set, &saved_target)).is_ok());
    });
    // No Check, target reconstruction or separate discovery may prime this process.
    let (observed_block, early) = await_blocked_catalog(&fixture, &rx);
    let completed_before_registration = early.is_some();
    fs::remove_file(fixture.root.path().join("block-model-catalog")).unwrap();
    let result = early.unwrap_or_else(|| rx.recv_timeout(Duration::from_secs(3)).unwrap());
    worker.join().unwrap();
    let authorization = result
        .unwrap()
        .unwrap()
        .apply_authorization(&mut http::HeaderMap::new());
    let after = fixture.spawns();
    let events = catalog_events(&fixture);
    set.shutdown().unwrap();
    assert!(
        observed_block,
        "retained lease bypassed the new process's catalog"
    );
    assert!(
        !completed_before_registration,
        "authority was issued while models were absent"
    );
    assert_eq!(authorization, Ok(()));
    assert_eq!(after, before + 1);
    assert!(
        events.iter().all(|event| event["event"] != "patch"),
        "ordinary recovery forced model refresh"
    );
}

#[test]
fn reconstructed_saved_provider_waits_for_catalog_and_serves_the_durable_target() {
    let fixture = Fixture::new();
    let (old_set, sessions) = selection(&fixture, 1);
    project(&old_set, &sessions[0], 1, CpaSourceManagementState::Enabled).unwrap();
    let saved_target = target(&selected(&old_set, &sessions[0]));
    old_set.shutdown().unwrap();
    arm_catalog(&fixture, true);
    let set = Arc::new(ManagedCpaRuntimeSet::new(fixture.templates.clone()).unwrap());
    let recovery_set = set.clone();
    let session = sessions[0].clone();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        assert!(
            tx.send(project(
                &recovery_set,
                &session,
                1,
                CpaSourceManagementState::Enabled
            ))
            .is_ok()
        );
    });
    let (observed_block, early) = await_blocked_catalog(&fixture, &rx);
    let completed_before_registration = early.is_some();
    fs::remove_file(fixture.root.path().join("block-model-catalog")).unwrap();
    let result = early.unwrap_or_else(|| rx.recv_timeout(Duration::from_secs(3)).unwrap());
    worker.join().unwrap();
    result.unwrap();
    // A durable publication can keep its old exact subject/model across daemon lifetimes.
    let authorization = lease(&*set, &saved_target)
        .unwrap()
        .unwrap()
        .apply_authorization(&mut http::HeaderMap::new());
    let events = catalog_events(&fixture);
    set.shutdown().unwrap();
    assert!(
        observed_block,
        "saved recovery trusted a persisted active snapshot"
    );
    assert!(!completed_before_registration);
    assert_eq!(authorization, Ok(()));
    assert!(events.iter().all(|event| event["event"] != "patch"));
}

#[test]
fn catalog_timeout_recovers_through_saved_maintenance_and_changed_models_fail_closed() {
    let fixture = Fixture::new();
    let (set, sessions) = selection(&fixture, 1);
    let session = &sessions[0];
    project(&set, session, 1, CpaSourceManagementState::Enabled).unwrap();
    let live = selected(&set, session);
    let saved_target = target(&live);
    let old_capability = lease(&*set, &saved_target).unwrap().unwrap();
    arm_catalog(&fixture, true);
    kill_owned_pending(&live);
    assert!(matches!(
        lease(&*set, &saved_target),
        Err(CpaAttemptError::Unavailable)
    ));
    let after_timeout = fixture.spawns();
    let before_rejected = catalog_events(&fixture).len();
    assert!(matches!(
        lease(&*set, &saved_target),
        Err(CpaAttemptError::RevokedCredential)
    ));
    assert_eq!(catalog_events(&fixture).len(), before_rejected);
    fs::remove_file(fixture.root.path().join("block-model-catalog")).unwrap();
    project(&set, session, 1, CpaSourceManagementState::Enabled).unwrap();
    lease(&*set, &saved_target)
        .unwrap()
        .unwrap()
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    assert_eq!(fixture.spawns(), after_timeout);
    assert_eq!(
        old_capability.apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
    // Stock readiness reads /v1/models on every admitted health check. Count the
    // exact account endpoints to distinguish inventory repair from those probes.
    let before_healthy = account_inventory_count(&fixture);
    project(&set, session, 1, CpaSourceManagementState::Enabled).unwrap();
    lease(&*set, &saved_target)
        .unwrap()
        .unwrap()
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    assert_eq!(account_inventory_count(&fixture), before_healthy);
    fs::write(fixture.root.path().join("catalog-model"), b"gpt-5.5").unwrap();
    kill_owned_pending(&live);
    assert!(matches!(
        lease(&*set, &saved_target),
        Err(CpaAttemptError::ModelUnavailable)
    ));
    let events = catalog_events(&fixture);
    set.shutdown().unwrap();
    assert!(events.iter().all(|event| event["event"] != "patch"));
}

#[test]
fn unchanged_native_evidence_recovers_only_through_the_saved_projection() {
    let fixture = Fixture::new();
    let source = fixture.root.path().join("native-fixture-auth.json");
    let original = serde_json::to_vec(&serde_json::json!({
        "auth_mode": "chatgpt", "OPENAI_API_KEY": null, "last_refresh": "fixture",
        "tokens": {"access_token": "fixture-native-access", "id_token": "fixture.id.token",
            "refresh_token": "fixture-native-refresh-never-import", "account_id": "codex-fixture-account"}
    })).unwrap();
    fs::write(&source, &original).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
    let executable = fixture.root.path().join("codex-fixture");
    fs::write(&executable, "#!/bin/sh\nprintf 'codex-cli 0.116.0\\n'\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let borrowed = BorrowedCodexAuthSpec::new(&source).with_executable(executable);
    let account = borrowed.inspect().unwrap().account_ref();
    let mut spec = fixture.templates[0].spec.clone();
    spec.instance_id = "native-catalog-fixture".into();
    spec.auth_dir = fixture.root.path().join("cpa/native-catalog-auth");
    spec.managed_oauth = None;
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
    let candidate = "candidate/cpa/codex/current";
    set.apply_saved_source(
        SOURCE,
        candidate,
        &account,
        1,
        CpaSourceManagementState::Enabled,
    )
    .unwrap();
    let saved_target = target(&native);
    let old_capability = lease(&set, &saved_target).unwrap().unwrap();
    arm_catalog(&fixture, false);
    fs::write(fixture.root.path().join("catalog-auth-unavailable"), b"").unwrap();
    kill_owned_pending(&native);
    assert!(matches!(
        lease(&set, &saved_target),
        Err(CpaAttemptError::Unavailable)
    ));
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(
        old_capability.apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
    let after_failure = fixture.spawns();
    fs::remove_file(fixture.root.path().join("catalog-auth-unavailable")).unwrap();
    // This is the same-evidence maintenance entry, with no Check or discovery side door.
    set.apply_saved_source(
        SOURCE,
        candidate,
        &account,
        1,
        CpaSourceManagementState::Enabled,
    )
    .unwrap();
    lease(&set, &saved_target)
        .unwrap()
        .unwrap()
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    assert_eq!(
        old_capability.apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
    assert_eq!(fixture.spawns(), after_failure);
    let healthy_count = account_inventory_count(&fixture);
    set.apply_saved_source(
        SOURCE,
        candidate,
        &account,
        1,
        CpaSourceManagementState::Enabled,
    )
    .unwrap();
    lease(&set, &saved_target)
        .unwrap()
        .unwrap()
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    assert_eq!(account_inventory_count(&fixture), healthy_count);
    set.suspend_saved_source(SOURCE, candidate, 1);
    let suspended_catalog = catalog_events(&fixture).len();
    assert!(native.ensure_saved_runtime_ready(&account, 1).is_err());
    assert!(lease(&set, &saved_target).is_err());
    assert_eq!(fixture.spawns(), after_failure);
    assert_eq!(catalog_events(&fixture).len(), suspended_catalog);
    set.apply_saved_source(
        SOURCE,
        candidate,
        &account,
        1,
        CpaSourceManagementState::Enabled,
    )
    .unwrap();
    lease(&set, &saved_target)
        .unwrap()
        .unwrap()
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    set.apply_saved_source(
        SOURCE,
        candidate,
        &account,
        2,
        CpaSourceManagementState::Disabled,
    )
    .unwrap();
    let denied_spawns = fixture.spawns();
    let denied_catalog = catalog_events(&fixture).len();
    assert!(matches!(
        set.apply_saved_source(
            SOURCE,
            candidate,
            &account,
            1,
            CpaSourceManagementState::Enabled
        ),
        Err(CpaLifecycleError::StaleSourceManagement)
    ));
    assert!(lease(&set, &saved_target).is_err());
    assert!(native.ensure_saved_runtime_ready(&account, 2).is_err());
    assert_eq!(fixture.spawns(), denied_spawns);
    assert_eq!(catalog_events(&fixture).len(), denied_catalog);
    set.apply_saved_source(
        SOURCE,
        candidate,
        &account,
        3,
        CpaSourceManagementState::Removed,
    )
    .unwrap();
    assert!(matches!(
        set.apply_saved_source(
            SOURCE,
            candidate,
            &account,
            2,
            CpaSourceManagementState::Enabled
        ),
        Err(CpaLifecycleError::StaleSourceManagement)
    ));
    assert!(lease(&set, &saved_target).is_err());
    assert!(native.ensure_saved_runtime_ready(&account, 3).is_err());
    assert_eq!(fixture.spawns(), denied_spawns);
    assert_eq!(catalog_events(&fixture).len(), denied_catalog);
    assert_eq!(fs::read(&source).unwrap(), original);
    set.shutdown().unwrap();
}
