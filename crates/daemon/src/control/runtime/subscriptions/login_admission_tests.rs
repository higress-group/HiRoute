//! Exercise credential withdrawal through the real daemon planner, writer and saved projection.
//! Only the external CPA process is synthetic; no native credential store is opened.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use hiroute_application::control::{
    ApplicationMutationPort, ComputeManagementControlError, ComputeManagementControlPort,
};
use hiroute_application::{
    PreparedTransactionV1, TransactionCoordinator, TransactionError, VerifiedPrincipal,
};
use hiroute_application_api::{
    COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2, ComputeConnectionApplyRequestV1,
    ComputeManagementChangeV2, ComputeManagementIntentV2, ComputeManagementSubjectV2,
    ComputeSubscriptionCheckResultV2, ComputeSubscriptionCheckStatusV2,
    ComputeSubscriptionLoginRequestV1, ComputeSubscriptionLoginSessionV1,
    SubscriptionLoginProviderV1, SubscriptionLoginStatusV1,
};
use hiroute_cpa_bridge::{
    CpaAccountKind, CpaLoginState, CpaProfileBinding, CpaRegisteredSourcePort, CpaRuntimeSpec,
    MANAGED_CPA_ARTIFACT_VERSION, ManagedCpaRuntime, ManagedCpaRuntimeSet, PinnedCpaArtifact,
    PinnedCpaBinaryLocator, RestartPolicy,
};
use hiroute_domain::{
    ComputeManagementRepositoryPort, ComputeManagementStoredSnapshotV2, ControlRepositoryPort,
    MaterializationState, OperationState, ProtectedSecret, WorkspaceId,
};
use hiroute_integrations::{
    AgentFilesystemLayoutV1, ClaudeRegistrationIndexV1, FilesystemAgentScannerV1,
};
use sha2::{Digest, Sha256};

use super::super::{LocalControlAdapter, ProductionControlRuntime, RuntimeOpenOverrides};

#[path = "discovery_tests.rs"]
mod discovery_tests;

