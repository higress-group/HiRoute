//! Real request admission against private native files and bounded, controlled I/O.
use super::*;
use std::sync::{Condvar as StdCondvar, Mutex as StdMutex, mpsc};
use std::thread;
use std::time::Instant;

use crate::borrowed_claude::ClaudeProfileReader;
use crate::{BorrowedClaudeAuthSpec, CpaDownstreamCredentialCapability, CpaRequestContext};
use hiroute_integrations::ClaudeSubscriptionLocation;

const PROMPT: Duration = Duration::from_millis(600);

#[derive(Default)]
struct IoGate {
    state: StdMutex<(usize, usize, bool)>,
    changed: StdCondvar,
}

impl IoGate {
    fn block(&self) {
        let mut state = self.state.lock().unwrap();
        state.0 += 1;
        self.changed.notify_all();
        while !state.2 {
            state = self.changed.wait(state).unwrap();
        }
        state.1 += 1;
        self.changed.notify_all();
    }

    fn wait_entered(&self) {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut state = self.state.lock().unwrap();
        while state.0 == 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "the controlled I/O was never reached");
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }

    fn release(&self) {
        self.state.lock().unwrap().2 = true;
        self.changed.notify_all();
    }
}

struct ReleaseOnDrop(Arc<IoGate>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[derive(Clone)]
struct ProfileAction {
    token: String,
    gate: Arc<IoGate>,
    fail: bool,
    account: String,
}

#[derive(Default)]
struct GatedProfile {
    calls: AtomicUsize,
    action: StdMutex<Option<ProfileAction>>,
}

impl GatedProfile {
    fn arm(&self, token: &str, gate: &Arc<IoGate>, fail: bool, account: &str) {
        *self.action.lock().unwrap() = Some(ProfileAction {
            token: token.into(),
            gate: Arc::clone(gate),
            fail,
            account: account.into(),
        });
    }
}

impl ClaudeProfileReader for GatedProfile {
    fn fetch(
        &self,
        token: &str,
        _proxy: &crate::proxy_environment::ProxyEnvironment,
    ) -> Result<String, CpaLifecycleError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let action = self.action.lock().unwrap().clone();
        if let Some(action) = action.filter(|action| action.token == token) {
            action.gate.block();
            if action.fail {
                return Err(CpaLifecycleError::BorrowedClaudeAuthUnavailable);
            }
            return Ok(action.account);
        }
        Ok("request-io-account-one".into())
    }
}

struct GatedControl {
    inner: Arc<FakeControl>,
    next_probe: StdMutex<Option<Arc<IoGate>>>,
}

impl CpaControlPlane for GatedControl {
    fn probe_ready(
        &self,
        address: SocketAddr,
        secrets: &InstanceSecrets,
        version: &str,
        timeout: Duration,
    ) -> Result<(), AccountDiscoveryError> {
        let gate = self.next_probe.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.block();
        }
        self.inner.probe_ready(address, secrets, version, timeout)
    }

    fn discover_and_pin(
        &self,
        address: SocketAddr,
        auth_dir: &Path,
        identities: &[ManagedAccountIdentity],
        secrets: &InstanceSecrets,
        version: &str,
        timeout: Duration,
        refresh_models: bool,
    ) -> Result<Vec<AccountSnapshotRecord>, AccountDiscoveryError> {
        self.inner.discover_and_pin(
            address,
            auth_dir,
            identities,
            secrets,
            version,
            timeout,
            refresh_models,
        )
    }
}

#[derive(Clone, Copy)]
pub(super) enum SourceKind {
    NativeClaude,
    NativeCodex,
    ManagedCodex,
}

pub(super) struct ActiveFixture {
    pub(super) runtime: Arc<ManagedCpaRuntime>,
    backend: Arc<FakeBackend>,
    control: Arc<GatedControl>,
    profile: Arc<GatedProfile>,
    pub(super) source_path: PathBuf,
    auth_path: PathBuf,
    account: String,
    target: crate::PreparedCpaTarget,
    pub(super) capability: CpaDownstreamCredentialCapability,
    _root: tempfile::TempDir,
}

