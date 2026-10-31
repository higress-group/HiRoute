use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(any(target_os = "linux", target_os = "macos", test))]
use std::process::Output;
use std::time::{Duration, Instant};

use hiroute_application_api::{LOCAL_CONTROL_SCHEMA_V2, LocalControlWireRequestV2, MachineStatus};
use hiroute_host_runtime::{StandaloneInstallRecordV1, StandaloneLayout};
use serde::Serialize;
use serde_json::json;

use crate::LocalControlClient;

#[cfg(any(target_os = "linux", target_os = "macos"))]
const SERVICE_LABEL: &str = "ai.hiroute.cli";
const MAX_LOG_BYTES: usize = 256 * 1024;

#[cfg(windows)]
mod restore_agents;
#[cfg(any(windows, test))]
mod restore_plan;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) fn windows_install_hidden(verb: &str, args: &[std::ffi::OsString]) -> u8 {
    windows::install_hidden(verb, args)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceFailure {
    Unavailable,
}

#[derive(Debug, Serialize)]
pub(crate) struct ServiceStatus {
    schema: &'static str,
    manager: &'static str,
    manager_active: bool,
    pub(crate) local_control_ready: bool,
    autostart_enabled: bool,
    runtime_root: PathBuf,
}

pub(crate) fn recovery() -> Result<serde_json::Value, ServiceFailure> {
    let layout = StandaloneLayout::from_environment().map_err(|_| ServiceFailure::Unavailable)?;
    let record = hiroute_host_runtime::StorageRecovery::read_for_failed_startup(
        &layout.storage_root(),
        &layout.diagnostics_root(),
    );
    let instructions = record
        .as_ref()
        .map(|record| recovery_instructions(record, &layout.storage_root()));
    Ok(
        json!({"schema":"hiroute.service-recovery/v1", "recovery":record, "instructions":instructions, "diagnostics_root":layout.diagnostics_root()}),
    )
}

fn recovery_instructions(record: &hiroute_host_runtime::StorageRecovery, storage: &Path) -> String {
    use hiroute_host_runtime::StorageRecoveryAction;
    let mut text = record.operator_guidance().to_owned();
    if let Some(space) = &record.storage_space {
        text.push_str(&format!(
            " Storage volume: approximately {} bytes required, {} bytes available.",
            space.required_bytes, space.available_bytes
        ));
    }
    if matches!(
        record.action,
        StorageRecoveryAction::ResumeCurrent | StorageRecoveryAction::RestoreCompleteGroup
    ) && let (Some(batch), Some(name), Some(parent), Some(leaf)) = (
        record.backup_batch.as_deref(),
        record.backup_name.as_deref(),
        storage.parent(),
        storage.file_name(),
    ) && [batch, name].iter().all(|value| {
        !value.is_empty()
            && value.len() <= 128
            && !value.contains("..")
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    }) {
        // The caller obtained this disposition through the storage owner's
        // complete batch verification; formatting does not infer authority.
        let mut directory = leaf.to_os_string();
        directory.push(".upgrade-backups");
        let source = parent.join(directory).join(name).join("source");
        text.push_str(&format!(" Verified original batch: {batch}. Recovery source directory: {}. Read RECOVERY.md (or 恢复说明.md), then PROGRAM-LOCATIONS.md where present. Use only this batch.",source.display()));
    }
    text
}

#[cfg(test)]
mod recovery_text_tests {
    use super::*;
    use hiroute_host_runtime::{StorageRecovery, StorageRecoveryAction, StorageRecoveryCause};