struct Fixture {
    runtime: ProductionControlRuntime,
    runtimes: Arc<ManagedCpaRuntimeSet>,
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let root = crate::test_support::private_tempdir();
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../cpa-bridge/src/managed_sessions/cpa_oauth_fixture.py"
        ))
        .replace("__HIR_CPA_VERSION__", MANAGED_CPA_ARTIFACT_VERSION)
        // This fixture consumes the current executable catalog, whose Claude
        // subscription models no longer include Sonnet 4.6.
        .replace("else 'claude-sonnet-4-6'", "else 'claude-sonnet-5'")
        // Record every authenticated readiness/status request as well as the
        // catalog calls, so exact sibling queries cannot hide extra probes.
        .replace("name = query.get('name', [''])[0]", "name = query.get('name', [''])[0]\n            record_catalog('credentials', name=name)")
        .replace("if (controls / 'catalog-auth-unavailable').exists():", "if (controls / 'catalog-auth-unavailable').exists() or (auth_dir.parent / 'catalog-auth-unavailable').exists():");
        let binary = root.path().join("cpa-fixture");
        fs::write(&binary, source.as_bytes()).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let locator = Arc::new(PinnedCpaBinaryLocator::new(PinnedCpaArtifact::new(
            root.path(),
            "cpa-fixture",
            MANAGED_CPA_ARTIFACT_VERSION.parse().unwrap(),
            format!("{:x}", Sha256::digest(source.as_bytes())),
        )));
        let catalog = Arc::new(crate::release_catalog::fixture_catalog());
        let templates = [CpaAccountKind::Codex, CpaAccountKind::Claude]
            .into_iter()
            .map(|kind| {
                let provider = kind.stock_provider();
                Arc::new(
                    ManagedCpaRuntime::new(
                        CpaRuntimeSpec {
                            instance_id: format!("template-{provider}"),
                            state_root: root.path().join("cpa/state"),
                            auth_dir: root.path().join(format!("cpa/template-{provider}-auth")),
                            borrowed_codex_auth: None,
                            borrowed_claude_auth: None,
                            managed_oauth: Some(kind),
                            bindings: vec![CpaProfileBinding {
                                account_kind: kind,
                                connector_id: format!("connector.cpa.{provider}"),
                                connection_option_id: format!("{provider}.subscription.global.v1"),
                                endpoint_profile_id: format!("endpoint.cpa.{provider}"),
                            }],
                            startup_timeout: Duration::from_secs(3),
                            control_timeout: Duration::from_secs(1),
                            shutdown_timeout: Duration::from_secs(1),
                            restart_policy: RestartPolicy::default(),
                        },
                        catalog.clone(),
                        locator.clone(),
                    )
                    .unwrap(),
                )
            })
            .collect();
        let runtimes = Arc::new(ManagedCpaRuntimeSet::new(templates).unwrap());
        // Every test enters isolated_agent_home before constructing this fixture. Its child
        // process owns fresh HOME, CODEX_HOME and CLAUDE_CONFIG_DIR paths and a sanitized PATH;
        // from_process therefore cannot discover the caller's native subscription stores.
        let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
        let mut layout = AgentFilesystemLayoutV1::from_process(&home, root.path());
        layout.codex_executable = root.path().join("missing-codex");
        layout.claude_executable = root.path().join("missing-claude");
        let scanner = FilesystemAgentScannerV1::new(
            layout,
            ClaudeRegistrationIndexV1::from_verified_model_data(
                catalog.registry(),
                &catalog.current_release_model_data().data,
            )
            .unwrap(),
        );
        let sources: Arc<dyn CpaRegisteredSourcePort + Send + Sync> = runtimes.clone();
        // The production storage initializer requires a fresh empty directory. Keep the
        // external CPA executable, credentials and process logs outside that boundary.
        let storage_root = root.path().join("storage");
        fs::create_dir(&storage_root).unwrap();
        fs::set_permissions(&storage_root, fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = ProductionControlRuntime::open_inner_with_cpa_and_overrides(
            &storage_root,
            (*catalog).clone(),
            Some(sources),
            Some(runtimes.clone()),
            false,
            RuntimeOpenOverrides {
                scanner: Some(scanner),
                ..Default::default()
            },
        )
        .unwrap();
        let target = Arc::new(crate::gateway_ports::GatewayPublicationAdapter::new(
            Arc::new(
                hiroute_gateway::server::publication::GatewayPublicationInstaller::open(
                    storage_root.join("gateway-lkg.json"),
                )
                .unwrap(),
            ),
        ));
        *runtime.adapter.publication_target.lock().unwrap() = Some(target);
        runtime.adapter.reconcile_startup_and_open().unwrap();
        runtime
            .adapter
            .finish_startup_publication_recovery()
            .unwrap();
        // Maintenance is intentionally not enabled: each interleaving below owns its clock.
        Self {
            runtime,
            runtimes,
            root,
        }
    }

    fn adapter(&self) -> &LocalControlAdapter {
        &self.runtime.adapter
    }

    fn authorize(
        &self,
        provider: SubscriptionLoginProviderV1,
    ) -> ComputeSubscriptionLoginSessionV1 {
        let mut started = self
            .adapter()
            .manage_subscription_login(ComputeSubscriptionLoginRequestV1::Start { provider })
            .unwrap();
        let session = started.sessions.remove(0);
        assert_eq!(session.status, SubscriptionLoginStatusV1::Pending);
        // The external fixture emits an unescaped ASCII state in this fixed query format.
        let (_, query) = session
            .authorization_url
            .as_deref()
            .unwrap()
            .split_once('?')
            .unwrap();
        let state = query
            .split('&')
            .find_map(|pair| pair.strip_prefix("state="))
            .unwrap();
        let callback = match provider {
            SubscriptionLoginProviderV1::Codex => format!(
                "http://localhost:1455/auth/callback?state={state}&code=fixture-authorization-code"
            ),
            SubscriptionLoginProviderV1::Claude => format!("fixture-authorization-code#{state}"),
        };
        let input_candidate = session.callback_input_candidate.unwrap();
        self.adapter()
            .manual_protected_inputs
            .lock()
            .unwrap()
            .insert(
                input_candidate.candidate_ref.clone(),
                ProtectedSecret::new(callback.into_bytes()).unwrap(),
            );
        let mut result = self
            .adapter()
            .manage_subscription_login(ComputeSubscriptionLoginRequestV1::Callback {
                login_ref: session.login_ref,
                input_candidate,
            })
            .unwrap();
        let authorized = result.sessions.remove(0);
        assert_eq!(authorized.status, SubscriptionLoginStatusV1::Authorized);
        assert!(authorized.authorization_url.is_none());
        authorized
    }

    fn check(
        &self,
        session: &ComputeSubscriptionLoginSessionV1,
    ) -> ComputeSubscriptionCheckResultV2 {
        let preview = self
            .adapter()
            .preview_subscription_check(session.candidate.clone().unwrap())
            .unwrap();
        let prepared = self
            .adapter()
            .prepare_subscription_check(
                ComputeConnectionApplyRequestV1 {
                    spec: preview.spec,
                    accept_digest: preview.accept_digest,
                    expected_revisions: preview.expected_revisions,
                    idempotency_key: format!("check-{}", session.login_ref),
                },
                None,
            )
            .unwrap();
        let operation = self
            .adapter()
            .apply_local_prepared_change(prepared)
            .unwrap();
        assert_eq!(operation.state, OperationState::Succeeded, "{operation:?}");
        let result = self
            .adapter()
            .compute_subscription_check_result(&super::operation_reference(&operation))
            .unwrap();
        assert_eq!(
            result.status,
            ComputeSubscriptionCheckStatusV2::Verified,
            "{result:?}"
        );
        assert!(!result.checked_candidate.as_ref().unwrap().models.is_empty());
        result
    }

    fn snapshot(&self) -> ComputeManagementStoredSnapshotV2 {
        self.adapter()
            .stores_lock()
            .unwrap()
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .unwrap()
    }

    fn candidate_request(
        &self,
        checked: &ComputeSubscriptionCheckResultV2,
        key: &str,
    ) -> ComputeConnectionApplyRequestV1 {
        let candidate = checked.checked_candidate.as_ref().unwrap();
        self.save_request(
            ComputeManagementChangeV2 {
                edit: None,
                schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
                subject: ComputeManagementSubjectV2::Candidate {
                    candidate: candidate.candidate.clone(),
                },
                expected_revisions: self.snapshot().revisions,
                selected_model_refs: candidate
                    .models
                    .iter()
                    .map(|model| model.model_ref.clone())
                    .collect(),
                intent: ComputeManagementIntentV2::SaveReady,
                key_edits: Vec::new(),
                validation: checked.validation.clone(),
            },
            key,
        )
    }

    fn saved_request(
        &self,
        checked: &ComputeSubscriptionCheckResultV2,
        intent: ComputeManagementIntentV2,
        key: &str,
    ) -> ComputeConnectionApplyRequestV1 {
        let snapshot = self.snapshot();
        assert_eq!(snapshot.sources.len(), 1);
        let source = &snapshot.sources[0];
        self.save_request(
            ComputeManagementChangeV2 {
                edit: None,
                schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
                subject: ComputeManagementSubjectV2::SavedSource {
                    source_id: source.source_id.clone(),
                },
                expected_revisions: snapshot.revisions,
                selected_model_refs: source
                    .models
                    .iter()
                    .map(|model| model.model_ref.clone())
                    .collect(),
                intent,
                key_edits: Vec::new(),
                validation: checked.validation.clone(),
            },
            key,
        )
    }

    fn save_request(
        &self,
        change: ComputeManagementChangeV2,
        key: &str,
    ) -> ComputeConnectionApplyRequestV1 {
        let preview = self.adapter().preview_compute_save(change).unwrap();
        ComputeConnectionApplyRequestV1 {
            spec: preview.spec,
            accept_digest: preview.accept_digest,
            expected_revisions: preview.expected_revisions,
            idempotency_key: key.into(),
        }
    }

    fn prepare(&self, request: ComputeConnectionApplyRequestV1) -> PreparedTransactionV1 {
        self.adapter().prepare_compute_save(request).unwrap()
    }

    fn save(&self, request: ComputeConnectionApplyRequestV1) {
        let operation = self
            .adapter()
            .apply_local_prepared_change(self.prepare(request))
            .unwrap();
        assert_eq!(operation.state, OperationState::Succeeded, "{operation:?}");
    }

    fn forget(&self, login_ref: &str) {
        let result = self
            .adapter()
            .manage_subscription_login(ComputeSubscriptionLoginRequestV1::Forget {
                login_ref: login_ref.into(),
            })
            .unwrap();
        assert_eq!(
            result.sessions[0].status,
            SubscriptionLoginStatusV1::Forgotten
        );
        assert!(!self.credential_path(login_ref).exists());
    }

    fn credential_path(&self, login_ref: &str) -> std::path::PathBuf {
        self.root
            .path()
            .join("cpa/managed-logins")
            .join(login_ref)
            .join("auth/credential.json")
    }

    fn spawns(&self) -> usize {
        fs::read_to_string(self.root.path().join("spawns.jsonl"))
            .unwrap()
            .lines()
            .count()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.runtimes.shutdown();
    }
}

