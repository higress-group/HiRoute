//! History coverage belongs to the matched output, not the whole request.
use super::*;
use crate::agent_turn_history::boundary::{BoundaryCapture, incoming_evidence};
use crate::context_hold::{branch_anchors, fingerprint::FingerprintBudget};
use crate::server::core_runtime::model_ir::{ModelEvent, ResponseItemStatus};
use serde_json::Value;

fn body(protocol: IngressProtocol) -> Value {
    let mut body = json!({"model":"route", "messages":[
        {"role":"user","content":"old ancestor"},
        {"role":"assistant","content":"old output","vendor_extension":{"state":"opaque"}},
        {"role":"user","content":"new B"},
        {"role":"assistant","content":"READY"},
        {"role":"user","content":"new C"}
    ]});
    if protocol == IngressProtocol::Responses {
        let messages = body.as_object_mut().unwrap().remove("messages").unwrap();
        body["input"] = messages;
    }
    if protocol == IngressProtocol::Messages {
        body["max_tokens"] = json!(128);
    }
    body
}

fn parse(
    protocol: IngressProtocol,
    body: &Value,
    replay: &ReplayStore,
    allowance: usize,
) -> ModelRequestIRV1 {
    super::reasoning_boundary::production_request_for(protocol, body, replay, allowance)
}

fn accepted() -> BoundaryCapture {
    let mut capture = BoundaryCapture::default();
    capture.accept(&ModelEvent::TextFinished {
        index: 0,
        text: "READY".into(),
        annotations: vec![],
        status: ResponseItemStatus::Completed,
        final_phase: None,
    });
    capture
}

fn association(request: &ModelRequestIRV1, replay: &ReplayStore) -> Option<bool> {
    let incoming = incoming_evidence(request, 3, replay, &mut FingerprintBudget::default())?;
    accepted().evidence().unwrap().matches(incoming)
}

#[test]
fn old_unknown_history_does_not_disable_new_actual_output_association() {
    for protocol in [
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
    ] {
        let (_dir, replay) = replay();
        let mut prefixes = Vec::new();
        for allowance in [0, 8 * 1024, 64 * 1024] {
            let request = parse(protocol, &body(protocol), &replay, allowance);
            assert!(request.native_only);
            assert_eq!(association(&request, &replay), Some(true));
            assert!(
                crate::context_hold::visible_history_with_replay(
                    &request,
                    &[19; 32],
                    Some(&replay)
                )
                .is_none()
            );
            prefixes.push(branch_anchors(&request, &[19; 32], Some(&replay), &[3]).unwrap());
        }
        for prefix in &prefixes[1..] {
            assert_eq!(prefix.prefixes, prefixes[0].prefixes);
            assert_eq!(prefix.complete, prefixes[0].complete);
        }
    }
}

#[test]
fn retained_unknown_prefix_is_bound_in_full_and_changed_suffix_stays_unmatched() {
    for protocol in [
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
    ] {
        let (_dir, replay) = replay();
        let field = if protocol == IngressProtocol::Responses {
            "input"
        } else {
            "messages"
        };
        for allowance in [0, 64 * 1024] {
            let value = body(protocol);
            let request = parse(protocol, &value, &replay, allowance);
            let prefix = branch_anchors(&request, &[19; 32], Some(&replay), &[3]).unwrap();
            for replacement in [
                json!("changed"),
                json!({"state":"changed"}),
                json!(["changed"]),
            ] {
                let mut changed = value.clone();
                changed[field][1]["vendor_extension"] = replacement;
                let changed = parse(protocol, &changed, &replay, allowance);
                let changed = branch_anchors(&changed, &[19; 32], Some(&replay), &[3]).unwrap();
                assert_ne!(changed.prefixes, prefix.prefixes);
            }
            let mut unknown_output = value.clone();
            unknown_output[field][3]["another_extension"] = json!({"meaning":"unknown"});
            assert_eq!(
                association(
                    &parse(protocol, &unknown_output, &replay, allowance),
                    &replay
                ),
                None
            );
            let mut unknown_current_user = value.clone();
            unknown_current_user[field][4]["another_extension"] = json!({"meaning":"unknown"});
            assert_eq!(
                association(
                    &parse(protocol, &unknown_current_user, &replay, allowance),
                    &replay
                ),
                Some(true)
            );
            let mut changed_output = value;
            changed_output[field][3]["content"] = json!("different output");
            assert_eq!(
                association(
                    &parse(protocol, &changed_output, &replay, allowance),
                    &replay
                ),
                None
            );
        }
    }
}

