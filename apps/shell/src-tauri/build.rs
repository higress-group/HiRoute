fn main() {
    #[cfg(feature = "shell-runtime")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&[
                "shell_status",
                "shell_retry_startup",
                "shell_capabilities",
                "shell_open_in_browser",
                "shell_open_external_url",
                "shell_save_classifier_openapi",
                "shell_login_start_get",
                "shell_login_start_set",
                "shell_restart_service",
                "shell_reconnect",
                "shell_update_status",
                "shell_update_check",
                "shell_update_download",
                "shell_update_install",
                "shell_update_cancel",
            ]),
        ))
        .expect("build the restricted Windows shell manifest");
    }
}
