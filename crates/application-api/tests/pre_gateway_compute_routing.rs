use hiroute_application_api::{
    CHANGE_SPEC_SCHEMA_V1, COMPUTE_CONNECTION_CHANGE_SCHEMA_V1, ComputeConnectionApplyRequestV1,
    ComputeConnectionPreviewRequestV1, PLAN_CONTENT_CHANGE_SCHEMA_V2, PlanContentApplyRequestV2,
    PlanContentPreviewRequestV2,
};

#[test]
fn pre_gateway_compute_routing_dtos_are_strict_and_desktop_round_trip_exactly() {
    let compute = serde_json::json!({
        "change": {
            "schema": COMPUTE_CONNECTION_CHANGE_SCHEMA_V1,
            "discovered_source_ref": "claude/settings/user/source-1",
            "connection_option_id": "zhipu.coding-plan.cn.v1",
            "model_configuration_id": "model.zhipu.glm-5.2",
            "expected_source_revision": 0,
            "expected_binding_revision": 0,
            "expected_inventory_revision": 0,
            "explicit_materialization": true
        }
    });
    let typed: ComputeConnectionPreviewRequestV1 = serde_json::from_value(compute.clone()).unwrap();
    assert_eq!(serde_json::to_value(typed).unwrap(), compute);
    let mut unknown = compute;
    unknown["wire_can_choose_endpoint"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ComputeConnectionPreviewRequestV1>(unknown).is_err());

    let apply_shape = serde_json::json!({
        "spec": {
            "schema_version": CHANGE_SPEC_SCHEMA_V1,
            "command_id": "compute.connection.apply",
            "resource_id": "source/fixture",
            "desired_state": {
                "connection_option_id": "zhipu.coding-plan.cn.v1",
                "source_id": "source/fixture",
                "explicit_materialization": true,
                "expected_source_revision": 0,
                "projection": {"sealed": "fixture-only-shape"}
            }
        },
        "accept_digest": hiroute_application_api::CanonicalDigest::of_bytes(b"preview"),
        "expected_revisions": {"target": 0},
        "idempotency_key": "desktop-apply-1"
    });
    let typed: ComputeConnectionApplyRequestV1 =
        serde_json::from_value(apply_shape.clone()).unwrap();
    assert_eq!(serde_json::to_value(typed).unwrap(), apply_shape);

    let routing = serde_json::json!({
        "change": {
            "schema": PLAN_CONTENT_CHANGE_SCHEMA_V2,
            "target": {"intent": "create", "creation_key": "desktop-plan-one"},
            "editor": {
                "schema": "hiroute.plan-editor/v2",
                "display_name": "Personal",
                "purpose": "personal",
                "mode": "fixed_model",
                "candidates": [{"binding_id": "binding.fixture"}],
                "smart": {"economy": [], "primary": [], "primary_fallback": false, "reselect_on_user_message": false, "classifier": {"kind":"local_rules"}, "complex_keywords": []},
                "free": {"candidates": [], "primary": [], "primary_fallback": false},
                "delegation_enabled": false,
                "requirements": {
                    "tool": false,
                    "vision": false,
                    "streaming": false,
                    "minimum_context_tokens": 0,
                    "minimum_output_tokens": 0
                },
                "limits": {"maximum_attempts": 1, "request_timeout_ms": 30000, "attempt_timeout_ms": 30000}
            },
            "consumed_draft": null
        }
    });
    let typed: PlanContentPreviewRequestV2 = serde_json::from_value(routing.clone()).unwrap();
    assert_eq!(serde_json::to_value(typed).unwrap(), routing);

    let routing_apply = serde_json::json!({
        "change": serde_json::from_value::<serde_json::Value>(routing["change"].clone()).unwrap(),
        "accept_digest": hiroute_application_api::CanonicalDigest::of_bytes(b"routing"),
        "expected_revisions": {"target": 1, "dependencies": {"release.registry": 1, "release.model_data": 1}},
        "idempotency_key": "desktop-routing-1"
    });
    let typed: PlanContentApplyRequestV2 = serde_json::from_value(routing_apply).unwrap();
    assert_eq!(typed.idempotency_key, "desktop-routing-1");
}