#[test]
fn unknown_setup_does_not_mask_unknown_output_or_disable_known_output() {
    for protocol in [
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
    ] {
        let (_dir, replay) = replay();
        let field = if protocol == IngressProtocol::Responses {
            "input"
        } else {
            "messages"
        };
        let mut value = body(protocol);
        value["vendor_request_control"] = json!(true);
        value[field][1]
            .as_object_mut()
            .unwrap()
            .remove("vendor_extension");
        assert_eq!(
            association(&parse(protocol, &value, &replay, 0), &replay),
            Some(true)
        );
        value[field][3]["unmodeled_output"] = json!(true);
        assert_eq!(
            association(&parse(protocol, &value, &replay, 0), &replay),
            None
        );
    }
}

#[test]
fn native_type_extensions_are_not_reinterpreted_as_another_protocol() {
    for protocol in [IngressProtocol::ChatCompletions, IngressProtocol::Messages] {
        let (_dir, replay) = replay();
        let mut value = body(protocol);
        value["messages"][1]["type"] = json!("agent_message");
        for allowance in [0, 64 * 1024] {
            assert_eq!(
                association(&parse(protocol, &value, &replay, allowance), &replay),
                Some(true)
            );
        }
    }
}

#[test]
fn old_unknown_reasoning_cannot_poison_new_known_response_boundary() {
    let (_dir, replay) = replay();
    for protocol in [IngressProtocol::Responses, IngressProtocol::ChatCompletions] {
        let mut value = body(protocol);
        let field = if protocol == IngressProtocol::Responses {
            "input"
        } else {
            "messages"
        };
        value[field][1]
            .as_object_mut()
            .unwrap()
            .remove("vendor_extension");
        if protocol == IngressProtocol::Responses {
            value[field][1] =
                json!({"type":"reasoning","summary":[],"future_reasoning":{"state":"opaque"}});
        } else {
            value[field][1]["reasoning"] = json!({"state":"opaque"});
        }
        for allowance in [0, 64 * 1024] {
            assert_eq!(
                association(&parse(protocol, &value, &replay, allowance), &replay),
                Some(true)
            );
        }
    }
}

#[test]
fn native_history_layout_is_local_and_rejects_missing_or_shifted_positions() {
    let (_dir, replay) = replay();
    let request = parse(
        IngressProtocol::Responses,
        &body(IngressProtocol::Responses),
        &replay,
        0,
    );
    let serialized = serde_json::to_value(&request).unwrap();
    assert!(serialized.get("native_history").is_none());
    let restored: ModelRequestIRV1 = serde_json::from_value(serialized).unwrap();
    assert!(restored.native_history.is_none());
    for mode in 0..4 {
        let mut changed = request.clone();
        match mode {
            0 => {
                changed.native_history = None;
            }
            1 => {
                changed.messages.remove(0);
            }
            2 => {
                changed.native_history.as_mut().unwrap()[1].native_index = Some(0);
            }
            _ => {
                changed.native_history.as_mut().unwrap()[1].native_index = Some(99);
            }
        }
        assert!(changed.native_history_layout().is_none());
        assert!(branch_anchors(&changed, &[19; 32], Some(&replay), &[3]).is_none());
        assert_eq!(association(&changed, &replay), None);
    }
}
