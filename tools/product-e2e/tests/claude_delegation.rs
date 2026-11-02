#![cfg(unix)]
mod worker_product_support;

#[test]
#[ignore = "requires the selected real Claude CLI, ACP adapter and Node"]
fn claude_worker_capability_admission_preserves_native_skills_and_exact_continue() {
    worker_product_support::run_real_worker_scenario(
        "HIROUTE_PRODUCT_CLAUDE_CORE",
        "worker-native-context-core",
    );
}
