#![cfg(windows)]
//! Required native product gate: actual Task Scheduler for the logged-in owner.
//! SCM remains separately selectable; logout continuity is outside this scope.
//! Run serially in a disposable CI profile with a verified candidate package.
//! A selected gate fails when prerequisites are missing; fixtures never stand in
//! for actual service launch, Gateway readiness, or Local Control.

#[path = "support/windows_account.rs"]
mod windows_account;
#[path = "support/windows_background_fixture.rs"]
mod windows_background_fixture;
#[path = "support/windows_background_standard.rs"]
mod windows_background_standard;
#[path = "support/windows_background_task.rs"]
mod windows_background_task;
#[path = "support/windows_background_upgrade.rs"]
mod windows_background_upgrade;
#[path = "support/windows_background_upgrade_business.rs"]
mod windows_background_upgrade_business;
#[path = "support/windows_business_http.rs"]
#[allow(dead_code)] // Upgrade consumers share the bounded framing owner.
mod windows_business_http;
#[path = "support/windows_candidate_source.rs"]
mod windows_candidate_source;
#[path = "support/windows_execution_host.rs"]
mod windows_execution_host;
#[path = "support/windows_gateway_recovery.rs"]
mod windows_gateway_recovery;
#[path = "support/windows_management_service.rs"]
mod windows_management_service;
#[path = "support/windows_subscription_authorization.rs"]
mod windows_subscription_authorization;
#[path = "support/windows_subscription_login.rs"]
mod windows_subscription_login;
#[path = "support/windows_token_launcher.rs"]
mod windows_token_launcher;

use std::ffi::OsString;
use std::fs::File;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use hiroute_platform::windows::background;
use serde_json::Value;
use windows_background_fixture as fixture;

fn assert_no_secret(bytes: &[u8]) {
    windows_background_upgrade_business::assert_no_secret(bytes);
}

struct Inputs {
    package_directory: PathBuf,
    manifest: PathBuf,
    admin_cli: PathBuf,
    version_directory: String,
    _source: windows_candidate_source::Sources,
}
impl Inputs {
    fn load() -> Self {
        Self::from_package(
            "HIROUTE_WINDOWS_PACKAGE_MANIFEST",
            "HIROUTE_WINDOWS_PACKAGE_ARCHIVE",
        )
    }
    fn baseline() -> Self {
        Self::from_package(
            "HIROUTE_WINDOWS_BASELINE_PACKAGE_MANIFEST",
            "HIROUTE_WINDOWS_BASELINE_PACKAGE_ARCHIVE",
        )
    }
    fn from_package(manifest_name: &str, archive_name: &str) -> Self {
        let manifest = fixture::required(manifest_name);
        let _archive = fixture::required(archive_name);
        let document: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
        assert_eq!(document["distribution"], "integration-candidate");
        assert_eq!(
            document["target"],
            hiroute_platform::windows::install::native_target().unwrap()
        );
        let version = document["version"].as_str().unwrap();
        let revision = document["revision"].as_str().unwrap();
        assert!(revision.len() >= 12);
        windows_execution_host::current();
        let source = windows_candidate_source::Sources::open();
        if manifest_name == "HIROUTE_WINDOWS_PACKAGE_MANIFEST"
            && let Some(expected) = windows_candidate_source::Sources::revision()
        {
            assert_eq!(
                revision, expected,
                "package and compiled gate candidate SHA differ"
            );
        }
        let admin_cli = fixture::prepared_cli();
        windows_execution_host::image(&admin_cli);
        windows_execution_host::image(&admin_cli.with_file_name("hirouted.exe"));
        windows_execution_host::image(&admin_cli.parent().unwrap().join("libexec/cliproxyapi.exe"));
        Self {
            package_directory: PathBuf::from(
                std::env::var_os(if manifest_name == "HIROUTE_WINDOWS_PACKAGE_MANIFEST" {
                    "HIROUTE_WINDOWS_PACKAGE_DIRECTORY"
                } else {
                    "HIROUTE_WINDOWS_BASELINE_PACKAGE_DIRECTORY"
                })
                .expect("native transaction package required"),
            ),
            manifest,
            admin_cli,
            version_directory: format!("{version}-{}", &revision[..12]),
            _source: source,
        }
    }
    fn install_request(&self, action: &str) -> fixture::Request {
        fixture::Request {
            action: action.into(),
            program: Some(self.admin_cli.clone()),
            arguments: vec![self.package_directory.to_string_lossy().into_owned()],
        }
    }
    fn install(&self, token: &File, scm: bool) -> Value {
        assert!(!scm, "SCM installation is not offered");
        fixture::accepted(&fixture::owner(
            token,
            self.install_request("install"),
            Duration::from_secs(90),
        ))
        .clone()
    }
    fn transaction_install(&self, token: &File, action: &str) -> Value {
        fixture::owner(token, self.install_request(action), Duration::from_secs(90))
    }
    fn uninstall(&self, token: &File, _cli: &std::path::Path, scm: bool) -> Value {
        assert!(!scm, "SCM installation is not offered");
        fixture::accepted(&fixture::owner(
            token,
            fixture::Request {
                action: "uninstall".into(),
                program: Some(self.admin_cli.clone()),
                arguments: vec![],
            },
            Duration::from_secs(90),
        ))
        .clone()
    }
    fn owner_layout(&self, token: &File) -> Value {
        fixture::owner(
            token,
            fixture::Request {
                action: "layout".into(),
                program: None,
                arguments: vec![],
            },
            Duration::from_secs(20),
        )
    }
    fn owner_cli(&self, layout: &Value) -> PathBuf {
        PathBuf::from(layout["home"].as_str().unwrap())
            .join(".local/lib/hiroute")
            .join(&self.version_directory)
            .join("hiroute.exe")
    }
}

