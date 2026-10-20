#![cfg(unix)]
mod worker_product_support;
#[test]
#[ignore = "requires selected official OpenCode and production binaries"]
fn opencode_worker_uses_native_skills_and_continues_the_frozen_task() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_OPENCODE_CORE",
        "worker-native-context-core",
    );
}
#[test]
#[ignore = "requires selected official OpenCode and production binaries"]
fn opencode_agent_saved_model_routes_preserve_defaults_and_restore_independently() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_OPENCODE_MODELS",
        "opencode-persisted-model-routes",
    );
}
#[test]
#[ignore = "requires selected official OpenCode and production binaries"]
fn opencode_agent_uses_installed_user_skill_to_delegate_through_public_cli() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_OPENCODE_MAIN",
        "opencode-main-agent-delegation",
    );
}

#[test]
#[ignore = "requires selected official OpenCode and production binaries"]
fn opencode_workers_route_independently_cancel_and_refuse_missing_history() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_OPENCODE_BOUNDARIES",
        "worker-native-context-boundaries",
    );
}
#[test]
#[ignore = "requires selected official OpenCode and production binaries"]
fn opencode_static_api_import_rechecks_source_and_preserves_native_files() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_OPENCODE_DISCOVERY",
        "opencode-static-source-import",
    );
}

#[test]
#[ignore = "requires selected official OpenCode and production binaries"]
fn opencode_messages_worker_preserves_tools_and_exact_continue() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_OPENCODE_MESSAGES",
        "worker-native-context-core",
    );
}

#[test]
#[ignore = "requires selected official OpenCode and production binaries"]
fn opencode_native_compaction_and_auxiliary_requests_keep_the_frozen_route() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_OPENCODE_COMPACTION",
        "opencode-worker-compaction-route",
    );
}
