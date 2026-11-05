use super::*;
use std::ffi::OsStr;
fn release(id: char) -> WindowsDesktopRelease {
    serde_json::from_value(serde_json::json!({
        "version":"0.5.1","notes":{"zh":"","en":""},"architecture":"x86_64",
        "distribution":"unsigned","filename":"HiRoute-0.5.1-4634fd2e0fd4-windows-x86_64-setup.exe",
        "sha256": id.to_string().repeat(64), "size":4
    }))
    .unwrap()
}
fn payload(stage: &Stage) {
    let parent = security::open_owner_only(&stage.path, true).unwrap();
    let mut file =
        security::create_owner_only_file_at(&parent, OsStr::new("installer.exe")).unwrap();
    file.write_all(b"test").unwrap();
}
#[test]
fn active_stage_survives_cleanup_and_retires_after_lease_release() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("updates");
    let stage = open_in(&root, &release('a'), false).unwrap();
    payload(&stage);
    cleanup_in(&root, None).unwrap();
    assert!(stage.path.join("installer.exe").exists());
    let path = stage.path.clone();
    drop(stage);
    cleanup_in(&root, Some(&"a".repeat(64))).unwrap();
    assert!(path.exists());
    cleanup_in(&root, None).unwrap();
    assert!(!path.exists());
}
#[test]
fn foreign_entry_prevents_retirement_and_same_package_can_retry() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("updates");
    let release = release('b');
    let stage = open_in(&root, &release, true).unwrap();
    payload(&stage);
    let path = stage.path.clone();
    drop(stage);
    let next = open_in(&root, &release, true).unwrap();
    assert!(next.path.join("installer.exe").exists());
    let parent = security::open_owner_only(&path, true).unwrap();
    let mut file = security::create_owner_only_file_at(&parent, OsStr::new("foreign.txt")).unwrap();
    file.write_all(b"keep foreign bytes").unwrap();
    drop(file);
    drop(parent);
    drop(next);
    cleanup_in(&root, None).unwrap();
    assert_eq!(
        fs::read(path.join("foreign.txt")).unwrap(),
        b"keep foreign bytes"
    );
    assert!(path.join("installer.exe").exists());
}
#[test]
fn concurrent_writer_and_live_finisher_cannot_lose_stage_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("updates");
    let release = release('c');
    let writer = open_in(&root, &release, true).unwrap();
    payload(&writer);
    assert_eq!(
        open_in(&root, &release, false).err().unwrap().code,
        "UPGRADE_BUSY"
    );
    let path = writer.path.clone();
    drop(writer);
    let reader = open_in(&root, &release, false).unwrap();
    assert_eq!(
        open_in(&root, &release, true).err().unwrap().code,
        "UPGRADE_BUSY"
    );
    drop(reader);
    let parent = finisher::lock(&path.join("parent.lock")).unwrap();
    parent.try_lock_exclusive().unwrap();
    cleanup_in(&root, None).unwrap();
    assert!(path.join("installer.exe").exists());
    drop(parent);
    cleanup_in(&root, None).unwrap();
    assert!(!path.exists());
}

#[test]
fn locked_installer_keeps_native_error_in_the_persisted_outcome() {
    use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("updates");
    let release = release('d');
    let stage = open_in(&root, &release, false).unwrap();
    payload(&stage);
    let locked = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(stage.path.join("installer.exe"))
        .unwrap();
    let failure =
        finisher::verify_installer(&stage.path.join("installer.exe"), &release).unwrap_err();
    assert_eq!(failure.code, "UPGRADE_STAGE_UNAVAILABLE");
    assert_eq!(failure.detail.step, "open-update-installer");
    assert_eq!(failure.detail.windows_error, Some(32));
    let outcome = temp.path().join("outcome");
    failure.record_at(&outcome, true).unwrap();
    let value: serde_json::Value = finisher::read(&outcome.join("update-outcome.json")).unwrap();
    assert_eq!(value["code"], "UPGRADE_STAGE_UNAVAILABLE");
    assert_eq!(value["diagnostic"]["windows_error"], 32);
    assert_eq!(value["diagnostic"]["step"], "open-update-installer");
    assert!(
        !fs::read_to_string(outcome.join("update-outcome.json"))
            .unwrap()
            .contains(temp.path().to_str().unwrap())
    );
    drop(locked);
}

