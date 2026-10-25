//! Actual V2 wire reads with real stores; no provider or native client is invoked.
use super::*;
use crate::control::LocalControlDaemon;
use hiroute_application::ApplicationService;
use hiroute_application_api::{self as api, LocalControlWireRequestV2};

#[test]
fn model_projection_omits_unrequested_facets_and_preserves_full_v2_status() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::projection::model_projection_omits_unrequested_facets_and_preserves_full_v2_status",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let (runtime, _) = open_with_codex_fixture_surfaces(root.path());
    let context = runtime
        .adapter
        .settings_context_for_agent("agent_codex_default")
        .unwrap();
    let service = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
    let read = |extra: serde_json::Value| {
        let mut payload = json!({"schema_version":{"major":2,"minor":0},"context_id":context});
        payload
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        service.dispatch_wire(LocalControlWireRequestV2 {
            schema_version: api::SchemaVersion::new(2, 0),
            request_id: "projection-test".into(),
            operation_id: "GetAgentConnectionStatus".into(),
            payload,
            protected_grant: None,
        })
    };
    let old = read(json!({}));
    assert!(old.error.is_none(), "{old:?}");
    let old = old.data.unwrap();
    assert!(old.get("collaboration").is_some());
    assert!(old.get("management_skill").is_none());
    let full = read(json!({"projection":"full","include_skills":true}));
    assert!(full.error.is_none(), "{full:?}");
    let full = full.data.unwrap();
    assert!(full.get("management_skill").is_some() && full.get("collaboration").is_some());
    let model = read(json!({"projection":"model"}));
    assert!(model.error.is_none(), "{model:?}");
    let model = model.data.unwrap();
    let mut expected = old.clone();
    expected.as_object_mut().unwrap().remove("collaboration");
    assert_eq!(model, expected);

    for extra in [
        json!({"projection":"unknown"}),
        json!({"projection":"model","include_skills":true}),
        json!({"projection":"model","effective":{"plan_id":"plan/fixture"}}),
    ] {
        assert_eq!(
            read(extra).error.unwrap().code,
            api::ErrorCode::InvalidArguments
        );
    }

    // A broken, unrequested Skill resource read must not block model selection.
    // This is also a negative control: the existing complete view must still fail.
    let db = rusqlite::Connection::open(root.path().join("live/control.db")).unwrap();
    db.execute("DROP TABLE agent_skill_installations", [])
        .unwrap();
    let narrow = read(json!({"projection":"model"}));
    assert!(narrow.error.is_none(), "{narrow:?}");
    assert_eq!(narrow.data.unwrap(), model);
    assert!(read(json!({"include_skills":true})).error.is_some());
}
