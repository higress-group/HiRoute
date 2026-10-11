use super::{DesktopFailure, DesktopState, main_window};
use hiroute_host_runtime::{
    SubscriptionProxyPolicy, SubscriptionProxyStore, SubscriptionProxyView,
};
use tauri::{Manager, State, WebviewWindow};

fn store(window: &WebviewWindow) -> Result<SubscriptionProxyStore, DesktopFailure> {
    let root = super::desktop_data_root(window.app_handle()).map_err(|_| "APP_DATA_UNAVAILABLE")?;
    Ok(SubscriptionProxyStore::new(&root))
}
#[tauri::command]
pub async fn subscription_proxy_status(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<SubscriptionProxyView, DesktopFailure> {
    main_window(&window)?;
    let mut view = store(&window)?.view().map_err(|e| e.to_string())?;
    view.applied &= state.0.lock().await.is_some();
    Ok(view)
}
#[tauri::command]
pub async fn subscription_proxy_apply(
    window: WebviewWindow,
    policy: SubscriptionProxyPolicy,
) -> Result<(), DesktopFailure> {
    main_window(&window)?;
    store(&window)?
        .configure(policy)
        .map_err(|e| e.to_string())?;
    window.app_handle().restart()
}