struct Cleanup<'a> {
    inputs: &'a Inputs,
    token: &'a File,
    cli: PathBuf,
    armed: bool,
}
impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Failed required gates still attempt exact owner-scoped maintenance.
        // Never use taskkill, guessed PIDs, guessed service names, or recursive
        // deletion of the runner's existing profile as a cleanup fallback.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = fixture::cli(self.token, &self.cli, &["service", "stop"]);
            self.inputs.uninstall(self.token, &self.cli, false);
        }));
        if result.is_err() {
            eprintln!(
                "native background gate exact-owner cleanup failed; preserve failed evidence"
            );
        }
    }
}

fn assert_status(token: &File, cli: &std::path::Path, manager: &str, active: bool) -> Value {
    let status = fixture::cli(token, cli, &["service", "status"]);
    assert_eq!(status["schema"], "hiroute.standalone-service-status/v1");
    assert_eq!(status["manager"], manager);
    assert_eq!(status["manager_active"], active);
    assert_eq!(status["local_control_ready"], active);
    status
}

/// Keep real CPA authorization URLs and their opaque state in memory. The
/// ordinary fixture::accepted diagnostic prints envelopes, so OAuth operations
/// use fixed diagnostics and return only the actual parsed public envelope.
fn subscription_cli(
    token: &File,
    cli: &std::path::Path,
    arguments: &[&str],
    request: Option<&Value>,
) -> Value {
    let arguments = arguments
        .iter()
        .map(OsString::from)
        .chain([OsString::from("--timeout"), OsString::from("45s")])
        .collect::<Vec<_>>();
    let input = request.map(|value| serde_json::to_vec(value).unwrap());
    let result = fixture::process(
        Some(token),
        cli,
        &arguments,
        input.as_deref(),
        false,
        Duration::from_secs(60),
        false,
    );
    assert!(result["code"] == 0, "public subscription command failed");
    assert!(
        result["result"].is_object(),
        "public subscription result is missing"
    );
    result["result"].clone()
}

fn await_actual_restart(
    token: &File,
    cli: &std::path::Path,
    manager: &str,
    previous: &Value,
) -> Value {
    // Both managers currently specify a real 60 second recovery delay. Waiting
    // here observes real production policy instead of editing a test task/service.
    let deadline = Instant::now() + Duration::from_secs(115);
    loop {
        let result = fixture::command(
            token,
            cli,
            vec![
                "service".into(),
                "status".into(),
                "--output".into(),
                "json".into(),
            ],
        );
        if result["code"] == 0
            && result["result"]["data"]["manager_active"] == true
            && result["result"]["data"]["local_control_ready"] == true
        {
            let recovered = fixture::inspect(token, false);
            assert!(
                recovered["pid"] != previous["pid"]
                    || recovered["creation_time"] != previous["creation_time"],
                "unexpected failure must create a new actual daemon process"
            );
            assert_status(token, cli, manager, true);
            return recovered;
        }
        assert!(
            Instant::now() < deadline,
            "required background recovery did not start a new healthy business runtime"
        );
        thread::sleep(Duration::from_secs(2));
    }
}

#[test]
fn windows_background_owner_fixture() {
    fixture::entry();
}