    #[test]
    fn offline_recovery_names_the_original_batch_and_exact_source_directory() {
        let directory = tempfile::tempdir().unwrap();
        let storage = directory.path().join("custom-storage");
        let mut report = StorageRecovery::new(
            StorageRecoveryCause::MigrationFailed,
            StorageRecoveryAction::RestoreCompleteGroup,
        );
        report.backup_batch = Some("set_original".into());
        report.backup_name = Some("observation-format-5".into());
        let text = recovery_instructions(&report, &storage);
        assert!(text.contains("Verified original batch: set_original"));
        assert!(
            text.contains(
                &directory
                    .path()
                    .join("custom-storage.upgrade-backups")
                    .join("observation-format-5")
                    .join("source")
                    .display()
                    .to_string()
            )
        );
        assert!(text.contains("RECOVERY.md"));
        assert!(text.contains("PROGRAM-LOCATIONS.md"));
        report.action = StorageRecoveryAction::PreserveAndDiagnose;
        assert!(!recovery_instructions(&report, &storage).contains("Recovery source directory:"));
        report.action = StorageRecoveryAction::RestoreCompleteGroup;
        report.backup_name = Some("../foreign".into());
        assert!(!recovery_instructions(&report, &storage).contains("Recovery source directory:"));
    }
}

pub(crate) fn status() -> Result<ServiceStatus, ServiceFailure> {
    let (layout, _) = installed()?;
    Ok(ServiceStatus {
        schema: "hiroute.standalone-service-status/v1",
        manager: manager_name()?,
        manager_active: manager_active(&layout),
        local_control_ready: control_ready(Duration::from_millis(750)),
        autostart_enabled: autostart_status(&layout)?,
        runtime_root: layout.runtime_root,
    })
}

pub(crate) fn start(timeout: Duration) -> Result<ServiceStatus, ServiceFailure> {
    #[cfg(windows)]
    let deadline = Instant::now() + timeout;
    let (layout, _) = installed()?;
    ensure_not_maintaining(&layout)?;
    if !manager_active(&layout) {
        hiroute_host_runtime::StorageRecovery::clear(&layout.storage_root())
            .map_err(|_| ServiceFailure::Unavailable)?;
        capture_proxy_environment(&layout)?;
    }
    manager_start(&layout)?;
    #[cfg(windows)]
    wait_ready(deadline.saturating_duration_since(Instant::now()))?;
    #[cfg(not(windows))]
    if let Err(error) = wait_ready(timeout) {
        #[cfg(target_os = "linux")]
        stop_failed_storage_start(&layout);
        return Err(error);
    }
    status()
}

pub(crate) fn stop(timeout: Duration) -> Result<ServiceStatus, ServiceFailure> {
    let (layout, _) = installed()?;
    #[cfg(windows)]
    windows::stop(&layout, timeout)?;
    #[cfg(not(windows))]
    {
        manager_stop(&layout)?;
        wait_stopped(&layout, timeout)?;
    }
    status()
}

pub(crate) fn restart(timeout: Duration) -> Result<ServiceStatus, ServiceFailure> {
    #[cfg(windows)]
    let deadline = Instant::now() + timeout;
    let (layout, _) = installed()?;
    ensure_not_maintaining(&layout)?;
    hiroute_host_runtime::StorageRecovery::clear(&layout.storage_root())
        .map_err(|_| ServiceFailure::Unavailable)?;
    capture_proxy_environment(&layout)?;
    #[cfg(windows)]
    windows::restart(&layout, timeout)?;
    #[cfg(not(windows))]
    manager_restart(&layout)?;
    #[cfg(windows)]
    wait_ready(deadline.saturating_duration_since(Instant::now()))?;
    #[cfg(not(windows))]
    if let Err(error) = wait_ready(timeout) {
        #[cfg(target_os = "linux")]
        stop_failed_storage_start(&layout);
        return Err(error);
    }
    status()
}

fn capture_proxy_environment(layout: &StandaloneLayout) -> Result<(), ServiceFailure> {
    hiroute_host_runtime::ServiceProxyEnvironment::capture(std::env::vars_os())
        .and_then(|environment| environment.store(&layout.home))
        .map_err(|_| ServiceFailure::Unavailable)
}

pub(crate) fn set_autostart(enable: bool) -> Result<ServiceStatus, ServiceFailure> {
    let (_layout, _) = installed()?;
    #[cfg(target_os = "linux")]
    {
        let verb = if enable { "enable" } else { "disable" };
        run_checked(Command::new("systemctl").args(["--user", verb, SERVICE_LABEL]))?;
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::symlink;
        let template = launchd_template(&_layout);
        let target = _layout
            .home
            .join("Library/LaunchAgents/ai.hiroute.cli.plist");
        if enable {
            let parent = target.parent().ok_or(ServiceFailure::Unavailable)?;
            std::fs::create_dir_all(parent).map_err(|_| ServiceFailure::Unavailable)?;
            match std::fs::symlink_metadata(&target) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    if std::fs::read_link(&target).ok().as_deref() != Some(template.as_path()) {
                        return Err(ServiceFailure::Unavailable);
                    }
                }
                Ok(_) => return Err(ServiceFailure::Unavailable),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    symlink(&template, &target).map_err(|_| ServiceFailure::Unavailable)?;
                }
                Err(_) => return Err(ServiceFailure::Unavailable),
            }
        } else {
            match std::fs::symlink_metadata(&target) {
                Ok(metadata)
                    if metadata.file_type().is_symlink()
                        && std::fs::read_link(&target).ok().as_deref()
                            == Some(template.as_path()) =>
                {
                    std::fs::remove_file(target).map_err(|_| ServiceFailure::Unavailable)?;
                }
                Ok(_) => return Err(ServiceFailure::Unavailable),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(ServiceFailure::Unavailable),
            }
        }
    }
    #[cfg(windows)]
    windows::set_autostart(&_layout, enable)?;
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (enable, _layout);
        return Err(ServiceFailure::Unavailable);
    }
    status()
}

