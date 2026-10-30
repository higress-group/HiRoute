use super::*;
use crate::LocalStorageSet;

fn revisions() -> RevisionSetV1 {
    RevisionSetV1 {
        target: 0,
        dependencies: Default::default(),
    }
}
fn registration(token: &str, expires: i64) -> ApplyCapabilityRegistrationV1 {
    ApplyCapabilityRegistrationV1::from_protected_launcher(
        token.into(),
        "interactive-user",
        WorkspaceId::default(),
        "ListSessionsV2",
        CanonicalDigest::of_bytes(b"exact query"),
        revisions(),
        expires,
    )
    .unwrap()
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
fn busy_policy(stores: &LocalStorageSet) -> u64 {
    stores
        .control()
        .connection
        .borrow()
        .pragma_query_value(None, "busy_timeout", |r| r.get(0))
        .unwrap()
}
fn grants(stores: &LocalStorageSet) -> u64 {
    stores
        .control()
        .connection
        .borrow()
        .query_row("SELECT COUNT(*) FROM apply_capabilities", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn busy_writer_refuses_without_a_row_and_restores_the_connection_policy() {
    let directory = crate::test_tempdir().unwrap();
    let root = directory.path().join("storage");
    let stores = LocalStorageSet::open_for_daemon_startup(&root).unwrap();
    let original_policy = busy_policy(&stores);
    let blocker = Connection::open(root.join("live/control.db")).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(
        stores
            .apply_capability_registrar()
            .try_register(registration(&"expired".repeat(8), now() - 1))
            .unwrap_err()
            .code,
        PortErrorCode::PermissionDenied
    );
    let started = std::time::Instant::now();
    let token = "a".repeat(64);
    let result = stores
        .apply_capability_registrar()
        .try_register(registration(&token, now() + 60));
    let elapsed = started.elapsed();
    blocker.execute_batch("ROLLBACK").unwrap();
    assert_eq!(result.unwrap(), ApplyCapabilityRegistrationOutcome::Busy);
    assert!(
        elapsed < Duration::from_millis(500),
        "writer admission waited {elapsed:?}"
    );
    assert_eq!(grants(&stores), 0);
    assert_eq!(busy_policy(&stores), original_policy);
    assert_eq!(
        stores
            .apply_capability_registrar()
            .try_register(registration(&token, now() + 60))
            .unwrap(),
        ApplyCapabilityRegistrationOutcome::Registered
    );
    assert_eq!(grants(&stores), 1);
    assert_eq!(busy_policy(&stores), original_policy);
}

#[test]
fn accepted_grant_is_durable_and_retains_all_exact_authorization_checks() {
    let directory = crate::test_tempdir().unwrap();
    let root = directory.path().join("storage");
    let stores = LocalStorageSet::open_for_daemon_startup(&root).unwrap();
    let synchronous: i64 = stores
        .control()
        .connection
        .borrow()
        .pragma_query_value(None, "synchronous", |r| r.get(0))
        .unwrap();
    let token = "b".repeat(64);
    assert_eq!(
        stores
            .apply_capability_registrar()
            .try_register(registration(&token, now() + 60))
            .unwrap(),
        ApplyCapabilityRegistrationOutcome::Registered
    );
    assert_eq!(
        stores
            .control()
            .connection
            .borrow()
            .pragma_query_value(None, "synchronous", |r| r.get::<_, i64>(0))
            .unwrap(),
        synchronous
    );
    drop(stores);
    let stores = LocalStorageSet::open_for_daemon_startup(&root).unwrap();
    let capability = ProtectedApplyCapability::new(token).unwrap();
    let digest = CanonicalDigest::of_bytes(b"exact query");
    let verify =
        |principal: &str, operation: &str, digest: &CanonicalDigest, revision: &RevisionSetV1| {
            stores.control().verify_apply_authorization(
                &capability,
                &WorkspaceId::default(),
                principal,
                operation,
                digest,
                revision,
            )
        };
    assert!(verify("interactive-user", "ListSessionsV2", &digest, &revisions()).is_ok());
    assert!(verify("desktop", "ListSessionsV2", &digest, &revisions()).is_err());
    assert!(
        verify(
            "interactive-user",
            "ApplyComputeConnection",
            &digest,
            &revisions()
        )
        .is_err()
    );
    assert!(
        verify(
            "interactive-user",
            "ListSessionsV2",
            &CanonicalDigest::of_bytes(b"other"),
            &revisions()
        )
        .is_err()
    );
    assert!(
        verify(
            "interactive-user",
            "ListSessionsV2",
            &digest,
            &RevisionSetV1 {
                target: 1,
                dependencies: Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn rejected_expiry_and_duplicate_are_errors_not_busy_or_success() {
    let directory = crate::test_tempdir().unwrap();
    let stores =
        LocalStorageSet::open_for_daemon_startup(directory.path().join("storage")).unwrap();
    let original_policy = busy_policy(&stores);
    for expires in [now() - 1, i64::MAX] {
        assert_eq!(
            stores
                .apply_capability_registrar()
                .try_register(registration(&"c".repeat(64), expires))
                .unwrap_err()
                .code,
            PortErrorCode::PermissionDenied
        );
        assert_eq!(grants(&stores), 0);
        assert_eq!(busy_policy(&stores), original_policy);
    }
    let token = "d".repeat(64);
    assert_eq!(
        stores
            .apply_capability_registrar()
            .try_register(registration(&token, now() + 60))
            .unwrap(),
        ApplyCapabilityRegistrationOutcome::Registered
    );
    assert!(
        stores
            .apply_capability_registrar()
            .try_register(registration(&token, now() + 60))
            .is_err()
    );
    assert_eq!(grants(&stores), 1);
    assert_eq!(busy_policy(&stores), original_policy);
}

#[test]
fn launcher_registrar_retains_startup_ownership_and_exact_durable_registration() {
    let directory = crate::test_tempdir().unwrap();
    let root = directory.path().join("storage");
    let stores = LocalStorageSet::open_for_daemon_startup(&root).unwrap();
    let registrar = stores.open_launcher_capability_registrar().unwrap();
    assert_eq!(
        registrar
            .connection
            .lock()
            .unwrap()
            .pragma_query_value(None, "synchronous", |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    let blocker = Connection::open(root.join("live/control.db")).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let token = "e".repeat(64);
    let started = std::time::Instant::now();
    let result = registrar.try_register(registration(&token, now() + 60));
    blocker.execute_batch("ROLLBACK").unwrap();
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(result.unwrap(), ApplyCapabilityRegistrationOutcome::Busy);
    assert_eq!(grants(&stores), 0);
    let guard = registrar.connection.lock().unwrap();
    assert_eq!(
        registrar
            .try_register(registration(&token, now() + 60))
            .unwrap(),
        ApplyCapabilityRegistrationOutcome::Busy
    );
    drop(guard);
    assert_eq!(
        registrar
            .try_register(registration(&token, now() - 1))
            .unwrap_err()
            .code,
        PortErrorCode::PermissionDenied
    );
    assert_eq!(
        registrar
            .try_register(registration(&token, now() + 60))
            .unwrap(),
        ApplyCapabilityRegistrationOutcome::Registered
    );
    assert!(
        registrar
            .try_register(registration(&token, now() + 60))
            .is_err()
    );
    assert_eq!(grants(&stores), 1);
    drop(stores);
    let refused = LocalStorageSet::open_for_daemon_startup(&root)
        .err()
        .expect("the registrar must retain startup exclusion");
    assert_eq!(
        refused.recovery().cause,
        hiroute_host_runtime::StorageRecoveryCause::Locked
    );
    drop(registrar);
    let stores = LocalStorageSet::open_for_daemon_startup(&root).unwrap();
    let capability = ProtectedApplyCapability::new(token).unwrap();
    assert!(
        stores
            .control()
            .verify_apply_authorization(
                &capability,
                &WorkspaceId::default(),
                "interactive-user",
                "ListSessionsV2",
                &CanonicalDigest::of_bytes(b"exact query"),
                &revisions()
            )
            .is_ok()
    );
    assert!(
        stores
            .control()
            .verify_apply_authorization(
                &capability,
                &WorkspaceId::default(),
                "interactive-user",
                "ListSessionsV2",
                &CanonicalDigest::of_bytes(b"wrong query"),
                &revisions()
            )
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn launcher_registrar_refuses_missing_or_replaced_control_without_creating_or_changing_it() {
    let directory = crate::test_tempdir().unwrap();
    let root = directory.path().join("storage");
    let stores = LocalStorageSet::open_for_daemon_startup(&root).unwrap();
    let path = root.join("live/control.db");
    let original = root.join("live/control.retained");
    std::fs::rename(&path, &original).unwrap();
    assert!(stores.open_launcher_capability_registrar().is_err());
    assert!(!path.exists(), "the registrar must never create a database");
    std::fs::write(&path, b"foreign-file-must-not-change").unwrap();
    assert!(stores.open_launcher_capability_registrar().is_err());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"foreign-file-must-not-change"
    );
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&original, &path).unwrap();
    let registrar = stores.open_launcher_capability_registrar().unwrap();
    assert_eq!(
        registrar
            .try_register(registration(&"f".repeat(64), now() + 60))
            .unwrap(),
        ApplyCapabilityRegistrationOutcome::Registered
    );
}
