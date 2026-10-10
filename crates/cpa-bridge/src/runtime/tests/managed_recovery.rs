use super::*;

fn runtime(
    root: &tempfile::TempDir,
    backend: Arc<FakeBackend>,
    control: Arc<FakeControl>,
) -> ManagedCpaRuntime {
    fixture_runtime(root, backend, control, 2)
        .fork_managed_oauth(
            "stop-only".into(),
            root.path().join("session/auth"),
            CpaAccountKind::Codex,
        )
        .unwrap()
}

fn leave_orphan(runtime: ManagedCpaRuntime) -> PathBuf {
    let owner_path = runtime
        .spec
        .state_root
        .join(&runtime.spec.instance_id)
        .join("owner.lock/owner.json");
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&owner_path).unwrap()).unwrap();
    record["owner_pid"] = json!(u32::MAX - 1);
    record
        .as_object_mut()
        .unwrap()
        .remove("proxy_environment_sha256");
    private_atomic_write(&owner_path, &serde_json::to_vec(&record).unwrap()).unwrap();
    drop(
        runtime
            .inner
            .lock()
            .live
            .as_mut()
            .unwrap()
            .auth_lease
            .take(),
    );
    std::mem::forget(runtime);
    owner_path
}

#[test]
fn stopped_or_never_started_managed_recovery_has_zero_spawn_or_control_contact() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    let managed = runtime(&root, backend.clone(), control.clone());
    managed.stop_existing_managed_process().unwrap();
    assert_eq!(backend.spawn_count(), 0);
    assert_eq!(control.probes.load(Ordering::SeqCst), 0);
    assert!(!root.path().join("session").exists());
    managed.start().unwrap();
    managed.shutdown().unwrap();
    let probes = control.probes.load(Ordering::SeqCst);
    let restored = runtime(&root, backend.clone(), control.clone());
    restored.stop_existing_managed_process().unwrap();
    assert_eq!(backend.spawn_count(), 1);
    assert_eq!(backend.attach_count(), 0);
    assert_eq!(control.probes.load(Ordering::SeqCst), probes);
    assert_eq!(control.discoveries.load(Ordering::SeqCst), 0);
}

