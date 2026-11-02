use super::*;
use crate::{
    CanonicalDigest, FrozenNativeProtocolV1, JudgmentSettingsV1, NativeProtocolV1,
    SMART_SAVING_SCOPE_ID,
};

fn context(window: u32) -> DecisionContextConfigurationV1 {
    let policy = DecisionContextPolicyV1 {
        mode: DecisionContextModeV1::Custom,
        window_tokens: Some(window),
        ..Default::default()
    };
    let mut resolved = ResolvedDecisionContextV1 {
        request_window_tokens: window,
        per_question_window_tokens: window,
        request_source: DecisionContextSourceV1::User,
        per_question_source: DecisionContextSourceV1::User,
        catalog_revision: 1,
        catalog_digest: CanonicalDigest::of_bytes(b"catalog"),
        metadata: None,
        typesafe_long_context_header: false,
        accounting_algorithm: DECISION_CONTEXT_ALGORITHM_V1.into(),
        reserve_policy: DECISION_CONTEXT_RESERVE_V1.into(),
        context_digest: CanonicalDigest::of_bytes(&[]),
    };
    resolved.context_digest = resolved.calculate_digest().unwrap();
    DecisionContextConfigurationV1 { policy, resolved }
}

fn service(window: u32) -> DecisionServiceV3 {
    DecisionServiceV3 {
        schema: DecisionServiceSchemaV3::V3,
        id: "decision/example".into(),
        revision: 1,
        name: "Example".into(),
        connection: DecisionConnectionV2::Native {
            provider: "compatible".into(),
            wire_protocol: DecisionWireProtocolV2::OpenaiDecisions,
            model: "custom-model".into(),
            endpoint: "https://example.test/v1/decisions".into(),
            timeout_ms: 3000,
            authentication: DecisionAuthenticationV2::Independent {
                auth_header: ClassifierAuthHeaderV1 {
                    name: "Authorization".into(),
                    value_secret_ref: "decision/example/secret".into(),
                },
            },
        },
        context: Some(Box::new(context(window))),
    }
}

#[test]
fn current_native_requires_snapshot_and_never_deserializes_as_legacy_content() {
    let service = service(32000);
    assert!(service.validate());
    let wire = serde_json::to_value(&service).unwrap();
    assert_eq!(
        serde_json::from_value::<DecisionServiceRecord>(wire.clone()).unwrap(),
        DecisionServiceRecord::V3(service)
    );
    assert!(serde_json::from_value::<DecisionServiceV2>(wire.clone()).is_err());
    let mut missing = wire.clone();
    missing.as_object_mut().unwrap().remove("context");
    assert!(serde_json::from_value::<DecisionServiceRecord>(missing).is_err());
    let mut forged = wire;
    forged["context"]["resolved"]["request_window_tokens"] = serde_json::json!(64000);
    assert!(serde_json::from_value::<DecisionServiceRecord>(forged).is_err());
}

#[test]
fn custom_extensions_cannot_acquire_native_context_or_header_policy() {
    let mut service = service(32000);
    service.connection = DecisionConnectionV2::Custom {
        endpoint: "http://127.0.0.1:8080/decide".into(),
        timeout_ms: 3000,
        auth_header: None,
    };
    assert!(!service.validate());
    service.context = None;
    assert!(service.validate());
    let wire = serde_json::to_value(&service).unwrap();
    assert!(wire.get("context").is_none());
    assert_eq!(
        serde_json::from_value::<DecisionServiceV3>(wire).unwrap(),
        service
    );
}

#[test]
fn context_budget_changes_assessment_identity_but_preserves_fixed_questions_and_legacy_bytes() {
    let protocol = NativeProtocolV1::c_v4();
    let mut judgment = JudgmentSettingsV1::default();
    protocol.stock_assessment(&mut judgment);
    let legacy = FrozenNativeProtocolV1::freeze(
        &protocol,
        judgment.degree.definition().into(),
        [(SMART_SAVING_SCOPE_ID.into(), judgment.clone())],
    )
    .unwrap();
    let legacy_wire = serde_json::to_value(&legacy).unwrap();
    assert!(legacy_wire.get("context_budget").is_none());
    let first = legacy
        .clone()
        .with_context(Some(&context(32000).resolved))
        .unwrap();
    let second = legacy
        .clone()
        .with_context(Some(&context(64000).resolved))
        .unwrap();
    first.validate_smart(&judgment).unwrap();
    second.validate_smart(&judgment).unwrap();
    assert_ne!(first.digest, second.digest);
    assert_ne!(
        first
            .execution_policy(SMART_SAVING_SCOPE_ID, &judgment, "Smart saving")
            .criteria_digest,
        second
            .execution_policy(SMART_SAVING_SCOPE_ID, &judgment, "Smart saving")
            .criteria_digest
    );
    assert_eq!(first.protocol, legacy.protocol);
    assert_eq!(first.scopes, legacy.scopes);
    assert_eq!(first.with_context(None).unwrap(), legacy);
    assert_eq!(
        serde_json::from_value::<FrozenNativeProtocolV1>(legacy_wire).unwrap(),
        legacy
    );
}
