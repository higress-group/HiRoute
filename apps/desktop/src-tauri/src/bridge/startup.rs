use super::{DesktopState, Session};
use hiroute_diagnostics::context::DiagnosticHandle;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicU64},
};
use tauri::{State, WebviewWindow};
use tokio::sync::Mutex;

#[derive(Default)]
pub struct StartupState {
    pub(super) error: OnceLock<String>,
    pub(super) cancelled: AtomicBool,
    pub(super) data_root: Option<PathBuf>,
    pub(super) upgrade_progress:
        Arc<std::sync::Mutex<Option<hiroute_host_runtime::StorageUpgradePhase>>>,
}

pub(super) fn start_session(
    diagnostics: DiagnosticHandle,
    data_root: Option<PathBuf>,
    initialize: impl FnOnce(
        &AtomicBool,
        Arc<std::sync::Mutex<Option<hiroute_host_runtime::StorageUpgradePhase>>>,
    ) -> Result<Session, String>
    + Send
    + 'static,
) -> DesktopState {
    let state = DesktopState(
        Arc::new(Mutex::new(None)),
        Arc::new(StartupState {
            data_root,
            ..StartupState::default()
        }),
        AtomicU64::new(0),
        diagnostics,
    );
    // Commands must await initialization without holding the WebView event loop.
    let mut session = state.0.clone().try_lock_owned().expect("new session lock");
    let startup = state.1.clone();
    tauri::async_runtime::spawn_blocking(move || {
        match initialize(&startup.cancelled, startup.upgrade_progress.clone()) {
            Ok(ready) => *session = Some(ready),
            Err(error) => {
                let _ = startup.error.set(error);
            }
        }
    });
    state
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub struct StartupReport {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    storage_recovery: Option<hiroute_host_runtime::StorageRecovery>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    recovery_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    backup_directory: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upgrade_phase: Option<hiroute_host_runtime::StorageUpgradePhase>,
}

fn published_backup(
    root: &std::path::Path,
    recovery: &hiroute_host_runtime::StorageRecovery,
) -> Option<PathBuf> {
    use hiroute_host_runtime::StorageRecoveryAction;
    if !matches!(
        recovery.action,
        StorageRecoveryAction::ResumeCurrent | StorageRecoveryAction::RestoreCompleteGroup
    ) {
        return None;
    }
    let name = recovery.backup_name.as_deref()?;
    let id = recovery.backup_batch.as_deref()?;
    let portable = |s: &str| {
        !s.is_empty()
            && s.len() <= 128
            && !s.contains("..")
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    if !portable(name) || !portable(id) {
        return None;
    }
    // This target comes from the storage owner's verified failing batch, never
    // from whichever old directory happens to be present.
    published_source(&root.join("storage.upgrade-backups").join(name), Some(id))
}

fn recovery_for_failure(
    code: &str,
    root: Option<&std::path::Path>,
) -> Option<hiroute_host_runtime::StorageRecovery> {
    use hiroute_diagnostics::event::StartupFailureCode as Failure;
    use hiroute_host_runtime::StorageRecovery;
    let storage = root.map(|root| root.join("storage"));
    let admission = match code {
        "DAEMON_STORAGE_UNREADABLE" => Some(Failure::StorageUnavailable),
        "DAEMON_UPGRADE_SOURCE_UNSUPPORTED" => Some(Failure::UpgradeSourceUnsupported),
        "DAEMON_STORAGE_ACCESS_DENIED" => Some(Failure::StorageAccessDenied),
        "DAEMON_STORAGE_KEY_UNAVAILABLE" => Some(Failure::StorageKeyUnavailable),
        "DAEMON_STORAGE_LOCKED" => Some(Failure::StorageLocked),
        _ => None,
    };
    if let Some(code) = admission {
        return StorageRecovery::read_for_admission_failure(code, storage.as_deref());
    }
    if code == "DAEMON_UPGRADE_STORAGE_FAILED" {
        return storage.and_then(|root| StorageRecovery::read(&root).ok().flatten());
    }
    None
}

fn published_source(batch: &std::path::Path, expected_id: Option<&str>) -> Option<PathBuf> {
    let backup = batch.join("source");
    let directory = hiroute_diagnostics::files::PrivateDir::open_existing(&backup).ok()?;
    let batch_directory = hiroute_diagnostics::files::PrivateDir::open_existing(batch).ok()?;
    if let Some(mut file) = batch_directory.open_read("source-location.json").ok()? {
        if file.len() > 16 * 1024 {
            return None;
        }
        let hint: serde_json::Value =
            serde_json::from_slice(&file.read_prefix(16 * 1024).ok()?).ok()?;
        file.recheck().ok()?;
        return (hint["schema"] == 1
            && hint["complete"] == true
            && expected_id.is_none_or(|id| hint["batch_id"] == id))
        .then_some(backup);
    }
    // Supported old small manifests predate the bounded directory hint.
    let mut manifest_file = directory.open_read("source-backup.json").ok()??;
    if manifest_file.len() > 4 * 1024 * 1024 {
        return None;
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&manifest_file.read_prefix(4 * 1024 * 1024).ok()?).ok()?;
    manifest_file.recheck().ok()?;
    (manifest["schema"] == "hiroute.upgrade-source-backup/v1"
        && manifest["complete"] == true
        && expected_id.is_none_or(|id| manifest["source_info"]["batch_id"] == id))
    .then_some(backup)
}

#[tauri::command]
pub async fn startup_status(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<StartupReport, String> {
    super::main_window(&window)?;
    if let Some(code) = state.1.error.get() {
        let recovery = recovery_for_failure(code, state.1.data_root.as_deref());
        let backup_directory = state.1.data_root.as_deref().and_then(|root| {
            recovery
                .as_ref()
                .and_then(|record| published_backup(root, record))
        });
        return Ok(StartupReport {
            state: "failed",
            storage_recovery: recovery,
            code: Some(code.clone()),
            recovery_available: state.1.data_root.as_ref().is_some_and(|root| {
                root.is_absolute()
                    && hiroute_diagnostics::files::PrivateDir::open_existing(root).is_ok()
            }),
            backup_directory,
            upgrade_phase: state.1.upgrade_progress.lock().ok().and_then(|p| *p),
        });
    }
    let ready = state.0.try_lock().is_ok_and(|session| session.is_some());
    Ok(StartupReport {
        state: if ready { "ready" } else { "starting" },
        storage_recovery: None,
        code: None,
        recovery_available: false,
        backup_directory: None,
        upgrade_phase: if ready {
            None
        } else {
            state.1.upgrade_progress.lock().ok().and_then(|p| *p)
        },
    })
}

/// Opens the preserved application data root after startup failure. Recovery stays explicit:
/// this command never renames, deletes, migrates or rewrites the failed store.
#[tauri::command]
pub async fn open_startup_recovery_directory(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    super::main_window(&window)?;
    let code = state.1.error.get().ok_or("STARTUP_RECOVERY_NOT_REQUIRED")?;
    let root = state
        .1
        .data_root
        .clone()
        .filter(|root| root.is_absolute())
        .ok_or("PRIVATE_PATH_UNAVAILABLE")?;
    hiroute_diagnostics::files::PrivateDir::open_existing(&root)
        .map_err(|_| "PRIVATE_PATH_INVALID")?;
    let recovery = recovery_for_failure(code, Some(&root));
    let directory = recovery
        .as_ref()
        .and_then(|record| published_backup(&root, record))
        .unwrap_or(root);
    super::diagnostics::open_in_file_manager(&directory)
        .map_err(|_| "STARTUP_RECOVERY_OPEN_FAILED".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    fn complete_upgrade_backup(root: &std::path::Path) -> Option<PathBuf> {
        recovery_for_failure("DAEMON_UPGRADE_STORAGE_FAILED", Some(root))
            .as_ref()
            .and_then(|record| published_backup(root, record))
    }

    #[test]
    #[cfg(unix)]
    fn observation_source_locator_requires_verified_material_and_rejects_another_batch() {
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let write = |path: PathBuf, bytes: &[u8]| {
            std::fs::write(&path, bytes).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        };
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let storage = root.join("storage");
        let batch = root.join("storage.upgrade-backups/observation-format-5");
        let source = batch.join("source");
        hiroute_diagnostics::files::PrivateDir::open_or_create(&storage).unwrap();
        hiroute_diagnostics::files::PrivateDir::open_or_create(&source).unwrap();
        std::fs::write(
            storage.join("observation-source-backup.json"),
            br#"{"schema":1,"backup_name":"observation-format-5","set_id":"retained-batch"}"#,
        )
        .unwrap();
        let history = source.join("storage/observation");
        hiroute_diagnostics::files::PrivateDir::open_or_create(&history).unwrap();
        let mut files = serde_json::Map::new();
        for name in [
            "storage/live/control.db",
            "storage/live/runtime.db",
            "storage/live/secrets.db",
            "storage/master-key",
            "storage/observation-hmac-key",
            "recovery-package/hirouted",
        ] {
            let path = source.join(name);
            hiroute_diagnostics::files::PrivateDir::open_or_create(path.parent().unwrap()).unwrap();
            write(path, b"retained original");
            files.insert(
                name.into(),
                serde_json::json!(format!("sha256:{:x}", Sha256::digest(b"retained original"))),
            );
        }
        let identity = std::fs::metadata(&history).unwrap();
        for (path, value) in [
            (
                source.join("source-backup.json"),
                serde_json::json!({"schema":"hiroute.upgrade-source-backup/v2","complete":true,"source_info":{"batch_id":"retained-batch"},"files":files,"sealed_observation":{"identity":[identity.dev(),identity.ino()]}}),
            ),
            (
                batch.join("backup-set.manifest.json"),
                serde_json::json!({"schema":"hiroute.sqlite-backup-set/v2","set_id":"retained-batch"}),
            ),
        ] {
            write(path, &serde_json::to_vec(&value).unwrap());
        }
        std::fs::write(
            batch.join("source-location.json"),
            br#"{"schema":1,"complete":true,"batch_id":"retained-batch"}"#,
        )
        .unwrap();
        let mut recovery = hiroute_host_runtime::StorageRecovery::new(
            hiroute_host_runtime::StorageRecoveryCause::MigrationFailed,
            hiroute_host_runtime::StorageRecoveryAction::ResumeCurrent,
        );
        recovery.backup_name = Some("observation-format-5".into());
        recovery.backup_batch = Some("retained-batch".into());
        recovery.store(&storage).unwrap();
        assert_eq!(complete_upgrade_backup(root), Some(source.clone()));
        for code in [
            "DAEMON_STORAGE_UNREADABLE",
            "DAEMON_STORAGE_LOCKED",
            "DAEMON_STORAGE_ACCESS_DENIED",
            "DAEMON_GATEWAY_UNAVAILABLE",
        ] {
            assert!(
                recovery_for_failure(code, Some(root))
                    .as_ref()
                    .and_then(|record| published_backup(root, record))
                    .is_none(),
                "a different current failure cannot offer an older report's recovery folder"
            );
        }
        let mut verified = recovery.clone();
        verified.cause = hiroute_host_runtime::StorageRecoveryCause::UnsupportedFormat;
        verified.action = hiroute_host_runtime::StorageRecoveryAction::RestoreCompleteGroup;
        verified.store(&storage).unwrap();
        assert_eq!(
            recovery_for_failure("DAEMON_UPGRADE_SOURCE_UNSUPPORTED", Some(root))
                .as_ref()
                .and_then(|record| published_backup(root, record)),
            Some(source)
        );
        recovery.store(&storage).unwrap();
        std::fs::write(
            batch.join("source-location.json"),
            br#"{"schema":1,"complete":true,"batch_id":"different-batch"}"#,
        )
        .unwrap();
        assert_eq!(complete_upgrade_backup(root), None);
        std::fs::write(
            storage.join("observation-source-backup.json"),
            br#"{"schema":1,"backup_name":"../outside","set_id":"retained-batch"}"#,
        )
        .unwrap();
        assert_eq!(complete_upgrade_backup(root), None);
    }

    #[test]
    #[cfg(unix)]
    fn recovery_directory_requires_a_private_complete_published_source() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("data");
        let backup = root.join("storage.upgrade-backups/migration-set/source");
        hiroute_diagnostics::files::PrivateDir::open_or_create(&backup).unwrap();
        let manifest = backup.join("source-backup.json");
        assert_eq!(complete_upgrade_backup(&root), None);
        std::fs::write(
            &manifest,
            br#"{"schema":"hiroute.upgrade-source-backup/v1","complete":false}"#,
        )
        .unwrap();
        std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(complete_upgrade_backup(&root), None);
        std::fs::write(
            &manifest,
            br#"{"schema":"hiroute.upgrade-source-backup/v1","complete":true}"#,
        )
        .unwrap();
        // Directory/manifest existence alone never grants restoration authority.
        assert_eq!(complete_upgrade_backup(&root), None);
        let other = backup.join("other.json");
        std::fs::rename(&manifest, &other).unwrap();
        symlink(&other, &manifest).unwrap();
        assert_eq!(complete_upgrade_backup(&root), None);
    }

    #[tokio::test]
    async fn startup_does_not_block_the_caller_or_expose_an_uninitialized_session() {
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let state = start_session(DiagnosticHandle::noop(), None, move |_, _| {
            let _ = entered.send(());
            wait.recv().unwrap();
            Err("DAEMON_READY_INVALID".into())
        });
        started.await.unwrap();
        assert!(state.0.try_lock().is_err());
        assert!(state.1.error.get().is_none());
        release.send(()).unwrap();
        let session = tokio::time::timeout(Duration::from_secs(2), state.0.lock())
            .await
            .unwrap();
        assert!(session.is_none());
        assert_eq!(
            state.1.error.get().map(String::as_str),
            Some("DAEMON_READY_INVALID")
        );
    }

    #[tokio::test]
    async fn exit_can_cancel_pending_startup_before_waiting_for_session_cleanup() {
        let (entered, started) = tokio::sync::oneshot::channel();
        let state = start_session(DiagnosticHandle::noop(), None, move |cancelled, _| {
            let _ = entered.send(());
            while !cancelled.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err("DAEMON_START_CANCELLED".into())
        });
        started.await.unwrap();
        state.1.cancelled.store(true, Ordering::SeqCst);
        let session = tokio::time::timeout(Duration::from_secs(2), state.0.lock())
            .await
            .unwrap();
        assert!(session.is_none());
        assert_eq!(
            state.1.error.get().map(String::as_str),
            Some("DAEMON_START_CANCELLED")
        );
    }
}
