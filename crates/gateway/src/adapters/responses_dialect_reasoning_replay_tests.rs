//! Retained typed reasoning state must govern the production Replay projection.
use super::super::reasoning::{prefix_request, producer_request};
use super::*;
use crate::content_ref::JsonValueExt;
use crate::server::core_runtime::model_ir::ContentPart;

#[test]
fn nullable_reasoning_production_replay_matches_inline_at_each_inline_budget() {
    for inline_budget in [0, 31, 4096] {
        for encrypted in [None, Some(Value::Null), Some(json!(""))] {
            let mut body = producer_request(encrypted);
            body["instructions"] = json!("synthetic stable instruction ".repeat(4096));
            body["input"][1]["summary"][0]["text"] =
                json!("synthetic summary \"中\"\n".repeat(4096));
            body["input"][3]["output"] = json!("synthetic tool receipt \"中\"\n".repeat(4096));
            let inline = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
            let backing = Backing::new();
            let request = backing.request_with_budget(&body, inline_budget).unwrap();
            let original = request.clone();
            assert_eq!(
                request.responses_reasoning_history[&1].encrypted_content,
                inline.responses_reasoning_history[&1].encrypted_content
            );
            for standard in [false, true] {
                let expected = project_candidate_request(&inline, &profile(standard)).unwrap();
                let (actual, encoding) = backing.project(&request, &profile(standard));
                assert_eq!(actual, expected.bytes);
                assert_eq!(encoding, expected.function_encoding_projection);
                let actual: Value = serde_json::from_slice(&actual).unwrap();
                assert_eq!(actual["input"], body["input"]);
                assert_eq!(request, original);
            }
            drop(request);
            drop(original);
            backing.release();
        }
    }
}

#[test]
fn opaque_reasoning_stays_rejected_after_production_replay_externalization() {
    for encrypted in [
        " ".repeat(4096),
        "synthetic ciphertext \"中\"\n".repeat(4096),
    ] {
        let body = producer_request(Some(json!(encrypted)));
        let backing = Backing::new();
        let request = backing.request(&body);
        let ContentPart::ProviderState { state } = &request.messages[1].content[0] else {
            panic!("opaque provider state")
        };
        assert!(
            state.value.content_ref().is_some()
                || state
                    .value
                    .as_str()
                    .and_then(ContentValueExt::content_ref)
                    .is_some()
        );
        let original = request.clone();
        assert_eq!(
            project_candidate_request_template(&request, &profile(true)).unwrap_err(),
            ProtocolAdapterError::ModelIr(ModelIrError::ProviderStateNotPortable)
        );
        let (actual, _) = backing.project(&request, &profile(false));
        assert_eq!(
            serde_json::from_slice::<Value>(&actual).unwrap()["input"],
            body["input"]
        );
        assert_eq!(request, original);
        drop(request);
        drop(original);
        backing.release();
    }
}

#[test]
fn malformed_reasoning_is_not_reinterpreted_by_production_replay() {
    for malformed in [json!([]), json!({}), json!(false), json!(42)] {
        let mut body = producer_request(Some(malformed));
        body["instructions"] = json!("synthetic instruction ".repeat(4096));
        let backing = Backing::new();
        assert_eq!(
            backing.request_with_budget(&body, 0).unwrap_err(),
            ModelIrError::InvalidField("encrypted_content")
        );
        backing.release();
    }
}

#[test]
fn production_replay_prefix_cleanup_uses_original_canonical_reasoning_positions() {
    for retained in [
        None,
        Some(Value::Null),
        Some(json!("")),
        Some(json!("retained opaque")),
    ] {
        let mut body = prefix_request(retained.clone());
        body["instructions"] = json!("synthetic instruction ".repeat(4096));
        body["input"][2]["encrypted_content"] = json!("old opaque ".repeat(4096));
        let inline = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
        let backing = Backing::new();
        let request = backing.request(&body);
        let original = request.clone();
        for end in [1, 2, 3] {
            let expected =
                project_candidate_request_template_with_cleanup(&inline, &profile(true), Some(end));
            let actual = project_candidate_request_template_with_cleanup(
                &request,
                &profile(true),
                Some(end),
            );
            if end == 1 || (end == 2 && retained.as_ref().is_some_and(|v| v == "retained opaque")) {
                assert_eq!(
                    expected.unwrap_err(),
                    ProtocolAdapterError::ModelIr(ModelIrError::ProviderStateNotPortable)
                );
                assert_eq!(
                    actual.unwrap_err(),
                    ProtocolAdapterError::ModelIr(ModelIrError::ProviderStateNotPortable)
                );
            } else {
                let expected = expected.unwrap();
                let mut reader = sequential_attempt_body(
                    actual.unwrap(),
                    backing.store.clone(),
                    &backing.budget,
                    37,
                )
                .unwrap();
                let mut bytes = Vec::new();
                while let Some(chunk) = reader.next_chunk().unwrap() {
                    bytes.extend_from_slice(chunk.bytes());
                }
                reader.release();
                assert_eq!(bytes, expected.bytes.as_ref());
            }
            assert_eq!(request, original);
        }
        drop(request);
        drop(original);
        backing.release();
    }
}
