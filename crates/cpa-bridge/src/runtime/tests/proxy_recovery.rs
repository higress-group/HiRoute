use super::*;

#[test]
fn changed_or_unrecorded_proxy_policy_replaces_only_an_authenticated_orphan() {
    for legacy in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let backend = Arc::new(FakeBackend::default());
        let control = Arc::new(FakeControl::default());
        let first = fixture_runtime(&root, backend.clone(), control.clone(), 2)
            .with_proxy_environment([("HTTPS_PROXY".into(), "http://old-proxy:1187".into())]);
        let CpaHealth::Ready { pid, .. } = first.start().unwrap() else {
            panic!("not ready");
        };
        let owner_path = root.path().join("state/fixture-cpa/owner.lock/owner.json");
        let mut owner: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&owner_path).unwrap()).unwrap();
        owner["owner_pid"] = json!(u32::MAX - 1);
        if legacy {
            owner
                .as_object_mut()
                .unwrap()
                .remove("proxy_environment_sha256");
        }
        private_atomic_write(&owner_path, &serde_json::to_vec(&owner).unwrap()).unwrap();
        drop(first.inner.lock().live.as_mut().unwrap().auth_lease.take());
        std::mem::forget(first);

        // Empty is intentional: the shell removed all proxy variables.
        let second =
            fixture_runtime(&root, backend.clone(), control.clone(), 2).with_proxy_environment([]);
        control.set_fail_probes(true);
        assert!(matches!(
            second.start(),
            Err(CpaLifecycleError::ControlUnavailable)
        ));
        assert!(backend.pid_is_running(pid).unwrap());
        assert_eq!(backend.spawn_count(), 1);
        assert_eq!(backend.attach_count(), 0);

        control.set_fail_probes(false);
        let CpaHealth::Ready {
            pid: replacement,
            adopted,
            ..
        } = second.start().unwrap()
        else {
            panic!("not ready");
        };
        assert_ne!(pid, replacement);
        assert!(!adopted);
        assert!(!backend.pid_is_running(pid).unwrap());
        assert_eq!(backend.attach_count(), 1);
        assert_eq!(backend.spawn_count(), 2);
        let environments = backend.proxy_environments.lock();
        assert_eq!(environments[0].iter().count(), 1);
        assert_eq!(environments[1].iter().count(), 0);
        second.shutdown().unwrap();
    }
}
