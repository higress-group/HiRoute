pub(crate) fn isolated_lock_test(test_name: &str) -> bool {
    // Other concurrently forked tests may temporarily retain a CLOEXEC descriptor.
    // Keep the production open-file-description lifetime and isolate its assertion.
    const CHILD: &str = "HIROUTE_ISOLATED_LOCK_TEST";
    if std::env::var(CHILD).as_deref() == Ok(test_name) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name])
        .env(CHILD, test_name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success()
            && stdout
                .lines()
                .any(|line| line == format!("test {test_name} ... ok")),
        "isolated lease test must execute its exact case: {stdout} {}",
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

use serde_json::{Value, json};

use super::*;

const ACCESS_ONE: &str = "fixture-access-lease-one";
const ACCESS_TWO: &str = "fixture-access-lease-two";
const ID_ONE: &str = "fixture.id.lease-one";
const ID_TWO: &str = "fixture.id.lease-two";
const REFRESH_SENTINEL: &str = "fixture-refresh-must-never-be-copied";

fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    ensure_private_dir(temp.path()).unwrap();
    let auth_dir = ensure_private_dir(&temp.path().join("cpa-auth")).unwrap();
    let source = temp.path().join("codex-auth.json");
    write_source(&source, "account-one", ACCESS_ONE, ID_ONE, "first");
    (temp, auth_dir, source)
}

fn write_source(
    path: &Path,
    account_id: &str,
    access_token: &str,
    id_token: &str,
    last_refresh: &str,
) {
    let bytes = serde_json::to_vec(&json!({
        "OPENAI_API_KEY": null,
        "auth_mode": "chatgpt",
        "last_refresh": last_refresh,
        "tokens": {
            "access_token": access_token,
            "id_token": id_token,
            "refresh_token": REFRESH_SENTINEL,
            "account_id": account_id
        }
    }))
    .unwrap();
    private_atomic_write(path, &bytes).unwrap();
}

fn flat_value(auth_dir: &Path) -> Value {
    serde_json::from_slice(&fs::read(auth_dir.join(MANAGED_FILE_NAME)).unwrap()).unwrap()
}

#[cfg(unix)]
#[test]
fn selected_engine_version_is_carried_to_cpa_and_refreshed_on_reacquire() {
    if isolated_lock_test(
        "borrowed_codex::tests::selected_engine_version_is_carried_to_cpa_and_refreshed_on_reacquire",
    ) {
        return;
    }

    use std::os::unix::fs::PermissionsExt;
    let (temp, auth_dir, source) = setup();
    let executable = temp.path().join("selected-codex");
    let spec = BorrowedCodexAuthSpec::new(&source).with_executable(executable.clone());
    for (version, expected) in [("0.158.0-alpha.2.1", "0.158.0"), ("0.999.1", "0.999.1")] {
        fs::write(
            &executable,
            format!("#!/bin/sh\nprintf 'codex-cli {version}\\n'\n"),
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let lease = ManagedAuthLease::acquire(&auth_dir, Some(&spec)).unwrap();
        assert_eq!(flat_value(&auth_dir)["hiroute_client_version"], expected);
        drop(lease);
    }
}

#[test]
fn missing_engine_does_not_invent_a_client_version() {
    let (temp, auth_dir, source) = setup();
    let spec =
        BorrowedCodexAuthSpec::new(&source).with_executable(temp.path().join("missing-codex"));
    let _lease = ManagedAuthLease::acquire(&auth_dir, Some(&spec)).unwrap();
    assert!(flat_value(&auth_dir)["hiroute_client_version"].is_null());
}

#[cfg(unix)]
#[test]
fn explicit_recheck_recovers_a_failed_version_within_the_same_access_lease() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, auth_dir, source) = setup();
    let executable = temp.path().join("selected-codex");
    fs::write(&executable, "#!/bin/sh\nexit 7\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let spec = BorrowedCodexAuthSpec::new(&source).with_executable(executable.clone());
    let original = fs::read(&source).unwrap();
    let evidence = spec.inspect().unwrap();
    let mut lease = ManagedAuthLease::acquire(&auth_dir, Some(&spec)).unwrap();
    let first = lease.refresh().unwrap();
    assert!(flat_value(&auth_dir)["hiroute_client_version"].is_null());
    fs::write(&executable, "#!/bin/sh\nprintf 'codex-cli 0.162.0\\n'\n").unwrap();
    assert_eq!(
        lease.refresh().unwrap(),
        first,
        "ordinary reads changed identity"
    );
    assert!(flat_value(&auth_dir)["hiroute_client_version"].is_null());
    let refreshed = lease.refresh_expected(Some(&evidence), None).unwrap();
    assert_eq!(refreshed[0].account_kind, first[0].account_kind);
    assert_eq!(refreshed[0].stock_file_name, first[0].stock_file_name);
    assert_eq!(refreshed[0].account_digest, first[0].account_digest);
    assert_eq!(refreshed[0].generation, first[0].generation);
    assert_eq!(refreshed[0].client_version.as_deref(), Some("0.162.0"));
    let flat = flat_value(&auth_dir);
    assert_eq!(flat["hiroute_client_version"], "0.162.0");
    assert!(flat.get("refresh_token").is_none());
    assert_eq!(fs::read(&source).unwrap(), original);
    // The existing source lock continues to exclude a concurrent CPA borrower.
    assert!(ManagedAuthLease::acquire(&auth_dir, Some(&spec)).is_err());
}

