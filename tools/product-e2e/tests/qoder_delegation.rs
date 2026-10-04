#![cfg(unix)]

mod worker_product_support;

#[test]
#[ignore = "requires an explicitly selected, normally logged-in Qoder context and native CLI"]
fn qoder_main_agent_uses_installed_user_skill_to_delegate_real_work() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_QODER_MAIN",
        "qoder-main-agent-delegation",
    );
}

#[test]
#[ignore = "requires an explicitly selected, normally logged-in Qoder context and native CLI"]
fn qoder_worker_uses_native_skills_and_continues_the_frozen_task() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_QODER_CORE",
        "worker-native-context-core",
    );
}

#[test]
#[ignore = "requires an explicitly selected, normally logged-in Qoder context and native CLI"]
fn qoder_workers_route_independently_and_cancel_only_owned_work() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_QODER_BOUNDARIES",
        "worker-native-context-boundaries",
    );
}

#[test]
#[ignore = "requires an explicitly selected, normally logged-in Qoder context and native CLI"]
fn qoder_worker_compaction_keeps_the_frozen_managed_route() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_QODER_COMPACTION",
        "qoder-worker-compaction-route",
    );
}

#[test]
#[ignore = "requires a dedicated normally logged-in Qoder MODEL context; writes only its owned test settings"]
fn qoder_main_agent_uses_persisted_additional_model_routes() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_QODER_MODELS",
        "qoder-persisted-model-routes",
    );
}