impl ActiveFixture {
    pub(super) fn account_subject(&self) -> &str {
        &self.account
    }

    pub(super) fn spawn_count(&self) -> usize {
        self.backend.spawn_count()
    }

    pub(super) fn profile_calls(&self) -> usize {
        self.profile.calls.load(Ordering::SeqCst)
    }

    pub(super) fn control_calls(&self) -> (usize, usize) {
        (
            self.control.inner.probes.load(Ordering::SeqCst),
            self.control.inner.discoveries.load(Ordering::SeqCst),
        )
    }
}

fn write_claude_source(path: &Path, token: &str) {
    private_atomic_write(
        path,
        &serde_json::to_vec(&json!({
            "claudeAiOauth": {
                "accessToken": token, "refreshToken": "TEST_REFRESH_REMAINS_NATIVE",
                "expiresAt": u64::MAX, "scopes": ["user:profile", "user:inference"]
            }
        }))
        .unwrap(),
    )
    .unwrap();
}

fn lease(
    runtime: &ManagedCpaRuntime,
    target: &crate::PreparedCpaTarget,
    context: &CpaRequestContext,
) -> Result<Option<CpaDownstreamCredentialCapability>, CpaAttemptError> {
    runtime.lease_downstream_capability_scoped(
        ExactCpaCredentialRequest {
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
        },
        context,
    )
}

fn fresh_context() -> CpaRequestContext {
    CpaRequestContext::new(Instant::now() + Duration::from_secs(5))
}

pub(super) fn active_fixture(kind: SourceKind) -> ActiveFixture {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let fake_control = Arc::new(FakeControl::default());
    let template = fixture_runtime(&root, Arc::clone(&backend), Arc::clone(&fake_control), 2);
    let profile = Arc::new(GatedProfile::default());
    let mut spec = template.spec.clone();
    let source_path;
    let account_kind;
    let model;
    let protocol;
    match kind {
        SourceKind::NativeClaude => {
            source_path = root.path().join("claude-native.json");
            write_claude_source(&source_path, "initial-access");
            let mut borrowed =
                BorrowedClaudeAuthSpec::new(ClaudeSubscriptionLocation::File(source_path.clone()));
            borrowed.profile_reader = profile.clone();
            spec.borrowed_codex_auth = None;
            spec.borrowed_claude_auth = Some(borrowed);
            account_kind = CpaAccountKind::Claude;
            model = "claude-sonnet-5";
            protocol = UpstreamProtocol::Messages;
        }
        SourceKind::NativeCodex => {
            source_path = spec
                .borrowed_codex_auth
                .as_ref()
                .unwrap()
                .source_path()
                .to_owned();
            account_kind = CpaAccountKind::Codex;
            model = "gpt-5.5";
            protocol = UpstreamProtocol::Responses;
        }
        SourceKind::ManagedCodex => {
            spec = template
                .fork_managed_oauth(
                    "request-io-managed".into(),
                    root.path().join("managed/auth"),
                    CpaAccountKind::Codex,
                )
                .unwrap()
                .spec
                .clone();
            ensure_private_dir(&spec.auth_dir).unwrap();
            source_path = spec.auth_dir.join("credential.json");
            private_atomic_write(
                &source_path,
                &serde_json::to_vec(&json!({
                    "type": "codex", "access_token": "fixture-access",
                    "refresh_token": "fixture-managed-refresh", "account_id": "managed-account-one"
                }))
                .unwrap(),
            )
            .unwrap();
            account_kind = CpaAccountKind::Codex;
            model = "gpt-5.5";
            protocol = UpstreamProtocol::Responses;
        }
    }
    spec.bindings
        .retain(|binding| binding.account_kind == account_kind);
    spec.startup_timeout = Duration::from_secs(3);
    spec.control_timeout = Duration::from_secs(2);
    spec.shutdown_timeout = Duration::from_millis(100);
    fake_control.set_accounts(vec![snapshot(account_kind, 'a', model)]);
    let control = Arc::new(GatedControl {
        inner: fake_control,
        next_probe: StdMutex::default(),
    });
    let runtime = Arc::new(
        ManagedCpaRuntime::with_components(
            spec,
            Arc::clone(&template.catalog),
            Arc::clone(&template.locator),
            backend.clone(),
            control.clone(),
        )
        .unwrap(),
    );
    runtime.start().unwrap();
    let (connector, endpoint) = match account_kind {
        CpaAccountKind::Claude => ("connector.cpa.claude", "endpoint.cpa.claude"),
        CpaAccountKind::Codex => ("connector.cpa.codex", "endpoint.cpa.codex"),
    };
    let materialized = runtime.materialize_account(connector, endpoint).unwrap();
    runtime
        .apply_account_management(
            &materialized.account_subject,
            1,
            CpaSourceManagementState::Enabled,
        )
        .unwrap();
    let target = prepare_target(
        &runtime,
        ExactCpaAttemptRequest {
            credential_ref: &materialized.credential_ref,
            upstream_model_id: model,
            protocol,
        },
    )
    .unwrap();
    let capability = lease(&runtime, &target, &fresh_context()).unwrap().unwrap();
    let auth_path = match kind {
        SourceKind::NativeClaude => runtime.spec.auth_dir.join("hiroute-managed-claude.json"),
        SourceKind::NativeCodex => runtime.spec.auth_dir.join("hiroute-managed-codex.json"),
        SourceKind::ManagedCodex => source_path.clone(),
    };
    ActiveFixture {
        runtime,
        backend,
        control,
        profile,
        source_path,
        auth_path,
        account: materialized.account_subject,
        target,
        capability,
        _root: root,
    }
}

