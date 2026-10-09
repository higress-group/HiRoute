use super::*;

fn source(root: &Path, token: &str, account: &str) -> BorrowedClaudeAuthSpec {
    ensure_private_dir(root).unwrap();
    let path = root.join(".credentials.json");
    write_native(&path, token, u64::MAX);
    let spec = BorrowedClaudeAuthSpec::new(ClaudeSubscriptionLocation::File(path));
    *spec.identity.lock() = Some((digest(token.as_bytes()), account.into()));
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
    *spec.identity.lock() = Some((digest(b"access-second"), "account-one".into()));
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
    *spec.identity.lock() = Some((digest(b"other-account-token"), "account-two".into()));
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
    *spec.identity.lock() = Some((digest(b"rotated-access"), "account-one".into()));
    drop(BorrowedClaudeLease::acquire(auth.path(), &spec, None).unwrap());
    let previous = fs::read(auth.path().join(FILE_NAME)).unwrap();
    write_native(spec.source_path(), "new-account-access", u64::MAX);
    *spec.identity.lock() = Some((digest(b"new-account-access"), "account-two".into()));
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
fn symlinks_and_shared_read_permissions_are_rejected() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let spec = source(root.path(), "access-first", "account-one");
    fs::set_permissions(spec.source_path(), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(spec.inspect().is_err());
    fs::set_permissions(spec.source_path(), fs::Permissions::from_mode(0o600)).unwrap();
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