#[test]
fn managed_orphan_stop_authenticates_but_never_replaces_for_proxy_change() {
    if isolated_owner_recovery_case(
        "runtime::tests::managed_recovery::managed_orphan_stop_authenticates_but_never_replaces_for_proxy_change",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    let first = runtime(&root, backend.clone(), control.clone())
        .with_proxy_environment([("HTTPS_PROXY".into(), "http://old-proxy:1187".into())]);
    let CpaHealth::Ready { pid, .. } = first.start().unwrap() else {
        panic!("not ready")
    };
    let auth_dir = first.spec.auth_dir.clone();
    // Stop-only cleanup must not parse this unfinished OAuth write.
    private_atomic_write(&auth_dir.join("unfinished.json"), b"{").unwrap();
    let owner_path = leave_orphan(first);
    let probes = control.probes.load(Ordering::SeqCst);
    let second = runtime(&root, backend.clone(), control.clone()).with_proxy_environment([]);
    second.stop_existing_managed_process().unwrap();
    assert!(!backend.pid_is_running(pid).unwrap());
    assert_eq!(backend.spawn_count(), 1);
    assert_eq!(backend.attach_count(), 1);
    assert_eq!(control.probes.load(Ordering::SeqCst), probes + 2);
    assert_eq!(control.discoveries.load(Ordering::SeqCst), 0);
    assert!(!owner_path.exists());
    assert_eq!(
        std::fs::read(auth_dir.join("unfinished.json")).unwrap(),
        b"{"
    );
}

#[test]
fn managed_stop_preserves_unauthenticated_or_owned_process_and_files() {
    if isolated_owner_recovery_case(
        "runtime::tests::managed_recovery::managed_stop_preserves_unauthenticated_or_owned_process_and_files",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    let first = runtime(&root, backend.clone(), control.clone());
    let CpaHealth::Ready { pid, .. } = first.start().unwrap() else {
        panic!("not ready")
    };
    let second = runtime(&root, backend.clone(), control.clone());
    assert!(matches!(
        second.stop_existing_managed_process(),
        Err(CpaLifecycleError::AlreadyOwned)
    ));
    let owner_path = leave_orphan(first);
    let original = crate::owner::read_record(owner_path.parent().unwrap()).unwrap();
    control.set_fail_probes(true);
    assert!(matches!(
        second.stop_existing_managed_process(),
        Err(CpaLifecycleError::ControlUnavailable)
    ));
    assert_eq!(
        crate::owner::read_record(owner_path.parent().unwrap()).unwrap(),
        original
    );
    assert!(backend.pid_is_running(pid).unwrap());
    assert_eq!(backend.spawn_count(), 1);
    assert_eq!(backend.attach_count(), 0);
    control.set_fail_probes(false);
    second.stop_existing_managed_process().unwrap();
    assert!(!backend.pid_is_running(pid).unwrap());
}

#[test]
fn dead_managed_child_cleanup_does_not_restart_or_probe() {
    if isolated_owner_recovery_case(
        "runtime::tests::managed_recovery::dead_managed_child_cleanup_does_not_restart_or_probe",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    let first = runtime(&root, backend.clone(), control.clone());
    first.start().unwrap();
    backend.crash_latest(2);
    let owner_path = leave_orphan(first);
    let probes = control.probes.load(Ordering::SeqCst);
    let restored = runtime(&root, backend.clone(), control.clone());
    restored.stop_existing_managed_process().unwrap();
    assert_eq!(backend.spawn_count(), 1);
    assert_eq!(backend.attach_count(), 0);
    assert_eq!(control.probes.load(Ordering::SeqCst), probes);
    assert!(!owner_path.exists());
}

#[test]
fn oauth_control_and_authentication_inspection_never_restart_crashed_writer() {
    for operation in ["status", "callback", "cancel", "inspect"] {
        let root = tempfile::tempdir().unwrap();
        let backend = Arc::new(FakeBackend::default());
        let control = Arc::new(FakeControl::default());
        let managed = runtime(&root, backend.clone(), control.clone());
        managed.start().unwrap();
        managed.inner.lock().oauth_state = Some("expected-state".into());
        // A complete private credential makes the old auto-restart path runnable:
        // the regression must catch an unwanted spawn, not fail early on missing auth.
        private_atomic_write(&managed.spec.auth_dir.join("credential.json"), &serde_json::to_vec(&json!({
            "type": "codex", "access_token": "fixture-access", "refresh_token": "independent-refresh",
            "account_id": "fixture-account"
        })).unwrap()).unwrap();
        backend.crash_latest(2);
        let probes = control.probes.load(Ordering::SeqCst);
        let outcome = match operation {
            "status" => managed.oauth_status("expected-state").map(|_| ()),
            "callback" => managed.oauth_submit_callback("expected-state", "protected-code"),
            "cancel" => managed.oauth_cancel("expected-state"),
            "inspect" => managed.inspect_subscription().map(|_| ()),
            _ => unreachable!(),
        };
        assert!(
            matches!(outcome, Err(CpaLifecycleError::ControlUnavailable)),
            "{operation}: {outcome:?}"
        );
        assert_eq!(
            backend.spawn_count(),
            1,
            "{operation} spawned a replacement"
        );
        assert_eq!(
            backend.attach_count(),
            0,
            "{operation} attached a replacement"
        );
        assert_eq!(
            control.probes.load(Ordering::SeqCst),
            probes,
            "{operation} probed after the known crash"
        );
        assert_eq!(control.discoveries.load(Ordering::SeqCst), 0);
        managed.stop_existing_managed_process().unwrap();
    }
}

fn write_managed_identity(runtime: &ManagedCpaRuntime, account: &str) -> String {
    ensure_private_dir(&runtime.spec.auth_dir).unwrap();
    private_atomic_write(&runtime.spec.auth_dir.join("credential.json"), &serde_json::to_vec(&json!({
        "type": "codex", "access_token": "fixture-access", "refresh_token": "independent-refresh",
        "account_id": account
    })).unwrap()).unwrap();
    runtime
        .managed_oauth_source()
        .unwrap()
        .inspect()
        .unwrap()
        .account_ref()
}

#[test]
fn saved_enabled_managed_recovery_preserves_bounded_restart_policy() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    let managed = runtime(&root, backend.clone(), control.clone());
    let account = write_managed_identity(&managed, "saved-account");
    assert!(matches!(
        managed.ensure_saved_runtime_ready(&account, 1),
        Err(CpaLifecycleError::InvalidSourceManagement)
    ));
    assert_eq!(backend.spawn_count(), 0);
    managed
        .apply_account_management(&account, 1, CpaSourceManagementState::Enabled)
        .unwrap();
    assert!(matches!(
        managed.ensure_saved_runtime_ready(&account, 1).unwrap(),
        CpaHealth::Ready {
            restart_count: 0,
            ..
        }
    ));
    for restart_count in 1..=2 {
        backend.crash_latest(2);
        assert!(
            matches!(managed.ensure_saved_runtime_ready(&account, 1).unwrap(), CpaHealth::Ready { restart_count: actual, .. } if actual == restart_count)
        );
    }
    backend.crash_latest(2);
    assert!(matches!(
        managed.ensure_saved_runtime_ready(&account, 1),
        Err(CpaLifecycleError::CrashLoop(_))
    ));
    assert_eq!(backend.spawn_count(), 3);
    assert!(
        managed
            .admission
            .state
            .lock()
            .subscription_execution_suspended
    );
    assert_eq!(control.discoveries.load(Ordering::SeqCst), 0);
    managed.stop_existing_managed_process().unwrap();
}

#[test]
fn managed_recovery_rejects_pending_disabled_stale_or_changed_account() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    let managed = runtime(&root, backend.clone(), control);
    let account = write_managed_identity(&managed, "saved-account");
    managed.start().unwrap();
    backend.crash_latest(2);
    assert!(matches!(
        managed.ensure_saved_runtime_ready(&account, 1),
        Err(CpaLifecycleError::InvalidSourceManagement)
    ));
    managed
        .apply_account_management(&account, 1, CpaSourceManagementState::Enabled)
        .unwrap();
    managed
        .apply_account_management(&account, 2, CpaSourceManagementState::Disabled)
        .unwrap();
    for revision in [1, 2] {
        assert!(matches!(
            managed.ensure_saved_runtime_ready(&account, revision),
            Err(CpaLifecycleError::StaleSourceManagement)
        ));
    }
    assert_eq!(backend.spawn_count(), 1);
    managed
        .apply_account_management(&account, 3, CpaSourceManagementState::Enabled)
        .unwrap();
    write_managed_identity(&managed, "replacement-account");
    assert!(matches!(
        managed.ensure_saved_runtime_ready(&account, 3),
        Err(CpaLifecycleError::ManagedOAuthAccountChanged)
    ));
    assert_eq!(backend.spawn_count(), 1);
    assert!(
        managed
            .admission
            .state
            .lock()
            .subscription_execution_suspended
    );
    write_managed_identity(&managed, "saved-account");
    managed
        .apply_account_management(&account, 3, CpaSourceManagementState::Enabled)
        .unwrap();
    assert!(matches!(
        managed.ensure_saved_runtime_ready(&account, 3).unwrap(),
        CpaHealth::Ready { .. }
    ));
    assert_eq!(backend.spawn_count(), 2);
    managed.stop_existing_managed_process().unwrap();
}

#[test]
fn native_saved_recovery_rejects_revoked_or_replaced_authority_before_restart() {
    for rejection in [
        "unsaved",
        "disabled",
        "stale",
        "suspended",
        "changed-account",
    ] {
        let root = tempfile::tempdir().unwrap();
        let backend = Arc::new(FakeBackend::default());
        let control = Arc::new(FakeControl::default());
        let native = fixture_runtime(&root, backend.clone(), control.clone(), 2);
        let account = native.inspect_subscription().unwrap().account_ref();
        native.start().unwrap();
        backend.crash_latest(2);
        if rejection != "unsaved" {
            native
                .apply_account_management(&account, 2, CpaSourceManagementState::Enabled)
                .unwrap();
        }
        if rejection == "disabled" {
            native
                .apply_account_management(&account, 3, CpaSourceManagementState::Disabled)
                .unwrap();
        } else if rejection == "suspended" {
            native.suspend_subscription_execution();
        } else if rejection == "changed-account" {
            write_fixture_codex_source(
                native
                    .spec
                    .borrowed_codex_auth
                    .as_ref()
                    .unwrap()
                    .source_path(),
                "replacement-account",
                "fixture-access",
                "fixture-id-token",
                "fixture-refresh-time",
            );
        }
        let probes = control.probes.load(Ordering::SeqCst);
        let revision = if rejection == "stale" {
            1
        } else if rejection == "disabled" {
            3
        } else {
            2
        };
        let error = native
            .ensure_saved_runtime_ready(&account, revision)
            .unwrap_err();
        match rejection {
            "unsaved" => assert!(matches!(error, CpaLifecycleError::InvalidSourceManagement)),
            "changed-account" => assert!(matches!(
                error,
                CpaLifecycleError::BorrowedCodexAuthSourceChanged
            )),
            _ => assert!(matches!(error, CpaLifecycleError::StaleSourceManagement)),
        }
        assert_eq!(backend.spawn_count(), 1, "{rejection}");
        assert_eq!(control.probes.load(Ordering::SeqCst), probes);
        assert_eq!(control.discoveries.load(Ordering::SeqCst), 0);
        native.shutdown().unwrap();
    }
}

fn lease_saved_target(
    runtime: &ManagedCpaRuntime,
    target: &crate::PreparedCpaTarget,
) -> Result<Option<crate::CpaDownstreamCredentialCapability>, CpaAttemptError> {
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

fn saved_managed_target(runtime: &ManagedCpaRuntime) -> crate::PreparedCpaTarget {
    let materialized = runtime
        .materialize_account("connector.cpa.codex", "endpoint.cpa.codex")
        .unwrap();
    prepare_target(
        runtime,
        ExactCpaAttemptRequest {
            credential_ref: &materialized.credential_ref,
            upstream_model_id: "gpt-5.5",
            protocol: UpstreamProtocol::Responses,
        },
    )
    .unwrap()
}

#[test]
fn saved_managed_recovery_restores_exact_routing_after_transient_failure() {
    saved_subscription_recovery_restores_exact_routing_after_transient_failure(true);
}

#[test]
fn saved_native_recovery_restores_exact_routing_after_transient_failure() {
    saved_subscription_recovery_restores_exact_routing_after_transient_failure(false);
}

fn saved_subscription_recovery_restores_exact_routing_after_transient_failure(is_managed: bool) {
    for fault in [
        "probe",
        "half-written-credential",
        "attempt-credential-read",
        "startup-catalog",
    ] {
        let root = tempfile::tempdir().unwrap();
        let backend = Arc::new(FakeBackend::default());
        let control = Arc::new(FakeControl::default());
        control.set_accounts(vec![snapshot(CpaAccountKind::Codex, 'a', "gpt-5.5")]);
        let managed = if is_managed {
            runtime(&root, backend.clone(), control.clone())
        } else {
            fixture_runtime(&root, backend.clone(), control.clone(), 2)
        };
        let account = if is_managed {
            write_managed_identity(&managed, "saved-account")
        } else {
            managed.inspect_subscription().unwrap().account_ref()
        };
        let credential_path = if is_managed {
            managed.spec.auth_dir.join("credential.json")
        } else {
            managed
                .spec
                .borrowed_codex_auth
                .as_ref()
                .unwrap()
                .source_path()
                .to_owned()
        };
        let original_credential = std::fs::read(&credential_path).unwrap();
        let original_evidence = if is_managed {
            None
        } else {
            Some(managed.inspect_subscription().unwrap())
        };
        managed
            .apply_account_management(&account, 7, CpaSourceManagementState::Enabled)
            .unwrap();
        managed.ensure_saved_runtime_ready(&account, 7).unwrap();
        let target = saved_managed_target(&managed);
        let old_capability = lease_saved_target(&managed, &target).unwrap().unwrap();
        let discoveries = control.discoveries.load(Ordering::SeqCst);

        if fault == "probe" {
            control.set_fail_probes(true);
        } else if fault == "startup-catalog" {
            backend.crash_latest(2);
            *control.discovery_error.lock() = Some(AccountDiscoveryError::PinNotApplied);
        } else {
            private_atomic_write(&credential_path, b"{").unwrap();
        }
        if fault == "attempt-credential-read" {
            assert!(matches!(
                lease_saved_target(&managed, &target),
                Err(CpaAttemptError::RevokedCredential)
            ));
        } else {
            assert!(managed.ensure_saved_runtime_ready(&account, 7).is_err());
        }
        assert!(!managed.inner.lock().live.as_ref().unwrap().accounts[0].active);
        assert_eq!(
            old_capability.apply_authorization(&mut http::HeaderMap::new()),
            Err(CpaAttemptError::RevokedCredential)
        );

        control.set_fail_probes(false);
        let repaired_credential =
            matches!(fault, "half-written-credential" | "attempt-credential-read");
        if repaired_credential {
            private_atomic_write(&credential_path, &original_credential).unwrap();
        }
        if let Some(original_evidence) = original_evidence {
            let current = managed.inspect_subscription().unwrap();
            if repaired_credential {
                assert_eq!(
                    current.binding_evidence_digest(),
                    original_evidence.binding_evidence_digest()
                );
            } else {
                // Probe/catalog outages leave the native owner's evidence untouched.
                assert_eq!(current, original_evidence);
            }
        }
        // Match maintenance's same saved revision. No Check, new target preparation
        // or external discovery is allowed to conceal an unrecovered account cache.
        managed
            .apply_account_management(&account, 7, CpaSourceManagementState::Enabled)
            .unwrap();
        managed.ensure_saved_runtime_ready(&account, 7).unwrap();
        let current = lease_saved_target(&managed, &target).unwrap().unwrap();
        current
            .apply_authorization(&mut http::HeaderMap::new())
            .unwrap();
        assert_eq!(
            current.credential_ref().credential_id(),
            target.credential_ref().credential_id()
        );
        assert!(current.generation() > old_capability.generation());
        assert_eq!(
            old_capability.apply_authorization(&mut http::HeaderMap::new()),
            Err(CpaAttemptError::RevokedCredential)
        );
        let expected_discoveries = discoveries + 1 + usize::from(fault == "startup-catalog");
        assert_eq!(
            control.discoveries.load(Ordering::SeqCst),
            expected_discoveries
        );
        assert!(!control.refresh_requested.load(Ordering::SeqCst));
        managed.ensure_saved_runtime_ready(&account, 7).unwrap();
        assert_eq!(
            control.discoveries.load(Ordering::SeqCst),
            expected_discoveries
        );
        assert_eq!(
            backend.spawn_count(),
            1 + usize::from(fault == "startup-catalog")
        );
        assert_eq!(
            std::fs::read(&credential_path).unwrap(),
            original_credential
        );
        managed.shutdown().unwrap();
    }
}

#[test]
fn saved_managed_recovery_revalidates_disappeared_account_and_model() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    control.set_accounts(vec![snapshot(CpaAccountKind::Codex, 'a', "gpt-5.5")]);
    let managed = runtime(&root, backend.clone(), control.clone());
    let account = write_managed_identity(&managed, "saved-account");
    managed
        .apply_account_management(&account, 7, CpaSourceManagementState::Enabled)
        .unwrap();
    managed.ensure_saved_runtime_ready(&account, 7).unwrap();
    let target = saved_managed_target(&managed);
    control.set_fail_probes(true);
    assert!(managed.ensure_saved_runtime_ready(&account, 7).is_err());
    control.set_fail_probes(false);
    control.set_accounts(vec![]);
    managed
        .apply_account_management(&account, 7, CpaSourceManagementState::Enabled)
        .unwrap();
    assert!(managed.ensure_saved_runtime_ready(&account, 7).is_err());
    assert!(
        managed
            .admission
            .state
            .lock()
            .subscription_execution_suspended
    );
    assert!(matches!(
        lease_saved_target(&managed, &target),
        Err(CpaAttemptError::RevokedCredential)
    ));

    // Recover the exact account with a changed inventory: the removed model must
    // remain unavailable even though the saved source and process are healthy.
    control.set_accounts(vec![snapshot(CpaAccountKind::Codex, 'a', "gpt-5.4")]);
    managed
        .apply_account_management(&account, 7, CpaSourceManagementState::Enabled)
        .unwrap();
    managed.ensure_saved_runtime_ready(&account, 7).unwrap();
    assert!(matches!(
        lease_saved_target(&managed, &target),
        Err(CpaAttemptError::ModelUnavailable)
    ));
    assert_eq!(backend.spawn_count(), 1);
    managed.stop_existing_managed_process().unwrap();
}

#[test]
fn revoked_requests_and_passive_discovery_never_restart_a_managed_writer() {
    for revocation in ["suspended", "disabled", "removed", "pending"] {
        let root = tempfile::tempdir().unwrap();
        let backend = Arc::new(FakeBackend::default());
        let control = Arc::new(FakeControl::default());
        control.set_accounts(vec![snapshot(CpaAccountKind::Codex, 'a', "gpt-5.5")]);
        let managed = runtime(&root, backend.clone(), control.clone());
        let account = write_managed_identity(&managed, "saved-account");
        managed
            .apply_account_management(&account, 7, CpaSourceManagementState::Enabled)
            .unwrap();
        managed.ensure_saved_runtime_ready(&account, 7).unwrap();
        let target = saved_managed_target(&managed);
        let batch = managed.begin_routing_batch().unwrap();
        match revocation {
            "suspended" => managed.suspend_subscription_execution(),
            "disabled" | "removed" => managed
                .apply_account_management(
                    &account,
                    8,
                    if revocation == "disabled" {
                        CpaSourceManagementState::Disabled
                    } else {
                        CpaSourceManagementState::Removed
                    },
                )
                .unwrap(),
            // A logged-in/checkable credential has no saved execution authority.
            "pending" => managed.admission.state.lock().source_management.clear(),
            _ => unreachable!(),
        }
        backend.crash_latest(2);
        let probes = control.probes.load(Ordering::SeqCst);
        let discoveries = control.discoveries.load(Ordering::SeqCst);
        assert!(matches!(
            lease_saved_target(&managed, &target),
            Err(CpaAttemptError::RevokedCredential)
        ));
        assert!(!batch.finish().unwrap_or(false));
        assert!(
            managed
                .discover_registered_sources()
                .map_or(true, |sources| sources.is_empty())
        );
        assert!(
            managed
                .materialize_account("connector.cpa.codex", "endpoint.cpa.codex")
                .is_err()
        );
        assert_eq!(backend.spawn_count(), 1, "{revocation} restarted CPA");
        assert_eq!(control.probes.load(Ordering::SeqCst), probes);
        assert_eq!(control.discoveries.load(Ordering::SeqCst), discoveries);
        managed.stop_existing_managed_process().unwrap();
    }
}

#[test]
fn admitted_request_and_explicit_disabled_check_keep_bounded_recovery() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    control.set_accounts(vec![snapshot(CpaAccountKind::Codex, 'a', "gpt-5.5")]);
    let managed = runtime(&root, backend.clone(), control);
    let account = write_managed_identity(&managed, "saved-account");
    managed
        .apply_account_management(&account, 7, CpaSourceManagementState::Enabled)
        .unwrap();
    managed.ensure_saved_runtime_ready(&account, 7).unwrap();
    let target = saved_managed_target(&managed);
    backend.crash_latest(2);
    // An exact admitted request still has authority to recover its own writer.
    lease_saved_target(&managed, &target)
        .unwrap()
        .unwrap()
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    assert_eq!(backend.spawn_count(), 2);
    managed
        .apply_account_management(&account, 8, CpaSourceManagementState::Disabled)
        .unwrap();
    backend.crash_latest(2);
    let expected = crate::BorrowedSubscriptionEvidence::Managed(
        managed.managed_oauth_source().unwrap().inspect().unwrap(),
    );
    let checked = managed.discover_materializations(Some(&expected)).unwrap();
    assert_eq!(checked.len(), 1);
    assert_eq!(backend.spawn_count(), 3);
    assert!(matches!(
        lease_saved_target(&managed, &target),
        Err(CpaAttemptError::RevokedCredential)
    ));
    assert_eq!(backend.spawn_count(), 3);
    managed.stop_existing_managed_process().unwrap();
}

