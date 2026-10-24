use super::*;
use crate::agent_turn_history::boundary::{BoundaryCapture, incoming_boundary};
use crate::context_hold::fingerprint::FingerprintBudget;
use crate::server::core_runtime::model_ir::{ModelEvent, ModelStreamEventV1, ResponseItemStatus};

fn production_request(
    wire: &serde_json::Value,
    replay: &crate::replay::ReplayStore,
    inline_limit: usize,
) -> ModelRequestIRV1 {
    super::reasoning_boundary::production_request_for(
        IngressProtocol::Messages,
        wire,
        replay,
        inline_limit,
    )
}

#[test]
fn unsigned_messages_thinking_boundary_survives_production_replay() {
    for thinking in [
        "",
        "Use the requested tool.\n中文",
        "__hiroute_content_ref_v2_1_0_1_1__",
    ] {
        for inline_limit in [0, 8 * 1024, 64 * 1024] {
            for signature in [None, Some("")] {
                let (_directory, replay) = super::replay();
                let mut capture = BoundaryCapture::default();
                capture.accept(&ModelEvent::ReasoningFinished {
                    index: 0,
                    text: thinking.into(),
                    status: ResponseItemStatus::Completed,
                });
                capture.accept(&ModelEvent::TextFinished {
                    index: 1,
                    text: "READY".into(),
                    annotations: vec![],
                    status: ResponseItemStatus::Completed,
                });
                let mut block = json!({"type":"thinking","thinking":thinking});
                if let Some(signature) = signature {
                    block["signature"] = json!(signature);
                }
                let wire = json!({"model":"route","max_tokens":128,"system":"x".repeat(8 * 1024),"messages":[
                    {"role":"assistant","content":[block,{"type":"text","text":"READY"}]},
                    {"role":"user","content":"next task"}
                ]});
                let request = production_request(&wire, &replay, inline_limit);
                assert_eq!(
                    incoming_boundary(&request, 0, &replay, &mut FingerprintBudget::default()),
                    capture.digest(),
                    "thinking {thinking:?}, signature {signature:?}, allowance {inline_limit}"
                );
                // Materialization seals its Replay owner. Give this independent
                // wire assertion its own request lifecycle before later probes.
                let (_wire_directory, wire_replay) = super::replay();
                let wire_request = production_request(&wire, &wire_replay, inline_limit);
                let mut profile = crate::server::core_runtime::profiles::CandidateProtocolProfile::exact_portable_path(
                    IngressProtocol::Messages, IngressProtocol::Messages, "physical",
                    crate::server::core_runtime::profiles::fixed_reasoning("fixed"),
                );
                profile.capability.native_provider_state =
                    crate::server::core_runtime::profiles::NativeProviderStateEmission::Native;
                profile.capability.request.provider_state =
                    crate::server::core_runtime::profiles::Fidelity::Exact;
                profile.capability.response.provider_state =
                    crate::server::core_runtime::profiles::Fidelity::Exact;
                let template =
                    crate::server::core_runtime::adapters::project_candidate_request_template(
                        &wire_request,
                        &profile,
                    )
                    .unwrap();
                wire_replay
                    .prevalidate(&crate::content_ref::model_content_refs(&wire_request))
                    .unwrap();
                let mut body = crate::server::core_runtime::adapters::sequential_attempt_body(
                    template,
                    wire_replay.clone(),
                    wire_replay.budget(),
                    127,
                )
                .unwrap();
                let mut bytes = Vec::new();
                while let Some(chunk) = body.next_chunk().unwrap() {
                    bytes.extend_from_slice(chunk.bytes());
                }
                let sent: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(sent["messages"], wire["messages"]);
                if thinking.is_empty() {
                    let mut empty_only = wire.clone();
                    empty_only["messages"][0]["content"]
                        .as_array_mut()
                        .unwrap()
                        .truncate(1);
                    let empty_only = production_request(&empty_only, &replay, inline_limit);
                    assert!(
                        incoming_boundary(
                            &empty_only,
                            0,
                            &replay,
                            &mut FingerprintBudget::default()
                        )
                        .is_none()
                    );
                    let mut empty_capture = BoundaryCapture::default();
                    empty_capture.accept(&ModelEvent::ReasoningFinished {
                        index: 0,
                        text: String::new(),
                        status: ResponseItemStatus::Completed,
                    });
                    assert!(empty_capture.digest().is_none());
                }
                let mut changed = wire.clone();
                changed["messages"][0]["content"][0]["thinking"] = json!("changed plaintext");
                let request = production_request(&changed, &replay, inline_limit);
                assert_ne!(
                    incoming_boundary(&request, 0, &replay, &mut FingerprintBudget::default()),
                    capture.digest()
                );
                for block in [
                    json!({"type":"thinking","thinking":thinking,"signature":"opaque signature"}),
                    json!({"type":"redacted_thinking","data":"opaque data"}),
                ] {
                    let mut opaque = wire.clone();
                    opaque["messages"][0]["content"][0] = block;
                    let request = production_request(&opaque, &replay, inline_limit);
                    assert!(
                        incoming_boundary(&request, 0, &replay, &mut FingerprintBudget::default())
                            .is_none()
                    );
                }
                let mut unknown = request;
                let ContentPart::ProviderState { state } = &mut unknown.messages[0].content[0]
                else {
                    panic!("thinking remains native provider state")
                };
                state.value["unknown"] = serde_json::Value::Null;
                assert!(
                    incoming_boundary(&unknown, 0, &replay, &mut FingerprintBudget::default())
                        .is_none()
                );
            }
        }
    }
}

