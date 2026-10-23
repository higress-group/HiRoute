//! A completed assistant answer has the same channel on every supported protocol.
use super::*;
use crate::server::core_runtime::model_ir::ModelRequestIRV1;
use serde_json::Value;

fn document(phase: &str) -> Value {
    json!({"model":"alias","input":[
        {"type":"message","role":"assistant","phase":phase,
            "content":[{"type":"output_text","text":"previous answer"}]},
        {"type":"message","role":"user",
            "content":[{"type":"input_text","text":"next question"}]}
    ]})
}

fn projected_bodies(request: &ModelRequestIRV1, target: IngressProtocol) -> [Value; 2] {
    let profile = exact_state_profile(IngressProtocol::Responses, target);
    let direct = project_candidate_request(request, &profile)
        .expect("the completed answer's phase is implicit on the target protocol");
    let template = project_candidate_request_template(request, &profile)
        .expect("candidate preflight must agree with direct request projection");
    [
        direct.body,
        serde_json::from_slice(&template.bytes).unwrap(),
    ]
}

#[test]
fn final_answer_request_history_projects_to_chat() {
    for envelope in [false, true] {
        let mut original = document("final_answer");
        if envelope {
            original["input"][0]["id"] = json!("completed-assistant-item");
            original["input"][0]["status"] = json!("completed");
        }
        let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
        assert!(!request.native_only);
        assert!(!request.requirements().provider_state);
        for body in projected_bodies(&request, IngressProtocol::ChatCompletions) {
            assert_eq!(body["model"], "physical");
            assert_eq!(
                body["messages"],
                json!([
                    {"role":"assistant","content":"previous answer"},
                    {"role":"user","content":"next question"}
                ])
            );
        }
        assert_eq!(request.native_body.as_ref(), Some(&original));
    }
}

#[test]
fn final_answer_request_history_projects_to_messages() {
    let original = document("final_answer");
    let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
    for body in projected_bodies(&request, IngressProtocol::Messages) {
        assert_eq!(
            body["messages"],
            json!([
                {"role":"assistant","content":[{"type":"text","text":"previous answer"}]},
                {"role":"user","content":[{"type":"text","text":"next question"}]}
            ])
        );
    }
    assert_eq!(request.native_body.as_ref(), Some(&original));
}

#[test]
fn non_answer_request_phases_remain_unrepresentable_cross_protocol() {
    for phase in ["commentary", "analysis", "future_phase", ""] {
        let request = decode_ingress_request(IngressProtocol::Responses, &document(phase)).unwrap();
        for target in [IngressProtocol::ChatCompletions, IngressProtocol::Messages] {
            let profile = exact_state_profile(IngressProtocol::Responses, target);
            assert!(
                matches!(
                    project_candidate_request(&request, &profile),
                    Err(ProtocolAdapterError::ClientUnrepresentable(_))
                ),
                "{phase:?} must not become an ordinary assistant answer"
            );
            assert!(matches!(
                project_candidate_request_template(&request, &profile),
                Err(ProtocolAdapterError::ClientUnrepresentable(_))
            ));
        }
    }
}

#[test]
fn native_responses_keeps_the_original_answer_and_non_answer_phases() {
    for phase in ["final_answer", "commentary", "future_phase"] {
        let original = document(phase);
        let mut request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
        for body in projected_bodies(&request, IngressProtocol::Responses) {
            assert_eq!(body["input"], original["input"]);
        }
        if !request.native_only {
            request.native_body = None;
            for body in projected_bodies(&request, IngressProtocol::Responses) {
                assert_eq!(body["input"], original["input"]);
            }
        }
    }
}

#[test]
fn final_answer_does_not_admit_other_native_metadata_or_non_assistant_phases() {
    let mut metadata = document("final_answer");
    metadata["input"][0]["internal_chat_message_metadata_passthrough"] =
        json!({"turn_id":"native-turn"});
    let mut unknown = document("final_answer");
    unknown["input"][0]["unknown_extension"] = json!({"state":"native-only"});
    let mut user_phase = document("final_answer");
    user_phase["input"][0]["role"] = json!("user");
    user_phase["input"][0]["content"][0]["type"] = json!("input_text");
    for original in [metadata, unknown, user_phase] {
        let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
        for target in [IngressProtocol::ChatCompletions, IngressProtocol::Messages] {
            let profile = exact_state_profile(IngressProtocol::Responses, target);
            assert!(project_candidate_request(&request, &profile).is_err());
            assert!(project_candidate_request_template(&request, &profile).is_err());
        }
        for body in projected_bodies(&request, IngressProtocol::Responses) {
            assert_eq!(body["input"], original["input"]);
        }
    }
}

#[test]
fn default_answer_phase_does_not_change_declared_reasoning_omission() {
    // This is a synthetic projection case, not a reconstruction of an observed DSH request.
    let mut original = document("final_answer");
    original["input"].as_array_mut().unwrap().insert(
        0,
        json!({
            "type":"reasoning","summary":[],"content":[],"encrypted_content":"native-ciphertext"
        }),
    );
    let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
    assert!(request.requirements().provider_state);
    for body in projected_bodies(&request, IngressProtocol::ChatCompletions) {
        assert_eq!(
            body["messages"],
            json!([
                {"role":"assistant","content":"previous answer"},
                {"role":"user","content":"next question"}
            ])
        );
    }
    assert_eq!(
        super::super::reasoning_loss::request_loss(
            &request,
            IngressProtocol::ChatCompletions,
            None
        ),
        1,
        "actual request materialization must account for the declared reasoning loss"
    );
    for body in projected_bodies(&request, IngressProtocol::Responses) {
        assert_eq!(body["input"], original["input"]);
    }
    assert_eq!(request.native_body.as_ref(), Some(&original));
}