pub(crate) fn logs() -> Result<serde_json::Value, ServiceFailure> {
    let (layout, _) = installed()?;
    #[cfg(target_os = "linux")]
    let output = run_capture(Command::new("journalctl").args([
        "--user",
        "--unit",
        SERVICE_LABEL,
        "--no-pager",
        "--lines",
        "200",
        "--output",
        "short-iso",
    ]))?;
    #[cfg(target_os = "macos")]
    {
        let stdout = bounded_file(&layout.state_root.join("logs/hirouted.log"));
        let stderr = bounded_file(&layout.state_root.join("logs/hirouted-error.log"));
        return Ok(json!({
            "schema": "hiroute.standalone-service-logs/v1",
            "manager": "launchd-user",
            "stdout": stdout,
            "stderr": stderr,
        }));
    };
    #[cfg(windows)]
    return windows::logs(&layout);
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    return Err(ServiceFailure::Unavailable);
    #[cfg(target_os = "linux")]
    Ok(json!({
        "schema": "hiroute.standalone-service-logs/v1",
        "manager": "systemd-user",
        "output": bounded_output(&output),
        "diagnostics_root": layout.diagnostics_root(),
    }))
}

pub(crate) fn doctor() -> serde_json::Value {
    let layout = StandaloneLayout::from_environment();
    let mut checks = Vec::new();
    let mut healthy = true;
    match layout {
        Ok(layout) => {
            let marker = hiroute_host_runtime::read_standalone_install_record(&layout.marker_path);
            check(&mut checks, &mut healthy, "install_marker", marker.is_ok());
            if let Ok(record) = marker {
                check(
                    &mut checks,
                    &mut healthy,
                    "hiroute_binary",
                    executable(&hiroute_host_runtime::standalone_cli_binary(
                        &record.install_root,
                    )),
                );
                check(
                    &mut checks,
                    &mut healthy,
                    "hirouted_binary",
                    executable(&hiroute_host_runtime::standalone_daemon_binary(
                        &record.install_root,
                    )),
                );
            }
            check(
                &mut checks,
                &mut healthy,
                "service_definition",
                service_definition(&layout).is_file(),
            );
            check(
                &mut checks,
                &mut healthy,
                "local_control",
                control_ready(Duration::from_millis(750)),
            );
            json!({
                "schema": "hiroute.standalone-service-doctor/v1",
                "healthy": healthy,
                "checks": checks,
                "runtime_root": layout.runtime_root,
                "diagnostics_root": layout.diagnostics_root(),
            })
        }
        Err(_) => json!({
            "schema": "hiroute.standalone-service-doctor/v1",
            "healthy": false,
            "checks": [{"name":"layout", "ok":false}],
        }),
    }
}

