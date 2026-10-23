use crate::agent_turn_history::boundary::{BoundaryCapture, incoming_boundary};
use crate::context_hold::fingerprint::FingerprintBudget;
use crate::server::core_runtime::adapters::decode_ingress_request;
use crate::server::core_runtime::model_ir::{ModelEvent, ResponseItemStatus};
use crate::server::request_plan::IngressProtocol;
use serde_json::json;

#[test]
fn native_reasoning_replay_compares_accepted_summary_before_visible_answer() {
    let (_directory, replay) = super::replay();
    for summary in ["", "Read the requested file first."] {
        let mut capture = BoundaryCapture::default();
        capture.accept(&ModelEvent::ReasoningDelta {
            index: 0,
            text: summary.into(),
        });
        capture.accept(&ModelEvent::ReasoningFinished {
            index: 0,
            text: summary.into(),
            status: ResponseItemStatus::Completed,
        });
        capture.accept(&ModelEvent::TextFinished {
            index: 1,
            text: "READY".into(),
            annotations: vec![],
            status: ResponseItemStatus::Completed,
        });
        let wire = json!({"model":"route","input":[
            {"type":"reasoning","id":"rs_0","status":"completed","summary":[{"type":"summary_text","text":summary}]},
            {"type":"message","id":"msg_1","role":"assistant","content":[{"type":"output_text","text":"READY","annotations":[]}]},
            {"role":"user","content":"next task"}
        ]});
        let decoded = decode_ingress_request(IngressProtocol::Responses, &wire).unwrap();
        let accepted = capture
            .digest()
            .expect("bounded plaintext reasoning is comparable");
        assert_eq!(
            incoming_boundary(&decoded, 0, &replay, &mut FingerprintBudget::default()),
            Some(accepted)
        );

        let mut changed = wire.clone();
        changed["input"][0]["summary"][0]["text"] = json!("rewritten reasoning");
        let decoded = decode_ingress_request(IngressProtocol::Responses, &changed).unwrap();
        assert_ne!(
            incoming_boundary(&decoded, 0, &replay, &mut FingerprintBudget::default()),
            Some(accepted)
        );
        let mut opaque = wire;
        opaque["input"][0]["encrypted_content"] = json!("uncaptured provider state");
        let decoded = decode_ingress_request(IngressProtocol::Responses, &opaque).unwrap();
        assert!(
            incoming_boundary(&decoded, 0, &replay, &mut FingerprintBudget::default()).is_none()
        );
    }
}
