use super::*;
use crate::agent_turn_history::boundary::{BoundaryCapture, incoming_boundary, incoming_evidence};
use crate::context_hold::fingerprint::FingerprintBudget;
use crate::server::core_runtime::adapters::{
    ClientResponseRenderer, NativeResponseDecoder, RenderedClientResponse, ToolIdProjection,
};
use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};

#[path = "function_encoding_compat_tests.rs"]
mod compat;

#[test]
fn codex_v2_native_encoding_three_states_bind_accepted_output_and_replay_exactly() {
    let (_dir, replay) = replay();
    let p = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let mut digests = Vec::new();
    for encoding in [None, Some(json!([])), Some(json!(["message"]))] {
        let mut item = json!({"id":"fc","type":"function_call","call_id":"call","name":"spawn","arguments":"{\"message\":\"same bytes\"}","status":"completed"});
        if let Some(encoding) = &encoding {
            item["encrypted_function_args"] = encoding.clone();
        }
        let body = json!({"id":"r","model":"physical","status":"completed","output":[item],"usage":{"input_tokens":1,"output_tokens":1}});
        let mut decoder = NativeResponseDecoder::new_for_observation(
            &p,
            200,
            false,
            ToolIdProjection::new(IngressProtocol::Responses),
            None,
        )
        .unwrap();
        decoder
            .feed(&serde_json::to_vec(&body).unwrap(), true)
            .unwrap();
        let decoded = decoder.finish().unwrap();
        let mut capture = BoundaryCapture::default();
        for event in &decoded.events {
            capture.accept(&event.event);
        }
        let expected = capture.digest().unwrap();
        let RenderedClientResponse::Json { body, .. } = ClientResponseRenderer::render_nonstream(
            IngressProtocol::Responses,
            "alias",
            &decoded.response,
        )
        .unwrap() else {
            panic!("JSON")
        };
        let wire = json!({"model":"alias","input":[body["output"][0].clone(),{"role":"user","content":"next"}]});
        for inline in [0, 8192] {
            let request = super::reasoning_boundary::production_request_for(
                IngressProtocol::Responses,
                &wire,
                &replay,
                inline,
            );
            assert_eq!(
                incoming_boundary(&request, 0, &replay, &mut FingerprintBudget::default()),
                Some(expected)
            );
            let encrypted = encoding
                .as_ref()
                .is_some_and(|v| !v.as_array().unwrap().is_empty());
            assert_eq!(request.native_only, encrypted);
            let incoming =
                incoming_evidence(&request, 0, &replay, &mut FingerprintBudget::default()).unwrap();
            assert_eq!(
                capture.evidence().unwrap().matches(incoming),
                Some(encrypted),
                "opaque tool equality proves contextual replay, never affinity"
            );
        }
        digests.push(expected);
    }
    assert_ne!(digests[0], digests[1]);
    assert_ne!(digests[1], digests[2]);
    assert_ne!(digests[0], digests[2]);
}