#[test]
fn unsigned_messages_thinking_tool_continuation_keeps_full_frozen_stage() {
    for inline_limit in [0, 8 * 1024, 64 * 1024] {
        let (_directory, replay) = super::replay();
        let store = AgentTurnHistoryStore::default();
        let scope = store
            .scope_key("workspace", "unsigned-thinking-tool")
            .unwrap();
        let begin = |request: &ModelRequestIRV1| {
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
                    Instant::now(),
                    || Some(()),
                )
                .unwrap()
                .unwrap()
                .0
        };
        let mut wire = json!({"model":"route","max_tokens":128,"system":"x".repeat(8 * 1024),"messages":[
            {"role":"user","content":"actual tool task A"}
        ]});
        let request = production_request(&wire, &replay, inline_limit);
        let (a, history) = super::new_turn(begin(&request));
        drop(history);
        store
            .commit_decision(&a, super::decision("regular"))
            .unwrap();
        store.bind_request(&a, "request-tool");
        let mut owner = super::accepted_output::owner();
        owner.upstream_protocol = IngressProtocol::Messages;
        store.accept_output(
            &a,
            &[
                ModelStreamEventV1::new(
                    0,
                    ModelEvent::ReasoningFinished {
                        index: 0,
                        text: "Use the requested tool.".into(),
                        status: ResponseItemStatus::Completed,
                    },
                ),
                ModelStreamEventV1::new(
                    1,
                    ModelEvent::ToolCallStarted {
                        index: 1,
                        logical_id: "call-1".into(),
                        native_id: "call-1".into(),
                        tool_kind: ToolKindV1::Function,
                        namespace: None,
                        name: "lookup".into(),
                        owner: Box::new(owner),
                        item_id: None,
                    },
                ),
                ModelStreamEventV1::new(
                    2,
                    ModelEvent::ToolCallFinished {
                        encrypted_function_args: None,
                        index: 1,
                        logical_id: "call-1".into(),
                        tool_kind: ToolKindV1::Function,
                        namespace: None,
                        name: "lookup".into(),
                        arguments: json!({"id":7}),
                        status: ResponseItemStatus::Completed,
                    },
                ),
            ],
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
                    Instant::now(),
                )
                .unwrap()
                .is_none()
        );
        wire["messages"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":[
                {"type":"thinking","thinking":"Use the requested tool.","signature":""},
                {"type":"tool_use","id":"call-1","name":"lookup","input":{"id":7}}
            ]}),
            json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","content":"tool result"}]}),
        ]);
        // Native clients may append request-local budget/context instructions
        // between a tool request and its response. They are not past execution.
        wire["system"] = json!([
            {"type":"text","text":"x".repeat(8 * 1024)},
            {"type":"text","text":"updated request budget"}
        ]);
        let request = production_request(&wire, &replay, inline_limit);
        let AgentTurnBegin::Continuation {
            ticket: final_request,
            decision,
        } = begin(&request)
        else {
            panic!("unsigned thinking/tool edge must inherit at allowance {inline_limit}")
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
                Instant::now(),
            )
            .unwrap()
            .unwrap();
        assert!(!completed.history_partial);
        let membership = completed.history_membership.as_ref().unwrap();
        assert_eq!(membership.request_ids, ["request-tool", "request-final"]);
        assert!(!membership.capture_partial);
        assert!(!membership.association_unverified);
        wire["messages"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":"actual accepted answer A"}),
            json!({"role":"user","content":"actual new task B"}),
        ]);
        let request = production_request(&wire, &replay, inline_limit);
        let (b, frozen) = super::new_turn(begin(&request));
        let target = frozen.assessment_target.as_ref().unwrap();
        assert_eq!(target.through_turn_id, a.agent_turn_id);
        assert!(!target.target_partial);
        assert_eq!(target.history_membership.as_ref().unwrap(), membership);
        assert!(
            matches!(&target.attribution, ExecutionAttribution::Single {model_configuration_id, ..}
            if model_configuration_id == "actual-model")
        );
        let visible = serde_json::to_string(&frozen.visible_conversation).unwrap();
        assert!(visible.contains("actual tool task A"));
        assert!(visible.contains("actual accepted answer A"));
        assert!(!visible.contains("Use the requested tool."));
        assert!(!visible.contains("tool result"));
        store.abort(&b);
        assert_eq!(target.history_membership.as_ref().unwrap(), membership);
    }
}