#[derive(Debug, Eq, PartialEq)]
enum LeaseOutcome {
    Issued,
    Empty,
    Rejected,
}

fn spawn_lease(
    fixture: &ActiveFixture,
    context: CpaRequestContext,
) -> (thread::JoinHandle<()>, mpsc::Receiver<LeaseOutcome>) {
    let runtime = Arc::clone(&fixture.runtime);
    let target = fixture.target.clone();
    let (send, receive) = mpsc::channel();
    let worker = thread::spawn(move || {
        let outcome = match lease(&runtime, &target, &context) {
            Ok(Some(_)) => LeaseOutcome::Issued,
            Ok(None) => LeaseOutcome::Empty,
            Err(_) => LeaseOutcome::Rejected,
        };
        let _ = send.send(outcome);
    });
    (worker, receive)
}

fn observe_and_disable(fixture: &ActiveFixture, reenable: bool) {
    let runtime = Arc::clone(&fixture.runtime);
    let account = fixture.account.clone();
    let (send, receive) = mpsc::channel();
    let control = thread::spawn(move || {
        assert!(matches!(runtime.health().unwrap(), CpaHealth::Ready { .. }));
        assert_eq!(runtime.last_exit(), None);
        runtime
            .apply_account_management(&account, 2, CpaSourceManagementState::Disabled)
            .unwrap();
        if reenable {
            runtime
                .apply_account_management(&account, 3, CpaSourceManagementState::Enabled)
                .unwrap();
        }
        let _ = send.send(());
    });
    receive
        .recv_timeout(PROMPT)
        .expect("health and management waited for slow request I/O");
    control.join().unwrap();
    assert_eq!(
        fixture
            .capability
            .apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
}

fn force_next_probe(fixture: &ActiveFixture, gate: &Arc<IoGate>) {
    // Expire the permitted health cache; the public contract does not probe every lease.
    fixture
        .runtime
        .inner
        .lock()
        .live
        .as_mut()
        .unwrap()
        .last_health = None;
    *fixture.control.next_probe.lock().unwrap() = Some(Arc::clone(gate));
}

fn assert_fresh_lease(fixture: &ActiveFixture) -> CpaDownstreamCredentialCapability {
    let capability = lease(&fixture.runtime, &fixture.target, &fresh_context())
        .unwrap()
        .unwrap();
    capability
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    capability
}

#[test]
fn profile_io_keeps_health_disable_and_capability_revocation_available() {
    let fixture = active_fixture(SourceKind::NativeClaude);
    let gate = Arc::new(IoGate::default());
    let _release = ReleaseOnDrop(Arc::clone(&gate));
    fixture
        .profile
        .arm("slow-access", &gate, false, "request-io-account-one");
    write_claude_source(&fixture.source_path, "slow-access");
    let auth_before = std::fs::read(&fixture.auth_path).unwrap();
    let (worker, done) = spawn_lease(&fixture, fresh_context());
    gate.wait_entered();
    observe_and_disable(&fixture, false);
    let epochs = fixture.runtime.epochs.current();
    gate.release();
    assert_eq!(done.recv_timeout(PROMPT).unwrap(), LeaseOutcome::Rejected);
    worker.join().unwrap();
    assert_eq!(std::fs::read(&fixture.auth_path).unwrap(), auth_before);
    assert_eq!(fixture.runtime.epochs.current(), epochs);
    assert_eq!(fixture.backend.spawn_count(), 1);
    fixture.runtime.shutdown().unwrap();
}

#[test]
fn late_profile_success_or_failure_cannot_revoke_a_new_enabled_revision() {
    for fail in [false, true] {
        let fixture = active_fixture(SourceKind::NativeClaude);
        let gate = Arc::new(IoGate::default());
        let _release = ReleaseOnDrop(Arc::clone(&gate));
        fixture
            .profile
            .arm("stale-access", &gate, fail, "obsolete-account-two");
        write_claude_source(&fixture.source_path, "stale-access");
        let auth_before = std::fs::read(&fixture.auth_path).unwrap();
        let (worker, done) = spawn_lease(&fixture, fresh_context());
        gate.wait_entered();
        observe_and_disable(&fixture, true);
        let epochs = fixture.runtime.epochs.current();
        let generation = fixture.runtime.admission.state.lock().auth_generation;
        let discoveries = fixture.control.inner.discoveries.load(Ordering::SeqCst);
        gate.release();
        assert_eq!(done.recv_timeout(PROMPT).unwrap(), LeaseOutcome::Rejected);
        worker.join().unwrap();
        assert!(
            !fixture
                .runtime
                .admission
                .state
                .lock()
                .subscription_execution_suspended
        );
        assert_eq!(
            fixture.runtime.admission.state.lock().auth_generation,
            generation
        );
        assert_eq!(fixture.runtime.epochs.current(), epochs);
        assert_eq!(std::fs::read(&fixture.auth_path).unwrap(), auth_before);
        assert_eq!(
            fixture.control.inner.discoveries.load(Ordering::SeqCst),
            discoveries
        );
        write_claude_source(&fixture.source_path, "newest-access");
        let native_before = std::fs::read(&fixture.source_path).unwrap();
        assert_fresh_lease(&fixture);
        assert!(
            fixture.control.inner.discoveries.load(Ordering::SeqCst) > discoveries,
            "a resumed source reused cached health instead of validating account routing"
        );
        assert_eq!(std::fs::read(&fixture.source_path).unwrap(), native_before);
        assert_eq!(fixture.backend.spawn_count(), 1);
        fixture.runtime.shutdown().unwrap();
    }
}

#[test]
fn source_rotation_after_preparation_never_fetches_profile_under_lifecycle_owner() {
    let fixture = active_fixture(SourceKind::NativeClaude);
    let gate = Arc::new(IoGate::default());
    let _release = ReleaseOnDrop(Arc::clone(&gate));
    force_next_probe(&fixture, &gate);
    let calls = fixture.profile.calls.load(Ordering::SeqCst);
    let auth_before = std::fs::read(&fixture.auth_path).unwrap();
    let epochs = fixture.runtime.epochs.current();
    let (worker, done) = spawn_lease(&fixture, fresh_context());
    gate.wait_entered();
    write_claude_source(&fixture.source_path, "changed-after-preparation");
    gate.release();
    assert_eq!(done.recv_timeout(PROMPT).unwrap(), LeaseOutcome::Rejected);
    worker.join().unwrap();
    assert_eq!(
        fixture.profile.calls.load(Ordering::SeqCst),
        calls,
        "a changed source triggered remote profile I/O under the lifecycle owner"
    );
    assert_eq!(std::fs::read(&fixture.auth_path).unwrap(), auth_before);
    assert_eq!(fixture.runtime.epochs.current(), epochs);
    assert!(
        !fixture
            .runtime
            .admission
            .state
            .lock()
            .subscription_execution_suspended
    );
    let fresh = assert_fresh_lease(&fixture);
    assert!(fresh.generation() > fixture.capability.generation());
    assert_eq!(fixture.profile.calls.load(Ordering::SeqCst), calls + 1);
    assert_eq!(
        fixture
            .capability
            .apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
    fixture.runtime.shutdown().unwrap();
}

#[test]
fn slow_probe_keeps_control_available_for_native_and_managed_sources() {
    for kind in [
        SourceKind::NativeClaude,
        SourceKind::NativeCodex,
        SourceKind::ManagedCodex,
    ] {
        let fixture = active_fixture(kind);
        let gate = Arc::new(IoGate::default());
        let _release = ReleaseOnDrop(Arc::clone(&gate));
        force_next_probe(&fixture, &gate);
        let auth_before = std::fs::read(&fixture.auth_path).unwrap();
        let (worker, done) = spawn_lease(&fixture, fresh_context());
        gate.wait_entered();
        observe_and_disable(&fixture, false);
        let epochs = fixture.runtime.epochs.current();
        gate.release();
        assert_eq!(done.recv_timeout(PROMPT).unwrap(), LeaseOutcome::Rejected);
        worker.join().unwrap();
        assert_eq!(std::fs::read(&fixture.auth_path).unwrap(), auth_before);
        assert_eq!(fixture.runtime.epochs.current(), epochs);
        let discoveries = fixture.control.inner.discoveries.load(Ordering::SeqCst);
        fixture
            .runtime
            .apply_account_management(&fixture.account, 3, CpaSourceManagementState::Enabled)
            .unwrap();
        assert_fresh_lease(&fixture);
        assert!(
            fixture.control.inner.discoveries.load(Ordering::SeqCst) > discoveries,
            "a resumed source skipped account validation"
        );
        assert_eq!(fixture.backend.spawn_count(), 1);
        fixture.runtime.shutdown().unwrap();
    }
}

#[test]
fn late_probe_error_cannot_suspend_a_new_enabled_revision() {
    for kind in [
        SourceKind::NativeClaude,
        SourceKind::NativeCodex,
        SourceKind::ManagedCodex,
    ] {
        let fixture = active_fixture(kind);
        let gate = Arc::new(IoGate::default());
        let _release = ReleaseOnDrop(Arc::clone(&gate));
        force_next_probe(&fixture, &gate);
        let auth_before = std::fs::read(&fixture.auth_path).unwrap();
        let (worker, done) = spawn_lease(&fixture, fresh_context());
        gate.wait_entered();
        observe_and_disable(&fixture, true);
        fixture.control.inner.set_fail_probes(true);
        let epochs = fixture.runtime.epochs.current();
        let generation = fixture.runtime.admission.state.lock().auth_generation;
        gate.release();
        assert_eq!(done.recv_timeout(PROMPT).unwrap(), LeaseOutcome::Rejected);
        worker.join().unwrap();
        assert!(
            !fixture
                .runtime
                .admission
                .state
                .lock()
                .subscription_execution_suspended
        );
        assert_eq!(
            fixture.runtime.admission.state.lock().auth_generation,
            generation
        );
        assert_eq!(fixture.runtime.epochs.current(), epochs);
        assert_eq!(std::fs::read(&fixture.auth_path).unwrap(), auth_before);
        fixture.control.inner.set_fail_probes(false);
        assert_fresh_lease(&fixture);
        assert_eq!(fixture.backend.spawn_count(), 1);
        fixture.runtime.shutdown().unwrap();
    }
}

#[test]
fn issuer_cancellation_prevents_auth_commit_without_waiter_poll() {
    for profile_io in [false, true] {
        let fixture = active_fixture(SourceKind::NativeClaude);
        let gate = Arc::new(IoGate::default());
        let _release = ReleaseOnDrop(Arc::clone(&gate));
        if profile_io {
            fixture.profile.arm(
                "issuer-cancelled-access",
                &gate,
                false,
                "request-io-account-one",
            );
        } else {
            force_next_probe(&fixture, &gate);
        }
        write_claude_source(&fixture.source_path, "issuer-cancelled-access");
        let auth_before = std::fs::read(&fixture.auth_path).unwrap();
        let epochs = fixture.runtime.epochs.current();
        let generation = fixture.runtime.admission.state.lock().auth_generation;
        let issuer_cancelled = Arc::new(AtomicBool::new(false));
        let source = Arc::clone(&issuer_cancelled);
        let context = fresh_context()
            .with_cancellation_check(Arc::new(move || source.load(Ordering::Acquire)));
        let (worker, done) = spawn_lease(&fixture, context);
        gate.wait_entered();
        // The issuer changes its own atomic source; no waiter poll, drop or cancel hook runs.
        issuer_cancelled.store(true, Ordering::Release);
        gate.release();
        assert_eq!(done.recv_timeout(PROMPT).unwrap(), LeaseOutcome::Rejected);
        worker.join().unwrap();
        assert_eq!(std::fs::read(&fixture.auth_path).unwrap(), auth_before);
        assert_eq!(fixture.runtime.epochs.current(), epochs);
        assert_eq!(
            fixture.runtime.admission.state.lock().auth_generation,
            generation
        );
        assert!(
            !fixture
                .runtime
                .admission
                .state
                .lock()
                .subscription_execution_suspended
        );
        write_claude_source(&fixture.source_path, "post-cancellation-access");
        assert_fresh_lease(&fixture);
        fixture.runtime.shutdown().unwrap();
    }
}

#[test]
fn request_deadline_rejects_late_profile_completion_without_auth_commit() {
    let fixture = active_fixture(SourceKind::NativeClaude);
    let gate = Arc::new(IoGate::default());
    let _release = ReleaseOnDrop(Arc::clone(&gate));
    fixture
        .profile
        .arm("deadline-access", &gate, false, "request-io-account-one");
    write_claude_source(&fixture.source_path, "deadline-access");
    let auth_before = std::fs::read(&fixture.auth_path).unwrap();
    let epochs = fixture.runtime.epochs.current();
    let context = CpaRequestContext::new(Instant::now() + Duration::from_millis(150));
    let (worker, done) = spawn_lease(&fixture, context);
    gate.wait_entered();
    assert!(
        matches!(
            done.recv_timeout(Duration::from_millis(200)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ),
        "the synchronous owner detached unfinished profile I/O"
    );
    assert_eq!(
        gate.state.lock().unwrap().1,
        0,
        "the controlled profile finished before its release"
    );
    gate.release();
    assert_eq!(done.recv_timeout(PROMPT).unwrap(), LeaseOutcome::Rejected);
    worker.join().unwrap();
    assert_eq!(std::fs::read(&fixture.auth_path).unwrap(), auth_before);
    assert_eq!(fixture.runtime.epochs.current(), epochs);
    assert!(
        !fixture
            .runtime
            .admission
            .state
            .lock()
            .subscription_execution_suspended
    );
    write_claude_source(&fixture.source_path, "post-deadline-access");
    assert_fresh_lease(&fixture);
    fixture.runtime.shutdown().unwrap();
}
