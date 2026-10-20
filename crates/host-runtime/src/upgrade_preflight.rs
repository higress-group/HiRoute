//! Read-only candidate preflight. It never starts the service or grants write authority.
use crate::StorageRecovery;
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpgradePreflight {
    pub schema: String,
    pub history_cutover_required: bool,
    pub required_bytes: u64,
    pub available_bytes: u64,
    pub recovery: Option<StorageRecovery>,
}
#[derive(Debug)]
pub struct UpgradePreflightFailure {
    pub code: &'static str,
    pub recovery: Option<Box<StorageRecovery>>,
}
impl std::fmt::Display for UpgradePreflightFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code)?;
        if let Some(recovery) = &self.recovery {
            write!(formatter, ": {}", recovery.operator_guidance())?;
            if let Some(space) = &recovery.storage_space {
                write!(
                    formatter,
                    " Required: {} bytes; available: {} bytes.",
                    space.required_bytes, space.available_bytes
                )?;
            }
        }
        Ok(())
    }
}
impl std::error::Error for UpgradePreflightFailure {}

pub fn run_upgrade_preflight(
    program: &Path,
    storage: &Path,
    package: &Path,
) -> Result<UpgradePreflight, &'static str> {
    checked_upgrade_preflight(program, storage, package).map_err(|failure| failure.code)
}

pub fn checked_upgrade_preflight(
    program: &Path,
    storage: &Path,
    package: &Path,
) -> Result<UpgradePreflight, UpgradePreflightFailure> {
    let (success, report) =
        transport(program, storage, package).map_err(|code| UpgradePreflightFailure {
            code,
            recovery: None,
        })?;
    if !success || report.recovery.is_some() {
        let code = match report.recovery.as_ref().map(|recovery| recovery.cause) {
            Some(crate::StorageRecoveryCause::InsufficientSpace) => "UPGRADE_INSUFFICIENT_SPACE",
            Some(crate::StorageRecoveryCause::MissingKey) => "UPGRADE_KEY_UNAVAILABLE",
            Some(crate::StorageRecoveryCause::MissingPackage) => "UPGRADE_RECOVERY_PACKAGE_MISSING",
            Some(crate::StorageRecoveryCause::InvalidPackage) => "UPGRADE_RECOVERY_PACKAGE_INVALID",
            _ => "UPGRADE_PREFLIGHT_REFUSED",
        };
        return Err(UpgradePreflightFailure {
            code,
            recovery: report.recovery.map(Box::new),
        });
    }
    Ok(report)
}

fn transport(
    program: &Path,
    storage: &Path,
    package: &Path,
) -> Result<(bool, UpgradePreflight), &'static str> {
    let mut command = Command::new(program);
    command
        .arg("--upgrade-preflight")
        .arg("--storage-root")
        .arg(storage)
        .arg("--recovery-package")
        .arg(package)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "UPGRADE_PREFLIGHT_UNAVAILABLE")?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("UPGRADE_PREFLIGHT_UNAVAILABLE");
            }
        }
    };
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .ok_or("UPGRADE_PREFLIGHT_INVALID")?
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "UPGRADE_PREFLIGHT_INVALID")?;
    if bytes.len() > 16 * 1024 {
        return Err("UPGRADE_PREFLIGHT_INVALID");
    }
    let report: UpgradePreflight =
        serde_json::from_slice(&bytes).map_err(|_| "UPGRADE_PREFLIGHT_UNAVAILABLE")?;
    if report.schema != "hiroute.upgrade-preflight/v1" {
        return Err("UPGRADE_PREFLIGHT_INVALID");
    }
    if let Some(recovery) = &report.recovery
        && (recovery.schema != crate::STORAGE_RECOVERY_SCHEMA
            || !matches!(
                recovery.action,
                crate::StorageRecoveryAction::PreserveAndDiagnose
                    | crate::StorageRecoveryAction::RetryCurrent
            )
            || recovery.backup_batch.is_some()
            || recovery.backup_name.is_some())
    {
        // Provisional checks may explain prerequisites, but can never grant a
        // current-batch resume, restoration or old-program launch authority.
        return Err("UPGRADE_PREFLIGHT_INVALID");
    }
    Ok((status.success(), report))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn typed_preflight_failure_preserves_measured_recovery_facts_and_rejects_extra_fields() {
        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("preflight");
        let report = serde_json::json!({
            "schema":"hiroute.upgrade-preflight/v1", "history_cutover_required":false,
            "required_bytes":4096, "available_bytes":12,
            "recovery":{
                "schema":"hiroute.storage-startup-recovery/v1", "cause":"insufficient_space",
                "action":"retry_current", "storage_space":{"required_bytes":4096,"available_bytes":12}
            }
        });
        let write = |report: &serde_json::Value| {
            std::fs::write(
                &program,
                format!("#!/bin/sh\ncat <<'REPORT'\n{report}\nREPORT\nexit 1\n"),
            )
            .unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        };
        write(&report);
        let failure =
            checked_upgrade_preflight(&program, directory.path(), directory.path()).unwrap_err();
        assert_eq!(failure.code, "UPGRADE_INSUFFICIENT_SPACE");
        assert_eq!(
            failure
                .recovery
                .as_ref()
                .unwrap()
                .storage_space
                .as_ref()
                .unwrap()
                .required_bytes,
            4096
        );
        assert!(failure.to_string().contains("4096 bytes"));
        assert!(failure.to_string().contains("12 bytes"));
        assert_eq!(
            run_upgrade_preflight(&program, directory.path(), directory.path()).unwrap_err(),
            "UPGRADE_INSUFFICIENT_SPACE"
        );
        let mut unknown = report;
        unknown["arbitrary_native_error"] = "never expose raw native output".into();
        write(&unknown);
        let failure =
            checked_upgrade_preflight(&program, directory.path(), directory.path()).unwrap_err();
        assert!(failure.recovery.is_none());
        assert!(!failure.to_string().contains("raw native output"));
        unknown
            .as_object_mut()
            .unwrap()
            .remove("arbitrary_native_error");
        unknown["recovery"]["action"] = "resume_current".into();
        write(&unknown);
        let failure =
            checked_upgrade_preflight(&program, directory.path(), directory.path()).unwrap_err();
        assert_eq!(failure.code, "UPGRADE_PREFLIGHT_INVALID");
        assert!(failure.recovery.is_none());
    }
}
