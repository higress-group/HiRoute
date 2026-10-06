#![cfg(unix)]
mod worker_product_support;

#[test]
#[ignore = "requires the selected official DSH CLI and real production binaries"]
fn dsh_worker_uses_native_skills_and_continues_the_frozen_task() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_DSH_CORE",
        "worker-native-context-core",
    );
}
#[test]
#[ignore = "requires the selected official DSH CLI and real production binaries"]
fn dsh_workers_route_independently_cancel_and_reject_missing_history() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_DSH_BOUNDARIES",
        "worker-native-context-boundaries",
    );
}
#[test]
#[ignore = "requires the selected official DSH CLI and real production binaries"]
fn dsh_agent_saved_model_routes_preserve_defaults_and_restore_independently() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_DSH_MODELS",
        "dsh-persisted-model-routes",
    );
}
#[test]
#[ignore = "requires the selected official DSH CLI and real production binaries"]
fn dsh_agent_uses_installed_user_skill_to_delegate_through_public_cli() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_DSH_MAIN",
        "dsh-main-agent-delegation",
    );
}
#[test]
#[ignore = "requires the selected official DSH CLI and real production binaries"]
fn dsh_static_api_discovery_imports_effective_source_and_rejects_stale_save() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_DSH_DISCOVERY",
        "dsh-static-source-import",
    );
}
