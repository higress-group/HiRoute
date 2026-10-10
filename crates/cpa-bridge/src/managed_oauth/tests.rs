use super::*;

fn fixture(kind: CpaAccountKind, account: &str, access: &str, refresh: &str) -> Vec<u8> {
    let mut value = serde_json::json!({
        "type": kind.stock_provider(),
        "access_token": access,
        "refresh_token": refresh,
        "expired": "2000-01-01T00:00:00Z",
        "email": "private@example.test",
    });
    value[match kind {
        CpaAccountKind::Codex => "account_id",
        CpaAccountKind::Claude => "account_uuid",
    }] = account.into();
    serde_json::to_vec(&value).unwrap()
}

#[test]
fn managed_token_rotation_preserves_binding_and_native_account_identity() {
    for kind in [CpaAccountKind::Codex, CpaAccountKind::Claude] {
        let directory = Path::new("/private/managed/fixture");
        let first = parse_credential(
            directory,
            "fixture.json",
            kind,
            &fixture(kind, "acct-fixture", "first-access", "first-refresh"),
        )
        .unwrap();
        let rotated = parse_credential(
            directory,
            "fixture.json",
            kind,
            &fixture(kind, "acct-fixture", "second-access", "second-refresh"),
        )
        .unwrap();
        assert_eq!(first, rotated);
        let expected_digest = match kind {
            CpaAccountKind::Codex => crate::borrowed_codex::account_digest("acct-fixture"),
            CpaAccountKind::Claude => format!(
                "{:x}",
                Sha256::digest(b"hiroute.cpa-account/v1\0claude\0acct-fixture\0")
            ),
        };
        assert_eq!(
            first.account_ref(),
            format!("account/cpa/{expected_digest}")
        );
        assert!(!format!("{first:?}").contains("fixture.json"));
        assert!(!format!("{:?}", first.summary()).contains("private@example.test"));
        // Account replacement must change the subject even if the file name is unchanged.
        let other = parse_credential(
            directory,
            "fixture.json",
            kind,
            &fixture(kind, "other-account", "first-access", "first-refresh"),
        )
        .unwrap();
        assert_ne!(first.account_ref(), other.account_ref());
        assert_ne!(
            first.binding_evidence_digest(),
            other.binding_evidence_digest()
        );
    }
}

#[test]
fn managed_credential_requires_own_refresh_authority_and_matching_provider() {
    let directory = Path::new("/private/managed/fixture");
    assert!(
        parse_credential(
            directory,
            "fixture.json",
            CpaAccountKind::Codex,
            &fixture(CpaAccountKind::Codex, "account", "access", "")
        )
        .is_err()
    );
    assert!(
        parse_credential(
            directory,
            "fixture.json",
            CpaAccountKind::Claude,
            &fixture(CpaAccountKind::Codex, "account", "access", "refresh")
        )
        .is_err()
    );
    assert!(
        parse_credential(
            directory,
            "fixture.json",
            CpaAccountKind::Claude,
            &fixture(CpaAccountKind::Claude, "", "access", "refresh")
        )
        .is_err()
    );
    let mut disabled: serde_json::Value = serde_json::from_slice(&fixture(
        CpaAccountKind::Codex,
        "account",
        "access",
        "refresh",
    ))
    .unwrap();
    disabled["disabled"] = true.into();
    assert!(
        parse_credential(
            directory,
            "fixture.json",
            CpaAccountKind::Codex,
            &serde_json::to_vec(&disabled).unwrap()
        )
        .is_err()
    );
}

#[test]
fn managed_store_reads_without_rewriting_tokens_and_rejects_account_pool() {
    let root = tempfile::tempdir().unwrap();
    let auth_dir = crate::config::ensure_private_dir(&root.path().join("auth")).unwrap();
    let kind = CpaAccountKind::Codex;
    let bytes = fixture(kind, "account", "access", "refresh");
    fs::write(auth_dir.join("one.json"), &bytes).unwrap();
    let source = ManagedOAuthCredentialSource {
        auth_dir: auth_dir.clone(),
        kind,
    };
    assert!(source.inspect().is_ok());
    assert_eq!(fs::read(auth_dir.join("one.json")).unwrap(), bytes);
    crate::config::validate_private_file(&auth_dir.join("one.json")).unwrap();
    fs::write(auth_dir.join("two.json"), &bytes).unwrap();
    assert!(source.inspect().is_err());
}

#[test]
fn managed_lease_is_single_writer_and_rejects_subject_replacement() {
    let root = tempfile::tempdir().unwrap();
    let auth_dir = crate::config::ensure_private_dir(&root.path().join("auth")).unwrap();
    let kind = CpaAccountKind::Codex;
    let mut lease = crate::borrowed_codex::ManagedAuthLease::acquire_subscription(
        &auth_dir,
        None,
        None,
        Some(kind),
        None,
    )
    .unwrap();
    // Pending login may own an empty store; a second process cannot own it concurrently.
    assert!(
        crate::borrowed_codex::ManagedAuthLease::acquire_subscription(
            &auth_dir,
            None,
            None,
            Some(kind),
            None
        )
        .is_err()
    );
    let path = auth_dir.join("one.json");
    fs::write(&path, fixture(kind, "first", "access", "refresh")).unwrap();
    let first = lease.refresh_subscription(None, None).unwrap();
    fs::write(
        &path,
        fixture(kind, "first", "rotated-access", "rotated-refresh"),
    )
    .unwrap();
    assert_eq!(
        lease
            .refresh_subscription(None, Some(&first[0].account_digest))
            .unwrap(),
        first
    );
    fs::write(&path, fixture(kind, "second", "access", "refresh")).unwrap();
    assert!(matches!(
        lease.refresh_subscription(None, None),
        Err(CpaLifecycleError::ManagedOAuthAccountChanged)
    ));
    fs::remove_file(&path).unwrap();
    assert!(matches!(
        lease.refresh_subscription(None, None),
        Err(CpaLifecycleError::ManagedOAuthCredentialsMissing)
    ));
}

#[cfg(unix)]
#[test]
fn managed_store_never_follows_symlink_or_accepts_hardlinked_credentials() {
    let root = tempfile::tempdir().unwrap();
    let auth_dir = crate::config::ensure_private_dir(&root.path().join("auth")).unwrap();
    let external = root.path().join("external");
    let bytes = fixture(CpaAccountKind::Claude, "account", "access", "refresh");
    fs::write(&external, &bytes).unwrap();
    let source = ManagedOAuthCredentialSource {
        auth_dir: auth_dir.clone(),
        kind: CpaAccountKind::Claude,
    };
    let path = auth_dir.join("one.json");
    std::os::unix::fs::symlink(&external, &path).unwrap();
    assert!(source.inspect().is_err());
    fs::remove_file(&path).unwrap();
    fs::hard_link(&external, &path).unwrap();
    assert!(source.inspect().is_err());
    assert_eq!(fs::read(external).unwrap(), bytes);
}
