//! Early admission can fail before a recovery file can be safely written.
//! Diagnostic facts here can explain a cause, never authorize data operations.
use super::*;
use hiroute_diagnostics::{
    event::{DiagnosticEvent, ProcessRole, StartupFailureCode, StartupOutcome},
    files::{CURRENT_LOG_FILE, FileSafetyError, PrivateDir, role_dir_name},
    identity::BootId,
    record::DiagnosticRecordV1,
};

/// A read-only watermark captured before asking the manager to start a service.
/// Unreadable or incomplete diagnostics cannot establish a fresh failure.
pub struct StartupFailureBoundary {
    // None is untrusted/unreadable; Some(None) is verified absence or an empty log.
    observed: Option<Option<(BootId, u64)>>,
}
impl StartupFailureBoundary {
    pub fn capture(diagnostics: &Path) -> Self {
        let observed = match diagnostic_tail(diagnostics) {
            Ok(bytes) if bytes.is_empty() => Some(None),
            Ok(bytes) if bytes.ends_with(b"\n") => bytes
                .split(|byte| *byte == b'\n')
                .rev()
                .find(|line| !line.is_empty())
                .and_then(|line| serde_json::from_slice::<DiagnosticRecordV1>(line).ok())
                .map(|record| Some((record.boot_id, record.sequence))),
            Err(FileSafetyError::NotFound)
            | Err(FileSafetyError::Io {
                kind: io::ErrorKind::NotFound,
                ..
            }) => Some(None),
            _ => None,
        };
        Self { observed }
    }

    pub fn read_failure(
        &self,
        storage: &Path,
        diagnostics: &Path,
    ) -> Option<(&'static str, Option<StorageRecovery>)> {
        let previous = self.observed?;
        let (record, code) = failed_startup(diagnostics)?;
        if previous
            .is_some_and(|(boot, sequence)| boot == record.boot_id && record.sequence <= sequence)
        {
            return None;
        }
        Some((
            startup_failure_host_code(code),
            StorageRecovery::for_failure_code(storage, code),
        ))
    }
}

impl StorageRecovery {
    pub fn read_failed_startup_code(diagnostics: &Path) -> Option<&'static str> {
        failed_startup_code(diagnostics).map(startup_failure_host_code)
    }
    pub fn from_admission_code(code: StartupFailureCode) -> Option<Self> {
        let cause = match code {
            StartupFailureCode::StorageUnavailable => StorageRecoveryCause::UnknownState,
            StartupFailureCode::StorageAccessDenied => StorageRecoveryCause::AccessDenied,
            StartupFailureCode::StorageLocked => StorageRecoveryCause::Locked,
            StartupFailureCode::StorageKeyUnavailable => StorageRecoveryCause::MissingKey,
            StartupFailureCode::UpgradeSourceUnsupported => StorageRecoveryCause::UnsupportedFormat,
            _ => return None,
        };
        let action = if matches!(
            cause,
            StorageRecoveryCause::Locked | StorageRecoveryCause::AccessDenied
        ) {
            StorageRecoveryAction::RetryCurrent
        } else {
            StorageRecoveryAction::PreserveAndDiagnose
        };
        Some(Self::new(cause, action))
    }

    pub fn read_for_admission_failure(
        code: StartupFailureCode,
        storage: Option<&Path>,
    ) -> Option<Self> {
        Some(Self::from_admission_code(code)?.with_verified_restore(storage))
    }

    fn with_verified_restore(self, storage: Option<&Path>) -> Self {
        // The current diagnostic supplies only the cause. Whole-group restore
        // authority still comes from revalidating the storage owner's exact
        // original batch. Never borrow program-only reinstall or resume advice.
        if matches!(
            self.cause,
            StorageRecoveryCause::MissingKey
                | StorageRecoveryCause::UnsupportedFormat
                | StorageRecoveryCause::UnknownState
        ) && let Some(storage) = storage
            && let Ok(Some(record)) = Self::read(storage)
            && record.cause == self.cause
            && record.action == StorageRecoveryAction::RestoreCompleteGroup
        {
            return record;
        }
        self
    }

    /// Use only on an already failed startup surface. Prefer this attempt's
    /// diagnostic admission failure over a report left by an earlier writer.
    /// A log record never grants reinstall, resume or restore authority.
    pub fn read_for_failed_startup(storage: &Path, diagnostics: &Path) -> Option<Self> {
        if let Some(code) = failed_startup_code(diagnostics) {
            return Self::for_failure_code(storage, code);
        }
        Self::read_report(storage)
    }

    fn for_failure_code(storage: &Path, code: StartupFailureCode) -> Option<Self> {
        if let Some(report) = Self::from_admission_code(code) {
            return Some(report.with_verified_restore(Some(storage)));
        }
        (code == StartupFailureCode::UpgradeStorageFailed)
            .then(|| Self::read_report(storage))
            .flatten()
    }

    fn read_report(storage: &Path) -> Option<Self> {
        match Self::read(storage) {
            Ok(report) => report,
            Err(error) => Some(Self::new(
                if error.kind() == io::ErrorKind::PermissionDenied {
                    StorageRecoveryCause::AccessDenied
                } else {
                    StorageRecoveryCause::UnknownState
                },
                StorageRecoveryAction::PreserveAndDiagnose,
            )),
        }
    }
}

