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
        .is_some_and(|name| name == "installer.exe" || name == "baseline.exe")
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
    nsis_lifecycle(Directory::Default);
}

#[test]
#[ignore = "requires the verified NSIS bundle and a fresh existing standard-user session"]
fn actual_nsis_custom_directory_repair_update_and_uninstall() {
    nsis_lifecycle(Directory::Direct);
}

#[test]
#[ignore = "requires the verified NSIS bundle and a fresh existing standard-user session"]
fn actual_nsis_shared_directory_uses_protected_program_and_preserves_foreign_files() {
    nsis_lifecycle(Directory::Shared);
}

#[test]
#[ignore = "requires the verified NSIS bundle with released v0.6.5 and a fresh standard-user session"]
fn actual_nsis_upgrades_released_custom_directory() {
    nsis_lifecycle(Directory::Released);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Directory {
    Default,
    Direct,
    Shared,
    Released,
}

fn saved_packages(folder: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|entry| {
            entry
                .extension()
                .is_some_and(|ext| ext == "hiroute-package")
        })
        .collect()
}

fn nsis_lifecycle(directory: Directory) {
    standard_session();
    let layout = StandaloneLayout::from_environment().unwrap();
    let default = shell_location::default_root().unwrap();
    let selection = if directory != Directory::Default {
        std::env::var_os("HIROUTE_VALIDATION_DESKTOP_PARENT")
            .map(PathBuf::from)
            .unwrap_or_else(|| background::local_app_data().unwrap())
            .join("HiRoute 测试")
    } else {
        default.clone()
    };
    let programs = if matches!(directory, Directory::Direct | Directory::Released) {
        selection.clone()
    } else {
        default.clone()
    };
    assert!(shell_location::registered_root().unwrap().is_none());
    for path in [
        &default,
        &programs,
        &selection,
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

    if directory == Directory::Direct {
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
        // Ordinary creation inherits the ACL. Keep unrelated content in the
        // chosen root, including across subsequent maintenance.
        std::fs::create_dir(&selection).unwrap();
        std::fs::write(selection.join("user-notes.txt"), b"user file").unwrap();
        run(
            &installer,
            &["/S", &format!("/D={}\\", programs.display())],
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
        assert_eq!(
            std::fs::read(selection.join("user-notes.txt")).unwrap(),
            b"user file"
        );
    } else if directory == Directory::Released {
        let baseline = bundle.join("baseline.exe");
        assert_eq!(
            hash(&baseline),
            "2fa78a011c38142c1c9d05191d33d813b0429519c85e28096723cc6e098f2ad7"
        );
        run(
            &baseline,
            &["/S", &format!("/D={}", selection.display())],
            &evidence,
            "install-released-v0-6-5",
        );
        let cli = layout.home.join(".local/bin/hiroute.exe");
        console_ready(&cli, &evidence, "released-v0-6-5");
        let marker = layout.state_root.join("preserve-on-upgrade.txt");
        std::fs::write(&marker, b"released-profile data").unwrap();
        let foreign_manifest = selection.join("other-program.manifest");
        std::fs::write(&foreign_manifest, b"belongs to another application").unwrap();
        run_expecting(
            &installer,
            &["/S"],
            &evidence,
            "foreign-input-before-upgrade",
            false,
        );
        assert_eq!(
            std::fs::read(&foreign_manifest).unwrap(),
            b"belongs to another application"
        );
        console_ready(&cli, &evidence, "released-after-refusal");
        std::fs::remove_file(foreign_manifest).unwrap();
        run(&installer, &["/S"], &evidence, "upgrade-released");
        assert_eq!(std::fs::read(marker).unwrap(), b"released-profile data");
        assert!(!default.exists());
    } else if directory == Directory::Shared {
        std::fs::create_dir(&selection).unwrap();
        std::fs::write(selection.join("user-notes.txt"), b"user file").unwrap();
        if shell_location::validate_custom(&selection).is_ok() {
            assert!(
                Command::new("icacls")
                    .arg(&selection)
                    .args(["/grant", "*S-1-1-0:(OI)(CI)M"])
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let selected = selection.clone();
        let moved = thread::spawn(move || {
            let saved = selected.join("user-kept-installer");
            let deadline = Instant::now() + Duration::from_secs(120);
            while Instant::now() < deadline {
                if let Some(copy) = saved_packages(&selected).first()
                    && std::fs::rename(copy, &saved).is_ok()
                {
                    return saved;
                }
                thread::sleep(Duration::from_millis(50));
            }
            panic!("the package copy was never published");
        });
        run(
            &installer,
            &["/S", &format!("/D={}", selection.display())],
            &evidence,
            "install-shared",
        );
        let saved = moved.join().unwrap();
        assert_eq!(hash(&saved), hash(&installer));
        assert!(saved_packages(&selection).is_empty());
        run(&installer, &["/S"], &evidence, "repair-moved-package");
        let copies = saved_packages(&selection);
        assert_eq!(copies.len(), 1);
        assert_eq!(hash(&copies[0]), hash(&installer));
        assert_eq!(hash(&saved), hash(&installer));
        assert!(!selection.join("hiroute-shell.exe").exists());
        assert!(!selection.join("uninstall.exe").exists());
    } else {
        run(&installer, &["/S"], &evidence, "install");
    }
    assert_eq!(
        shell_location::registered_root().unwrap().as_ref(),
        Some(&programs)
    );
    if directory == Directory::Direct {
        let retained = tempfile::tempdir_in(programs.parent().unwrap()).unwrap();
        let saved = retained.path().join("desktop");
        std::fs::rename(&programs, &saved).unwrap();
        std::fs::create_dir(&programs).unwrap();
        let replacement = programs.join("foreign.txt");
        std::fs::write(&replacement, b"replacement directory").unwrap();
        run_expecting(
            &installer,
            &["/S"],
            &evidence,
            "refuse-replaced-directory",
            false,
        );
        assert_eq!(
            std::fs::read(&replacement).unwrap(),
            b"replacement directory"
        );
        assert!(!programs.join("hiroute-shell.exe").exists());
        std::fs::remove_file(replacement).unwrap();
        std::fs::remove_dir(&programs).unwrap();
        assert!(!programs.exists());
        let helper = saved.join("hiroute-shell.exe");
        let manifest = saved.join("standalone/package-manifest.json");
        let daemon = saved.join("standalone/hirouted.exe");
        let prepare_args = [
            "--installer-prepare",
            manifest.to_str().unwrap(),
            daemon.to_str().unwrap(),
            selection.to_str().unwrap(),
            installer.to_str().unwrap(),
            "",
        ];
        // A transient failure while preparing recovery must not leave an unbound
        // replacement directory. Release the test's exclusive read handle to retry.
        use std::os::windows::fs::OpenOptionsExt;
        let backups = background::local_app_data()
            .unwrap()
            .join("HiRoute/shell/install-backups");
        let busy = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS)
            .open(backups)
            .unwrap();
        run_expecting(
            &helper,
            &prepare_args,
            &evidence,
            "repair-backups-in-use",
            false,
        );
        assert!(!programs.exists());
        drop(busy);
        run(
            &helper,
            &prepare_args,
            &evidence,
            "prepare-recreated-directory",
        );
        assert!(programs.is_dir());
        run(
            &helper,
            &["--installer-rollback"],
            &evidence,
            "rollback-recreated-directory",
        );
        run(&installer, &["/S"], &evidence, "repair-missing-directory");
        assert!(!default.exists());
        assert_eq!(
            std::fs::read(saved.join("user-notes.txt")).unwrap(),
            b"user file"
        );
    } else if directory == Directory::Shared {
        let saved = selection.with_extension("original");
        std::fs::rename(&selection, &saved).unwrap();
        std::fs::create_dir(&selection).unwrap();
        let foreign = selection.join("foreign.txt");
        std::fs::write(&foreign, b"replacement directory").unwrap();
        run_expecting(
            &installer,
            &["/S"],
            &evidence,
            "refuse-replaced-store",
            false,
        );
        assert_eq!(std::fs::read(&foreign).unwrap(), b"replacement directory");
        assert!(saved_packages(&selection).is_empty());
        std::fs::remove_file(foreign).unwrap();
        std::fs::remove_dir(&selection).unwrap();
        std::fs::rename(saved, &selection).unwrap();
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
    if directory == Directory::Shared {
        let helper = evidence.join("prepare-interrupted.exe");
        std::fs::copy(programs.join("hiroute-shell.exe"), &helper).unwrap();
        assert_eq!(hash(&helper), shell_sha256);
        let standalone = programs.join("standalone");
        std::fs::copy(
            standalone.join("package-manifest.json"),
            evidence.join("shared-manifest.json"),
        )
        .unwrap();
        run(
            &helper,
            &[
                "--installer-prepare",
                standalone.join("package-manifest.json").to_str().unwrap(),
                standalone.join("hirouted.exe").to_str().unwrap(),
                selection.to_str().unwrap(),
                installer.to_str().unwrap(),
                "",
            ],
            &evidence,
            "interrupt-after-package-prepare",
        );
        assert!(
            background::local_app_data()
                .unwrap()
                .join("HiRoute/shell/install-pending.json")
                .is_file()
        );
        assert_eq!(saved_packages(&selection).len(), 2);
    }
    run(&installer, &["/S"], &evidence, "repeat");
    if directory == Directory::Shared {
        assert_eq!(saved_packages(&selection).len(), 1);
        assert!(
            !background::local_app_data()
                .unwrap()
                .join("HiRoute/shell/install-pending.json")
                .exists()
        );
    }
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
    if directory == Directory::Direct {
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
    if directory == Directory::Direct {
        assert!(!default.exists());
        assert_eq!(std::fs::read(&unrelated).unwrap(), b"user file");
        std::fs::remove_file(unrelated).unwrap();
        std::fs::remove_dir(&programs).unwrap();
    }
    if directory == Directory::Shared {
        assert!(saved_packages(&selection).is_empty());
        let helper = evidence.join("prepare-interrupted.exe");
        run(
            &helper,
            &[
                "--installer-prepare",
                evidence.join("shared-manifest.json").to_str().unwrap(),
                bundle.join("hirouted.exe").to_str().unwrap(),
                selection.to_str().unwrap(),
                installer.to_str().unwrap(),
                "",
            ],
            &evidence,
            "prepare-fresh-shared",
        );
        // Model an interrupted NSIS process after InstallLocation is written and
        // before postinstall publishes DisplayVersion. The journal and package are real.
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\HiRoute";
        assert!(
            Command::new("reg.exe")
                .args([
                    "add",
                    key,
                    "/v",
                    "InstallLocation",
                    "/t",
                    "REG_SZ",
                    "/d",
                    &format!("\"{}\"", programs.display()),
                    "/f",
                    "/reg:64"
                ])
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(
            shell_location::registered_root().unwrap().as_ref(),
            Some(&programs)
        );
        run(
            &installer,
            &["/S"],
            &evidence,
            "retry-after-partial-registration",
        );
        assert_eq!(saved_packages(&selection).len(), 1);
        assert_eq!(hash(&saved_packages(&selection)[0]), hash(&installer));
        console_ready(&cli_path, &evidence, "retry-after-registration");
        std::fs::copy(programs.join("uninstall.exe"), &uninstall).unwrap();
        run(
            &uninstall,
            &["/S", &format!("_?={}", programs.display())],
            &evidence,
            "uninstall-retried-shared",
        );
        assert!(saved_packages(&selection).is_empty());
        assert!(shell_location::registered_root().unwrap().is_none());
        assert_eq!(
            std::fs::read(selection.join("user-notes.txt")).unwrap(),
            b"user file"
        );
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
