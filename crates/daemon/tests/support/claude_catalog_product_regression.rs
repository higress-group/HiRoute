//! Real CLI, Local Control, saved bindings and Gateway with the shipped bundle.
//! Only the external CPA OAuth/catalog/inference transport is synthetic.

#[test]
fn real_process_claude_haiku_catalog_restores_vision_limits_and_thinking() {
    super::run_script_case("claude_subscription_catalog_product.py", Some("haiku"));
}

#[test]
fn real_process_claude_sonnet_catalog_preserves_adaptive_effort() {
    super::run_script_case("claude_subscription_catalog_product.py", Some("sonnet"));
}