pub(crate) fn foreground_run() -> u8 {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let Ok((layout, record)) = installed() else {
            eprintln!("standalone installation is unavailable");
            return 6;
        };
        if capture_proxy_environment(&layout).is_err() {
            eprintln!("standalone proxy environment is unavailable");
            return 6;
        }
        let daemon = record.install_root.join("hirouted");
        if !executable(&daemon) {
            eprintln!("standalone daemon is unavailable");
            return 6;
        }
        let mut command = Command::new(daemon);
        command.args(["--role", "all", "--standalone"]);
        if let (Some(binary), Some(digest)) = (record.cpa_binary, record.cpa_sha256) {
            command
                .arg("--cpa-binary")
                .arg(binary)
                .arg("--cpa-sha256")
                .arg(digest);
        }
        let error = command.exec();
        eprintln!("standalone daemon exec failed: {error}");
        6
    }
    #[cfg(windows)]
    {
        let Ok((layout, record)) = installed() else {
            return 6;
        };
        if ensure_not_maintaining(&layout).is_err() {
            return 6;
        }
        if capture_proxy_environment(&layout).is_err() {
            return 6;
        }
        let daemon = hiroute_host_runtime::standalone_daemon_binary(&record.install_root);
        let Ok(wait) = hiroute_platform::windows::console::ForegroundWaitGuard::install() else {
            eprintln!("standalone foreground console wait unavailable");
            return 6;
        };
        let result = Command::new(daemon)
            .args([
                "--role",
                "all",
                "--standalone",
                "--foreground-parent-console",
            ])
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status();
        if wait.finish().is_err() {
            eprintln!("standalone foreground console wait unavailable");
            return 6;
        }
        match result {
            Ok(status) => status
                .code()
                .and_then(|code| u8::try_from(code).ok())
                .unwrap_or(6),
            Err(error) => {
                eprintln!("standalone daemon launch failed: {error}");
                6
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        eprintln!("standalone mode is unsupported on this platform");
        6
    }
}

fn installed() -> Result<(StandaloneLayout, StandaloneInstallRecordV1), ServiceFailure> {
    let layout = StandaloneLayout::from_environment().map_err(|_| ServiceFailure::Unavailable)?;
    let record = hiroute_host_runtime::read_standalone_install_record(&layout.marker_path)
        .map_err(|_| ServiceFailure::Unavailable)?;
    Ok((layout, record))
}

fn ensure_not_maintaining(layout: &StandaloneLayout) -> Result<(), ServiceFailure> {
    if hiroute_host_runtime::standalone_maintenance_pending(layout)
        .map_err(|_| ServiceFailure::Unavailable)?
    {
        return Err(ServiceFailure::Unavailable);
    }
    Ok(())
}

fn wait_ready(timeout: Duration) -> Result<(), ServiceFailure> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if control_ready(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(750)),
        ) {
            return Ok(());
        }
        if StandaloneLayout::from_environment().is_ok_and(|layout| {
            hiroute_host_runtime::StorageRecovery::read(&layout.storage_root())
                .is_ok_and(|report| report.is_some())
        }) {
            return Err(ServiceFailure::Unavailable);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(ServiceFailure::Unavailable)
}

#[cfg(target_os = "linux")]
fn stop_failed_storage_start(layout: &StandaloneLayout) {
    if hiroute_host_runtime::StorageRecovery::read(&layout.storage_root())
        .is_ok_and(|report| report.is_some())
    {
        // Deterministic storage admission needs correction, not an automatic
        // manager restart loop. Stop only this verified installation's manager.
        let _ = run_checked(Command::new("systemctl").args([
            "--user",
            "--no-block",
            "stop",
            SERVICE_LABEL,
        ]));
    }
}

