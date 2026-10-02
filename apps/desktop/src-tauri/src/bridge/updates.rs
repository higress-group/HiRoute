#[cfg(target_os = "macos")]
pub use crate::updates::*;

#[cfg(not(target_os = "macos"))]
mod unsupported {
    use tauri::WebviewWindow;
    fn denied(window: &WebviewWindow) -> Result<serde_json::Value, String> {
        super::super::main_window(window)?;
        Err("UPGRADE_PLATFORM_UNSUPPORTED".into())
    }
    #[tauri::command]
    pub fn update_status(window: WebviewWindow) -> Result<serde_json::Value, String> {
        denied(&window)
    }
    #[tauri::command]
    pub fn update_check(window: WebviewWindow) -> Result<serde_json::Value, String> {
        denied(&window)
    }
    #[tauri::command]
    pub fn update_download(window: WebviewWindow) -> Result<serde_json::Value, String> {
        denied(&window)
    }
    #[tauri::command]
    pub fn update_install(window: WebviewWindow) -> Result<serde_json::Value, String> {
        denied(&window)
    }
    #[tauri::command]
    pub fn update_cancel(window: WebviewWindow) -> Result<serde_json::Value, String> {
        denied(&window)
    }
}
#[cfg(not(target_os = "macos"))]
pub use unsupported::*;