#[test]
fn evidence_scan_is_read_only_and_redacts_the_source() {
    let temp = tempfile::tempdir().unwrap();
    ensure_private_dir(temp.path()).unwrap();
    let source = temp.path().join("codex-auth.json");
    let untouched_auth_dir = temp.path().join("must-not-exist");
    write_source(&source, "account-one", ACCESS_ONE, ID_ONE, "first");

    let evidence = BorrowedCodexAuthSpec::new(&source).inspect().unwrap();

    assert_eq!(
        evidence.account_ref(),
        format!("account/cpa/{}", account_digest("account-one"))
    );
    assert!(!untouched_auth_dir.exists());
    let debug = format!("{evidence:?}");
    assert!(!debug.contains(source.to_string_lossy().as_ref()));
    assert!(!debug.contains("account-one"));
    assert!(!debug.contains(ACCESS_ONE));
}

#[test]
fn expected_evidence_rejects_account_replacement_before_access_materialization() {
    let (_temp, auth_dir, source) = setup();
    let spec = BorrowedCodexAuthSpec::new(&source);
    let expected = spec.inspect().unwrap();
    write_source(&source, "account-two", ACCESS_TWO, ID_TWO, "second");

    assert!(matches!(
        ManagedAuthLease::acquire_expected(&auth_dir, Some(&spec), Some(&expected)),
        Err(CpaLifecycleError::BorrowedCodexAuthSourceChanged)
    ));
    assert!(!auth_dir.join(MANAGED_FILE_NAME).exists());
    assert!(!auth_dir.join(STATE_FILE_NAME).exists());
}

#[test]
fn bounded_reader_rejects_a_source_that_changes_during_every_attempt() {
    let (_temp, _auth_dir, source) = setup();
    let mut revision = 0_u64;
    let result = read_nested_source_after_read(&source, || {
        revision += 1;
        write_source(
            &source,
            &format!("account-changing-{revision}"),
            ACCESS_TWO,
            ID_TWO,
            &format!("refresh-{revision}"),
        );
    });

    assert!(matches!(
        result,
        Err(CpaLifecycleError::BorrowedCodexAuthSourceChanged)
    ));
}

#[test]
fn missing_source_has_a_distinct_local_failure() {
    let temp = tempfile::tempdir().unwrap();
    let result = BorrowedCodexAuthSpec::new(temp.path().join("missing.json")).inspect();
    assert!(matches!(
        result,
        Err(CpaLifecycleError::BorrowedCodexAuthMissing)
    ));
}

#[test]
fn nested_auth_is_imported_as_an_owner_only_access_lease() {
    let (_temp, auth_dir, source) = setup();
    let mut lease =
        ManagedAuthLease::acquire(&auth_dir, Some(&BorrowedCodexAuthSpec::new(source.clone())))
            .unwrap();
    let identity = lease.refresh().unwrap().pop().unwrap();
    let flat = flat_value(&auth_dir);

    assert_eq!(flat["type"], "codex");
    assert_eq!(flat["request_retry"], 0);
    assert_eq!(flat["disable_cooling"], true);
    assert_eq!(flat["prefix"], identity.prefix().unwrap());
    assert!(flat.get("refresh_token").is_none());
    assert_eq!(identity.account_digest, account_digest("account-one"));
    assert!(!identity.account_digest.contains("codex-auth.json"));
    validate_private_file(&auth_dir.join(MANAGED_FILE_NAME)).unwrap();
    validate_private_file(&auth_dir.join(STATE_FILE_NAME)).unwrap();

    let flat_bytes = fs::read(auth_dir.join(MANAGED_FILE_NAME)).unwrap();
    assert!(
        !flat_bytes
            .windows(REFRESH_SENTINEL.len())
            .any(|window| window == REFRESH_SENTINEL.as_bytes())
    );
}

