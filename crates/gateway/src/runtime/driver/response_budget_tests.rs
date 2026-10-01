use super::*;
use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
use hiroute_gateway_core::runtime::body::BudgetTree;

#[test]
fn cleanup_error_classification_requires_an_explicit_history_rejection() {
    let responses = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "model",
        fixed_reasoning("fixed"),
    );
    let messages = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Messages,
        IngressProtocol::Messages,
        "model",
        fixed_reasoning("fixed"),
    );
    let error = |code: &str, message: &str| ModelError {
        status: Some(400),
        code: Some(code.into()),
        message: Some(message.into()),
        retryable: None,
    };
    assert_eq!(
        sanitized_provider_kind(&responses, &error("invalid_encrypted_content", "redacted")),
        Some(ProviderFailureKind::ReasoningHistory)
    );
    assert_eq!(
        sanitized_provider_kind(
            &messages,
            &error(
                "invalid_request_error",
                "Invalid `signature` in `thinking` block"
            )
        ),
        Some(ProviderFailureKind::ReasoningHistory)
    );
    for message in [
        "invalid model",
        "thinking budget exceeds max_tokens",
        "invalid tool signature",
        "unsupported tools",
    ] {
        assert_eq!(
            sanitized_provider_kind(&messages, &error("invalid_request_error", message)),
            Some(ProviderFailureKind::PermanentClient)
        );
    }
    assert_ne!(
        sanitized_provider_kind(
            &responses,
            &error("invalid_request_error", "thinking failed")
        ),
        Some(ProviderFailureKind::ReasoningHistory)
    );
}

#[test]
fn protocol_prefix_grows_beyond_initial_queue_capacity_and_releases_memory() {
    let budget = BudgetTree::new(16 * 1024 * 1024, 16 * 1024 * 1024)
        .unwrap()
        .stream(16 * 1024 * 1024)
        .unwrap();
    let plan = hiroute_gateway_core::runtime::body::BodyPlan::PassThrough {
        max_chunk_bytes: 64 * 1024,
    };
    let mut queue =
        ChargedBodyQueue::new(&budget, MemoryRole::ResponsePrefix, &plan, usize::MAX, 1).unwrap();
    let expected = vec![b'x'; 2 * 1024 * 1024];
    push_queue_bytes(&mut queue, &budget, expected.clone()).unwrap();
    let mut actual = Vec::new();
    while let Some(chunk) = queue.pop_front() {
        actual.extend_from_slice(chunk.bytes());
    }
    assert_eq!(actual, expected);
    queue.clear_and_release();
    assert_eq!(budget.snapshot().unwrap().live, 0);
}

#[test]
fn precommit_decoder_is_bounded_by_bytes_not_transport_chunk_count() {
    let budget = BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024)
        .unwrap()
        .stream(4 * 1024 * 1024)
        .unwrap();
    let mut decoder = PrecommitDecoderBudget::new(&budget).unwrap();
    for _ in 0..1_000 {
        decoder.charge_frame(128).unwrap();
    }
    decoder.charge_frame(256 * 1024).unwrap();
    assert!(
        decoder.charge_frame(4 * 1024 * 1024).is_err(),
        "the actual memory budget remains authoritative"
    );
}

#[test]
fn connector_error_semantics_are_sealed_by_profile_not_connector_allowlist() {
    let mut profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "gpt-codex",
        fixed_reasoning("fixed"),
    );
    profile.connector.connector_id = "connector.cpa.codex".into();
    assert!(connector_error_profile(&profile).is_ok());

    profile.connector.request_path = "/registered/provider/responses".into();
    assert!(
        connector_error_profile(&profile).is_ok(),
        "a catalog-bound exact provider path need not equal the public ingress path"
    );

    profile.connector.schema_version = "hiroute.connector-profile/v2".into();
    assert!(connector_error_profile(&profile).is_err());
}
