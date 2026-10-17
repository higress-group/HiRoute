use super::*;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::TrayIconBuilder,
};
pub(super) fn create(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open HiRoute", true, None::<&str>)?;
    let browser = MenuItem::with_id(app, "browser", "Open in browser", true, None::<&str>)?;
    let enabled = commands::login_get().is_ok_and(|v| v["enabled"] == true);
    let login =
        CheckMenuItem::with_id(app, "login", "Start at login", true, enabled, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &browser, &login, &quit])?;
    TrayIconBuilder::new()
        .icon(tauri::image::Image::from_bytes(include_bytes!(
            "../../../../desktop/src-tauri/icons/icon.png"
        ))?)
        .tooltip("HiRoute")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "open" => focus(app),
            "browser" => {
                tauri::async_runtime::spawn(async {
                    if let Ok((_, url)) = service::launch().await {
                        let _ = hiroute_platform::windows::shell_open::url(url.as_str());
                    }
                });
            }
            "login" => {
                let login = login.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Ok(current) = commands::login_get() {
                        if let Ok(next) = commands::login_set(current["enabled"] != true) {
                            let _ = login.set_checked(next["enabled"] == true);
                        }
                    }
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}
