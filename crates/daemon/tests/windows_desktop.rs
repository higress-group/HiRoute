#![cfg(windows)]
//! Bounded NSIS acceptance in the configured, already logged-on standard session.
//! This does not cover WebView interaction, Agent restoration or update recovery.

#[path = "support/windows_account.rs"]
#[allow(dead_code)]
mod windows_account;

use hiroute_host_runtime::{StandaloneInstallRecordV1, StandaloneLayout};
use hiroute_platform::windows::{background, install, shell_login, task};
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
use windows_sys::Win32::Security::{
    CreateWellKnownSid, EqualSid, TOKEN_GROUPS, TokenGroups, WinBuiltinAdministratorsSid,
};

fn standard_session() {
    install::require_x64().unwrap();
    let token = windows_account::process_token(std::process::id());
    let sid = background::sid_string(&windows_account::token_sid(&token)).unwrap();
    assert_ne!(windows_account::assert_standard(&token, &sid), 0);
    let mut admin = [0_usize; 9];
    let mut size = std::mem::size_of_val(&admin) as u32;
    // SAFETY: aligned native SID buffer with its actual writable capacity.
    assert_ne!(
        unsafe {
            CreateWellKnownSid(
                WinBuiltinAdministratorsSid,
                std::ptr::null_mut(),
                admin.as_mut_ptr().cast(),
                &mut size,
            )
        },
        0
    );
    let groups = windows_account::information(&token, TokenGroups);
    // SAFETY: GetTokenInformation returned this aligned, complete TOKEN_GROUPS.
    let groups = unsafe { &*groups.as_ptr().cast::<TOKEN_GROUPS>() };
    for group in
        unsafe { std::slice::from_raw_parts(groups.Groups.as_ptr(), groups.GroupCount as usize) }
    {
        assert_eq!(
            unsafe { EqualSid(group.Sid, admin.as_mut_ptr().cast()) },
            0,
            "standard-user evidence rejects deny-only Administrators membership too"
        );
    }
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
    let output = evidence.join(format!("{phase}.stdout"));
    let error = evidence.join(format!("{phase}.stderr"));
    let mut child = Command::new(program)
        .args(args)
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
            assert!(status.success(), "native phase {phase} failed: {status}");
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
    standard_session();
    let layout = StandaloneLayout::from_environment().unwrap();
    let programs = background::local_app_data()
        .unwrap()
        .join("Programs/HiRoute");
    for path in [
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

    run(&installer, &["/S"], &evidence, "install");
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
    let uninstall = programs.join("uninstall.exe");
    run(&uninstall, &["/S"], &evidence, "uninstall");
    let deadline = Instant::now() + Duration::from_secs(30);
    while programs.join("hiroute-shell.exe").exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
    assert!(!programs.join("hiroute-shell.exe").exists());
    assert!(!layout.marker_path.exists());
    assert!(!record.service_definition.exists());
    assert!(!cli_path.exists());
    assert!(
        !record.install_root.exists(),
        "uninstall left its versioned package behind"
    );
    assert_eq!(
        std::fs::read(&sentinel).unwrap(),
        b"native NSIS acceptance data"
    );
    assert!(!shell_login::enabled(&programs.join("hiroute-shell.exe")).unwrap());
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