#[cfg(not(windows))]
fn wait_stopped(layout: &StandaloneLayout, timeout: Duration) -> Result<(), ServiceFailure> {
    let endpoint = hiroute_client_core::LocalEndpoint::from_runtime_root(&layout.runtime_root);
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if std::fs::symlink_metadata(endpoint.path()).is_err() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(ServiceFailure::Unavailable)
}

fn control_ready(timeout: Duration) -> bool {
    let request = LocalControlWireRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: format!("service-status-{}", std::process::id()),
        operation_id: "GetSystemStatus".into(),
        payload: json!({}),
        protected_grant: None,
    };
    LocalControlClient::cli_from_environment()
        .map(|client| client.with_timeout(timeout))
        .and_then(|client| client.call(request))
        .is_ok_and(|response| {
            response.status == MachineStatus::Succeeded
                && response.error.is_none()
                && response
                    .data
                    .is_some_and(|data| data["daemon"] == "role_all" && data["gateway"] == "ready")
        })
}

#[cfg(target_os = "linux")]
fn manager_start(_layout: &StandaloneLayout) -> Result<(), ServiceFailure> {
    run_checked(Command::new("systemctl").args(["--user", "start", SERVICE_LABEL]))
}

#[cfg(target_os = "linux")]
fn manager_stop(_layout: &StandaloneLayout) -> Result<(), ServiceFailure> {
    run_checked(Command::new("systemctl").args(["--user", "stop", SERVICE_LABEL]))
}

#[cfg(target_os = "linux")]
fn manager_restart(_layout: &StandaloneLayout) -> Result<(), ServiceFailure> {
    run_checked(Command::new("systemctl").args(["--user", "restart", SERVICE_LABEL]))
}

#[cfg(target_os = "linux")]
fn manager_active(_layout: &StandaloneLayout) -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", SERVICE_LABEL])
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "linux")]
fn autostart_status(_layout: &StandaloneLayout) -> Result<bool, ServiceFailure> {
    Ok(Command::new("systemctl")
        .args(["--user", "is-enabled", "--quiet", SERVICE_LABEL])
        .status()
        .is_ok_and(|status| status.success()))
}

#[cfg(target_os = "linux")]
fn manager_name() -> Result<&'static str, ServiceFailure> {
    Ok("systemd-user")
}

#[cfg(target_os = "linux")]
fn service_definition(layout: &StandaloneLayout) -> PathBuf {
    layout
        .home
        .join(".config/systemd/user/ai.hiroute.cli.service")
}

#[cfg(target_os = "macos")]
fn manager_start(layout: &StandaloneLayout) -> Result<(), ServiceFailure> {
    let domain = format!("gui/{}", nix::unistd::geteuid().as_raw());
    if !manager_loaded(layout) {
        let template = launchd_template(layout);
        run_checked(Command::new("/bin/launchctl").args([
            "bootstrap",
            &domain,
            template.to_str().ok_or(ServiceFailure::Unavailable)?,
        ]))?;
    }
    run_checked(Command::new("/bin/launchctl").args([
        "kickstart",
        "-k",
        &format!("{domain}/{SERVICE_LABEL}"),
    ]))
}

#[cfg(target_os = "macos")]
fn manager_stop(_layout: &StandaloneLayout) -> Result<(), ServiceFailure> {
    if !manager_loaded(_layout) {
        return Ok(());
    }
    let service = format!("gui/{}/{}", nix::unistd::geteuid().as_raw(), SERVICE_LABEL);
    run_checked(Command::new("/bin/launchctl").args(["bootout", &service]))
}

#[cfg(target_os = "macos")]
fn manager_restart(layout: &StandaloneLayout) -> Result<(), ServiceFailure> {
    manager_start(layout)
}