fn failed_startup_code(root: &Path) -> Option<StartupFailureCode> {
    failed_startup(root).map(|(_, code)| code)
}

fn diagnostic_tail(root: &Path) -> Result<Vec<u8>, FileSafetyError> {
    let root = PrivateDir::open_existing(root)?;
    root.recheck_identity()?;
    let role = PrivateDir::open_existing(&root.path().join(role_dir_name(ProcessRole::Daemon)))?;
    root.recheck_identity()?;
    let Some(mut file) = role.open_read(CURRENT_LOG_FILE)? else {
        return Ok(Vec::new());
    };
    file.recheck()?;
    let bytes = file.read_tail(64 * 1024)?;
    file.recheck()?;
    Ok(bytes)
}

fn failed_startup(root: &Path) -> Option<(DiagnosticRecordV1, StartupFailureCode)> {
    let bytes = diagnostic_tail(root).ok()?;
    if !bytes.ends_with(b"\n") {
        return None;
    }
    // The newest startup/process event is the boundary. Unknown or unfinished
    // attempts cannot borrow an older admission reason from rotated history.
    for line in bytes
        .split(|byte| *byte == b'\n')
        .rev()
        .filter(|line| !line.is_empty())
    {
        let value: serde_json::Value = serde_json::from_slice(line).ok()?;
        let event = value.get("event")?;
        if event.get("process_start").is_some() {
            return None;
        }
        if event.get("startup_end").is_some() {
            let record: DiagnosticRecordV1 = serde_json::from_value(value).ok()?;
            if let DiagnosticEvent::StartupEnd(end) = &record.event
                && let StartupOutcome::Failure { code } = end.outcome
            {
                return Some((record, code));
            }
            return None;
        }
    }
    None
}

