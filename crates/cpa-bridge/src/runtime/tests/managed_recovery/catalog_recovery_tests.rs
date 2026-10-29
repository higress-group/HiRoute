use super::*;

struct EmptyCatalogControl {
    inner: Arc<FakeControl>,
    empty: AtomicBool,
    transient: bool,
    recoverable: bool,
    calls: Mutex<Vec<(bool, Duration)>>,
}

impl CpaControlPlane for EmptyCatalogControl {
    fn probe_ready(
        &self,
        address: SocketAddr,
        secrets: &InstanceSecrets,
        version: &str,
        timeout: Duration,
    ) -> Result<(), AccountDiscoveryError> {
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
        refresh: bool,
        diagnostics: &hiroute_diagnostics::context::DiagnosticContext,
    ) -> Result<Vec<AccountSnapshotRecord>, AccountDiscoveryError> {
        self.calls.lock().push((refresh, timeout));
        if self.empty.load(Ordering::SeqCst) {
            if !self.transient {
                return Err(AccountDiscoveryError::PinNotApplied);
            }
            // Healthy upstream alone does not alter CPA's empty registry. An
            // actual registration refresh is required, just as after startup.
            if !refresh || !self.recoverable {
                return Err(AccountDiscoveryError::TransientModelCatalog { process_id: 123 });
            }
            self.empty.store(false, Ordering::SeqCst);
        }
        self.inner.discover_and_pin(
            address,
            auth_dir,
            identities,
            secrets,
            version,
            timeout,
            refresh,
            diagnostics,
        )
    }
}

#[test]
fn saved_catalog_recovers_without_check_while_passive_reads_do_not_refresh() {
    if isolated_owner_recovery_case(
        "runtime::tests::managed_recovery::catalog_recovery_tests::saved_catalog_recovers_without_check_while_passive_reads_do_not_refresh",
    ) {
        return;
    }
    for managed in [false, true] {
        for (transient, recoverable) in [(true, true), (true, false), (false, false)] {
            let root = crate::private_test_support::tempdir().unwrap();
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
            first.shutdown().unwrap();
            drop(first);

            let empty = Arc::new(EmptyCatalogControl {
                inner: control.clone(),
                empty: AtomicBool::new(true),
                transient,
                recoverable,
                calls: Mutex::new(Vec::new()),
            });
            let mut restored = make_runtime();
            restored.control = empty.clone();
            restored
                .apply_account_management(&account, 7, CpaSourceManagementState::Enabled)
                .unwrap();
            assert!(restored.discover_materializations(None).is_err());
            assert!(empty.calls.lock().iter().all(|(refresh, _)| !refresh));
            let result = restored.ensure_saved_runtime_ready(&account, 7);
            assert_eq!(
                result.is_ok(),
                recoverable,
                "managed={managed}, transient={transient}, result={result:?}"
            );
            let calls = empty.calls.lock().clone();
            assert_eq!(
                calls.iter().filter(|(refresh, _)| *refresh).count(),
                usize::from(transient)
            );
            assert!(
                calls
                    .iter()
                    .all(|(_, timeout)| *timeout <= restored.spec.control_timeout)
            );
            if recoverable {
                let target = saved_managed_target(&restored);
                assert!(lease_saved_target(&restored, &target).unwrap().is_some());
            } else {
                // Same saved-source maintenance admission, as on the daemon's
                // next cycle. A failed retry cannot become an immediate loop.
                restored
                    .apply_account_management(&account, 7, CpaSourceManagementState::Enabled)
                    .unwrap();
                assert!(restored.ensure_saved_runtime_ready(&account, 7).is_err());
                assert_eq!(
                    empty
                        .calls
                        .lock()
                        .iter()
                        .filter(|(refresh, _)| *refresh)
                        .count(),
                    usize::from(transient)
                );
            }
            restored
                .apply_account_management(&account, 8, CpaSourceManagementState::Disabled)
                .unwrap();
            let before = empty.calls.lock().len();
            assert!(restored.ensure_saved_runtime_ready(&account, 8).is_err());
            assert_eq!(empty.calls.lock().len(), before);
            restored.shutdown().unwrap();
        }
    }
}