#[test]
fn retained_targets_validate_each_process_catalog_in_both_subscription_modes() {
    if isolated_owner_recovery_case(
        "runtime::tests::managed_recovery::retained_targets_validate_each_process_catalog_in_both_subscription_modes",
    ) {
        return;
    }
    for managed in [false, true] {
        for recovery in ["fresh", "adopt", "crash"] {
            let root = tempfile::tempdir().unwrap();
            let backend = Arc::new(FakeBackend::default());
            let control = Arc::new(FakeControl::default());
            control.set_accounts(vec![snapshot(CpaAccountKind::Codex, 'a', "gpt-5.5")]);
            let make_runtime = || {
                if managed {
                    runtime(&root, backend.clone(), control.clone())
                } else {
                    fixture_runtime(&root, backend.clone(), control.clone(), 2)
                }
            };
            let first = make_runtime();
            if managed {
                write_managed_identity(&first, "saved-account");
            }
            first.start().unwrap();
            let account = first
                .materialize_account("connector.cpa.codex", "endpoint.cpa.codex")
                .unwrap()
                .account_subject;
            first
                .apply_account_management(&account, 7, CpaSourceManagementState::Enabled)
                .unwrap();
            let target = saved_managed_target(&first);
            lease_saved_target(&first, &target).unwrap().unwrap();
            let discoveries = control.discoveries.load(Ordering::SeqCst);

            // Health stays green and the retained snapshot still allows gpt-5.5,
            // but the actual process inventory no longer contains that model.
            control.set_accounts(vec![snapshot(CpaAccountKind::Codex, 'a', "gpt-5.4")]);
            let restored = match recovery {
                "fresh" => {
                    first.shutdown().unwrap();
                    drop(first);
                    make_runtime()
                }
                "adopt" => {
                    // The older stop-only helper removes the proxy fingerprint;
                    // preserve it here so recovery adopts instead of replacing.
                    let proxy_digest = first.proxy_environment.digest();
                    let path = leave_orphan(first);
                    let mut record: serde_json::Value =
                        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                    record["proxy_environment_sha256"] = json!(proxy_digest);
                    private_atomic_write(&path, &serde_json::to_vec(&record).unwrap()).unwrap();
                    make_runtime()
                }
                _ => {
                    backend.crash_latest(2);
                    first
                }
            };
            restored
                .apply_account_management(&account, 7, CpaSourceManagementState::Enabled)
                .unwrap();
            if managed {
                restored.ensure_saved_runtime_ready(&account, 7).unwrap();
            } else if recovery != "crash" {
                restored.start().unwrap();
            }
            // No Check, target preparation or inventory read may mask the gap.
            assert!(matches!(
                lease_saved_target(&restored, &target),
                Err(CpaAttemptError::ModelUnavailable)
            ));
            assert_eq!(control.discoveries.load(Ordering::SeqCst), discoveries + 1);
            assert!(!control.refresh_requested.load(Ordering::SeqCst));
            assert_eq!(
                backend.spawn_count(),
                if recovery == "adopt" { 1 } else { 2 }
            );
            assert_eq!(backend.attach_count(), usize::from(recovery == "adopt"));
            if managed {
                restored.ensure_saved_runtime_ready(&account, 7).unwrap();
            }
            assert!(matches!(
                lease_saved_target(&restored, &target),
                Err(CpaAttemptError::ModelUnavailable)
            ));
            assert_eq!(control.discoveries.load(Ordering::SeqCst), discoveries + 1);
            restored.shutdown().unwrap();
        }
    }
}
