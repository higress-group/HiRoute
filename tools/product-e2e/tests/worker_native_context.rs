#![cfg(unix)]

mod worker_product_support;

#[test]
#[ignore = "requires explicitly selected real Codex and Claude ACP installations"]
fn native_worker_context_loads_skills_and_continues_exact_history() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_NATIVE_CONTEXT",
        "worker-native-context-core",
    );
}

#[test]
#[ignore = "requires explicitly selected real Codex and Claude ACP installations"]
fn concurrent_native_workers_keep_routes_and_cancel_only_owned_work() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_NATIVE_CONTEXT_BOUNDARIES",
        "worker-native-context-boundaries",
    );
}
