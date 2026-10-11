use super::*;

#[cfg(target_os = "macos")]
fn assert_keychain_waiter_budget(test_name: &str, cancel_waiter: bool) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Instant;

    const CHILD: &str = "HIROUTE_KEYCHAIN_LOCK_BUDGET_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }

    // Hold only the process-policy mutex: no Keychain item or credential is read.
    let holder = KEYCHAIN_INTERACTION.lock();
    let reads = Arc::new(AtomicUsize::new(0));
    let worker_reads = Arc::clone(&reads);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let budget = if cancel_waiter {
            Duration::from_secs(10)
        } else {
            Duration::from_millis(100)
        };
        let request = crate::CpaRequestContext::new(Instant::now() + budget);
        request.run(|| {
            request.ensure_active().unwrap();
            entered_tx.send(request.clone()).unwrap();
            let result = with_keychain_interaction(false, || {
                worker_reads.fetch_add(1, Ordering::SeqCst);
                Ok(())
            });
            finished_tx.send(result).unwrap();
        });
    });
    let request = entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    if cancel_waiter {
        request.cancel();
    }

    let outcome = finished_rx.recv_timeout(Duration::from_secs(2));
    let returned_before_release = outcome.is_ok();
    let reads_before_release = reads.load(Ordering::SeqCst);
    assert!(KEYCHAIN_INTERACTION.try_lock().is_none());
    // Release and join even on the old blocking implementation, then assert the
    // original outcome. A regression must fail without stranding its waiter.
    drop(holder);
    let result =
        outcome.unwrap_or_else(|_| finished_rx.recv_timeout(Duration::from_secs(2)).unwrap());
    waiter.join().unwrap();

    assert!(
        returned_before_release,
        "an inactive waiter must return while the holder still owns the lock"
    );
    assert!(matches!(result, Err(CpaLifecycleError::OperationCancelled)));
    assert_eq!(reads_before_release, 0);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[cfg(target_os = "macos")]
#[test]
fn keychain_interaction_cancelled_waiter_returns_before_holder_release() {
    assert_keychain_waiter_budget(
        "borrowed_claude::tests::keychain_interaction_cancelled_waiter_returns_before_holder_release",
        true,
    );
}

#[cfg(target_os = "macos")]
#[test]
fn keychain_interaction_expired_waiter_returns_before_holder_release() {
    assert_keychain_waiter_budget(
        "borrowed_claude::tests::keychain_interaction_expired_waiter_returns_before_holder_release",
        false,
    );
}

