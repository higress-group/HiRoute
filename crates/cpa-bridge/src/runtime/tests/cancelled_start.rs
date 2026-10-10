//! Cancellation cannot release a refresh writer's ownership until it really stops.
use super::*;
use std::sync::{Condvar as StdCondvar, Mutex as StdMutex, mpsc};
use std::thread;
use std::time::Instant;

#[derive(Default)]
struct ReadyBarrier {
    state: StdMutex<(bool, bool)>,
    changed: StdCondvar,
}

impl ReadyBarrier {
    fn ready(&self) {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.changed.notify_all();
        while !state.1 {
            state = self.changed.wait(state).unwrap();
        }
    }

    fn wait_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut state = self.state.lock().unwrap();
        while !state.0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "initial authenticated readiness was never reached"
            );
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }

    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.changed.notify_all();
    }
}

struct ReleaseOnDrop(Arc<ReadyBarrier>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct ReadyControl {
    control: Arc<FakeControl>,
    barrier: Arc<ReadyBarrier>,
    first: AtomicBool,
}

impl CpaControlPlane for ReadyControl {
    fn probe_ready(
        &self,
        address: SocketAddr,
        secrets: &InstanceSecrets,
        version: &str,
        timeout: Duration,
    ) -> Result<(), AccountDiscoveryError> {
        self.control
            .probe_ready(address, secrets, version, timeout)?;
        if self.first.swap(false, Ordering::SeqCst) {
            self.barrier.ready();
        }
        Ok(())
    }

    fn discover_and_pin(
        &self,
        address: SocketAddr,
        auth_dir: &Path,
        identities: &[ManagedAccountIdentity],
        secrets: &InstanceSecrets,
        version: &str,
        timeout: Duration,
        refresh: bool,
    ) -> Result<Vec<AccountSnapshotRecord>, AccountDiscoveryError> {
        self.control.discover_and_pin(
            address, auth_dir, identities, secrets, version, timeout, refresh,
        )
    }
}

struct FailingStopBackend {
    backend: Arc<FakeBackend>,
    stop_fails: Arc<AtomicBool>,
    stop_calls: Arc<AtomicUsize>,
}

struct FailingStopHandle {
    process: Box<dyn CpaProcessHandle>,
    stop_fails: Arc<AtomicBool>,
    stop_calls: Arc<AtomicUsize>,
}

impl CpaProcessBackend for FailingStopBackend {
    fn spawn(&self, launch: &CpaLaunch) -> Result<Box<dyn CpaProcessHandle>, CpaProcessError> {
        Ok(Box::new(FailingStopHandle {
            process: self.backend.spawn(launch)?,
            stop_fails: self.stop_fails.clone(),
            stop_calls: self.stop_calls.clone(),
        }))
    }
    fn attach_authenticated(&self, pid: u32) -> Result<Box<dyn CpaProcessHandle>, CpaProcessError> {
        Ok(Box::new(FailingStopHandle {
            process: self.backend.attach_authenticated(pid)?,
            stop_fails: self.stop_fails.clone(),
            stop_calls: self.stop_calls.clone(),
        }))
    }
    fn pid_is_running(&self, pid: u32) -> Result<bool, CpaProcessError> {
        self.backend.pid_is_running(pid)
    }
}

impl CpaProcessHandle for FailingStopHandle {
    fn pid(&self) -> u32 {
        self.process.pid()
    }
    fn try_exit(&mut self) -> Result<Option<CpaExit>, CpaProcessError> {
        self.process.try_exit()
    }
    fn shutdown(&mut self, timeout: Duration) -> Result<CpaExit, CpaProcessError> {
        self.stop_calls.fetch_add(1, Ordering::SeqCst);
        if self.stop_fails.load(Ordering::SeqCst) {
            return Err(CpaProcessError::ShutdownTimeout);
        }
        self.process.shutdown(timeout)
    }
}

#[test]
fn cancelled_ready_start_retains_owner_and_auth_lock_when_stop_fails_until_retry() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let fake_control = Arc::new(FakeControl::default());
    let template = fixture_runtime(&root, backend.clone(), fake_control.clone(), 2)
        .fork_managed_oauth(
            "cancelled-start".into(),
            root.path().join("session/auth"),
            CpaAccountKind::Codex,
        )
        .unwrap();
    ensure_private_dir(&template.spec.auth_dir).unwrap();
    let credential_path = template.spec.auth_dir.join("credential.json");
    private_atomic_write(&credential_path, &serde_json::to_vec(&json!({
        "type": "codex", "access_token": "fixture-access", "refresh_token": "fixture-managed-refresh",
        "account_id": "cancelled-start-account"
    })).unwrap()).unwrap();
    let evidence = template.managed_oauth_source().unwrap().inspect().unwrap();
    let identity = evidence.identity();
    let mut account = snapshot(CpaAccountKind::Codex, 'a', "gpt-5.5");
    account.account_digest = identity.account_digest.clone();
    account.stock_file_name = identity.stock_file_name.clone();
    account.prefix = identity.prefix().unwrap();
    fake_control.set_accounts(vec![account.clone()]);
    let layout = template.prepare_layout().unwrap();
    save_account_state(&layout.accounts_path, &[account.clone()]).unwrap();
    let materialized = account.materialize(&template.spec.bindings[0]).unwrap();
    let barrier = Arc::new(ReadyBarrier::default());
    let _release = ReleaseOnDrop(barrier.clone());
    let control = Arc::new(ReadyControl {
        control: fake_control,
        barrier: barrier.clone(),
        first: AtomicBool::new(true),
    });
    let stop_fails = Arc::new(AtomicBool::new(true));
    let stop_calls = Arc::new(AtomicUsize::new(0));
    let failing = Arc::new(FailingStopBackend {
        backend: backend.clone(),
        stop_fails: stop_fails.clone(),
        stop_calls: stop_calls.clone(),
    });
    let mut spec = template.spec.clone();
    spec.startup_timeout = Duration::from_secs(3);
    spec.control_timeout = Duration::from_secs(2);
    let runtime = Arc::new(
        ManagedCpaRuntime::with_components(
            spec,
            template.catalog.clone(),
            template.locator.clone(),
            failing,
            control,
        )
        .unwrap(),
    );
    runtime
        .apply_account_management(
            &evidence.account_ref(),
            1,
            CpaSourceManagementState::Enabled,
        )
        .unwrap();
    let epochs = runtime.epochs.current();
    let context = crate::CpaRequestContext::new(Instant::now() + Duration::from_secs(5));
    let worker_context = context.clone();
    let worker_runtime = runtime.clone();
    let (completed, completion) = mpsc::channel();
    let worker = thread::spawn(move || {
        completed
            .send(worker_context.run(|| worker_runtime.start()))
            .unwrap();
    });
    barrier.wait_ready();
    let owner_before = std::fs::read(layout.lock_dir.join("owner.json")).unwrap();
    let record = crate::owner::read_record(&layout.lock_dir).unwrap();
    context.cancel();
    barrier.release();
    assert!(matches!(
        completion.recv_timeout(Duration::from_millis(600)).unwrap(),
        Err(CpaLifecycleError::OperationCancelled)
    ));
    worker.join().unwrap();
    assert_eq!(stop_calls.load(Ordering::SeqCst), 1);
    assert!(backend.pid_is_running(record.cpa_pid).unwrap());
    assert!(
        layout.lock_dir.join("owner.json").exists(),
        "a live writer lost its owner record after failed cancellation cleanup"
    );
    assert_eq!(
        std::fs::read(layout.lock_dir.join("owner.json")).unwrap(),
        owner_before
    );
    assert!(
        runtime
            .inner
            .lock()
            .live
            .as_ref()
            .is_some_and(|live| live.auth_lease.is_some())
    );
    assert!(matches!(
        crate::borrowed_codex::ManagedAuthLease::acquire_subscription(
            &runtime.spec.auth_dir,
            None,
            None,
            Some(CpaAccountKind::Codex),
            None
        ),
        Err(CpaLifecycleError::BorrowedCodexAuthAlreadyLeased)
    ));
    assert_eq!(runtime.epochs.current(), epochs);
    assert!(matches!(
        runtime.health().unwrap(),
        CpaHealth::Unhealthy { .. }
    ));
    assert!(
        runtime
            .lease_downstream_capability_scoped(
                ExactCpaCredentialRequest {
                    credential_id: materialized.credential_ref.credential_id(),
                    connector_id: "connector.cpa.codex",
                    upstream_model_id: "gpt-5.5",
                    protocol: UpstreamProtocol::Responses,
                    address: record.address,
                    request_path: "/v1/responses",
                    native_transport_model: "hiroute-codex-current/gpt-5.5",
                    runtime_epoch: epochs.0,
                    target_epoch: epochs.1,
                    excluded_key_ids: &[],
                },
                &context
            )
            .is_err()
    );
    let _ = runtime.start();
    assert_eq!(
        backend.spawn_count(),
        1,
        "retrying a retained live writer launched another process"
    );
    assert!(
        runtime
            .ensure_saved_runtime_ready(&evidence.account_ref(), 1)
            .is_err(),
        "failed-start cleanup was promoted to saved-source readiness"
    );
    assert_eq!(backend.spawn_count(), 1);
    assert!(backend.pid_is_running(record.cpa_pid).unwrap());
    assert!(matches!(
        runtime.health().unwrap(),
        CpaHealth::Unhealthy { .. }
    ));

    stop_fails.store(false, Ordering::SeqCst);
    let recovered = runtime
        .ensure_saved_runtime_ready(&evidence.account_ref(), 1)
        .expect("the same saved revision could not retry cleanup and normal admission");
    assert!(matches!(recovered, CpaHealth::Ready { .. }));
    assert!(!backend.pid_is_running(record.cpa_pid).unwrap());
    assert_eq!(backend.spawn_count(), 2);
    let recovered_epochs = runtime.epochs.current();
    assert_ne!(recovered_epochs, epochs);
    assert!(recovered_epochs.0 > 0 && recovered_epochs.1 > 0);
    let current = runtime
        .materialize_account("connector.cpa.codex", "endpoint.cpa.codex")
        .unwrap();
    let target = prepare_target(
        &runtime,
        ExactCpaAttemptRequest {
            credential_ref: &current.credential_ref,
            upstream_model_id: "gpt-5.5",
            protocol: UpstreamProtocol::Responses,
        },
    )
    .unwrap();
    let capability = runtime
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
        .unwrap()
        .unwrap();
    capability
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    assert_eq!(backend.spawn_count(), 2);
    runtime.shutdown().unwrap();
    assert!(!backend.pid_is_running(record.cpa_pid).unwrap());
    assert!(!layout.lock_dir.exists());
    assert!(runtime.inner.lock().live.is_none());
    assert!(
        crate::borrowed_codex::ManagedAuthLease::acquire_subscription(
            &runtime.spec.auth_dir,
            None,
            None,
            Some(CpaAccountKind::Codex),
            None
        )
        .is_ok()
    );
}