#[cfg(target_os = "macos")]
fn manager_active(_layout: &StandaloneLayout) -> bool {
    Command::new("/bin/launchctl")
        .args([
            "print",
            &format!("gui/{}/{}", nix::unistd::geteuid().as_raw(), SERVICE_LABEL),
        ])
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok_and(|output| launchd_process_running(&output))
}

// A registered RunAtLoad=false job is loaded even when it has no process.
// Keep registration separate from the activity check used to capture proxy policy.
#[cfg(target_os = "macos")]
fn manager_loaded(_layout: &StandaloneLayout) -> bool {
    Command::new("/bin/launchctl")
        .args([
            "print",
            &format!("gui/{}/{}", nix::unistd::geteuid().as_raw(), SERVICE_LABEL),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(any(target_os = "macos", test))]
fn launchd_process_running(output: &Output) -> bool {
    output.status.success()
        && String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            line.strip_prefix("\tpid = ")
                .and_then(|pid| pid.parse::<u32>().ok())
                .is_some_and(|pid| pid > 0)
        })
}

#[cfg(target_os = "macos")]
fn autostart_status(layout: &StandaloneLayout) -> Result<bool, ServiceFailure> {
    let target = layout
        .home
        .join("Library/LaunchAgents/ai.hiroute.cli.plist");
    Ok(std::fs::symlink_metadata(&target).is_ok_and(|metadata| {
        metadata.file_type().is_symlink()
            && std::fs::read_link(target).ok().as_deref()
                == Some(launchd_template(layout).as_path())
    }))
}

#[cfg(target_os = "macos")]
fn manager_name() -> Result<&'static str, ServiceFailure> {
    Ok("launchd-user")
}

#[cfg(target_os = "macos")]
fn launchd_template(layout: &StandaloneLayout) -> PathBuf {
    layout.data_root.join("service/ai.hiroute.cli.plist")
}

#[cfg(target_os = "macos")]
fn service_definition(layout: &StandaloneLayout) -> PathBuf {
    launchd_template(layout)
}

#[cfg(windows)]
fn manager_start(layout: &StandaloneLayout) -> Result<(), ServiceFailure> {
    windows::start(layout)
}
#[cfg(windows)]
fn manager_active(layout: &StandaloneLayout) -> bool {
    windows::active(layout).unwrap_or(false)
}
#[cfg(windows)]
fn autostart_status(layout: &StandaloneLayout) -> Result<bool, ServiceFailure> {
    windows::autostart(layout)
}
#[cfg(windows)]
fn manager_name() -> Result<&'static str, ServiceFailure> {
    windows::manager_name(&installed()?.0)
}
#[cfg(windows)]
fn service_definition(layout: &StandaloneLayout) -> PathBuf {
    windows::definition_path(layout)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn manager_start(_layout: &StandaloneLayout) -> Result<(), ServiceFailure> {
    Err(ServiceFailure::Unavailable)
}
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn manager_stop(_layout: &StandaloneLayout) -> Result<(), ServiceFailure> {
    Err(ServiceFailure::Unavailable)
}
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn manager_restart(_layout: &StandaloneLayout) -> Result<(), ServiceFailure> {
    Err(ServiceFailure::Unavailable)
}
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn manager_active(_layout: &StandaloneLayout) -> bool {
    false
}
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn autostart_status(_layout: &StandaloneLayout) -> Result<bool, ServiceFailure> {
    Err(ServiceFailure::Unavailable)
}
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn manager_name() -> Result<&'static str, ServiceFailure> {
    Err(ServiceFailure::Unavailable)
}
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn service_definition(_layout: &StandaloneLayout) -> PathBuf {
    PathBuf::new()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run_checked(command: &mut Command) -> Result<(), ServiceFailure> {
    command
        .status()
        .map_err(|_| ServiceFailure::Unavailable)?
        .success()
        .then_some(())
        .ok_or(ServiceFailure::Unavailable)
}

#[cfg(target_os = "linux")]
fn run_capture(command: &mut Command) -> Result<Output, ServiceFailure> {
    command.output().map_err(|_| ServiceFailure::Unavailable)
}

#[cfg(any(target_os = "linux", test))]
fn bounded_output(output: &Output) -> String {
    let bytes = if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };
    String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(MAX_LOG_BYTES)..]).into_owned()
}

