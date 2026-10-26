use crate::agent_turn_history::boundary::{BoundaryCapture, incoming_boundary, incoming_evidence};
use crate::context_hold::fingerprint::FingerprintBudget;
use crate::server::core_runtime::adapters::{
    decode_ingress_request, decode_ingress_request_with_replay,
};
use crate::server::core_runtime::model_ir::{ModelEvent, ResponseItemStatus};
use crate::server::request_plan::IngressProtocol;
use serde_json::json;

fn production_request(
    wire: &serde_json::Value,
    replay: &crate::replay::ReplayStore,
    inline_limit: usize,
) -> crate::server::core_runtime::model_ir::ModelRequestIRV1 {
    production_request_for(IngressProtocol::Responses, wire, replay, inline_limit)
}

pub(super) fn production_request_for(
    protocol: IngressProtocol,
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
        parse_ingress_document(protocol, replay, &raw, stats).unwrap();
    compact_ingress_document_with_markers(
        protocol,
        &mut document,
        replay,
        workspace.generated_markers(),
    )
    .unwrap();
    let mut request =
        decode_ingress_request_with_replay(protocol, &document, Some(replay)).unwrap();
    externalize_model_request(&mut request, replay, inline_limit).unwrap();
    request
}

#[test]
fn production_reasoning_boundary_is_independent_of_aggregate_inline_allowance() {
    for summary in [
        "",
        "Read \\\"notes\\\" first.\n中文",
        "__hiroute_content_ref_v2_1_0_1_1__",
    ] {
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
                incoming_evidence(&opaque, 0, &replay, &mut FingerprintBudget::default())
                    .and_then(|incoming| capture.evidence().unwrap().matches(incoming))
                    .is_none(),
                "uncaptured encrypted state cannot match accepted plaintext"
            );
        }
    }
}

#[test]
fn production_reasoning_boundary_accepts_empty_native_fields_without_accepting_hidden_content() {
    for inline_limit in [0, 8 * 1024, 64 * 1024] {
        let (_directory, replay) = super::replay();
        let mut capture = BoundaryCapture::default();
        capture.accept(&ModelEvent::ReasoningFinished {
            index: 0,
            text: "Use the requested tool.".into(),
            status: ResponseItemStatus::Completed,
        });
        capture.accept(&ModelEvent::TextFinished {
            index: 1,
            text: "READY".into(),
            annotations: vec![],
            status: ResponseItemStatus::Completed,
        });
        for encrypted in [serde_json::Value::Null, json!("")] {
            for content in [serde_json::Value::Null, json!([])] {
                let wire = json!({"model":"route","instructions":"x".repeat(8 * 1024),"input":[
                    {"type":"reasoning","id":"rs-tool","status":null,
                     "summary":[{"type":"summary_text","text":"Use the requested tool."}],
                     "encrypted_content":encrypted,"content":content},
                    {"role":"assistant","content":"READY"},
                    {"role":"user","content":"next task"}
                ]});
                let request = production_request(&wire, &replay, inline_limit);
                assert_eq!(
                    incoming_boundary(&request, 0, &replay, &mut FingerprintBudget::default()),
                    capture.digest(),
                    "empty optional reasoning fields at allowance {inline_limit}"
                );
                for (field, value) in [
                    ("encrypted_content", json!("opaque provider state")),
                    (
                        "content",
                        json!([{"type":"reasoning_text","text":"uncaptured text"}]),
                    ),
                    ("unrecognized_native_field", serde_json::Value::Null),
                ] {
                    let mut changed = wire.clone();
                    changed["input"][0][field] = value;
                    let request = production_request(&changed, &replay, inline_limit);
                    assert!(
                        incoming_evidence(&request, 0, &replay, &mut FingerprintBudget::default())
                            .and_then(|incoming| capture.evidence().unwrap().matches(incoming))
                            .is_none()
                    );
                }
            }
        }
    }
}

