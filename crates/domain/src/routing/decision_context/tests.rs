use super::*;

#[test]
fn policy_rejects_invalid_modes_and_non_u32_json_values() {
    assert!(DecisionContextPolicyV1::default().validate());
    for value in [
        serde_json::json!(0),
        serde_json::json!(-1),
        serde_json::json!(1.5),
        serde_json::json!(4_294_967_296u64),
    ] {
        let value = serde_json::json!({"mode":"custom","window_tokens":value,"typesafe_long_context":"automatic"});
        assert!(
            !serde_json::from_value::<DecisionContextPolicyV1>(value).is_ok_and(|p| p.validate())
        );
    }
    for value in [
        serde_json::json!({"mode":"automatic","window_tokens":32000,"typesafe_long_context":"automatic"}),
        serde_json::json!({"mode":"custom","typesafe_long_context":"automatic"}),
        serde_json::json!({"mode":"custom","window_tokens":32000,"per_question_tokens":32001,"typesafe_long_context":"automatic"}),
    ] {
        assert!(
            !serde_json::from_value::<DecisionContextPolicyV1>(value)
                .unwrap()
                .validate()
        );
    }
}

#[test]
fn reserve_is_checked_at_tiny_large_and_rounding_boundaries() {
    assert_eq!(decision_context_usable_tokens(1), 0);
    assert_eq!(decision_context_usable_tokens(1024), 0);
    assert_eq!(decision_context_usable_tokens(32000), 30400);
    assert_eq!(decision_context_usable_tokens(65536), 62259);
    assert_eq!(decision_context_usable_tokens(u32::MAX), 4_080_218_930);
}

#[test]
fn provider_header_requires_exact_canonical_https_destination_and_codec() {
    for endpoint in [
        "https://api.typesafe.ai/v1/systemone",
        "https://API.TYPESAFE.AI:443/v1/systemone",
    ] {
        assert!(official_typesafe_long_context_destination(
            endpoint,
            DecisionWireProtocolV2::SystemOne
        ));
        assert!(!official_typesafe_long_context_destination(
            endpoint,
            DecisionWireProtocolV2::OpenaiDecisions
        ));
    }
    for endpoint in [
        "http://api.typesafe.ai/v1/systemone",
        "https://api.typesafe.ai:8443/v1/systemone",
        "https://api.typesafe.ai.example/v1/systemone",
        "https://api.typesafe.ai/v1/systemone/",
        "https://user@api.typesafe.ai/v1/systemone",
        "https://api.typesafe.ai/v1/systemone?proxy=1",
        "https://api.typesafe.ai/v1/systemone#fragment",
        "https://openrouter.ai/api/v1/decisions",
    ] {
        assert!(!official_typesafe_long_context_destination(
            endpoint,
            DecisionWireProtocolV2::SystemOne
        ));
    }
}