fn isolated(name: &str) -> bool {
    crate::test_support::isolated_agent_home(&format!(
        "control::runtime::subscriptions::login_admission_tests::{name}"
    ))
}

#[test]
fn unsaved_prepared_save_cannot_resurrect_forgotten_login() {
    if isolated("unsaved_prepared_save_cannot_resurrect_forgotten_login") {
        return;
    }
    for provider in [
        SubscriptionLoginProviderV1::Codex,
        SubscriptionLoginProviderV1::Claude,
    ] {
        let fixture = Fixture::new();
        let login = fixture.authorize(provider);
        let checked = fixture.check(&login);
        let prepared = fixture.prepare(fixture.candidate_request(&checked, "stale-unsaved"));
        let before = fixture.snapshot();
        fixture.forget(&login.login_ref);
        assert_eq!(
            fixture.snapshot(),
            before,
            "unsaved Forget must exercise the unchanged DB revision window"
        );
        let spawns = fixture.spawns();
        assert!(matches!(
            fixture.adapter().apply_local_prepared_change(prepared),
            Err(TransactionError::ChangePreviewStale)
        ));
        assert!(fixture.snapshot().sources.is_empty());
        assert_eq!(fixture.spawns(), spawns, "stale Apply must not restart CPA");
    }
}

#[test]
fn saved_reenable_with_retained_models_revalidates_login_at_admission() {
    if isolated("saved_reenable_with_retained_models_revalidates_login_at_admission") {
        return;
    }
    let fixture = Fixture::new();
    let login = fixture.authorize(SubscriptionLoginProviderV1::Claude);
    let checked = fixture.check(&login);
    fixture.save(fixture.candidate_request(&checked, "first-save"));
    fixture.save(fixture.saved_request(
        &checked,
        ComputeManagementIntentV2::SaveDisabled,
        "disable",
    ));
    let prepared = fixture.prepare(fixture.saved_request(
        &checked,
        ComputeManagementIntentV2::SaveReady,
        "stale-reenable",
    ));
    let before = fixture.snapshot();
    assert_eq!(before.sources[0].state, MaterializationState::Disabled);
    assert!(!before.sources[0].models.is_empty());
    fixture.forget(&login.login_ref);
    assert_eq!(
        fixture.snapshot(),
        before,
        "already disabled Forget cannot hide behind a revision conflict"
    );
    assert!(matches!(
        fixture.adapter().apply_local_prepared_change(prepared),
        Err(TransactionError::ChangePreviewStale)
    ));
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn unfinished_admitted_writer_prevents_forget_and_preserves_credential() {
    if isolated("unfinished_admitted_writer_prevents_forget_and_preserves_credential") {
        return;
    }
    let fixture = Fixture::new();
    let login = fixture.authorize(SubscriptionLoginProviderV1::Codex);
    let checked = fixture.check(&login);
    let adapter = fixture.adapter();
    let coordinator = TransactionCoordinator::new(
        adapter,
        adapter,
        adapter,
        adapter,
        adapter,
        &adapter.admission,
    );
    let accepted = coordinator
        .accept_prepared(
            &WorkspaceId::default(),
            &VerifiedPrincipal::for_local_control(),
            fixture.prepare(fixture.candidate_request(&checked, "accepted-before-forget")),
        )
        .unwrap();
    assert_eq!(accepted.operation().state, OperationState::Accepted);
    assert!(
        adapter
            .stores_lock()
            .unwrap()
            .control()
            .writer_recovery_required()
            .unwrap()
    );
    assert!(matches!(
        adapter.manage_subscription_login(ComputeSubscriptionLoginRequestV1::Forget {
            login_ref: login.login_ref.clone(),
        }),
        Err(ComputeManagementControlError::Conflict)
    ));
    assert_eq!(
        fixture
            .runtimes
            .login_session(&login.login_ref)
            .unwrap()
            .state,
        CpaLoginState::Authorized
    );
    assert!(fixture.credential_path(&login.login_ref).exists());
    let operation = coordinator.run_accepted(accepted).unwrap();
    assert_eq!(operation.state, OperationState::Succeeded);
    fixture.forget(&login.login_ref);
    assert_eq!(
        fixture.snapshot().sources[0].state,
        MaterializationState::Disabled
    );
}

#[test]
fn forget_waits_for_save_lifecycle_and_disables_the_new_binding() {
    if isolated("forget_waits_for_save_lifecycle_and_disables_the_new_binding") {
        return;
    }
    let fixture = Fixture::new();
    let login = fixture.authorize(SubscriptionLoginProviderV1::Claude);
    let checked = fixture.check(&login);
    let prepared = fixture.prepare(fixture.candidate_request(&checked, "save-holds-lifecycle"));
    let adapter = fixture.runtime.adapter.clone();
    let (saved_tx, saved_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let save = std::thread::spawn(move || {
        let lifecycle = adapter.lock_subscription_lifecycle().unwrap();
        let operation = adapter
            .apply_local_prepared_with_subscription_guard(prepared, &lifecycle)
            .unwrap();
        assert_eq!(operation.state, OperationState::Succeeded);
        saved_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        drop(lifecycle);
    });
    saved_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let adapter = fixture.runtime.adapter.clone();
    let login_ref = login.login_ref.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let forget = std::thread::spawn(move || {
        entered_tx.send(()).unwrap();
        let result = adapter
            .manage_subscription_login(ComputeSubscriptionLoginRequestV1::Forget { login_ref });
        finished_tx.send(result).unwrap();
    });
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(matches!(
        finished_rx.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(fixture.credential_path(&login.login_ref).exists());
    release_tx.send(()).unwrap();
    save.join().unwrap();
    let result = finished_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    forget.join().unwrap();
    assert_eq!(
        result.sessions[0].status,
        SubscriptionLoginStatusV1::Forgotten
    );
    assert_eq!(
        fixture.snapshot().sources[0].state,
        MaterializationState::Disabled
    );
    assert!(!fixture.credential_path(&login.login_ref).exists());
}

#[test]
fn save_waits_for_withdrawal_lifecycle_then_rejects_the_old_prepared() {
    if isolated("save_waits_for_withdrawal_lifecycle_then_rejects_the_old_prepared") {
        return;
    }
    let fixture = Fixture::new();
    let login = fixture.authorize(SubscriptionLoginProviderV1::Codex);
    let checked = fixture.check(&login);
    let prepared = fixture.prepare(fixture.candidate_request(&checked, "save-after-withdrawal"));
    let lifecycle = fixture.adapter().lock_subscription_lifecycle().unwrap();
    let adapter = fixture.runtime.adapter.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let save = std::thread::spawn(move || {
        entered_tx.send(()).unwrap();
        finished_tx
            .send(adapter.apply_local_prepared_change(prepared))
            .unwrap();
    });
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(matches!(
        finished_rx.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    // The unsaved Forget boundary has no source to disable. Perform its exact CPA withdrawal
    // while holding the same production lifecycle guard; the public Forget path is covered above.
    let forgotten = fixture
        .runtimes
        .cancel_login(&login.login_ref, true)
        .unwrap();
    assert_eq!(forgotten.state, CpaLoginState::Forgotten);
    let spawns = fixture.spawns();
    drop(lifecycle);
    assert!(matches!(
        finished_rx.recv_timeout(Duration::from_secs(10)).unwrap(),
        Err(TransactionError::ChangePreviewStale)
    ));
    save.join().unwrap();
    assert!(fixture.snapshot().sources.is_empty());
    assert_eq!(fixture.spawns(), spawns);
}

#[test]
fn succeeded_replay_after_forget_preserves_disabled_source_without_restart() {
    if isolated("succeeded_replay_after_forget_preserves_disabled_source_without_restart") {
        return;
    }
    let fixture = Fixture::new();
    let login = fixture.authorize(SubscriptionLoginProviderV1::Claude);
    let checked = fixture.check(&login);
    let request = fixture.candidate_request(&checked, "replay-after-forget");
    let first = fixture.prepare(request.clone());
    let replay = fixture.prepare(request);
    let original = fixture
        .adapter()
        .apply_local_prepared_change(first)
        .unwrap();
    assert_eq!(original.state, OperationState::Succeeded);
    fixture.forget(&login.login_ref);
    let before = fixture.snapshot();
    let spawns = fixture.spawns();
    let repeated = fixture
        .adapter()
        .apply_local_prepared_change(replay)
        .unwrap();
    assert_eq!(repeated.operation_id, original.operation_id);
    assert_eq!(
        repeated.state,
        OperationState::Succeeded,
        "a succeeded replay is not a fresh save"
    );
    assert_eq!(fixture.snapshot(), before);
    assert_eq!(before.sources[0].state, MaterializationState::Disabled);
    assert_eq!(fixture.spawns(), spawns);
    assert!(!fixture.credential_path(&login.login_ref).exists());
}

#[test]
fn deleting_saved_connection_keeps_managed_login_and_does_not_restart_provider() {
    if isolated("deleting_saved_connection_keeps_managed_login_and_does_not_restart_provider") {
        return;
    }
    for provider in [
        SubscriptionLoginProviderV1::Codex,
        SubscriptionLoginProviderV1::Claude,
    ] {
        let fixture = Fixture::new();
        let login = fixture.authorize(provider);
        let checked = fixture.check(&login);
        fixture.save(fixture.candidate_request(&checked, "first-save"));
        let snapshot = fixture.snapshot();
        let source = &snapshot.sources[0];
        let path = fixture.credential_path(&login.login_ref);
        let before = fs::read(&path).unwrap();
        let spawns = fixture.spawns();
        let request = fixture.save_request(
            ComputeManagementChangeV2 {
                schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
                subject: ComputeManagementSubjectV2::SavedSource {
                    source_id: source.source_id.clone(),
                },
                expected_revisions: snapshot.revisions,
                selected_model_refs: Vec::new(),
                intent: ComputeManagementIntentV2::SaveReady,
                key_edits: Vec::new(),
                validation: None,
                edit: Some(hiroute_application_api::ComputeManagementEditV1::Delete),
            },
            "delete-connection",
        );
        fixture.save(request);
        assert!(fixture.snapshot().sources.is_empty());
        assert_eq!(
            fs::read(path).unwrap(),
            before,
            "connection deletion must preserve the independent login"
        );
        assert_eq!(fixture.spawns(), spawns);
    }
}