#[cfg(target_os = "macos")]
fn bounded_file(path: &Path) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let read = || -> std::io::Result<String> {
        let mut file = std::fs::File::open(path)?;
        let length = file.metadata()?.len();
        file.seek(SeekFrom::Start(length.saturating_sub(MAX_LOG_BYTES as u64)))?;
        let mut bytes = Vec::new();
        file.take(MAX_LOG_BYTES as u64).read_to_end(&mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    };
    read().unwrap_or_default()
}

#[allow(
    dead_code,
    reason = "Retained unpublished manager adapter; no public installation entry"
)]
pub(crate) fn install_manager(
    manager: &str,
    password_stdin: bool,
) -> Result<serde_json::Value, ServiceFailure> {
    #[cfg(windows)]
    {
        return windows::install(manager, password_stdin);
    }
    #[cfg(not(windows))]
    {
        let _ = (manager, password_stdin);
        Err(ServiceFailure::Unavailable)
    }
}

#[cfg(windows)]
pub(crate) fn windows_scm_owner_helper_verb() -> &'static str {
    windows::scm_owner_helper_verb()
}
#[cfg(windows)]
pub(crate) fn windows_scm_owner_helper(args: &[std::ffi::OsString]) -> u8 {
    windows::scm_owner_helper(args)
}

#[cfg(windows)]
pub(crate) fn windows_background_finalize_verb() -> &'static str {
    windows::FINALIZE_VERB
}
#[cfg(windows)]
pub(crate) fn windows_background_finalize(args: &[std::ffi::OsString]) -> u8 {
    windows::finalize_hidden(args)
}

#[allow(
    dead_code,
    reason = "Retained unpublished manager adapter; no public installation entry"
)]
pub(crate) fn uninstall_manager(timeout: Duration) -> Result<serde_json::Value, ServiceFailure> {
    #[cfg(windows)]
    {
        return windows::uninstall(timeout);
    }
    #[cfg(not(windows))]
    {
        let _ = timeout;
        Err(ServiceFailure::Unavailable)
    }
}

pub(crate) fn windows_layout() -> Result<serde_json::Value, ServiceFailure> {
    #[cfg(windows)]
    {
        let layout =
            StandaloneLayout::from_environment().map_err(|_| ServiceFailure::Unavailable)?;
        return Ok(
            json!({"schema":"hiroute.windows-install-layout/v1", "home":layout.home,
            "state_root":layout.state_root, "data_root":layout.data_root,"runtime_root":layout.runtime_root,
            "marker_path":layout.marker_path, "target":hiroute_host_runtime::standalone_host_target(),
            "service_definition":windows::definition_path(&layout)}),
        );
    }
    #[cfg(not(windows))]
    Err(ServiceFailure::Unavailable)
}

#[allow(
    dead_code,
    reason = "Retained unpublished manager adapter; no public installation entry"
)]
pub(crate) fn prepare_windows_install() -> Result<serde_json::Value, ServiceFailure> {
    #[cfg(windows)]
    {
        let layout =
            StandaloneLayout::from_environment().map_err(|_| ServiceFailure::Unavailable)?;
        for path in [
            layout.data_root.clone(),
            layout.data_root.join("service"),
            layout.home.join(".local/lib/hiroute"),
            layout.home.join(".local/bin"),
            layout.home.join(".agents/skills/hiroute-management"),
            layout.home.join(".claude/skills/hiroute-management"),
        ] {
            hiroute_platform::windows::security::ensure_private_directory(&path)
                .map_err(|_| ServiceFailure::Unavailable)?;
        }
        return windows_layout();
    }
    #[cfg(not(windows))]
    Err(ServiceFailure::Unavailable)
}

