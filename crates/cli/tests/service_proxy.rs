#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn private_home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

fn snapshot_path(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".local/share/hiroute/service/proxy-environment.json")
}

fn variables(home: &std::path::Path) -> serde_json::Map<String, serde_json::Value> {
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(snapshot_path(home)).unwrap()).unwrap();
    assert_eq!(value["schema"], "hiroute.standalone-proxy-environment/v1");
    value["variables"].as_object().unwrap().clone()
}

fn executable(path: &std::path::Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn installation(home: &std::path::Path) -> Command {
    let directory = home.join(".local/share/hiroute");
    fs::create_dir_all(&directory).unwrap();
    fs::create_dir_all(home.join("bin")).unwrap();
    for parent in [".local", ".local/share", ".local/share/hiroute"] {
        fs::set_permissions(home.join(parent), fs::Permissions::from_mode(0o700)).unwrap();
    }
    fs::write(
        directory.join("standalone.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema_version": "hiroute.standalone-install/v1", "version":"0.1.0",
            "target":"x86_64-unknown-linux-gnu", "install_root":home.join("bin"),
            "service_definition":home.join("unit")
        }))
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(
        directory.join("standalone.json"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    executable(
        &home.join("bin/systemctl"),
        r#"#!/bin/sh
if [ "$2" = is-active ]; then test "$TEST_SERVICE_ACTIVE" = 1; exit $?; fi
if [ "$2" = is-enabled ]; then exit 1; fi
printf '%s\n' "$@" > "$HOME/manager-call"
test -f "$HOME/.local/share/hiroute/service/proxy-environment.json" || exit 89
# The fixture deliberately refuses startup. Readiness must not be fabricated.
exit 90
"#,
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_hiroute"));
    command
        .env_clear()
        .env("HOME", home)
        .env("PATH", home.join("bin"))
        .env("HTTPS_PROXY", "http://user:private-proxy@127.0.0.1:1187")
        .env("NO_PROXY", "localhost,127.0.0.1")
        .env("OPENAI_API_KEY", "private-provider-key");
    command
}

#[test]
fn service_start_restart_capture_before_manager_without_forwarding_values_in_arguments() {
    for verb in ["start", "restart"] {
        let home = private_home();
        let result = installation(home.path())
            .args(["service", verb, "--output", "json"])
            .output()
            .unwrap();
        assert!(
            !result.status.success(),
            "refused service startup must remain unavailable"
        );
        let variables = variables(home.path());
        assert_eq!(variables.len(), 2);
        assert_eq!(
            variables.get("HTTPS_PROXY").unwrap(),
            "http://user:private-proxy@127.0.0.1:1187"
        );
        for bytes in [
            &result.stdout,
            &result.stderr,
            &fs::read(home.path().join("manager-call")).unwrap(),
        ] {
            let text = String::from_utf8_lossy(bytes);
            assert!(!text.contains("private-proxy"));
            assert!(!text.contains("private-provider-key"));
        }
    }
}

#[test]
fn active_start_keeps_policy_and_invalid_capture_never_calls_manager_start() {
    let home = private_home();
    let _ = installation(home.path());
    fs::create_dir(snapshot_path(home.path()).parent().unwrap()).unwrap();
    fs::set_permissions(
        snapshot_path(home.path()).parent().unwrap(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::write(
        snapshot_path(home.path()),
        br#"{"schema":"hiroute.standalone-proxy-environment/v1","variables":{}}"#,
    )
    .unwrap();
    fs::set_permissions(
        snapshot_path(home.path()),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let result = installation(home.path())
        .env("TEST_SERVICE_ACTIVE", "1")
        .args(["service", "start", "--output", "json"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(variables(home.path()).len(), 0);
    fs::remove_file(home.path().join("manager-call")).unwrap();
    let result = installation(home.path())
        .env("HTTPS_PROXY", "http://proxy\nprivate-proxy")
        .args(["service", "restart", "--output", "json"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!home.path().join("manager-call").exists());
    assert!(!String::from_utf8_lossy(&result.stdout).contains("private-proxy"));
}

#[test]
fn foreground_run_captures_before_exec() {
    let home = private_home();
    let mut command = installation(home.path());
    executable(
        &home.path().join("bin/hirouted"),
        r#"#!/bin/sh
test -f "$HOME/.local/share/hiroute/service/proxy-environment.json" || exit 89
test "$1" = --role && test "$2" = all && test "$3" = --standalone
"#,
    );
    assert!(command.args(["service", "run"]).status().unwrap().success());
    assert_eq!(variables(home.path()).len(), 2);
}

#[test]
fn subscription_proxy_set_persists_pending_policy_without_starting_service_or_replacing_snapshot() {
    let home = private_home();
    let _ = installation(home.path());
    fs::create_dir_all(snapshot_path(home.path()).parent().unwrap()).unwrap();
    let snapshot = br#"{"schema":"hiroute.standalone-proxy-environment/v1","variables":{}}"#;
    fs::write(snapshot_path(home.path()), snapshot).unwrap();
    fs::set_permissions(
        snapshot_path(home.path()),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();

    let cases: &[(&[&str], serde_json::Value)] = &[
        (
            &[
                "--mode",
                "manual",
                "--url",
                "http://127.0.0.1:1187",
                "--no-proxy",
                "example.com",
            ],
            serde_json::json!({"mode":"manual","url":"http://127.0.0.1:1187","no_proxy":"example.com"}),
        ),
        (&["--mode", "direct"], serde_json::json!({"mode":"direct"})),
        (
            &["--mode", "inherit"],
            serde_json::json!({"mode":"inherit"}),
        ),
    ];
    for (arguments, expected_policy) in cases {
        let result = installation(home.path())
            .args(["subscription-proxy", "set"])
            .args(*arguments)
            .args(["--output", "json"])
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(0));
        assert!(result.stderr.is_empty());
        let envelope: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(envelope["data"]["config"]["policy"], *expected_policy);
        assert_eq!(envelope["data"]["applied"], false);
        assert_eq!(envelope["data"]["action"], "service_restart_required");

        let shown = installation(home.path())
            .args(["subscription-proxy", "show", "--output", "json"])
            .output()
            .unwrap();
        assert_eq!(shown.status.code(), Some(0));
        let shown: serde_json::Value = serde_json::from_slice(&shown.stdout).unwrap();
        assert_eq!(shown["data"]["config"], envelope["data"]["config"]);
        assert_eq!(shown["data"]["applied"], false);
        assert_eq!(fs::read(snapshot_path(home.path())).unwrap(), snapshot);
        assert!(!home.path().join("manager-call").exists());
        assert!(!String::from_utf8_lossy(&result.stdout).contains("private-proxy"));
    }
}

#[test]
fn subscription_proxy_rejects_invalid_arguments_without_changing_saved_policy() {
    let home = private_home();
    let initial = installation(home.path())
        .args([
            "subscription-proxy",
            "set",
            "--mode",
            "manual",
            "--url",
            "http://127.0.0.1:1187",
            "--output",
            "json",
        ])
        .output()
        .unwrap();
    assert_eq!(initial.status.code(), Some(0));
    let policy_path = home
        .path()
        .join(".local/state/hiroute/subscription-proxy.json");
    let saved = fs::read(&policy_path).unwrap();
    let invalid: &[&[&str]] = &[
        &[],
        &["--mode"],
        &["--mode", "unknown"],
        &["--mode", "manual"],
        &["--mode", "direct", "--url", "http://127.0.0.1:1187"],
        &["--mode", "inherit", "--no-proxy", "example.com"],
        &["--mode", "direct", "--mode", "inherit"],
        &["--mode", "direct", "--unknown", "value"],
        &["--mode", "manual", "--url", "socks5://127.0.0.1:1186"],
        &[
            "--mode",
            "manual",
            "--url",
            "http://user:secret@127.0.0.1:1187",
        ],
        &[
            "--mode",
            "manual",
            "--url",
            "http://127.0.0.1:1187",
            "--url",
            "http://127.0.0.1:1188",
        ],
        &[
            "--mode",
            "manual",
            "--url",
            "http://127.0.0.1:1187",
            "--no-proxy",
            "a",
            "--no-proxy",
            "b",
        ],
    ];
    for arguments in invalid {
        let result = installation(home.path())
            .args(["subscription-proxy", "set"])
            .args(*arguments)
            .args(["--output", "json"])
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2), "{arguments:?}");
        let envelope: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(envelope["status"], "usage_error", "{arguments:?}");
        assert_eq!(fs::read(&policy_path).unwrap(), saved, "{arguments:?}");
        assert!(!home.path().join("manager-call").exists());
        assert!(!snapshot_path(home.path()).exists());
        for bytes in [&result.stdout, &result.stderr] {
            let text = String::from_utf8_lossy(bytes);
            assert!(!text.contains("private-proxy"));
            assert!(!text.contains("user:secret"));
        }
    }
}
