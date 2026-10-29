//! Historical phase projection is separate from live output termination semantics.
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
fn unknown_request_phases_remain_unrepresentable_cross_protocol() {
    for phase in ["analysis", "future_phase", ""] {
        let request = decode_ingress_request(IngressProtocol::Responses, &document(phase)).unwrap();
        for target in [IngressProtocol::ChatCompletions, IngressProtocol::Messages] {
            let profile = exact_state_profile(IngressProtocol::Responses, target);
            assert!(
                matches!(
                    project_candidate_request(&request, &profile),
                    Err(ProtocolAdapterError::RequestUnrepresentable { .. })
                ),
                "{phase:?} must not become an ordinary assistant answer"
            );
            assert!(matches!(
                project_candidate_request_template(&request, &profile),
                Err(ProtocolAdapterError::RequestUnrepresentable { .. })
            ));
        }
    }
}

#[test]
fn completed_commentary_history_projects_to_chat_with_explicit_phase_loss() {
    for status in [None, Some("completed")] {
        let mut original = document("commentary");
        original["input"][0]["id"] = json!("native-commentary");
        if let Some(status) = status {
            original["input"][0]["status"] = json!(status);
        }
        let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
        let profile =
            exact_state_profile(IngressProtocol::Responses, IngressProtocol::ChatCompletions);
        let direct = project_candidate_request(&request, &profile).unwrap();
        let template = project_candidate_request_template(&request, &profile).unwrap();
        assert_eq!(
            direct.body["messages"],
            json!([
                {"role":"assistant","content":"previous answer"},
                {"role":"user","content":"next question"}
            ])
        );
        assert_eq!(
            direct.request_projection_loss,
            template.request_projection_loss
        );
        assert_eq!(direct.request_projection_loss.message_phases_omitted, 1);
        assert_eq!(direct.request_projection_loss.first_message_index, Some(0));
        assert_eq!(request.native_body.as_ref(), Some(&original));
        assert_eq!(
            super::super::reasoning_loss::request_loss(
                &request,
                IngressProtocol::ChatCompletions,
                None
            ),
            0
        );
        let messages = exact_state_profile(IngressProtocol::Responses, IngressProtocol::Messages);
        assert!(matches!(
            project_candidate_request_template(&request, &messages),
            Err(ProtocolAdapterError::RequestUnrepresentable {
                reason:
                    hiroute_diagnostics::event::RequestProjectionReason::MessagePhaseUnsupported,
                message_index: Some(0),
            })
        ));
    }
}

#[test]
fn unfinished_commentary_and_unknown_controls_have_closed_rejection_positions() {
    use hiroute_diagnostics::event::RequestProjectionReason;
    for (field, value, reason, message_index) in [
        (
            "status",
            json!("in_progress"),
            RequestProjectionReason::ItemLifecycleUnsupported,
            Some(0),
        ),
        (
            "phase",
            json!("future_phase"),
            RequestProjectionReason::NativeSemanticsUnsupported,
            None,
        ),
        (
            "control_extension",
            json!({"resume":"private-control-value"}),
            RequestProjectionReason::NativeSemanticsUnsupported,
            None,
        ),
    ] {
        let mut original = document("commentary");
        original["input"][0][field] = value;
        original["input"]
            .as_array_mut()
            .unwrap()
            .insert(0, json!({"type":"additional_tools","tools":[]}));
        let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
        let profile =
            exact_state_profile(IngressProtocol::Responses, IngressProtocol::ChatCompletions);
        for error in [
            project_candidate_request(&request, &profile).unwrap_err(),
            project_candidate_request_template(&request, &profile).unwrap_err(),
        ] {
            assert_eq!(
                error,
                ProtocolAdapterError::RequestUnrepresentable {
                    reason,
                    message_index
                }
            );
            assert!(!error.to_string().contains("private-control-value"));
        }
    }
}

#[test]
fn phase_lifecycle_validation_preserves_supported_reasoning_history_projection() {
    for status in [None, Some(json!(null)), Some(json!("completed"))] {
        let mut original = document("commentary");
        let mut reasoning = json!({"type":"reasoning","id":"rs-prior","summary":[
            {"type":"summary_text","text":"prior thought"}
        ],"encrypted_content":null});
        if let Some(status) = status {
            reasoning["status"] = status;
        }
        original["input"]
            .as_array_mut()
            .unwrap()
            .insert(0, reasoning);
        let request = decode_ingress_request(IngressProtocol::Responses, &original).unwrap();
        let profile =
            exact_state_profile(IngressProtocol::Responses, IngressProtocol::ChatCompletions);
        let direct = project_candidate_request(&request, &profile).unwrap();
        let template = project_candidate_request_template(&request, &profile).unwrap();
        assert_eq!(
            direct.body["messages"][0],
            json!({"role":"assistant","reasoning_content":"prior thought"})
        );
        assert_eq!(direct.body["messages"][1]["content"], "previous answer");
        assert_eq!(direct.request_projection_loss.first_message_index, Some(1));
        assert_eq!(
            direct.request_projection_loss,
            template.request_projection_loss
        );
        assert_eq!(request.native_body.as_ref(), Some(&original));
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