pub fn startup_failure_host_code(code: StartupFailureCode) -> &'static str {
    match code {
        StartupFailureCode::StorageUnavailable => "DAEMON_STORAGE_UNREADABLE",
        StartupFailureCode::StorageAccessDenied => "DAEMON_STORAGE_ACCESS_DENIED",
        StartupFailureCode::StorageLocked => "DAEMON_STORAGE_LOCKED",
        StartupFailureCode::StorageKeyUnavailable => "DAEMON_STORAGE_KEY_UNAVAILABLE",
        StartupFailureCode::UpgradeSourceUnsupported => "DAEMON_UPGRADE_SOURCE_UNSUPPORTED",
        StartupFailureCode::UpgradeStorageFailed => "DAEMON_UPGRADE_STORAGE_FAILED",
        StartupFailureCode::ReleaseFactsInvalid => "DAEMON_RELEASE_INVALID",
        StartupFailureCode::DependencyUnavailable => "DAEMON_DEPENDENCY_UNAVAILABLE",
        StartupFailureCode::GatewayUnavailable => "DAEMON_GATEWAY_UNAVAILABLE",
        StartupFailureCode::ControlUnavailable => "DAEMON_CONTROL_UNAVAILABLE",
        StartupFailureCode::Cancelled => "DAEMON_START_CANCELLED",
        StartupFailureCode::InvalidConfiguration
        | StartupFailureCode::WorkerRejected
        | StartupFailureCode::ReadyChannelFailed
        | StartupFailureCode::Internal => "DAEMON_START_FAILED",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_diagnostics::{
        Component, DiagnosticRuntime, EmitOutcome, RuntimeConfig,
        event::{ProcessStart, StartupEnd},
        files::WRITER_LOCK_FILE,
        identity::{TargetTriple, VersionToken},
        record::RecordSchema,
    };
    use std::time::{Duration, Instant};

    // Consumer cases need complete admitted bytes, not a promise that an asynchronous
    // writer finished within the product's deliberately bounded shutdown budget.
    fn fixture_attempt(diagnostics: &Path, outcome: Option<StartupOutcome>) {
        let root = PrivateDir::open_or_create(diagnostics).unwrap();
        let role = root.child_dir(role_dir_name(ProcessRole::Daemon)).unwrap();
        if let Some(file) = role.open_read(CURRENT_LOG_FILE).unwrap() {
            let identity = file.identity();
            drop(file);
            role.remove_verified(CURRENT_LOG_FILE, identity).unwrap();
        }
        let boot_id = BootId::random().unwrap();
        let mut events = vec![DiagnosticEvent::ProcessStart(ProcessStart {
            role: ProcessRole::Daemon,
            version: VersionToken::current_package(),
            source_revision: VersionToken::source_revision(),
            target: TargetTriple::current(),
            parent_session_id: None,
        })];
        if let Some(outcome) = outcome {
            events.push(DiagnosticEvent::StartupEnd(StartupEnd {
                outcome,
                elapsed_ms: 1,
            }));
        }
        let mut file = role.create_new(CURRENT_LOG_FILE).unwrap();
        for (index, event) in events.into_iter().enumerate() {
            let record = DiagnosticRecordV1 {
                schema: RecordSchema,
                timestamp_ms: index as u64,
                monotonic_ms: index as u64,
                component: Component::Daemon,
                boot_id,
                sequence: index as u64 + 1,
                level: event.level(),
                level_revision: 0,
                parent_session_id: None,
                span_id: None,
                parent_span_id: None,
                event,
            };
            file.append(&record.encode_jsonl().unwrap()).unwrap();
        }
        file.sync().unwrap();
    }

    #[test]
    fn boundary_requires_a_new_failure_and_rejects_incomplete_or_unreadable_attempts() {
        let directory = tempfile::tempdir().unwrap();
        let diagnostics = directory.path().join("diagnostics");
        let emit = |outcome| fixture_attempt(&diagnostics, outcome);
        let first_attempt = StartupFailureBoundary::capture(&diagnostics);
        emit(Some(StartupOutcome::Failure {
            code: StartupFailureCode::StorageLocked,
        }));
        let (code, report) = first_attempt
            .read_failure(directory.path(), &diagnostics)
            .unwrap();
        assert_eq!(code, "DAEMON_STORAGE_LOCKED");
        assert_eq!(report.unwrap().action, StorageRecoveryAction::RetryCurrent);
        let boundary = StartupFailureBoundary::capture(&diagnostics);
        assert!(
            boundary
                .read_failure(directory.path(), &diagnostics)
                .is_none()
        );
        emit(Some(StartupOutcome::Failure {
            code: StartupFailureCode::GatewayUnavailable,
        }));
        let (code, report) = boundary
            .read_failure(directory.path(), &diagnostics)
            .unwrap();
        assert_eq!(code, "DAEMON_GATEWAY_UNAVAILABLE");
        assert!(report.is_none());
        emit(Some(StartupOutcome::Success));
        assert!(
            boundary
                .read_failure(directory.path(), &diagnostics)
                .is_none()
        );
        emit(None);
        assert!(
            boundary
                .read_failure(directory.path(), &diagnostics)
                .is_none()
        );
        let log = diagnostics.join("daemon").join(CURRENT_LOG_FILE);
        std::fs::write(&log, b"incomplete record").unwrap();
        let unknown = StartupFailureBoundary::capture(&diagnostics);
        emit(Some(StartupOutcome::Failure {
            code: StartupFailureCode::StorageLocked,
        }));
        assert!(
            unknown
                .read_failure(directory.path(), &diagnostics)
                .is_none()
        );
    }

    #[test]
    fn boundary_accepts_completion_of_the_observed_boot_without_reusing_its_old_failure() {
        let directory = tempfile::tempdir().unwrap();
        let diagnostics = directory.path().join("diagnostics");
        fixture_attempt(
            &diagnostics,
            Some(StartupOutcome::Failure {
                code: StartupFailureCode::StorageLocked,
            }),
        );
        let log = diagnostics.join("daemon").join(CURRENT_LOG_FILE);
        let complete = std::fs::read(&log).unwrap();
        let start = complete
            .split(|byte| *byte == b'\n')
            .find(|line| {
                serde_json::from_slice::<DiagnosticRecordV1>(line)
                    .is_ok_and(|r| matches!(r.event, DiagnosticEvent::ProcessStart(_)))
            })
            .unwrap();
        std::fs::write(&log, [start, b"\n"].concat()).unwrap();
        let boundary = StartupFailureBoundary::capture(&diagnostics);
        std::fs::write(&log, &complete).unwrap();
        let (code, report) = boundary
            .read_failure(directory.path(), &diagnostics)
            .unwrap();
        assert_eq!(code, "DAEMON_STORAGE_LOCKED");
        assert_eq!(report.unwrap().action, StorageRecoveryAction::RetryCurrent);
        let after = StartupFailureBoundary::capture(&diagnostics);
        assert!(after.read_failure(directory.path(), &diagnostics).is_none());
    }

    #[test]
    fn current_non_storage_and_unknown_failures_cannot_borrow_an_old_storage_report() {
        for code in [
            StartupFailureCode::DependencyUnavailable,
            StartupFailureCode::GatewayUnavailable,
            StartupFailureCode::InvalidConfiguration,
            StartupFailureCode::StorageUnavailable,
            StartupFailureCode::StorageLocked,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let storage = directory.path();
            let previous = StorageRecovery::new(
                StorageRecoveryCause::MissingPackage,
                StorageRecoveryAction::PreserveAndDiagnose,
            );
            previous.store(storage).unwrap();
            assert_eq!(
                StorageRecovery::read(storage).unwrap(),
                Some(previous.clone())
            );
            let diagnostics = storage.join("diagnostics");
            fixture_attempt(&diagnostics, Some(StartupOutcome::Failure { code }));
            let report = StorageRecovery::read_for_failed_startup(storage, &diagnostics);
            match code {
                StartupFailureCode::StorageUnavailable => assert_eq!(
                    report,
                    Some(StorageRecovery::new(
                        StorageRecoveryCause::UnknownState,
                        StorageRecoveryAction::PreserveAndDiagnose,
                    ))
                ),
                StartupFailureCode::StorageLocked => assert_eq!(
                    report,
                    Some(StorageRecovery::new(
                        StorageRecoveryCause::Locked,
                        StorageRecoveryAction::RetryCurrent,
                    ))
                ),
                _ => assert!(report.is_none()),
            }
            assert_eq!(StorageRecovery::read(storage).unwrap(), Some(previous));
        }
    }

    #[test]
    fn latest_failed_startup_code_preserves_non_storage_causes_and_stops_at_a_healthy_boot() {
        for (code, expected) in [
            (
                StartupFailureCode::DependencyUnavailable,
                "DAEMON_DEPENDENCY_UNAVAILABLE",
            ),
            (
                StartupFailureCode::GatewayUnavailable,
                "DAEMON_GATEWAY_UNAVAILABLE",
            ),
            (StartupFailureCode::StorageLocked, "DAEMON_STORAGE_LOCKED"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("diagnostics");
            fixture_attempt(&root, Some(StartupOutcome::Failure { code }));
            assert_eq!(
                StorageRecovery::read_failed_startup_code(&root),
                Some(expected)
            );
            fixture_attempt(&root, Some(StartupOutcome::Success));
            assert_eq!(StorageRecovery::read_failed_startup_code(&root), None);
        }
    }
    #[test]
    fn early_admission_remains_actionable_without_writing_or_trusting_storage() {
        for (code, cause) in [
            (
                StartupFailureCode::StorageLocked,
                StorageRecoveryCause::Locked,
            ),
            (
                StartupFailureCode::StorageAccessDenied,
                StorageRecoveryCause::AccessDenied,
            ),
            (
                StartupFailureCode::StorageKeyUnavailable,
                StorageRecoveryCause::MissingKey,
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let diagnostics = directory.path().join("diagnostics");
            fixture_attempt(&diagnostics, Some(StartupOutcome::Failure { code }));
            let storage = directory.path().join("inaccessible-storage");
            let report = StorageRecovery::read_for_failed_startup(&storage, &diagnostics).unwrap();
            assert_eq!(report.cause, cause);
            assert_eq!(
                report.action,
                if matches!(
                    cause,
                    StorageRecoveryCause::Locked | StorageRecoveryCause::AccessDenied
                ) {
                    StorageRecoveryAction::RetryCurrent
                } else {
                    StorageRecoveryAction::PreserveAndDiagnose
                }
            );
            assert_eq!(report.backup_batch, None);
            assert!(!storage.exists());
        }
    }

    #[test]
    fn real_writer_boots_are_read_after_complete_output_and_role_release() {
        let directory = tempfile::tempdir().unwrap();
        let diagnostics = directory.path().join("diagnostics");
        for outcome in [
            Some(StartupOutcome::Failure {
                code: StartupFailureCode::StorageLocked,
            }),
            Some(StartupOutcome::Failure {
                code: StartupFailureCode::GatewayUnavailable,
            }),
            Some(StartupOutcome::Success),
            None,
        ] {
            let boundary = StartupFailureBoundary::capture(&diagnostics);
            let runtime = DiagnosticRuntime::start(RuntimeConfig {
                root: diagnostics.clone(),
                role: ProcessRole::Daemon,
                component: Component::Daemon,
                parent_session_id: None,
                level_override: None,
            });
            assert_eq!(runtime.status().unavailable, None);
            let boot = runtime.boot_id().unwrap();
            if let Some(outcome) = outcome {
                assert_eq!(
                    runtime.emit(DiagnosticEvent::StartupEnd(StartupEnd {
                        outcome,
                        elapsed_ms: 1,
                    })),
                    EmitOutcome::Queued,
                );
            }
            runtime.shutdown();
            // Shutdown is bounded. This integration case separately waits for actual
            // role release, so neither reading nor the next boot races the producer.
            let deadline = Instant::now() + Duration::from_secs(5);
            let role = PrivateDir::open_existing(&diagnostics.join("daemon")).unwrap();
            let lock = role.open_lock(WRITER_LOCK_FILE).unwrap();
            loop {
                match lock.try_lock_exclusive() {
                    Ok(()) => break,
                    Err(FileSafetyError::Locked) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    error => panic!("diagnostic writer did not release its role: {error:?}"),
                }
            }
            let bytes = diagnostic_tail(&diagnostics).unwrap();
            assert!(bytes.ends_with(b"\n"));
            let records = bytes
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .map(|line| DiagnosticRecordV1::parse_line(line).unwrap())
                .collect::<Vec<_>>();
            assert!(records.iter().all(|record| record.boot_id == boot));
            assert!(
                records
                    .iter()
                    .any(|record| matches!(record.event, DiagnosticEvent::ProcessStart(_)))
            );
            let terminal = records.iter().find_map(|record| match &record.event {
                DiagnosticEvent::StartupEnd(end) => Some(end.outcome),
                _ => None,
            });
            assert_eq!(terminal, outcome);
            match outcome {
                Some(StartupOutcome::Failure { code }) => {
                    let (actual, report) = boundary
                        .read_failure(directory.path(), &diagnostics)
                        .unwrap();
                    assert_eq!(actual, startup_failure_host_code(code));
                    assert_eq!(report, StorageRecovery::from_admission_code(code));
                }
                _ => assert!(
                    boundary
                        .read_failure(directory.path(), &diagnostics)
                        .is_none()
                ),
            }
            lock.unlock().unwrap();
        }
    }
}
