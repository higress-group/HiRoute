//! Release the resident while the macOS event loop can still complete pending IPC replies.
use super::{DesktopState, DiagnosticsState};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager, RunEvent};

pub(super) fn on_run_event(handle: &AppHandle, event: &RunEvent) {
    match event {
        RunEvent::ExitRequested { api, .. } => {
            let state = handle.state::<DesktopState>();
            if state.1.cancelled.swap(true, Ordering::SeqCst) {
                // Another quit request cannot bypass the still-running shutdown. The final
                // app.exit below is admitted only after the owned resident has been dropped.
                if state.0.try_lock().map_or(true, |session| session.is_some()) {
                    api.prevent_exit();
                }
                return;
            }
            api.prevent_exit();
            let sessions = state.0.clone();
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                drop(sessions.lock().await.take());
                handle.exit(0);
            });
        }
        RunEvent::Exit => handle.state::<DiagnosticsState>().shutdown(),
        _ => {}
    }
}
