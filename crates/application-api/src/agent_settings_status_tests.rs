use super::*;

#[test]
fn model_status_union_keeps_existing_v2_bytes() {
    let old = r#"{"schema":"hiroute.agent-model-settings-status/v2","context_id":"agent-context/codex","state":"not_configured","operation_id":null,"operation_state":null,"restore_point_ref":null,"applied_revision":null,"surface_results":[],"model_verified":false}"#;
    let status: AgentSettingsStatusV2 = serde_json::from_str(old).unwrap();
    assert!(status.valid());
    assert!(status.model().is_some());
    assert_eq!(serde_json::to_string(&status).unwrap(), old);
}

#[test]
fn collaboration_only_status_has_no_model_shell_and_rejects_mixed_shape() {
    let wire = r#"{"schema":"hiroute.agent-collaboration-only-settings-status/v2","context_id":"agent-context/qoder","collaboration":{"schema":"hiroute.agent-collaboration-settings-status/v2","state":"not_configured","operation_id":null,"operation_state":null,"restore_point_ref":null}}"#;
    let status: AgentSettingsStatusV2 = serde_json::from_str(wire).unwrap();
    assert!(status.valid());
    assert!(status.model().is_none());
    assert!(status.collaboration().is_some());
    assert_eq!(serde_json::to_string(&status).unwrap(), wire);
    let mut mixed: serde_json::Value = serde_json::from_str(wire).unwrap();
    mixed["model_verified"] = false.into();
    assert!(serde_json::from_value::<AgentSettingsStatusV2>(mixed).is_err());
    let mut missing: serde_json::Value = serde_json::from_str(wire).unwrap();
    missing.as_object_mut().unwrap().remove("collaboration");
    assert!(serde_json::from_value::<AgentSettingsStatusV2>(missing).is_err());
    let invalid: AgentSettingsStatusV2 = serde_json::from_str(&wire.replace(
        "collaboration-only-settings-status/v2",
        "model-settings-status/v2",
    ))
    .unwrap();
    assert!(!invalid.valid());
}
