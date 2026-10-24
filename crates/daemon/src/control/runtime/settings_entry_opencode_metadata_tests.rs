//! Metadata fallback uses the ordinary public settings and protected Restore lifecycle.
use super::models::{model_spec, model_status, restore_spec};
use super::*;

fn spec(fixture: &QoderFixture, enabled: bool) -> serde_json::Value {
    let mut value = model_spec(fixture);
    value["model"]["settings"]["mode"] = json!("opencode_additional");
    if !enabled {
        value["model"]["settings"]["routing_metadata"] = json!(false);
    }
    value
}
fn adapter(fixture: &QoderFixture) -> std::path::PathBuf {
    fixture
        .runtime
        .adapter
        .scanner
        .native_routing_adapter_target(AgentKindV1::OpenCode)
        .unwrap()
}
fn preview(fixture: &QoderFixture, spec: &serde_json::Value) -> serde_json::Value {
    let result = fixture.service.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(result.error.is_none(), "{result:?}");
    result.data.unwrap()
}
fn blocked(fixture: &QoderFixture, selection: &serde_json::Value, reason: &str) {
    let before = fs::read(&fixture.native_config).unwrap();
    let resource = fs::read(adapter(fixture)).ok();
    let result = preview(fixture, selection);
    assert_eq!(result["applicable"], false, "{result}");
    assert!(
        result["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["reason"] == reason),
        "{result}"
    );
    let rejected = fixture.service.dispatch_wire(request(
        "ApplyAgentConnectionChange",
        json!({"spec":selection, "accept_digest":result["accept_digest"],
            "dependency_digest":result["dependency_digest"],
            "expected_revisions":result["expected_revisions"],
            "idempotency_key":format!("rejected-{reason}")}),
        None,
    ));
    assert!(rejected.error.is_some(), "{rejected:?}");
    assert_eq!(fs::read(&fixture.native_config).unwrap(), before);
    assert_eq!(fs::read(adapter(fixture)).ok(), resource);
}
fn restore(fixture: &QoderFixture, key: &str) {
    apply(
        &fixture.service,
        &fixture.runtime,
        restore_spec(fixture, &model_status(fixture)),
        key,
    );
    assert_eq!(model_status(fixture)["state"], "not_configured");
}

#[test]
fn opencode_metadata_cold_configuration_persists_and_recovers_normal_adapter() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::opencode_metadata::opencode_metadata_cold_configuration_persists_and_recovers_normal_adapter",
    ) {
        return;
    }
    let fixture = QoderFixture::with_kind(true, AgentKindV1::OpenCode);
    let path = adapter(&fixture);
    let other = path.parent().unwrap().join("user-plugin.js");
    private_file(&other, b"export default async () => ({})\n");
    let baseline = fs::read(&fixture.native_config).unwrap();
    let degraded = spec(&fixture, false);
    apply(
        &fixture.service,
        &fixture.runtime,
        degraded.clone(),
        "metadata-off-cold",
    );
    assert!(!path.exists());
    assert_eq!(model_status(&fixture)["state"], "configured");
    assert_eq!(
        model_status(&fixture)["current_selection"]["routing_metadata"],
        false
    );
    let configured: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.native_config).unwrap()).unwrap();
    assert!(!configured["provider"].as_object().unwrap().is_empty());
    let fixture = super::native_routing::reopen(fixture);
    assert_eq!(
        model_status(&fixture)["current_selection"]["routing_metadata"],
        false
    );
    assert_eq!(model_status(&fixture)["state"], "configured");
    let mut rotation = degraded;
    rotation["access_token"] = json!({"intent":"regenerate"});
    apply(
        &fixture.service,
        &fixture.runtime,
        rotation,
        "metadata-off-rotate",
    );
    assert!(!path.exists());
    let routed_configuration = fs::read(&fixture.native_config).unwrap();
    // Re-enabling uses the existing journal upgrade from a plugin-free Operation.
    apply(
        &fixture.service,
        &fixture.runtime,
        spec(&fixture, true),
        "metadata-on-recover",
    );
    assert_eq!(
        fs::read(&fixture.native_config).unwrap(),
        routed_configuration
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        hiroute_integrations::OPENCODE_ROUTING_ADAPTER.as_bytes()
    );
    assert!(
        model_status(&fixture)["current_selection"]
            .get("routing_metadata")
            .is_none()
    );
    blocked(
        &fixture,
        &spec(&fixture, false),
        "native_routing_adapter_restore_required",
    );
    restore(&fixture, "metadata-normal-restore");
    assert!(!path.exists());
    assert_eq!(fs::read(&fixture.native_config).unwrap(), baseline);
    apply(
        &fixture.service,
        &fixture.runtime,
        spec(&fixture, false),
        "metadata-off-after-restore",
    );
    restore(&fixture, "metadata-off-restore");
    assert_eq!(fs::read(&fixture.native_config).unwrap(), baseline);
    assert_eq!(
        fs::read(&other).unwrap(),
        b"export default async () => ({})\n"
    );
}

#[test]
fn opencode_metadata_fallback_never_adopts_borrowed_or_foreign_plugins() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::opencode_metadata::opencode_metadata_fallback_never_adopts_borrowed_or_foreign_plugins",
    ) {
        return;
    }
    let fixture = QoderFixture::with_kind(true, AgentKindV1::OpenCode);
    let path = adapter(&fixture);
    private_file(
        &path,
        hiroute_integrations::OPENCODE_ROUTING_ADAPTER.as_bytes(),
    );
    apply(
        &fixture.service,
        &fixture.runtime,
        spec(&fixture, true),
        "metadata-borrow",
    );
    blocked(
        &fixture,
        &spec(&fixture, false),
        "native_routing_adapter_restore_required",
    );
    restore(&fixture, "metadata-borrow-restore");
    assert!(path.exists());
    blocked(
        &fixture,
        &spec(&fixture, false),
        "native_routing_adapter_external_resource",
    );
    private_file(&path, b"unrelated user-owned plugin\n");
    blocked(
        &fixture,
        &spec(&fixture, false),
        "native_routing_adapter_external_resource",
    );
    assert_eq!(fs::read(&path).unwrap(), b"unrelated user-owned plugin\n");
}

#[test]
fn opencode_metadata_plugin_appearing_after_preview_rejects_before_model_write() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::opencode_metadata::opencode_metadata_plugin_appearing_after_preview_rejects_before_model_write",
    ) {
        return;
    }
    let fixture = QoderFixture::with_kind(true, AgentKindV1::OpenCode);
    let selection = spec(&fixture, false);
    let p = preview(&fixture, &selection);
    assert_eq!(p["applicable"], true);
    let before = fs::read(&fixture.native_config).unwrap();
    private_file(&adapter(&fixture), b"user created plugin after preview\n");
    let response = fixture.service.dispatch_wire(request(
        "ApplyAgentConnectionChange",
        json!({"spec":selection,
        "accept_digest":p["accept_digest"],"dependency_digest":p["dependency_digest"],
        "expected_revisions":p["expected_revisions"],"idempotency_key":"metadata-off-stale"}),
        None,
    ));
    assert!(response.error.is_some(), "{response:?}");
    assert_eq!(fs::read(&fixture.native_config).unwrap(), before);
    assert_eq!(
        fs::read(adapter(&fixture)).unwrap(),
        b"user created plugin after preview\n"
    );
    assert_eq!(model_status(&fixture)["state"], "not_configured");
}
