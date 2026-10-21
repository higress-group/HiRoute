//! One main-window recovery action; no path, command, grant or principal comes from IPC.
use super::*;

#[tauri::command]
pub(super) async fn recover_native_connection(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<(), DesktopFailure> {
    main_window(&window)?;
    let handle = window.app_handle();
    // Expire prompts before waiting for the session lock, including callbacks already accepted.
    expire_window_confirmations(handle, window.label());
    let mut sessions = state.0.clone().lock_owned().await;
    tokio::task::spawn_blocking(move || {
        let current = sessions.as_mut().ok_or("RESIDENT_UNAVAILABLE")?;
        current.invalidate_confirmation();
        let mut native = crate::bootstrap::resident(current)?;
        let replacement = native.recover_authority()?;
        drop(native);
        *sessions = Some(crate::session_from_resident(replacement));
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| "AUTHORITY_RECOVERY_UNAVAILABLE")??;
    Ok(())
}
