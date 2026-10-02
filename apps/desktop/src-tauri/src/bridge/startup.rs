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
    code: Option<String>,
    recovery_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    backup_directory: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upgrade_phase: Option<hiroute_host_runtime::StorageUpgradePhase>,
}

fn complete_upgrade_backup(root: &std::path::Path) -> Option<PathBuf> {
    // The storage startup boundary publishes the source outside storage, under the host's
    // existing private data root. No path from JSON or from the WebView is used for opening.
    let backup = root.join("storage.upgrade-backups/migration-set/source");
    let directory = hiroute_diagnostics::files::PrivateDir::open_existing(&backup).ok()?;
    let mut manifest_file = directory.open_read("source-backup.json").ok()??;
    if manifest_file.len() > 4 * 1024 * 1024 {
        return None;
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&manifest_file.read_prefix(4 * 1024 * 1024).ok()?).ok()?;
    manifest_file.recheck().ok()?;
    (manifest["schema"] == "hiroute.upgrade-source-backup/v1" && manifest["complete"] == true)
        .then_some(backup)
}

#[tauri::command]
pub async fn startup_status(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<StartupReport, String> {
    super::main_window(&window)?;
    if let Some(code) = state.1.error.get() {
        return Ok(StartupReport {
            state: "failed",
            code: Some(code.clone()),
            recovery_available: state.1.data_root.as_ref().is_some_and(|root| {
                root.is_absolute()
                    && hiroute_diagnostics::files::PrivateDir::open_existing(root).is_ok()
            }),
            backup_directory: state
                .1
                .data_root
                .as_deref()
                .and_then(complete_upgrade_backup),
            upgrade_phase: state.1.upgrade_progress.lock().ok().and_then(|p| *p),
        });
    }
    let ready = state.0.try_lock().is_ok_and(|session| session.is_some());
    Ok(StartupReport {
        state: if ready { "ready" } else { "starting" },
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
    if state.1.error.get().is_none() {
        return Err("STARTUP_RECOVERY_NOT_REQUIRED".into());
    }
    let root = state
        .1
        .data_root
        .clone()
        .filter(|root| root.is_absolute())
        .ok_or("PRIVATE_PATH_UNAVAILABLE")?;
    hiroute_diagnostics::files::PrivateDir::open_existing(&root)
        .map_err(|_| "PRIVATE_PATH_INVALID")?;
    let directory = complete_upgrade_backup(&root).unwrap_or(root);
    super::diagnostics::open_in_file_manager(&directory)
        .map_err(|_| "STARTUP_RECOVERY_OPEN_FAILED".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

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
        assert_eq!(complete_upgrade_backup(&root), Some(backup.clone()));
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
