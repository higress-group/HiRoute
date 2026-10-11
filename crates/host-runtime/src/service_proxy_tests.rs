use super::*;

fn private_home() -> tempfile::TempDir {
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    builder.tempdir().unwrap()
}

fn snapshot(values: &[(&str, &str)]) -> ServiceProxyEnvironment {
    ServiceProxyEnvironment::capture(values.iter().map(|(k, v)| (k.into(), v.into()))).unwrap()
}

#[test]
fn handoff_is_private_bounded_redacted_and_replaces_the_whole_environment() {
    let home = private_home();
    assert!(
        ServiceProxyEnvironment::load(home.path())
            .unwrap()
            .is_none()
    );
    let value = snapshot(&[
        (
            "https_proxy",
            "http://user:private-proxy-password@127.0.0.1:1187",
        ),
        ("NO_PROXY", "localhost,.example.test"),
        ("OPENAI_API_KEY", "must-not-forward"),
        ("BASH_ENV", "must-not-execute"),
    ]);
    value.store(home.path()).unwrap();
    let loaded = ServiceProxyEnvironment::load(home.path()).unwrap().unwrap();
    assert_eq!(loaded.variables, value.variables);
    assert_eq!(loaded.variables.len(), 2);
    assert!(!format!("{loaded:?}").contains("private-proxy-password"));
    let path = ServiceProxyEnvironment::path(home.path());
    let bytes = fs::read(&path).unwrap();
    assert!(!String::from_utf8(bytes).unwrap().contains("must-not"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    snapshot(&[]).store(home.path()).unwrap();
    assert!(
        ServiceProxyEnvironment::load(home.path())
            .unwrap()
            .unwrap()
            .variables
            .is_empty()
    );
}

#[test]
fn unknown_malformed_and_oversized_snapshots_fail_without_exposing_values() {
    let home = private_home();
    snapshot(&[]).store(home.path()).unwrap();
    let path = ServiceProxyEnvironment::path(home.path());
    for data in [
        format!(r#"{{"schema":"{SCHEMA}","variables":{{"LD_PRELOAD":"secret"}}}}"#),
        format!(r#"{{"schema":"{SCHEMA}","variables":{{}},"extra":"secret"}}"#),
        "secret invalid JSON".into(),
        "x".repeat(MAX_BYTES as usize + 1),
    ] {
        fs::write(&path, data).unwrap();
        let error = ServiceProxyEnvironment::load(home.path()).unwrap_err();
        assert!(!error.to_string().contains("secret"));
    }
    for value in [
        "http://proxy\nLD_PRELOAD=bad".to_owned(),
        "x".repeat(MAX_BYTES as usize),
    ] {
        assert!(ServiceProxyEnvironment::capture([("HTTPS_PROXY".into(), value.into())]).is_err());
    }
}

#[cfg(unix)]
#[test]
fn accepts_accessible_modes_but_refuses_symlink_parents_and_files() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let home = private_home();
    let other = private_home();
    symlink(other.path(), home.path().join(".local")).unwrap();
    assert!(snapshot(&[]).store(home.path()).is_err());
    assert!(ServiceProxyEnvironment::load(home.path()).is_err());
    fs::remove_file(home.path().join(".local")).unwrap();
    snapshot(&[]).store(home.path()).unwrap();
    let path = ServiceProxyEnvironment::path(home.path());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        ServiceProxyEnvironment::load(home.path())
            .unwrap()
            .is_some()
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );
    snapshot(&[]).store(home.path()).unwrap();
    fs::remove_file(&path).unwrap();
    let target = other.path().join("untouched");
    symlink(&target, &path).unwrap();
    assert!(ServiceProxyEnvironment::load(home.path()).is_err());
    assert!(snapshot(&[]).store(home.path()).is_err());
    assert!(!target.exists());
}
