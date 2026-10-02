//! Launch transport regression. Product CPA authentication is separately exercised with the
//! exact pinned binary; this test checks the parent's argv/environment/pipe contract.
use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn bootstrap_secret_uses_a_closed_pipe_and_never_argv_or_environment() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("child");
    let secret = "synthetic_credential_012345678901234567890123456789";
    std::fs::write(
        &binary,
        format!(
            r#"#!/bin/sh
test -z "$MANAGEMENT_PASSWORD" || exit 10
test "$4" = '--local-password-stdin' || exit 11
test "$#" = 4 || exit 12
value=$(cat)
test "$value" = '{secret}' || exit 13
printf 'pipe-eof-ok' > result
"#
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let launch = CpaLaunch {
        binary: VerifiedCpaBinary::fixture(
            binary,
            semver::Version::new(7, 2, 140),
            "fixture".into(),
        ),
        config_path: root.path().join("config.yaml"),
        work_dir: root.path().to_owned(),
        management_password: Arc::new(
            crate::config::SecretText::from_owned(secret.into()).unwrap(),
        ),
    };
    assert!(!format!("{launch:?}").contains(secret));
    // An executable script on the validation workbench can briefly report ETXTBSY
    // just after the fixture is written. Retry only that fixture/filesystem race;
    // every other launch error must still fail this pipe-contract regression.
    let spawn_deadline = Instant::now() + Duration::from_secs(1);
    let mut child = loop {
        match StdCpaProcessBackend.spawn(&launch) {
            Ok(child) => break child,
            Err(CpaProcessError::Spawn(error))
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && Instant::now() < spawn_deadline =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("fixture CPA spawn failed: {error}"),
        }
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(exit) = child.try_exit().unwrap() {
            assert_eq!(exit.code, Some(0));
            break;
        }
        assert!(Instant::now() < deadline, "bootstrap pipe was not closed");
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read(root.path().join("result")).unwrap(),
        b"pipe-eof-ok"
    );
}

#[test]
fn proxy_environment_preserves_bypass_without_forwarding_credentials_or_hooks() {
    let vars = [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "no_proxy",
        "OPENAI_API_KEY",
        "MANAGEMENT_PASSWORD",
        "LD_PRELOAD",
        "BASH_ENV",
        "HOME",
    ];
    let selected: Vec<_> =
        proxy_environment(vars.map(|key| (key.into(), "sentinel".into()))).collect();
    assert_eq!(selected.len(), 6);
    for (i, (key, value)) in selected.iter().enumerate() {
        assert_eq!(key, vars[i]);
        assert_eq!(value, "sentinel");
    }
}