#[test]
fn access_rotation_advances_generation_but_account_replacement_is_rejected() {
    let (_temp, auth_dir, source) = setup();
    let spec = BorrowedCodexAuthSpec::new(source.clone());
    let first_evidence = spec.inspect().unwrap();
    let mut lease = ManagedAuthLease::acquire(&auth_dir, Some(&spec)).unwrap();
    let first = lease.refresh().unwrap().pop().unwrap();

    write_source(&source, "account-one", ACCESS_TWO, ID_TWO, "second");
    let rotated_evidence = spec.inspect().unwrap();
    assert_ne!(
        rotated_evidence.evidence_digest(),
        first_evidence.evidence_digest()
    );
    assert_eq!(
        rotated_evidence.binding_evidence_digest(),
        first_evidence.binding_evidence_digest()
    );
    let rotated = lease.refresh().unwrap().pop().unwrap();
    assert_eq!(rotated.account_digest, first.account_digest);
    assert_eq!(rotated.generation, first.generation + 1);

    let managed_before = fs::read(auth_dir.join(MANAGED_FILE_NAME)).unwrap();
    let state_before = fs::read(auth_dir.join(STATE_FILE_NAME)).unwrap();
    write_source(&source, "account-two", ACCESS_ONE, ID_ONE, "third");
    let replacement_evidence = spec.inspect().unwrap();
    assert_ne!(
        replacement_evidence.binding_evidence_digest(),
        first_evidence.binding_evidence_digest()
    );
    assert!(matches!(
        lease.refresh(),
        Err(CpaLifecycleError::BorrowedCodexAuthSourceChanged)
    ));
    assert_eq!(
        fs::read(auth_dir.join(MANAGED_FILE_NAME)).unwrap(),
        managed_before
    );
    assert_eq!(
        fs::read(auth_dir.join(STATE_FILE_NAME)).unwrap(),
        state_before
    );
}

#[test]
fn canonical_source_and_auth_directory_are_exclusive_across_instances() {
    if isolated_lock_test(
        "borrowed_codex::tests::canonical_source_and_auth_directory_are_exclusive_across_instances",
    ) {
        return;
    }
    let (temp, auth_dir, source) = setup();
    let first =
        ManagedAuthLease::acquire(&auth_dir, Some(&BorrowedCodexAuthSpec::new(source.clone())))
            .unwrap();
    assert!(matches!(
        ManagedAuthLease::acquire(&auth_dir, Some(&BorrowedCodexAuthSpec::new(source.clone()))),
        Err(CpaLifecycleError::BorrowedCodexAuthAlreadyLeased)
    ));

    let second_auth = ensure_private_dir(&temp.path().join("second-cpa-auth")).unwrap();
    assert!(matches!(
        ManagedAuthLease::acquire(
            &second_auth,
            Some(&BorrowedCodexAuthSpec::new(source.clone()))
        ),
        Err(CpaLifecycleError::BorrowedCodexAuthAlreadyLeased)
    ));
    let inherited_source = first
        .codex
        .as_ref()
        .unwrap()
        ._source_lock
        .try_clone()
        .unwrap();
    drop(first);
    assert!(matches!(
        ManagedAuthLease::acquire(
            &second_auth,
            Some(&BorrowedCodexAuthSpec::new(source.clone()))
        ),
        Err(CpaLifecycleError::BorrowedCodexAuthAlreadyLeased)
    ));
    drop(inherited_source);
    ManagedAuthLease::acquire(&second_auth, Some(&BorrowedCodexAuthSpec::new(source))).unwrap();
}