/// Requires a real logged-on runner account, then uses LUA owner processes and
/// the real Task action. A fresh LogonUser token cannot fake an interactive session.
#[test]
fn task_native_background_lua_gateway_control_stop_crash_recovery_and_autostart() {
    let _privileges = windows_account::Privileges::enable();
    let input = Inputs::load();
    let windows_background_task::Owner {
        token,
        session: expected_session,
    } = windows_background_task::owner();
    let staged = windows_token_launcher::StagedFixture::create(&token);
    let sid = background::current_sid_string().unwrap();
    windows_account::assert_standard(&token, &sid);
    let layout = input.owner_layout(&token);
    fixture::require_fresh_lua_state(&layout);
    let cli = input.owner_cli(&layout);
    let installed = input.install(&token, false);
    let mut cleanup = Cleanup {
        inputs: &input,
        token: &token,
        cli: cli.clone(),
        armed: true,
    };
    assert_eq!(installed["installed"], true);
    assert_eq!(installed["started"], false);
    assert_eq!(
        assert_status(&token, &cli, "windows-task-user", false)["autostart_enabled"],
        false
    );
    assert_eq!(
        fixture::cli(&token, &cli, &["service", "autostart", "enable"])["autostart_enabled"],
        true
    );
    assert_eq!(
        fixture::cli(&token, &cli, &["service", "autostart", "disable"])["autostart_enabled"],
        false
    );
    assert_eq!(
        fixture::cli(&token, &cli, &["service", "start"])["local_control_ready"],
        true
    );
    let first = fixture::inspect(&token, false);
    assert_eq!(first["sid"], sid);
    assert_eq!(first["session"], expected_session);
    assert_status(&token, &cli, "windows-task-user", true);
    // This Task gate retains the actual same-SID logged-on owner. Public CLI
    // calls use its limited token; the independent held log read uses that same
    // account, never the optional SCM fixture's different administrative owner.
    windows_management_service::run(&mut |args| subscription_cli(&token, &cli, args, None));
    windows_gateway_recovery::exercise(&token, &cli, &first, &sid, expected_session);
    let oauth_pending = windows_subscription_login::run_before_restart(&mut |args, request| {
        subscription_cli(&token, &cli, args, request)
    });
    assert_eq!(
        fixture::cli(&token, &cli, &["service", "stop"])["local_control_ready"],
        false
    );
    assert_status(&token, &cli, "windows-task-user", false);
    // Fail after real file publication using a native deny-delete handle on the
    // last destination. Re-entering the shipped install command runs rollback
    // from its prepared journal before completing repair of this same package.
    let failure = input.transaction_install(&token, "transaction-fail-install");
    assert_ne!(failure["command"]["code"], 0);
    assert_eq!(failure["after"]["journal"]["phase"], "prepared");
    assert_eq!(input.install(&token, false)["installed"], true);
    let repaired = fixture::transaction_snapshot(&token, true);
    assert_eq!(repaired["marker"], failure["before"]["marker"]);
    assert_eq!(repaired["definition"], failure["before"]["definition"]);
    assert_eq!(repaired["manifest"], failure["before"]["manifest"]);
    assert_status(&token, &cli, "windows-task-user", false);
    // A deliberate stop must stay stopped rather than invoke failure recovery.
    thread::sleep(Duration::from_secs(2));
    assert_status(&token, &cli, "windows-task-user", false);
    fixture::cli(&token, &cli, &["service", "start"]);
    windows_subscription_login::assert_after_restart(
        &mut |args, request| subscription_cli(&token, &cli, args, request),
        oauth_pending,
    );
    let crashed = fixture::inspect(&token, true);
    let recovered = await_actual_restart(&token, &cli, "windows-task-user", &crashed);
    assert_eq!(recovered["session"], expected_session);
    fixture::forced_stop(&token);
    assert_status(&token, &cli, "windows-task-user", false);
    // This crosses the real PT1M failure recovery interval. Merely observing an
    // immediate exit cannot prove Scheduler interpreted this as a user stop.
    thread::sleep(Duration::from_secs(65));
    assert_status(&token, &cli, "windows-task-user", false);
    assert_eq!(input.uninstall(&token, &cli, false)["uninstalled"], true);
    cleanup.armed = false;
    assert!(!PathBuf::from(layout["marker"].as_str().unwrap()).exists());
    assert!(
        !PathBuf::from(layout["home"].as_str().unwrap())
            .join(".local/bin/hiroute.exe")
            .exists()
    );
    let preserved = fixture::owner(
        &token,
        fixture::Request {
            action: "cleanup-state".into(),
            program: None,
            arguments: vec![],
        },
        Duration::from_secs(20),
    );
    assert_eq!(preserved["preserved"], true);
    assert_eq!(preserved["removed"], false);
    drop(staged);
    println!(
        "actual Task product gate: fresh Lua state, WTS session, Gateway v2, Local Control, graceful stop, forced recovery, autostart, installation deletion verified; user state preserved"
    );
}
