#![cfg(unix)]
mod worker_product_support;

#[test]
#[ignore = "requires the selected official Pi SDK/Node and real production binaries"]
fn pi_worker_uses_native_skills_and_continues_the_frozen_task() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_PI_CORE",
        "worker-native-context-core",
    );
}
#[test]
#[ignore = "requires the selected official Pi SDK/Node and real production binaries"]
fn pi_workers_route_independently_and_reject_missing_or_corrupt_history() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_PI_BOUNDARIES",
        "worker-native-context-boundaries",
    );
}

#[test]
#[ignore = "requires the selected official Pi SDK/Node and real production binaries"]
fn pi_agent_saved_model_routes_preserve_defaults_and_restore_independently() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_PI_MODELS",
        "pi-persisted-model-routes",
    );
}

#[test]
#[ignore = "requires the selected official Pi SDK/Node and real production binaries"]
fn pi_agent_uses_installed_user_skill_to_delegate_through_public_cli() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_PI_MAIN",
        "pi-main-agent-delegation",
    );
}
#[test]
#[ignore = "requires the selected official Pi SDK/Node and real production binaries"]
fn pi_worker_compaction_uses_frozen_route_and_continues_exact_history() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_PI_COMPACTION",
        "pi-worker-compaction-route",
    );
}

#[test]
#[ignore = "requires the selected official Pi installation and real production binaries"]
fn pi_static_api_discovery_imports_effective_source_and_rejects_stale_save() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_PI_DISCOVERY",
        "pi-static-source-import",
    );
}
