const MODEL_CONNECTION_COMMANDS: &[&str] = &[
    "agent_home_summary",
    "agent_model_associations",
    "trial_agent_choices",
    "trial_agent_model",
    "compute_management_snapshot",
    "compute_key_masks",
    "compute_scan",
    "prepare_discovered_model_connection",
    "register_protected_model_input",
    "release_protected_model_input",
    "check_model_connection",
    "check_registered_model_connection",
    "check_saved_model_connection",
    "cancel_model_connection_check",
    "preview_compute_save",
    "apply_compute_save",
    "get_compute_save_result",
    "compute_subscriptions",
    "manage_subscription_login",
    "submit_subscription_login_callback",
    "check_subscription",
    "get_subscription_check_result",
    "recover_subscription_check",
    "close_subscription_check",
    "release_subscription_check",
];
const CLASSIFIER_COMMANDS: &[&str] = &[
    "test_classifier_decision",
    "decision_services",
    "save_decision_service",
    "decision_connection_sources",
    "decision_services_v2",
    "save_decision_service_v2",
    "test_decision_service_v2",
    "test_decision_service_v3",
    "save_classifier_header_secret",
    "save_classifier_openapi",
];
const WORKER_TASK_COMMANDS: &[&str] = &[
    "worker_settings_get",
    "worker_settings_set",
    "worker_task_plans",
    "worker_executor_availability",
    "worker_dependencies_discover",
    "worker_dependencies_select_prepare",
    "worker_dependencies_select_confirm",
    "worker_dependencies_select_cancel",
    "worker_task_list",
    "worker_task_status",
    "worker_task_result",
    "worker_task_read",
    "worker_task_wait",
    "worker_task_cancel",
    "worker_task_confirm_residual",
    "worker_task_continue",
];
const DIAGNOSTIC_COMMANDS: &[&str] = &[
    "diagnostic_status",
    "set_diagnostic_level",
    "open_diagnostic_directory",
];
const STARTUP_COMMANDS: &[&str] = &[
    "startup_status",
    "open_startup_recovery_directory",
    "recover_native_connection",
];
const CONFIRMATION_COMMANDS: &[&str] = &["web_confirmation_snapshot", "resolve_web_confirmation"];
const EXTERNAL_URL_COMMANDS: &[&str] = &["open_external_url"];
const WINDOW_BEHAVIOR_COMMANDS: &[&str] = &["perform_titlebar_double_click"];
const HOST_SETTINGS_COMMANDS: &[&str] = &[
    "cli_entry_status",
    "cli_entry_install",
    "cli_entry_remove",
    "gateway_listener_status",
    "subscription_proxy_status",
    "subscription_proxy_apply",
    "gateway_listener_apply",
    "gateway_listener_recover",
];

#[test]
fn native_clipboard_exposes_only_text_writes_in_the_local_main_window() {
    let capability: serde_json::Value =
        serde_json::from_str(include_str!("../capabilities/main.json")).unwrap();
    assert_eq!(capability["local"], true);
    assert_eq!(capability["windows"], serde_json::json!(["main"]));
    assert!(capability.get("remote").is_none());
    let clipboard: Vec<_> = capability["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value.as_str())
        .filter(|permission| permission.starts_with("clipboard-manager:"))
        .collect();
    assert_eq!(clipboard, ["clipboard-manager:allow-write-text"]);
}

fn assert_commands(commands: &[&str]) {
    let handler = include_str!("bridge.rs");
    let build_manifest = include_str!("../build.rs");
    let capability = include_str!("../capabilities/main.json");
    for command in commands {
        assert!(
            handler.contains(&format!("            {command},")),
            "{command} is absent from the Tauri invoke handler"
        );
        assert!(
            build_manifest.contains(&format!("            \"{command}\",")),
            "{command} is absent from the restricted Tauri command manifest"
        );
        let permission = format!("    \"allow-{}\"", command.replace('_', "-"));
        assert!(
            capability.contains(&permission),
            "{command} is absent from the main-window capability"
        );
    }
}

#[test]
fn diagnostic_commands_are_exposed_to_the_main_window() {
    assert_commands(DIAGNOSTIC_COMMANDS);
}

#[test]
fn startup_recovery_commands_are_exposed_to_the_main_window() {
    assert_commands(STARTUP_COMMANDS);
}
#[test]
fn updater_commands_accept_only_native_actions_in_the_main_window() {
    assert_commands(&[
        "update_status",
        "update_check",
        "update_download",
        "update_install",
        "update_cancel",
    ]);
}

#[test]
fn external_urls_use_only_the_restricted_main_window_command() {
    assert_commands(EXTERNAL_URL_COMMANDS);
}

#[test]
fn titlebar_double_click_uses_the_restricted_native_command() {
    assert_commands(WINDOW_BEHAVIOR_COMMANDS);
}

#[test]
fn host_settings_commands_are_exposed_only_through_the_main_window_handlers() {
    assert_commands(HOST_SETTINGS_COMMANDS);
}

#[test]
fn web_confirmation_resolution_is_exposed_only_through_the_typed_main_window_command() {
    assert_commands(CONFIRMATION_COMMANDS);
}

#[test]
fn model_connection_commands_are_exposed_to_the_main_window() {
    assert_commands(MODEL_CONNECTION_COMMANDS);
}

#[test]
fn classifier_commands_are_exposed_to_the_main_window() {
    assert_commands(CLASSIFIER_COMMANDS);
}

#[test]
fn worker_task_commands_are_exposed_to_the_main_window() {
    assert_commands(WORKER_TASK_COMMANDS);
}

#[test]
fn the_explicit_quit_command_is_exposed_to_the_main_window() {
    assert_commands(&["quit_desktop"]);
}

#[test]
fn profile_restore_retry_is_limited_to_the_main_window_capability() {
    assert_commands(&["retry_agent_settings"]);
}