#[cfg(target_os = "macos")]
#[test]
fn keychain_interaction_restores_process_policy() {
    const CHILD: &str = "HIROUTE_KEYCHAIN_INTERACTION_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "borrowed_claude::tests::keychain_interaction_restores_process_policy",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    use security_framework::os::macos::keychain::SecKeychain;
    let allowed = || SecKeychain::user_interaction_allowed().unwrap();
    assert!(allowed(), "isolated test process starts with UI permitted");
    for fail in [false, true] {
        let result = with_keychain_interaction(false, || {
            assert!(!allowed());
            if fail {
                Err(CpaLifecycleError::BorrowedClaudeAuthUnavailable)
            } else {
                Ok(())
            }
        });
        assert_eq!(result.is_err(), fail);
        assert!(
            allowed(),
            "both success and error restore the original flag"
        );
    }
    let disabled = SecKeychain::disable_user_interaction().unwrap();
    for explicit_check in [false, true] {
        with_keychain_interaction(explicit_check, || {
            assert!(!allowed());
            Ok(())
        })
        .unwrap();
        assert!(!allowed(), "never enable an externally disabled UI policy");
    }
    drop(disabled);
    assert!(allowed());

    // An interactive caller cannot observe the temporary suppression of another read.
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let first = std::thread::spawn(move || {
        with_keychain_interaction(false, || {
            assert!(!SecKeychain::user_interaction_allowed().unwrap());
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
        .unwrap();
    });
    entered_rx.recv().unwrap();
    assert!(KEYCHAIN_INTERACTION.try_lock().is_none());
    let second = std::thread::spawn(|| {
        with_keychain_interaction(true, || {
            assert!(SecKeychain::user_interaction_allowed().unwrap());
            Ok(())
        })
        .unwrap();
    });
    release_tx.send(()).unwrap();
    first.join().unwrap();
    second.join().unwrap();
    assert!(allowed());
}

fn source(root: &Path, token: &str, account: &str) -> BorrowedClaudeAuthSpec {
    ensure_private_dir(root).unwrap();
    let path = root.join(".credentials.json");
    write_native(&path, token, u64::MAX);
    let spec = BorrowedClaudeAuthSpec::new(ClaudeSubscriptionLocation::File(path));
    spec.seed_identity(token, account);
    spec
}
fn write_native(path: &Path, token: &str, expiry: u64) {
    private_atomic_write(
        path,
        &serde_json::to_vec(&serde_json::json!({"claudeAiOauth": {
            "accessToken": token, "refreshToken": "NATIVE_REFRESH_MUST_NEVER_REACH_CPA",
            "expiresAt": expiry, "scopes": ["user:profile", "user:inference"]
        }}))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn native_refresh_stays_untouched_and_rotation_preserves_account_binding() {
    let root = tempfile::tempdir().unwrap();
    let auth = tempfile::tempdir().unwrap();
    ensure_private_dir(auth.path()).unwrap();
    let spec = source(root.path(), "access-first", "account-one");
    let original = fs::read(spec.source_path()).unwrap();
    let before = spec.inspect().unwrap();
    let mut lease = BorrowedClaudeLease::acquire(auth.path(), &spec, Some(&before)).unwrap();
    let first = lease.refresh(Some(&before), None).unwrap();
    let cpa = fs::read(auth.path().join(FILE_NAME)).unwrap();
    assert!(!String::from_utf8_lossy(&cpa).contains("refresh"));
    assert_eq!(fs::read(spec.source_path()).unwrap(), original);
    write_native(spec.source_path(), "access-second", u64::MAX);
    spec.seed_identity("access-second", "account-one");
    let after = spec.inspect().unwrap();
    assert_eq!(before.account_ref(), after.account_ref());
    assert_eq!(
        before.binding_evidence_digest(),
        after.binding_evidence_digest()
    );
    assert_ne!(before.evidence_digest(), after.evidence_digest());
    let second = lease.refresh(None, Some(&first.account_digest)).unwrap();
    assert!(second.generation > first.generation);
    assert_eq!(first.prefix().unwrap(), second.prefix().unwrap());
    assert!(
        !String::from_utf8_lossy(&fs::read(auth.path().join(FILE_NAME)).unwrap())
            .contains("refresh")
    );
}

#[test]
fn switched_account_is_rejected_before_replacing_cpa_credentials() {
    let root = tempfile::tempdir().unwrap();
    let auth = tempfile::tempdir().unwrap();
    ensure_private_dir(auth.path()).unwrap();
    let spec = source(root.path(), "access-first", "account-one");
    let evidence = spec.inspect().unwrap();
    let mut lease = BorrowedClaudeLease::acquire(auth.path(), &spec, Some(&evidence)).unwrap();
    let previous = fs::read(auth.path().join(FILE_NAME)).unwrap();
    write_native(spec.source_path(), "other-account-token", u64::MAX);
    spec.seed_identity("other-account-token", "account-two");
    assert!(matches!(
        lease.refresh(None, None),
        Err(CpaLifecycleError::BorrowedClaudeAuthSourceChanged)
    ));
    assert_eq!(fs::read(auth.path().join(FILE_NAME)).unwrap(), previous);
    // A fresh approved check can bind the new account; passive refresh cannot.
    let approved = spec.inspect().unwrap();
    let replacement = lease.refresh(Some(&approved), None).unwrap();
    assert_eq!(
        format!("account/cpa/{}", replacement.account_digest),
        approved.account_ref()
    );
    assert_ne!(fs::read(auth.path().join(FILE_NAME)).unwrap(), previous);
}

#[test]
fn reacquired_lease_keeps_persisted_account_until_explicit_recheck() {
    if crate::borrowed_codex::tests::isolated_lock_test(
        "borrowed_claude::tests::reacquired_lease_keeps_persisted_account_until_explicit_recheck",
    ) {
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let auth = tempfile::tempdir().unwrap();
    ensure_private_dir(auth.path()).unwrap();
    let spec = source(root.path(), "access-first", "account-one");
    let original = spec.inspect().unwrap();
    drop(BorrowedClaudeLease::acquire(auth.path(), &spec, Some(&original)).unwrap());
    // Reopening after native token refresh remains valid for the same account.
    write_native(spec.source_path(), "rotated-access", u64::MAX);
    spec.seed_identity("rotated-access", "account-one");
    drop(BorrowedClaudeLease::acquire(auth.path(), &spec, None).unwrap());
    let previous = fs::read(auth.path().join(FILE_NAME)).unwrap();
    write_native(spec.source_path(), "new-account-access", u64::MAX);
    spec.seed_identity("new-account-access", "account-two");
    assert!(matches!(
        BorrowedClaudeLease::acquire(auth.path(), &spec, None),
        Err(CpaLifecycleError::BorrowedClaudeAuthSourceChanged)
    ));
    assert_eq!(fs::read(auth.path().join(FILE_NAME)).unwrap(), previous);
    assert!(matches!(
        BorrowedClaudeLease::acquire(auth.path(), &spec, Some(&original)),
        Err(CpaLifecycleError::BorrowedClaudeAuthSourceChanged)
    ));
    assert_eq!(fs::read(auth.path().join(FILE_NAME)).unwrap(), previous);
    let approved = spec.inspect().unwrap();
    drop(BorrowedClaudeLease::acquire(auth.path(), &spec, Some(&approved)).unwrap());
    let replaced = fs::read(auth.path().join(FILE_NAME)).unwrap();
    assert_ne!(replaced, previous);
    assert!(!String::from_utf8_lossy(&replaced).contains("refresh"));
}

#[test]
fn expired_or_deleted_native_auth_does_not_reuse_cached_access() {
    let root = tempfile::tempdir().unwrap();
    let spec = source(root.path(), "access-first", "account-one");
    assert!(spec.inspect().is_ok());
    write_native(spec.source_path(), "access-first", 1);
    assert!(matches!(
        spec.inspect(),
        Err(CpaLifecycleError::InvalidBorrowedClaudeAuth)
    ));
    fs::remove_file(spec.source_path()).unwrap();
    assert!(matches!(
        spec.inspect(),
        Err(CpaLifecycleError::BorrowedClaudeAuthMissing)
    ));
}

#[cfg(unix)]
#[test]
fn shared_read_permissions_are_preserved_and_symlinks_are_rejected() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let spec = source(root.path(), "access-first", "account-one");
    fs::set_permissions(spec.source_path(), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(spec.inspect().is_ok());
    assert_eq!(
        fs::metadata(spec.source_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    let alias = root.path().join("alias");
    symlink(spec.source_path(), &alias).unwrap();
    assert!(read_private(&alias).is_err());
}

#[test]
fn refresh_material_in_managed_file_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let auth = tempfile::tempdir().unwrap();
    ensure_private_dir(auth.path()).unwrap();
    let spec = source(root.path(), "access-first", "account-one");
    let mut lease = BorrowedClaudeLease::acquire(auth.path(), &spec, None).unwrap();
    private_atomic_write(
        &auth.path().join(FILE_NAME),
        br#"{"refresh_token":"foreign"}"#,
    )
    .unwrap();
    assert!(lease.refresh(None, None).is_err());
}