#[test]
fn access_only_file_cannot_enter_stock_unauthorized_refresh_replay() {
    let (_temp, auth_dir, source) = setup();
    let _lease =
        ManagedAuthLease::acquire(&auth_dir, Some(&BorrowedCodexAuthSpec::new(source))).unwrap();
    let flat = flat_value(&auth_dir);

    // Mirrors CLIProxyAPI v7.2.140 authHasRefreshCredential: a local 401 is replayed only when
    // one of these two metadata keys contains a non-empty string.
    let stock_has_refresh_credential = ["refresh_token", "refreshToken"].iter().any(|key| {
        flat.get(*key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    });
    assert!(!stock_has_refresh_credential);
    assert_eq!(flat["request_retry"], 0);
}

#[cfg(unix)]
#[test]
fn symlink_and_hardlink_sources_fail_but_readable_modes_are_preserved() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let (temp, auth_dir, source) = setup();
    let symlink_path = temp.path().join("symlink-auth.json");
    symlink(&source, &symlink_path).unwrap();
    assert!(matches!(
        ManagedAuthLease::acquire(&auth_dir, Some(&BorrowedCodexAuthSpec::new(symlink_path))),
        Err(CpaLifecycleError::InvalidBorrowedCodexAuth)
    ));

    let hardlink_path = temp.path().join("hardlink-auth.json");
    fs::hard_link(&source, &hardlink_path).unwrap();
    let other_auth = ensure_private_dir(&temp.path().join("hardlink-cpa-auth")).unwrap();
    assert!(matches!(
        ManagedAuthLease::acquire(
            &other_auth,
            Some(&BorrowedCodexAuthSpec::new(&hardlink_path))
        ),
        Err(CpaLifecycleError::InvalidBorrowedCodexAuth)
    ));
    fs::remove_file(hardlink_path).unwrap();

    fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
    let third_auth = ensure_private_dir(&temp.path().join("mode-cpa-auth")).unwrap();
    let original = fs::read(&source).unwrap();
    let _lease =
        ManagedAuthLease::acquire(&third_auth, Some(&BorrowedCodexAuthSpec::new(&source))).unwrap();
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(
        fs::metadata(&source).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[test]
fn native_modes_and_optional_metadata_preserve_account_and_access_only_lease() {
    let (_temp, _auth_dir, path) = setup();
    let original = fs::read(&path).unwrap();
    let expected = BorrowedCodexAuthSpec::new(&path)
        .inspect()
        .unwrap()
        .account_ref();
    for mode in [None, Some(serde_json::Value::Null), Some(json!("chatgpt"))] {
        let mut value: Value = serde_json::from_slice(&original).unwrap();
        value.as_object_mut().unwrap().remove("auth_mode");
        if let Some(mode) = mode {
            value["auth_mode"] = mode;
        }
        value.as_object_mut().unwrap().remove("last_refresh");
        value["tokens"].as_object_mut().unwrap().remove("id_token");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let source = read_nested_source(&path).unwrap();
        assert_eq!(
            BorrowedCodexAuthSpec::new(&path)
                .inspect()
                .unwrap()
                .account_ref(),
            expected
        );
        let flat = render_access_only_auth(&source, "fixture", None).unwrap();
        assert!(!String::from_utf8_lossy(&flat).contains(REFRESH_SENTINEL));
        value["auth_mode"] = json!("chatgpt");
        value["OPENAI_API_KEY"] = json!("supplementary-api-key");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            BorrowedCodexAuthSpec::new(&path)
                .inspect()
                .unwrap()
                .account_ref(),
            expected
        );
        value.as_object_mut().unwrap().remove("auth_mode");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            BorrowedCodexAuthSpec::new(&path).inspect(),
            Err(CpaLifecycleError::BorrowedCodexLoginUnsupported)
        ));
    }
}

#[test]
fn account_claim_fallback_never_invents_an_account() {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    let (_temp, _auth_dir, path) = setup();
    let original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let expected = BorrowedCodexAuthSpec::new(&path)
        .inspect()
        .unwrap()
        .account_ref();
    let claims = URL_SAFE_NO_PAD.encode(br#"{"https://api.openai.com/auth":{"chatgpt_account_id":"account-one"},"email":"ignored@example.test"}"#);
    for field in ["id_token", "access_token"] {
        let mut value = original.clone();
        value["tokens"]
            .as_object_mut()
            .unwrap()
            .remove("account_id");
        value["tokens"][field] = json!(format!("e30.{claims}.fixture"));
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            BorrowedCodexAuthSpec::new(&path)
                .inspect()
                .unwrap()
                .account_ref(),
            expected
        );
    }
    let mut value = original;
    value["tokens"]["account_id"] = Value::Null;
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(matches!(
        BorrowedCodexAuthSpec::new(&path).inspect(),
        Err(CpaLifecycleError::BorrowedCodexAccountMissing)
    ));
}

#[test]
fn native_store_changes_never_borrow_a_stale_file_and_explicit_override_wins() {
    let (temp, auth_dir, path) = setup();
    let config = temp.path().join("config.toml");
    let spec = BorrowedCodexAuthSpec::new(&path).with_store_config(Some(config.clone()));
    let mut lease = ManagedAuthLease::acquire(&auth_dir, Some(&spec)).unwrap();
    let original = fs::read(&path).unwrap();
    let flat = fs::read(auth_dir.join(MANAGED_FILE_NAME)).unwrap();
    for store in ["keyring", "auto", "ephemeral", "future-store"] {
        fs::write(
            &config,
            format!("cli_auth_credentials_store = \"{store}\"\n"),
        )
        .unwrap();
        assert!(matches!(
            spec.inspect(),
            Err(CpaLifecycleError::BorrowedCodexStoreUnsupported)
        ));
        assert!(matches!(
            lease.refresh(),
            Err(CpaLifecycleError::BorrowedCodexStoreUnsupported)
        ));
        assert!(BorrowedCodexAuthSpec::new(&path).inspect().is_ok());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read(auth_dir.join(MANAGED_FILE_NAME)).unwrap(), flat);
    }
    fs::write(&config, "cli_auth_credentials_store = \"file\"\n").unwrap();
    assert!(lease.refresh().is_ok());
}