pub(crate) fn describe_windows_manager(
    manager: &str,
    daemon: &Path,
) -> Result<serde_json::Value, ServiceFailure> {
    #[cfg(windows)]
    {
        use hiroute_platform::windows::background::{Definition, Manager};
        let layout =
            StandaloneLayout::from_environment().map_err(|_| ServiceFailure::Unavailable)?;
        if !daemon.starts_with(layout.home.join(".local/lib/hiroute")) {
            return Err(ServiceFailure::Unavailable);
        }
        let manager = match manager {
            "task" => Manager::Task,
            "scm" => Manager::Scm,
            _ => return Err(ServiceFailure::Unavailable),
        };
        return serde_json::to_value(
            Definition::new(manager, daemon.to_owned()).map_err(|_| ServiceFailure::Unavailable)?,
        )
        .map_err(|_| ServiceFailure::Unavailable);
    }
    #[cfg(not(windows))]
    {
        let _ = (manager, daemon);
        Err(ServiceFailure::Unavailable)
    }
}

#[allow(
    dead_code,
    reason = "Retained unpublished manager adapter; no public installation entry"
)]
pub(crate) fn publish_install_record() -> Result<serde_json::Value, ServiceFailure> {
    #[cfg(windows)]
    {
        let bytes = bounded_install_stdin()?;
        return windows::publish_record(&bytes);
    }
    #[cfg(not(windows))]
    Err(ServiceFailure::Unavailable)
}

#[allow(
    dead_code,
    reason = "Retained unpublished manager adapter; no public installation entry"
)]
pub(crate) fn write_private_record(path: &Path) -> Result<serde_json::Value, ServiceFailure> {
    #[cfg(windows)]
    {
        let bytes = bounded_install_stdin()?;
        windows::private_record(path, &bytes)?;
        return Ok(json!({"schema":"hiroute.windows-background-install/v1","written":true}));
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(ServiceFailure::Unavailable)
    }
}

#[cfg(windows)]
fn bounded_install_stdin() -> Result<Vec<u8>, ServiceFailure> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::io::stdin()
        .lock()
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ServiceFailure::Unavailable)?;
    if bytes.is_empty()
        || bytes.len() > 64 * 1024
        || serde_json::from_slice::<serde_json::Value>(&bytes).is_err()
    {
        return Err(ServiceFailure::Unavailable);
    }
    Ok(bytes)
}

fn executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn check(checks: &mut Vec<serde_json::Value>, healthy: &mut bool, name: &str, ok: bool) {
    *healthy &= ok;
    checks.push(json!({"name": name, "ok": ok}));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loaded_launchd_job_without_process_is_inactive_for_proxy_capture() {
        for stdout in [
            "gui/501/ai.hiroute.cli = {\n\tstate = not running\n}\n",
            "gui/501/ai.hiroute.cli = {\n\tstate = exited\n\tenvironment = {\n\t\tpid = 42\n\t}\n}\n",
            "gui/501/ai.hiroute.cli = {\n\tpid = 0\n}\n",
        ] {
            assert!(!launchd_process_running(&Output {
                status: success_status(),
                stdout: stdout.as_bytes().to_vec(),
                stderr: Vec::new(),
            }));
        }
        assert!(launchd_process_running(&Output {
            status: success_status(),
            stdout: b"gui/501/ai.hiroute.cli = {\n\tstate = running\n\tpid = 42\n}\n".to_vec(),
            stderr: Vec::new(),
        }));
    }

    #[test]
    fn log_output_is_bounded_from_the_tail() {
        let output = Output {
            status: success_status(),
            stdout: vec![b'x'; MAX_LOG_BYTES + 7],
            stderr: Vec::new(),
        };
        assert_eq!(bounded_output(&output).len(), MAX_LOG_BYTES);
    }

    #[cfg(unix)]
    fn success_status() -> std::process::ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(0)
    }

    #[cfg(windows)]
    fn success_status() -> std::process::ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(0)
    }
}
