//! Orphan ownership rollback is cleanup, independent of the failed caller's ticket.
use super::*;

struct CancellingControl {
    context: crate::CpaRequestContext,
    owner_path: PathBuf,
}

impl CpaControlPlane for CancellingControl {
    fn probe_ready(
        &self,
        _address: SocketAddr,
        _secrets: &InstanceSecrets,
        _version: &str,
        _timeout: Duration,
    ) -> Result<(), AccountDiscoveryError> {
        let transferred = crate::owner::read_record(self.owner_path.parent().unwrap()).unwrap();
        assert_eq!(transferred.owner_pid, std::process::id());
        self.context.cancel();
        Err(AccountDiscoveryError::NotReady)
    }

    fn discover_and_pin(
        &self,
        _address: SocketAddr,
        _auth_dir: &Path,
        _identities: &[ManagedAccountIdentity],
        _secrets: &InstanceSecrets,
        _version: &str,
        _timeout: Duration,
        _refresh: bool,
    ) -> Result<Vec<AccountSnapshotRecord>, AccountDiscoveryError> {
        panic!("a rejected orphan must never discover accounts")
    }
}

fn runtime(
    root: &tempfile::TempDir,
    backend: Arc<FakeBackend>,
    control: Arc<FakeControl>,
    managed: bool,
) -> ManagedCpaRuntime {
    let mut runtime = fixture_runtime(root, backend, control, 2);
    omit_fixture_codex_version(&mut runtime, root.path());
    runtime.spec.startup_timeout = Duration::from_secs(3);
    runtime.spec.control_timeout = Duration::from_secs(2);
    runtime.spec.shutdown_timeout = Duration::from_secs(2);
    if managed {
        runtime
            .fork_managed_oauth(
                "rollback".into(),
                root.path().join("managed/auth"),
                CpaAccountKind::Codex,
            )
            .unwrap()
    } else {
        runtime
    }
}

#[test]
fn cancelled_orphan_authentication_restores_owner_before_retry() {
    if isolated_owner_recovery_case(
        "runtime::tests::owner_rollback::cancelled_orphan_authentication_restores_owner_before_retry",
    ) {
        return;
    }
    for (managed, stop_only) in [(false, false), (true, false), (true, true)] {
        let root = tempfile::tempdir().unwrap();
        let backend = Arc::new(FakeBackend::default());
        let control = Arc::new(FakeControl::default());
        let first = runtime(&root, backend.clone(), control.clone(), managed);
        let CpaHealth::Ready { pid, .. } = first.start().unwrap() else {
            panic!("initial fixture did not start")
        };
        let owner_path = first
            .spec
            .state_root
            .join(&first.spec.instance_id)
            .join("owner.lock/owner.json");
        let mut owner: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&owner_path).unwrap()).unwrap();
        owner["owner_pid"] = json!(u32::MAX - 1);
        private_atomic_write(&owner_path, &serde_json::to_vec(&owner).unwrap()).unwrap();
        let original = crate::owner::read_record(owner_path.parent().unwrap()).unwrap();
        drop(first.inner.lock().live.as_mut().unwrap().auth_lease.take());
        std::mem::forget(first);

        let context = crate::CpaRequestContext::new(Instant::now() + Duration::from_secs(10));
        let mut second = runtime(&root, backend.clone(), control.clone(), managed);
        second.control = Arc::new(CancellingControl {
            context: context.clone(),
            owner_path: owner_path.clone(),
        });
        let outcome = context.run(|| {
            if stop_only {
                second.stop_existing_managed_process()
            } else {
                second.start().map(|_| ())
            }
        });
        assert_eq!(
            crate::owner::read_record(owner_path.parent().unwrap()).unwrap(),
            original,
            "cancelled ownership rollback failed: managed={managed}, stop_only={stop_only}, outcome={outcome:?}"
        );
        assert!(matches!(outcome, Err(CpaLifecycleError::ControlUnavailable)));
        assert!(context.ensure_active().is_err(), "rollback revived its caller");
        assert!(backend.pid_is_running(pid).unwrap());
        assert_eq!(backend.spawn_count(), 1);
        assert_eq!(backend.attach_count(), 0);
        assert_eq!(control.discoveries.load(Ordering::SeqCst), 0);
        assert_eq!(second.epochs.current(), (0, 0));
        assert!(!matches!(second.health().unwrap(), CpaHealth::Ready { .. }));

        // A new authorized attempt can authenticate the same live orphan. Failed
        // cancellation must not strand it under this still-running owner's PID.
        second.control = control;
        if stop_only {
            second.stop_existing_managed_process().unwrap();
        } else {
            assert!(matches!(
                second.start().unwrap(),
                CpaHealth::Ready { adopted: true, .. }
            ));
            second.shutdown().unwrap();
        }
        assert_eq!(backend.spawn_count(), 1);
        assert_eq!(backend.attach_count(), 1);
        assert!(!backend.pid_is_running(pid).unwrap());
        assert!(!owner_path.exists());
    }
}