#[test]
#[ignore = "requires the measured candidate installed in the configured standard-user session"]
fn installed_finisher_failure_preserves_native_diagnostic_and_service() {
    use std::os::windows::process::CommandExt;
    use std::{
        fs::OpenOptions,
        os::windows::fs::OpenOptionsExt,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let layout = StandaloneLayout::from_environment().unwrap();
    let installed =
        hiroute_host_runtime::read_standalone_install_record(&layout.marker_path).unwrap();
    let revision = option_env!("HIROUTE_BUILD_SOURCE_SHA").unwrap();
    assert!(
        installed
            .install_root
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with(&revision[..12])
    );
    assert!(service::available_blocking());
    let programs = background::local_app_data()
        .unwrap()
        .join("Programs/HiRoute");
    let shell = programs.join("hiroute-shell.exe");
    let bundle = PathBuf::from(std::env::var_os("HIROUTE_VALIDATION_PRODUCT_BIN_DIR").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(bundle.join("manifest.json")).unwrap()).unwrap();
    let hash = super::super::super::install_transaction::hash(&shell).unwrap();
    assert_eq!(manifest["shell_sha256"], hash);
    let release = release('e');
    assert!(
        !root().unwrap().join(&release.sha256).exists(),
        "the diagnostic fixture must never adopt an existing stage"
    );
    let stage = open(&release).unwrap();
    payload(&stage);
    let parent = security::open_owner_only(&stage.path, true).unwrap();
    let helper = stage.path.join("finisher.exe");
    let mut input = security::open_identity(&shell, false).unwrap();
    let mut output =
        security::create_owner_only_file_at(&parent, OsStr::new("finisher.exe")).unwrap();
    std::io::copy(&mut input, &mut output).unwrap();
    output.sync_all().unwrap();
    drop(output);
    drop(input);
    drop(parent);
    let generation = "native-sharing-violation";
    finisher::write(
        &stage.path.join("job.json"),
        &serde_json::json!({
            "schema":"hiroute.windows-shell-update-job/v1", "release":release,
            "helper_sha256":hash, "generation":generation
        }),
    )
    .unwrap();
    let data = super::super::super::install_transaction::data().unwrap();
    let outcome_path = data.join("update-outcome.json");
    let old_outcome = fs::read(&outcome_path).ok();
    let locked = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(stage.path.join("installer.exe"))
        .unwrap();
    let mut child = Command::new(&helper)
        .args(["--update-finish", &release.sha256])
        .stdin(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let exit = loop {
        if let Some(exit) = child.try_wait().unwrap() {
            break exit;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("The owned diagnostic finisher exceeded its bounded wait");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(!exit.success());
    let failure_record: serde_json::Value =
        finisher::read(&stage.path.join("failed.json")).unwrap();
    assert_eq!(failure_record["generation"], generation);
    let failure = finisher::child_failure(&failure_record).unwrap();
    assert_eq!(failure.detail.windows_error, Some(32));
    assert_eq!(failure.detail.step, "open-update-installer");
    let outcome: serde_json::Value = finisher::read(&outcome_path).unwrap();
    assert_eq!(outcome["code"], "UPGRADE_STAGE_UNAVAILABLE");
    assert_eq!(outcome["diagnostic"]["windows_error"], 32);
    assert!(service::available_blocking());
    let evidence = PathBuf::from(std::env::var_os("HIROUTE_VALIDATION_EVIDENCE_DIR").unwrap());
    fs::write(
        evidence.join("finisher-failure.json"),
        serde_json::to_vec_pretty(&outcome).unwrap(),
    )
    .unwrap();
    // Restore only the exact outcome written by our child after reading it back.
    let current: serde_json::Value = finisher::read(&outcome_path).unwrap();
    assert_eq!(current, outcome);
    if let Some(bytes) = old_outcome {
        hiroute_platform::durable::write_private_atomic(&outcome_path, &bytes).unwrap();
    } else {
        fs::remove_file(&outcome_path).unwrap();
    }
    drop(locked);
    drop(stage);
    discard(&release).unwrap();
    assert!(!root().unwrap().join(&release.sha256).exists());
}

#[test]
fn verified_released_job_can_retire_but_incomplete_legacy_stage_is_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("updates");
    directory(&root).unwrap();
    let mut release = release('f');
    release.sha256 = format!("{:x}", Sha256::digest(b"test"));
    let path = root.join(&release.sha256);
    let parent = directory(&path).unwrap();
    let mut installer =
        security::create_owner_only_file_at(&parent, OsStr::new("installer.exe")).unwrap();
    installer.write_all(b"test").unwrap();
    drop(installer);
    let mut helper =
        security::create_owner_only_file_at(&parent, OsStr::new("finisher.exe")).unwrap();
    helper.write_all(b"helper").unwrap();
    drop(helper);
    drop(parent);
    cleanup_in(&root, None).unwrap();
    assert!(path.join("installer.exe").exists());
    // V1 Job fields match the released 0.5.1 producer at 4634fd2e0fd4.
    // The fixture's small payloads exercise admission, rather than installer execution.
    finisher::write(
        &path.join("job.json"),
        &serde_json::json!({
            "schema":"hiroute.windows-shell-update-job/v1", "release":release,
            "helper_sha256":format!("{:x}", Sha256::digest(b"helper")), "generation":"1-2"
        }),
    )
    .unwrap();
    cleanup_in(&root, None).unwrap();
    assert!(!path.exists());
}

#[test]
#[ignore = "requires the candidate installed, Desktop quit, and the verified native NSIS bundle"]
fn installed_finisher_runs_actual_nsis_and_retires_successful_stage() {
    use std::os::windows::process::CommandExt;
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let layout = StandaloneLayout::from_environment().unwrap();
    let installed =
        hiroute_host_runtime::read_standalone_install_record(&layout.marker_path).unwrap();
    let revision = option_env!("HIROUTE_BUILD_SOURCE_SHA").unwrap();
    assert!(
        installed
            .install_root
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with(&revision[..12])
    );
    assert!(service::available_blocking());
    let bundle = PathBuf::from(std::env::var_os("HIROUTE_VALIDATION_PRODUCT_BIN_DIR").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(bundle.join("manifest.json")).unwrap()).unwrap();
    let shell = background::local_app_data()
        .unwrap()
        .join("Programs/HiRoute/hiroute-shell.exe");
    let hash = super::super::super::install_transaction::hash(&shell).unwrap();
    assert_eq!(manifest["shell_sha256"], hash);
    let mut release = release('f');
    release.version = installed.version.clone();
    release.filename = format!(
        "HiRoute-{}-{}-windows-x86_64-setup.exe",
        release.version,
        &revision[..12]
    );
    release.sha256 = manifest["files"]["installer.exe"]["sha256"]
        .as_str()
        .unwrap()
        .into();
    release.size = manifest["files"]["installer.exe"]["bytes"]
        .as_u64()
        .unwrap();
    assert!(
        !root().unwrap().join(&release.sha256).exists(),
        "a successful update fixture must start with a fresh stage"
    );
    let stage = open(&release).unwrap();
    let parent = security::open_owner_only(&stage.path, true).unwrap();
    for (name, source) in [
        ("installer.exe", bundle.join("installer.exe")),
        ("finisher.exe", shell),
    ] {
        let mut input = security::open_identity(&source, false).unwrap();
        let mut output = security::create_owner_only_file_at(&parent, OsStr::new(name)).unwrap();
        std::io::copy(&mut input, &mut output).unwrap();
        output.sync_all().unwrap();
    }
    drop(parent);
    finisher::write(
        &stage.path.join("job.json"),
        &serde_json::json!({
            "schema":"hiroute.windows-shell-update-job/v1", "release":release,
            "helper_sha256":hash, "generation":"native-success-replacement"
        }),
    )
    .unwrap();
    finisher::commit(&release).unwrap();
    let sentinel = layout.state_root.join("native-update-preservation.txt");
    assert!(!sentinel.exists());
    fs::write(&sentinel, b"native update preserves application data").unwrap();
    let mut child = Command::new(stage.path.join("finisher.exe"))
        .args(["--update-finish", &release.sha256])
        .stdin(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(720);
    let exit = loop {
        if let Some(exit) = child.try_wait().unwrap() {
            break exit;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("The owned native updater exceeded its whole-process watchdog");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(exit.success());
    let data = super::super::super::install_transaction::data().unwrap();
    let outcome: serde_json::Value = finisher::read(&data.join("update-outcome.json")).unwrap();
    assert_eq!(outcome["status"], "succeeded");
    assert!(service::available_blocking());
    let status = Command::new(layout.home.join(".local/bin/hiroute.exe"))
        .args(["system", "status", "--output", "json"])
        .output()
        .unwrap();
    assert!(status.status.success());
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["data"]["gateway"], "ready");
    assert_eq!(
        fs::read(&sentinel).unwrap(),
        b"native update preserves application data"
    );
    fs::remove_file(sentinel).unwrap();
    let evidence = PathBuf::from(std::env::var_os("HIROUTE_VALIDATION_EVIDENCE_DIR").unwrap());
    fs::write(evidence.join("finisher-success.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "revision":revision,"installer_sha256":release.sha256,"status":"succeeded","gateway":"ready","data_preserved":true,
        "native_entry":"finisher -> NSIS /P /UPDATE"
    })).unwrap()).unwrap();
    drop(stage);
    discard(&release).unwrap();
    assert!(!root().unwrap().join(&release.sha256).exists());
}
