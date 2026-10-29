use super::*;
use crate::content_ref::ContentRef;
use crate::server::core_runtime::adapters::decode_ingress_request;
use crate::server::request_plan::IngressProtocol;
use serde_json::json;

#[test]
fn oversized_prior_content_loses_only_fingerprints_and_next_request_recovers() {
    let mut request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({
            "model":"route", "input":[
                {"role":"user", "content":"A"},
                {"role":"assistant", "content":"prior result"},
                {"role":"user", "content":"B"}
            ]
        }),
    )
    .unwrap();
    let original = request.clone();
    for text in [
        "x".repeat(super::super::fingerprint::MAX_BYTES + 1),
        ContentRef::new(1, 0, u64::MAX, u64::MAX).wire_marker(),
    ] {
        request.messages[1].content = vec![ContentPart::Text { text }];
        let evidence = observed_history_with_replay(&request, &[7; 32], None).unwrap();
        assert!(evidence.branch_anchors(&[1]).is_none());
        assert!(evidence.measure(Some(1)).is_none());
        assert_eq!(request.messages[2], original.messages[2]);
    }
    let evidence = observed_history_with_replay(&original, &[7; 32], None).unwrap();
    assert!(evidence.branch_anchors(&[1]).is_some());
    assert!(evidence.measure(Some(1)).is_some());
}

#[test]
fn large_peer_json_and_instruction_metadata_cannot_bypass_evidence_budget() {
    let large = "x".repeat(super::super::fingerprint::MAX_BYTES / 6 + 1);
    let mut request = decode_ingress_request(IngressProtocol::Responses, &json!({
        "model":"route", "input":[{"type":"agent_message", "author":"child", "recipient":"parent",
            "content":[{"type":"encrypted_content", "encrypted_content":large}]}]
    })).unwrap();
    assert!(
        observed_history_with_replay(&request, &[7; 32], None)
            .unwrap()
            .branch_anchors(&[])
            .is_none()
    );
    request.native_body = Some(json!({"output_config":{"format":{"schema":large}}}));
    assert!(observed_history_with_replay(&request, &[7; 32], None).is_none());
}

#[test]
fn oversized_current_setup_does_not_disable_bounded_execution_prefixes() {
    let mut request = decode_ingress_request(IngressProtocol::Responses,
        &json!({"model":"route","instructions":"normal setup","input":[{"role":"user","content":"actual A"}]})).unwrap();
    let before = branch_anchors(&request, &[7; 32], None, &[1]).unwrap();
    request.instructions[0].content = vec![ContentPart::Text {
        text: "x".repeat(super::super::fingerprint::MAX_BYTES + 1),
    }];
    assert!(observed_history_with_replay(&request, &[7; 32], None).is_none());
    let after = branch_anchors(&request, &[7; 32], None, &[1]).unwrap();
    assert_eq!(before.complete, after.complete);
    assert_eq!(before.prefixes, after.prefixes);
}