#[test]
fn production_reasoning_tool_continuation_keeps_complete_frozen_request_membership() {
    use super::super::*;
    use crate::server::core_runtime::model_ir::{ModelStreamEventV1, ToolKindV1};

    for (inline_limit, null_fields, omitted, plaintext_content) in
        [0, 8 * 1024, 64 * 1024].into_iter().flat_map(|limit| {
            [
                (limit, false, false, false),
                (limit, true, false, false),
                (limit, false, true, false),
                (limit, true, false, true),
            ]
        })
    {
        let (_directory, replay) = super::replay();
        let store = AgentTurnHistoryStore::default();
        let scope = store.scope_key("workspace", "reasoning-tool").unwrap();
        let begin = |request: &crate::server::core_runtime::model_ir::ModelRequestIRV1| {
            store
                .begin_with_admission(
                    scope.clone(),
                    super::plan(1),
                    store
                        .prepare_scoped_request(&scope, request, &replay)
                        .unwrap(),
                    TurnDecisionInputs {
                        native_turn: None,
                        history_continuity: crate::context_hold::HistoryContinuity::Continuing,
                        reselect_on_user_message: true,
                    },
                    std::time::Instant::now(),
                    || Some(()),
                )
                .unwrap()
                .unwrap()
                .0
        };
        let mut wire = json!({"model":"route","instructions":"x".repeat(8 * 1024),"input":[
            {"role":"user","content":"actual tool task A"}
        ]});
        let request = production_request(&wire, &replay, inline_limit);
        let (a, history) = super::new_turn(begin(&request));
        drop(history);
        store
            .commit_decision(&a, super::decision("regular"))
            .unwrap();
        store.bind_request(&a, "request-tool");
        store.accept_output(
            &a,
            &[ModelStreamEventV1::new(
                0,
                ModelEvent::ReasoningFinished {
                    index: 0,
                    text: "Use the requested tool.".into(),
                    status: ResponseItemStatus::Completed,
                },
            )],
        );
        super::accepted_output::tool(&store, &a, "call-1", "lookup");
        store.accept_output(
            &a,
            &[ModelStreamEventV1::new(
                2,
                ModelEvent::ToolCallFinished {
                    encrypted_function_args: None,
                    index: 1,
                    logical_id: "call-1".into(),
                    tool_kind: ToolKindV1::Function,
                    namespace: Some("functions".into()),
                    name: "lookup".into(),
                    arguments: json!({"id":7}),
                    status: ResponseItemStatus::Completed,
                },
            )],
        );
        assert!(
            store
                .finish_request(
                    &a,
                    "request-tool".into(),
                    AgentTurnStatus::Completed,
                    vec![super::execution(
                        "actual-model",
                        "profile",
                        "regular",
                        "request-tool"
                    )],
                    false,
                    std::time::Instant::now(),
                )
                .unwrap()
                .is_none()
        );
        wire["input"].as_array_mut().unwrap().extend([
            json!({"type":"reasoning","id":"rs-tool","status":"completed","summary":[{"type":"summary_text","text":"Use the requested tool."}]}),
            json!({"type":"function_call","call_id":"call-1","namespace":"functions","name":"lookup","arguments":"{\"id\":7}"}),
            json!({"type":"function_call_output","call_id":"call-1","output":"tool result"}),
        ]);
        if null_fields {
            wire["input"][1]["encrypted_content"] = serde_json::Value::Null;
            wire["input"][1]["content"] = serde_json::Value::Null;
            wire["input"][1]["status"] = serde_json::Value::Null;
        }
        if plaintext_content {
            wire["input"][1]["summary"] = json!([]);
            wire["input"][1]["content"] =
                json!([{"type":"reasoning_text","text":"Use the requested tool."}]);
        }
        if omitted {
            wire["input"].as_array_mut().unwrap().remove(1);
        }
        let request = production_request(&wire, &replay, inline_limit);
        let AgentTurnBegin::Continuation {
            ticket: final_request,
            decision,
        } = begin(&request)
        else {
            panic!("the accepted reasoning/tool edge must inherit at allowance {inline_limit}")
        };
        assert_eq!(final_request.key, a.key);
        assert_eq!(final_request.agent_turn_id, a.agent_turn_id);
        assert_eq!(decision.branch_id, "regular");
        store.bind_request(&final_request, "request-final");
        super::accepted_output::text_output(&store, &final_request, "actual accepted answer A");
        let completed = store
            .finish_request(
                &final_request,
                "request-final".into(),
                AgentTurnStatus::Completed,
                vec![super::execution(
                    "actual-model",
                    "profile",
                    "regular",
                    "request-final",
                )],
                true,
                std::time::Instant::now(),
            )
            .unwrap()
            .unwrap();
        assert!(!completed.history_partial);
        let membership = completed.history_membership.as_ref().unwrap();
        assert_eq!(membership.request_ids, ["request-tool", "request-final"]);
        assert!(!membership.capture_partial);
        assert_eq!(membership.association_unverified, omitted);
        wire["input"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":"actual accepted answer A"}),
            json!({"role":"user","content":"actual new task B"}),
        ]);
        let request = production_request(&wire, &replay, inline_limit);
        let (b, frozen) = super::new_turn(begin(&request));
        let target = frozen.assessment_target.as_ref().unwrap();
        assert_eq!(target.through_turn_id, a.agent_turn_id);
        assert!(!target.target_partial);
        assert_eq!(target.history_membership.as_ref().unwrap(), membership);
        assert!(matches!(
            &target.attribution,
            ExecutionAttribution::Single { model_configuration_id, .. }
                if model_configuration_id == "actual-model"
        ));
        let visible = serde_json::to_string(&frozen.visible_conversation).unwrap();
        assert!(visible.contains("actual tool task A"));
        assert!(visible.contains("actual accepted answer A"));
        assert!(visible.contains("completed"));
        assert!(!visible.contains("Use the requested tool."));
        assert!(!visible.contains("tool result"));
        store.abort(&b);
        assert_eq!(target.history_membership.as_ref().unwrap(), membership);
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
            incoming_evidence(&decoded, 0, &replay, &mut FingerprintBudget::default())
                .and_then(|incoming| capture.evidence().unwrap().matches(incoming))
                .is_none()
        );
    }
}
