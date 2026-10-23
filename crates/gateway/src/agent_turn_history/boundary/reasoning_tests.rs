use crate::agent_turn_history::boundary::{BoundaryCapture, incoming_boundary};
use crate::context_hold::fingerprint::FingerprintBudget;
use crate::server::core_runtime::adapters::decode_ingress_request;
use crate::server::core_runtime::model_ir::{ModelEvent, ResponseItemStatus};
use crate::server::request_plan::IngressProtocol;
use serde_json::json;

fn production_request(
    wire: &serde_json::Value,
    replay: &crate::replay::ReplayStore,
    inline_limit: usize,
) -> crate::server::core_runtime::model_ir::ModelRequestIRV1 {
    use crate::content_ref::{
        compact_ingress_document_with_markers, externalize_model_request, parse_ingress_document,
        scan_ingress_document,
    };
    let mut raw = replay.begin_raw().unwrap();
    raw.append(&serde_json::to_vec(wire).unwrap()).unwrap();
    let raw = raw.seal().unwrap();
    let stats = scan_ingress_document(replay.reader(&raw).unwrap()).unwrap();
    let (mut document, mut workspace) =
        parse_ingress_document(IngressProtocol::Responses, replay, &raw, stats).unwrap();
    compact_ingress_document_with_markers(
        IngressProtocol::Responses,
        &mut document,
        replay,
        workspace.generated_markers(),
    )
    .unwrap();
    let mut request = decode_ingress_request(IngressProtocol::Responses, &document).unwrap();
    externalize_model_request(&mut request, replay, inline_limit).unwrap();
    request
}

#[test]
fn production_reasoning_boundary_is_independent_of_aggregate_inline_allowance() {
    for summary in ["", "Read \\\"notes\\\" first.\n中文", "__hiroute_content_ref_v2_1_0_1_1__"] {
        for inline_limit in [0, 8 * 1024, 64 * 1024] {
            let (_directory, replay) = super::replay();
            let mut capture = BoundaryCapture::default();
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
            // The exact-size instructions consume the aggregate allowance even
            // though the reasoning summary itself is small.
            let wire = json!({"model":"route","instructions":"x".repeat(8 * 1024),"input":[
                {"type":"reasoning","id":"rs_0","status":"completed","summary":[{"type":"summary_text","text":summary}]},
                {"role":"assistant","content":"READY"},
                {"role":"user","content":"next task"}
            ]});
            let request = production_request(&wire, &replay, inline_limit);
            assert_eq!(
                incoming_boundary(&request, 0, &replay, &mut FingerprintBudget::default()),
                capture.digest(),
                "summary {summary:?}, allowance {inline_limit}"
            );
            let mut changed = wire.clone();
            changed["input"][0]["summary"][0]["text"] = json!("changed summary");
            let changed = production_request(&changed, &replay, inline_limit);
            assert_ne!(
                incoming_boundary(&changed, 0, &replay, &mut FingerprintBudget::default()),
                capture.digest()
            );
            let mut opaque = wire;
            opaque["input"][0]["encrypted_content"] = json!("opaque provider state");
            let opaque = production_request(&opaque, &replay, inline_limit);
            assert!(
                incoming_boundary(&opaque, 0, &replay, &mut FingerprintBudget::default()).is_none()
            );
        }
    }
}

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
