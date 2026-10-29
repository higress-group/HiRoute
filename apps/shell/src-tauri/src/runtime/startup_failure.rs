//! Manager failures may show only diagnostic facts observed during this startup.
use hiroute_host_runtime::{StartupFailureBoundary, StorageRecovery};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(super) struct Failure {
    pub code: &'static str,
    pub recovery: Option<StorageRecovery>,
}
impl From<&'static str> for Failure {
    fn from(code: &'static str) -> Self {
        Self {
            code,
            recovery: None,
        }
    }
}

pub(super) struct Attempt {
    boundary: StartupFailureBoundary,
    storage: PathBuf,
    diagnostics: PathBuf,
}
impl Attempt {
    pub fn begin(storage: &Path, diagnostics: &Path) -> Self {
        Self {
            boundary: StartupFailureBoundary::capture(diagnostics),
            storage: storage.to_owned(),
            diagnostics: diagnostics.to_owned(),
        }
    }
    pub fn failed(&self, code: &'static str) -> Failure {
        match self.boundary.read_failure(&self.storage, &self.diagnostics) {
            Some((current, recovery)) => Failure {
                code: current,
                recovery,
            },
            None => code.into(),
        }
    }
}

pub(super) fn command_result(success: bool, value: Value) -> Result<Value, &'static str> {
    if !success || value.get("status").is_some_and(|s| s != "succeeded") {
        return Err("SERVICE_COMMAND_FAILED");
    }
    Ok(value.get("data").cloned().unwrap_or(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_diagnostics::{
        Component, DiagnosticRuntime, EmitOutcome, RuntimeConfig,
        event::{DiagnosticEvent, ProcessRole, StartupEnd, StartupFailureCode, StartupOutcome},
        files::{CURRENT_LOG_FILE, WRITER_LOCK_FILE},
        record::DiagnosticRecordV1,
    };
    use hiroute_host_runtime::{StorageRecoveryAction as Action, StorageRecoveryCause as Cause};
    use std::time::{Duration, Instant};

    fn write_failure(diagnostics: &Path, code: StartupFailureCode) {
        let runtime = DiagnosticRuntime::start(RuntimeConfig {
            root: diagnostics.to_owned(),
            role: ProcessRole::Daemon,
            component: Component::Daemon,
            parent_session_id: None,
            level_override: None,
        });
        let boot = runtime
            .boot_id()
            .expect("fixture diagnostics must be enabled");
        let role = runtime.role_dir().expect("fixture diagnostics role");
        assert_eq!(
            runtime.emit(DiagnosticEvent::StartupEnd(StartupEnd {
                outcome: StartupOutcome::Failure { code },
                elapsed_ms: 1,
            })),
            EmitOutcome::Queued
        );
        // Diagnostics intentionally have a bounded, lossy shutdown. Establish
        // the fixture's current fact before testing the Shell's cause mapping.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let written = role
                .open_read(CURRENT_LOG_FILE)
                .ok()
                .flatten()
                .and_then(|mut file| file.read_tail(64 * 1024).ok())
                .filter(|bytes| bytes.ends_with(b"\n"))
                .is_some_and(|bytes| {
                    bytes.split(|byte| *byte == b'\n').any(|line| {
                        serde_json::from_slice::<DiagnosticRecordV1>(line)
                            .ok()
                            .is_some_and(|record| {
                                record.boot_id == boot
                                    && matches!(record.event, DiagnosticEvent::StartupEnd(end)
                                        if matches!(end.outcome, StartupOutcome::Failure { code: actual } if actual == code))
                            })
                    })
                });
            if written {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "fixture failure was not written: {:?}, {:?}",
                runtime.status(),
                runtime.queue_snapshot()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        runtime.shutdown();
        // The next fixture runtime uses this same role. Do not race a writer
        // that is still releasing its lock after the bounded shutdown returned.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let lock = role.open_lock(WRITER_LOCK_FILE).expect("fixture role lock");
            if lock.try_lock_exclusive().is_ok() {
                lock.unlock().expect("release fixture role lock");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "fixture writer did not release its role"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn stopped_service_command_failure_keeps_this_attempts_typed_cause() {
        for (code, expected, cause, action) in [
            (
                StartupFailureCode::StorageLocked,
                "DAEMON_STORAGE_LOCKED",
                Cause::Locked,
                Action::RetryCurrent,
            ),
            (
                StartupFailureCode::StorageKeyUnavailable,
                "DAEMON_STORAGE_KEY_UNAVAILABLE",
                Cause::MissingKey,
                Action::PreserveAndDiagnose,
            ),
            (
                StartupFailureCode::UpgradeStorageFailed,
                "DAEMON_UPGRADE_STORAGE_FAILED",
                Cause::MigrationFailed,
                Action::PreserveAndDiagnose,
            ),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let storage = temp.path().to_owned();
            let diagnostics = temp.path().join("diagnostics");
            let old = StorageRecovery::new(Cause::MissingPackage, Action::PreserveAndDiagnose);
            old.store(&storage).unwrap();
            write_failure(&diagnostics, StartupFailureCode::DependencyUnavailable);
            let attempt = Attempt::begin(&storage, &diagnostics);
            // CLI start waits for readiness and returns a failed envelope. This
            // path never reaches the shell's separate wait_ready operation.
            write_failure(&diagnostics, code);
            if code == StartupFailureCode::UpgradeStorageFailed {
                StorageRecovery::new(cause, action).store(&storage).unwrap();
            }
            let error = command_result(false, serde_json::json!({"status":"failed"})).unwrap_err();
            let failure = attempt.failed(error);
            assert_eq!(failure.code, expected);
            let report = failure.recovery.unwrap();
            assert_eq!(report.cause, cause);
            assert_eq!(report.action, action);
            if code != StartupFailureCode::UpgradeStorageFailed {
                assert_eq!(StorageRecovery::read(&storage).unwrap(), Some(old));
            }
        }
    }

    #[test]
    fn unchanged_diagnostics_and_unproven_attempts_cannot_borrow_old_advice() {
        let temp = tempfile::tempdir().unwrap();
        let diagnostics = temp.path().join("diagnostics");
        write_failure(&diagnostics, StartupFailureCode::StorageKeyUnavailable);
        let attempt = Attempt::begin(temp.path(), &diagnostics);
        for code in [
            "SERVICE_COMMAND_FAILED",
            "SERVICE_COMMAND_TIMED_OUT",
            "SERVICE_NOT_READY",
        ] {
            let failure = attempt.failed(code);
            assert_eq!(failure.code, code);
            assert!(failure.recovery.is_none());
        }
        let missing = temp.path().join("absent");
        let attempt = Attempt::begin(&missing, &missing);
        let failure = attempt.failed("SERVICE_COMMAND_FAILED");
        assert_eq!(failure.code, "SERVICE_COMMAND_FAILED");
        assert!(failure.recovery.is_none());
    }
}
