#![cfg(windows)]
//! Bounded NSIS acceptance in the configured, already logged-on standard session.
//! This does not cover WebView interaction, Agent restoration or update recovery.

#[path = "support/windows_account.rs"]
#[allow(dead_code)]
mod windows_account;

use hiroute_host_runtime::{StandaloneInstallRecordV1, StandaloneLayout};
use hiroute_platform::windows::{
    background, install, installed_apps, shell_location, shell_login, task,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
fn standard_session() {
    install::require_x64().unwrap();
    windows_account::assert_current_standard_account();
}

fn hash(path: &Path) -> String {
    let mut input = File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut bytes = [0; 64 * 1024];
    loop {
        let count = input.read(&mut bytes).unwrap();
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    format!("{hash:x}", hash = hash.finalize())
}

fn run(program: &Path, args: &[&str], evidence: &Path, phase: &str) -> Vec<u8> {
    run_expecting(program, args, evidence, phase, true)
}

fn run_expecting(
    program: &Path,
    args: &[&str],
    evidence: &Path,
    phase: &str,
    success: bool,
) -> Vec<u8> {
    let output = evidence.join(format!("{phase}.stdout"));
    let error = evidence.join(format!("{phase}.stderr"));
    use std::os::windows::process::CommandExt;
    let mut command = Command::new(program);
    for argument in args {
        if argument.starts_with("_?=") || argument.starts_with("/D=") {
            command.raw_arg(argument);
        } else {
            command.arg(argument);
        }
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(File::create(&output).unwrap())
        .stderr(File::create(error).unwrap())
        .spawn()
        .unwrap();
    // NSIS wraps the product's 600-second install request plus extraction/hooks.
    // Keep CLI and uninstall requests on their existing watchdog.
    let seconds = if program
        .file_name()
        .is_some_and(|name| name == "installer.exe")
    {
        720
    } else {
        150
    };
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(
                status.success(),
                success,
                "unexpected native result for {phase}: {status}"
            );
            return std::fs::read(output).unwrap();
        }
        if Instant::now() >= deadline {
            // Only our still-held installer/CLI child is stopped. Keep its native
            // installation and evidence for the normal product recovery path.
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("native phase {phase} exceeded its whole-process watchdog");
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn cli(program: &Path, args: &[&str], evidence: &Path, phase: &str) -> Value {
    let args: Vec<_> = args.iter().copied().chain(["--output", "json"]).collect();
    let envelope: Value = serde_json::from_slice(&run(program, &args, evidence, phase)).unwrap();
    assert_eq!(envelope["status"], "succeeded");
    assert!(envelope["error"].is_null());
    envelope["data"].clone()
}

fn console_ready(cli_path: &Path, evidence: &Path, phase: &str) {
    let system = cli(
        cli_path,
        &["system", "status"],
        evidence,
        &format!("{phase}-system"),
    );
    assert_eq!(system["gateway"], "ready");
    let console = cli(
        cli_path,
        &["console", "status"],
        evidence,
        &format!("{phase}-console"),
    );
    assert_eq!(console["desired"]["mode"], "loopback");
    assert!(console["failure_code"].is_null());
    let address: SocketAddr = console["applied_address"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(address.ip().is_loopback());
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .write_all(
            format!("GET / HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .unwrap();
    let mut reply = String::new();
    stream.take(128 * 1024).read_to_string(&mut reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 200 "));
    assert!(
        reply
            .to_ascii_lowercase()
            .contains("content-security-policy:")
    );
    assert!(
        reply.contains("/assets/"),
        "actual packaged console assets must be served"
    );
}

#[test]
#[ignore = "requires the verified NSIS bundle and a fresh existing standard-user session"]
fn actual_nsis_silent_install_repeat_console_and_uninstall_preserve_data() {
    nsis_lifecycle(false);
}

#[test]
#[ignore = "requires the verified NSIS bundle and a fresh existing standard-user session"]
fn actual_nsis_custom_directory_repair_update_and_uninstall() {
    nsis_lifecycle(true);
}

fn nsis_lifecycle(custom: bool) {
    standard_session();
    let layout = StandaloneLayout::from_environment().unwrap();
    let default = shell_location::default_root().unwrap();
    let programs = if custom {
        std::env::var_os("HIROUTE_VALIDATION_DESKTOP_PARENT")
            .map(PathBuf::from)
            .unwrap_or_else(|| background::local_app_data().unwrap())
            .join("HiRoute 测试")
    } else {
        default.clone()
    };
    assert!(shell_location::registered_root().unwrap().is_none());
    for path in [
        &default,
        &programs,
        &layout.data_root,
        &layout.state_root,
        &layout.home.join(".local/bin/hiroute.exe"),
    ] {
        assert!(
            !path.exists(),
            "native NSIS case must not adopt an existing profile"
        );
    }
    let bundle = PathBuf::from(std::env::var_os("HIROUTE_VALIDATION_PRODUCT_BIN_DIR").unwrap());
    let evidence = PathBuf::from(std::env::var_os("HIROUTE_VALIDATION_EVIDENCE_DIR").unwrap());
    let installer = bundle.join("installer.exe");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(bundle.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["installer_format"], "nsis");
    assert_eq!(
        manifest["files"]["installer.exe"]["sha256"],
        hash(&installer)
    );
    let revision = manifest["source_revision"].as_str().unwrap();
    assert_eq!(Some(revision), option_env!("HIROUTE_BUILD_SOURCE_SHA"));
    let shell_sha256 = manifest["shell_sha256"].as_str().unwrap();
    assert_eq!(shell_sha256.len(), 64);

    if custom {
        run_expecting(
            &installer,
            &["/S", "/D=relative"],
            &evidence,
            "invalid-directory-argument",
            false,
        );
        assert!(!default.exists());
        assert!(shell_location::registered_root().unwrap().is_none());
        let occupied = tempfile::tempdir_in(programs.parent().unwrap()).unwrap();
        let invalid = occupied.path().join("Hi?Route");
        run_expecting(
            &installer,
            &["/S", &format!("/D={}", invalid.display())],
            &evidence,
            "altered-directory-argument",
            false,
        );
        assert!(!occupied.path().join("HiRoute").exists());
        assert!(!default.exists());
        assert!(shell_location::registered_root().unwrap().is_none());
        let occupied_programs = occupied.path().join("HiRoute");
        hiroute_platform::windows::security::create_owner_only_directory(&occupied_programs)
            .unwrap();
        let foreign = occupied_programs.join("uninstall.exe");
        std::fs::write(&foreign, b"belongs to another application").unwrap();
        run_expecting(
            &installer,
            &["/S", &format!("/D={}", occupied_programs.display())],
            &evidence,
            "occupied-directory",
            false,
        );
        assert_eq!(
            std::fs::read(foreign).unwrap(),
            b"belongs to another application"
        );
        run(
            &installer,
            &["/S", &format!("/D={}", programs.display())],
            &evidence,
            "install",
        );
        assert!(!default.exists());
        run_expecting(
            &installer,
            &["/S", &format!("/D={}", default.display())],
            &evidence,
            "conflicting-repair",
            false,
        );
        assert!(!default.exists());
    } else {
        run(&installer, &["/S"], &evidence, "install");
    }
    assert_eq!(
        shell_location::registered_root().unwrap().as_ref(),
        Some(&programs)
    );
    if custom {
        let retained = tempfile::tempdir_in(programs.parent().unwrap()).unwrap();
        std::fs::rename(&programs, retained.path().join("desktop")).unwrap();
        assert!(!programs.exists());
        run(&installer, &["/S"], &evidence, "repair-missing-directory");
        assert!(!default.exists());
    }
    assert!(programs.join("hiroute-shell.exe").is_file());
    assert_eq!(hash(&programs.join("hiroute-shell.exe")), shell_sha256);
    assert!(!programs.starts_with(&layout.state_root));
    assert!(!layout.state_root.starts_with(&programs));
    let cli_path = layout.home.join(".local/bin/hiroute.exe");
    let record: StandaloneInstallRecordV1 =
        serde_json::from_slice(&std::fs::read(&layout.marker_path).unwrap()).unwrap();
    assert!(
        record
            .install_root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(&revision[..12])
    );
    assert_eq!(
        hash(&cli_path),
        manifest["files"]["hiroute.exe"]["sha256"].as_str().unwrap()
    );
    assert_eq!(
        hash(&record.install_root.join("hirouted.exe")),
        manifest["files"]["hirouted.exe"]["sha256"]
            .as_str()
            .unwrap()
    );
    let definition = background::read_definition(&record.service_definition).unwrap();
    assert_eq!(definition.manager, background::Manager::Task);
    assert!(task::active(&definition).unwrap());
    assert!(task::autostart(&definition).unwrap());
    assert!(shell_login::enabled(&programs.join("hiroute-shell.exe")).unwrap());
    let preferences: Value =
        serde_json::from_slice(&std::fs::read(programs.join("shell-install.json")).unwrap())
            .unwrap();
    assert_eq!(preferences["tray_enabled"], true);
    console_ready(&cli_path, &evidence, "fresh");
    run(&installer, &["/S"], &evidence, "repeat");
    assert_eq!(hash(&programs.join("hiroute-shell.exe")), shell_sha256);
    console_ready(&cli_path, &evidence, "repeated");
    hiroute_platform::windows::lifecycle::drain(
        &layout.lifecycle_endpoint(),
        &record.install_root.join("hirouted.exe"),
        Duration::from_secs(30),
    )
    .unwrap();
    run(&installer, &["/P", "/UPDATE"], &evidence, "passive-update");
    assert_eq!(hash(&programs.join("hiroute-shell.exe")), shell_sha256);
    console_ready(&cli_path, &evidence, "passive-updated");

    let sentinel = layout.state_root.join("nsis-preserved-data.txt");
    std::fs::write(&sentinel, b"native NSIS acceptance data").unwrap();
    let unrelated = programs.join("user-notes.txt");
    if custom {
        std::fs::write(&unrelated, b"user file").unwrap();
    }
    let uninstall = evidence.join("uninstall-worker.exe");
    std::fs::copy(programs.join("uninstall.exe"), &uninstall).unwrap();
    assert_eq!(hash(&uninstall), hash(&programs.join("uninstall.exe")));
    run(
        &uninstall,
        &["/S", &format!("_?={}", programs.display())],
        &evidence,
        "uninstall",
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while programs.join("hiroute-shell.exe").exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
    assert!(!programs.join("hiroute-shell.exe").exists());
    assert!(!layout.marker_path.exists());
    assert!(!record.service_definition.exists());
    assert!(!cli_path.exists());
    assert!(!task::active(&definition).unwrap());
    assert!(task::recovery_xml(&definition).is_err());
    assert!(!installed_apps::uninstall_locations("HiRoute").contains(&programs));
    assert!(
        !record.install_root.exists(),
        "uninstall left its versioned package behind"
    );
    assert_eq!(
        std::fs::read(&sentinel).unwrap(),
        b"native NSIS acceptance data"
    );
    assert!(shell_login::configured_binary().unwrap().is_none());
    assert!(shell_location::registered_root().unwrap().is_none());
    if custom {
        assert!(!default.exists());
        assert_eq!(std::fs::read(&unrelated).unwrap(), b"user file");
        std::fs::remove_file(unrelated).unwrap();
        std::fs::remove_dir(&programs).unwrap();
    }
    std::fs::write(
        evidence.join("nsis-result.json"),
        serde_json::to_vec_pretty(&json!({
            "revision":revision,"installer_sha256":hash(&installer),"standard_user":true,
            "silent_defaults":"green","repeat_install":"green","console_gateway_ready":"green",
            "passive_update":"green","versioned_package_removed":"green",
            "uninstall_preserves_data":"green","agent_restore":"not_executed",
            "native_webview":"not_executed","update_recovery":"not_executed",
            "preserved_state":true
        }))
        .unwrap(),
    )
    .unwrap();
}
