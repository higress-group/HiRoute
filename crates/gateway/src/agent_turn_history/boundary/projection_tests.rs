use super::*;
use crate::agent_turn_history::boundary::{BoundaryCapture, incoming_evidence};
use crate::context_hold::fingerprint::FingerprintBudget;
use crate::server::core_runtime::model_ir::{ModelEvent, ResponseItemStatus};

fn accepted(reasoning: &str, visible: bool) -> BoundaryCapture {
    let mut capture = BoundaryCapture::default();
    capture.accept(&ModelEvent::ReasoningFinished {
        index: 0,
        text: reasoning.into(),
        status: ResponseItemStatus::Completed,
    });
    if visible {
        capture.accept(&ModelEvent::TextFinished {
            index: 1,
            text: "READY".into(),
            annotations: vec![],
            status: ResponseItemStatus::Completed,
        });
    }
    capture
}

fn wire(protocol: IngressProtocol, reasoning: Option<&str>) -> serde_json::Value {
    match protocol {
        IngressProtocol::Responses => {
            let mut input = Vec::new();
            if let Some(text) = reasoning {
                input.push(
                    json!({"type":"reasoning","summary":[{"type":"summary_text","text":text}]}),
                );
            }
            input.extend([
                json!({"role":"assistant","content":"READY"}),
                json!({"role":"user","content":"B"}),
            ]);
            json!({"model":"route","input":input})
        }
        IngressProtocol::Messages => {
            let mut parts = Vec::new();
            if let Some(text) = reasoning {
                parts.push(json!({"type":"thinking","thinking":text,"signature":""}));
            }
            parts.push(json!({"type":"text","text":"READY"}));
            json!({"model":"route","max_tokens":128,"messages":[
                {"role":"assistant","content":parts},{"role":"user","content":"B"}
            ]})
        }
        IngressProtocol::ChatCompletions => {
            let mut assistant = json!({"role":"assistant","content":"READY"});
            if let Some(text) = reasoning {
                assistant["reasoning_content"] = json!(text);
            }
            json!({"model":"route","messages":[assistant,{"role":"user","content":"B"}]})
        }
    }
}

#[test]
fn omitted_plaintext_is_only_a_visible_projection_with_bounded_actual_content() {
    for protocol in [
        IngressProtocol::Responses,
        IngressProtocol::Messages,
        IngressProtocol::ChatCompletions,
    ] {
        for allowance in [0, 8 * 1024, 64 * 1024] {
            let (_dir, replay) = replay();
            let capture = accepted("actually accepted reasoning", true);
            let proof = capture.evidence().unwrap();
            for (reasoning, expected) in [
                (None, Some(true)),
                (Some("changed"), None),
                (Some(""), None),
            ] {
                let request = super::reasoning_boundary::production_request_for(
                    protocol,
                    &wire(protocol, reasoning),
                    &replay,
                    allowance,
                );
                let incoming =
                    incoming_evidence(&request, 0, &replay, &mut FingerprintBudget::default())
                        .unwrap();
                assert_eq!(proof.matches(incoming), expected);
                assert!(
                    accepted("actually accepted reasoning", false)
                        .evidence()
                        .unwrap()
                        .matches(incoming)
                        .is_none()
                );
            }
            let mut unsupported = capture;
            unsupported.accept(&ModelEvent::ProviderState {
                state: Box::new(crate::server::core_runtime::model_ir::OpaqueProviderState {
                    block_index: None,
                    kind: "opaque".into(),
                    value: json!({"data":"opaque"}),
                }),
            });
            assert!(
                unsupported.evidence().is_none(),
                "opaque output has no visible fallback"
            );
        }
    }
}

#[test]
fn chat_reasoning_alias_tool_boundary_survives_inline_and_json_replay() {
    for allowance in [0, 8 * 1024, 64 * 1024] {
        for field in ["reasoning_content", "reasoning"] {
            let (_dir, replay) = replay();
            let mut capture = accepted("actual reasoning", false);
            let owner = super::accepted_output::owner();
            capture.accept(&ModelEvent::ToolCallStarted {
                index: 1,
                logical_id: "call".into(),
                native_id: "call".into(),
                tool_kind: ToolKindV1::Function,
                namespace: None,
                name: "read".into(),
                owner: Box::new(owner),
                item_id: None,
            });
            capture.accept(&ModelEvent::ToolCallFinished {
                encrypted_function_args: None,
                index: 1,
                logical_id: "call".into(),
                tool_kind: ToolKindV1::Function,
                namespace: None,
                name: "read".into(),
                arguments: json!({"path":"fixture"}),
                status: ResponseItemStatus::Completed,
            });
            let mut body = json!({"model":"route","messages":[
                {"role":"assistant","content":"","tool_calls":[{"id":"call","type":"function","function":{"name":"read","arguments":"{\"path\":\"fixture\"}"}}]},
                {"role":"tool","tool_call_id":"call","content":"actual tool result"}
            ]});
            body["messages"][0][field] = json!("actual reasoning");
            let request = super::reasoning_boundary::production_request_for(
                IngressProtocol::ChatCompletions,
                &body,
                &replay,
                allowance,
            );
            let incoming =
                incoming_evidence(&request, 0, &replay, &mut FingerprintBudget::default()).unwrap();
            assert_eq!(capture.evidence().unwrap().matches(incoming), Some(false));
        }
    }
}

fn begin_native(
    store: &AgentTurnHistoryStore,
    scope: &AgentTurnHistoryKey,
    request: &ModelRequestIRV1,
    replay: &ReplayStore,
    native_turn: Option<u8>,
) -> AgentTurnBegin {
    store
        .begin_with_admission(
            scope.clone(),
            plan(1),
            store
                .prepare_scoped_request(scope, request, replay)
                .unwrap(),
            TurnDecisionInputs {
                native_turn: native_turn.map(|v| [v; 32]),
                history_continuity: crate::context_hold::HistoryContinuity::Continuing,
                reselect_on_user_message: true,
            },
            Instant::now(),
            || Some(()),
        )
        .unwrap()
        .unwrap()
        .0
}

fn finish_output(
    store: &AgentTurnHistoryStore,
    ticket: &AgentTurnTicket,
    reasoning: &str,
    answer: &str,
    id: &str,
    terminal: bool,
) -> Option<CompletedAgentTurn> {
    use crate::server::core_runtime::model_ir::ModelStreamEventV1;
    if ticket.new_turn {
        store.commit_decision(ticket, decision("regular")).unwrap();
    }
    store.accept_output(
        ticket,
        &[
            ModelStreamEventV1::new(
                0,
                ModelEvent::ReasoningFinished {
                    index: 0,
                    text: reasoning.into(),
                    status: ResponseItemStatus::Completed,
                },
            ),
            ModelStreamEventV1::new(
                1,
                ModelEvent::TextFinished {
                    index: 1,
                    text: answer.into(),
                    annotations: vec![],
                    status: ResponseItemStatus::Completed,
                },
            ),
        ],
    );
    store
        .finish_request(
            ticket,
            id.into(),
            AgentTurnStatus::Completed,
            vec![execution("model", "profile", "regular", id)],
            terminal,
            Instant::now(),
        )
        .unwrap()
}

#[test]
fn visible_projection_uncertainty_survives_later_full_edges_in_the_same_stage() {
    let (_dir, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store.scope_key("workspace", "sticky-projection").unwrap();
    let mut messages = vec![text(MessageRole::User, "A")];
    let (first, _) = new_turn(begin_native(
        &store,
        &scope,
        &request(messages.clone()),
        &replay,
        None,
    ));
    finish_output(&store, &first, "private reasoning", "READY", "r1", false);
    messages.push(text(MessageRole::Assistant, "READY"));
    let AgentTurnBegin::Continuation { ticket: second, .. } =
        begin_native(&store, &scope, &request(messages.clone()), &replay, None)
    else {
        panic!("visible response must continue")
    };
    assert!(!second.allow_affinity);
    finish_output(&store, &second, "", "NEXT", "r2", false);
    messages.push(text(MessageRole::Assistant, "NEXT"));
    let AgentTurnBegin::Continuation { ticket: third, .. } =
        begin_native(&store, &scope, &request(messages.clone()), &replay, None)
    else {
        panic!("full edge must continue")
    };
    let completed = finish_output(&store, &third, "", "DONE", "r3", true).unwrap();
    let membership = completed.history_membership.unwrap();
    assert_eq!(membership.request_ids, ["r1", "r2", "r3"]);
    assert_eq!(
        membership.relation,
        hiroute_domain::HistoryRelationV1::Contextual
    );
    assert!(membership.association_unverified);
    assert!(!membership.capture_partial);
    messages.extend([
        text(MessageRole::Assistant, "DONE"),
        text(MessageRole::User, "B"),
    ]);
    let (b, snapshot) = new_turn(begin_native(
        &store,
        &scope,
        &request(messages.clone()),
        &replay,
        None,
    ));
    let target = snapshot.assessment_target.as_ref().unwrap();
    assert!(!target.target_partial);
    assert_eq!(target.history_membership.as_ref().unwrap(), &membership);
    assert!(
        !serde_json::to_string(&snapshot.visible_conversation)
            .unwrap()
            .contains("private reasoning")
    );
    // A new fully matched user turn can share the same model segment. Its
    // exact edge must not launder uncertainty in the earlier segment members.
    let b_completed = finish_output(&store, &b, "", "B DONE", "r4", true).unwrap();
    assert!(
        !b_completed
            .history_membership
            .as_ref()
            .unwrap()
            .association_unverified
    );
    messages.extend([
        text(MessageRole::Assistant, "B DONE"),
        text(MessageRole::User, "C"),
    ]);
    let (c, next) = new_turn(begin_native(
        &store,
        &scope,
        &request(messages),
        &replay,
        None,
    ));
    let mixed = next
        .assessment_target
        .as_ref()
        .unwrap()
        .history_membership
        .as_ref()
        .unwrap();
    assert_eq!(mixed.request_ids, ["r1", "r2", "r3", "r4"]);
    assert!(mixed.association_unverified);
    assert_eq!(
        mixed.relation,
        hiroute_domain::HistoryRelationV1::Contextual
    );
    assert!(!mixed.capture_partial);
    store.abort(&c);
}

#[test]
fn projection_ambiguity_cannot_be_resolved_by_native_turn_or_admission_order() {
    for second_reasoning in ["different reasoning", ""] {
        for latest_native in [1, 2] {
            let (_dir, replay) = replay();
            let store = AgentTurnHistoryStore::default();
            let scope = store
                .scope_key("workspace", "ambiguous-projection")
                .unwrap();
            let input = request(vec![text(MessageRole::User, "same A")]);
            let (a, _) = new_turn(begin_native(&store, &scope, &input, &replay, Some(1)));
            let (b, _) = new_turn(begin_native(&store, &scope, &input, &replay, Some(2)));
            finish_output(&store, &b, second_reasoning, "SAME", "b", false);
            finish_output(&store, &a, "first reasoning", "SAME", "a", false);
            let next = request(vec![
                text(MessageRole::User, "same A"),
                text(MessageRole::Assistant, "SAME"),
            ]);
            let (c, _) = new_turn(begin_native(
                &store,
                &scope,
                &next,
                &replay,
                Some(latest_native),
            ));
            assert_ne!(c.key, a.key);
            assert_ne!(c.key, b.key);
            assert_eq!(
                store.inner.lock().entries[&c.key].branch_relation,
                super::super::branches::HistoryRelation::AmbiguousPrefix
            );
            store.abort(&c);
        }
    }
}

#[test]
fn omitted_new_response_reasoning_never_masks_rewritten_old_request_history() {
    let (_dir, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store.scope_key("workspace", "old-prefix").unwrap();
    let old = json!({"model":"route","input":[
        {"type":"reasoning","summary":[{"type":"summary_text","text":"retained older reasoning"}]},
        {"role":"assistant","content":"older answer"},{"role":"user","content":"A"}
    ]});
    let original = super::reasoning_boundary::production_request_for(
        IngressProtocol::Responses,
        &old,
        &replay,
        0,
    );
    let (a, _) = new_turn(begin_native(&store, &scope, &original, &replay, None));
    finish_output(&store, &a, "new reasoning", "READY", "a", false);
    let mut changed = old;
    changed["input"].as_array_mut().unwrap().remove(0);
    changed["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"role":"assistant","content":"READY"}));
    let changed = super::reasoning_boundary::production_request_for(
        IngressProtocol::Responses,
        &changed,
        &replay,
        0,
    );
    let (b, _) = new_turn(begin_native(&store, &scope, &changed, &replay, None));
    assert_ne!(b.key, a.key);
    store.abort(&b);
}

#[test]
fn json_backing_and_inline_charge_the_same_semantic_bytes_once() {
    use crate::context_hold::fingerprint::MAX_BYTES;
    let (_directory, replay) = replay();
    for text in [
        "x".repeat(600),
        "\n\"\\".repeat(200),
        "__hiroute_content_ref_v2_1_0_1_1__".to_string(),
    ] {
        let mut results = Vec::new();
        for allowance in [0, 64 * 1024] {
            let request = super::reasoning_boundary::production_request_for(
                IngressProtocol::ChatCompletions,
                &wire(IngressProtocol::ChatCompletions, Some(&text)),
                &replay,
                allowance,
            );
            let mut budget = FingerprintBudget::default();
            budget.bytes(MAX_BYTES - 1000).unwrap();
            results.push(incoming_evidence(&request, 0, &replay, &mut budget).unwrap());
        }
        assert_eq!(results[0].matches(results[1]), Some(false));
    }
}

#[test]
fn native_unknown_reasoning_cannot_hide_behind_a_full_visible_digest_match() {
    let (_directory, replay) = replay();
    let capture = accepted("", true).evidence().unwrap();
    for value in [json!({"opaque":"x"}), json!(["unknown"])] {
        let mut body = wire(IngressProtocol::ChatCompletions, None);
        body["messages"][0]["reasoning"] = value;
        let request = super::reasoning_boundary::production_request_for(
            IngressProtocol::ChatCompletions,
            &body,
            &replay,
            0,
        );
        assert!(request.native_only);
        let incoming = incoming_evidence(&request, 0, &replay, &mut FingerprintBudget::default());
        assert!(incoming.is_none_or(|e| capture.matches(e).is_none()));
    }
}

#[test]
fn chat_omitted_reasoning_tool_stage_freezes_both_actual_requests_for_b() {
    use crate::server::core_runtime::model_ir::ModelStreamEventV1;
    for allowance in [0, 8 * 1024, 64 * 1024] {
        let (_dir, replay) = replay();
        let store = AgentTurnHistoryStore::default();
        let scope = store.scope_key("workspace", "chat-actual-tool").unwrap();
        let mut body = json!({"model":"route","messages":[{"role":"user","content":"read A"}]});
        let parse = |body: &serde_json::Value| {
            super::reasoning_boundary::production_request_for(
                IngressProtocol::ChatCompletions,
                body,
                &replay,
                allowance,
            )
        };
        let (a, _) = new_turn(begin_native(&store, &scope, &parse(&body), &replay, None));
        store.commit_decision(&a, decision("regular")).unwrap();
        let mut owner = super::accepted_output::owner();
        owner.upstream_protocol = IngressProtocol::ChatCompletions;
        store.accept_output(
            &a,
            &[
                ModelStreamEventV1::new(
                    0,
                    ModelEvent::ReasoningFinished {
                        index: 0,
                        text: "actual planning".into(),
                        status: ResponseItemStatus::Completed,
                    },
                ),
                ModelStreamEventV1::new(
                    1,
                    ModelEvent::ToolCallStarted {
                        index: 1,
                        logical_id: "call".into(),
                        native_id: "call".into(),
                        tool_kind: ToolKindV1::Function,
                        namespace: None,
                        name: "read".into(),
                        owner: Box::new(owner),
                        item_id: None,
                    },
                ),
                ModelStreamEventV1::new(
                    2,
                    ModelEvent::ToolCallFinished {
                        encrypted_function_args: None,
                        index: 1,
                        logical_id: "call".into(),
                        tool_kind: ToolKindV1::Function,
                        namespace: None,
                        name: "read".into(),
                        arguments: json!({"path":"fixture"}),
                        status: ResponseItemStatus::Completed,
                    },
                ),
            ],
        );
        assert!(
            store
                .finish_request(
                    &a,
                    "tool-request".into(),
                    AgentTurnStatus::Completed,
                    vec![execution(
                        "actual-model",
                        "actual-profile",
                        "regular",
                        "tool-request"
                    )],
                    false,
                    Instant::now()
                )
                .unwrap()
                .is_none()
        );
        body["messages"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":"","tool_calls":[{"id":"call","type":"function","function":{"name":"read","arguments":"{ \"path\" : \"fixture\" }"}}]}),
            json!({"role":"tool","tool_call_id":"call","content":"real tool result"})
        ]);
        let AgentTurnBegin::Continuation {
            ticket: final_request,
            ..
        } = begin_native(&store, &scope, &parse(&body), &replay, None)
        else {
            panic!("actual Chat tool response must retain the stage")
        };
        assert_eq!(a.agent_turn_id, final_request.agent_turn_id);
        assert!(!final_request.allow_affinity);
        super::accepted_output::text_output(&store, &final_request, "A DONE");
        let completed = store
            .finish_request(
                &final_request,
                "final-request".into(),
                AgentTurnStatus::Completed,
                vec![execution(
                    "actual-model",
                    "actual-profile",
                    "regular",
                    "final-request",
                )],
                true,
                Instant::now(),
            )
            .unwrap()
            .unwrap();
        let membership = completed.history_membership.unwrap();
        assert_eq!(membership.request_ids, ["tool-request", "final-request"]);
        assert!(membership.association_unverified);
        assert!(!membership.capture_partial);
        body["messages"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":"A DONE"}),
            json!({"role":"user","content":"new B"}),
        ]);
        let (b, snapshot) = new_turn(begin_native(&store, &scope, &parse(&body), &replay, None));
        let target = snapshot.assessment_target.as_ref().unwrap();
        assert_eq!(target.history_membership.as_ref().unwrap(), &membership);
        assert!(!target.target_partial);
        assert!(
            matches!(&target.attribution,ExecutionAttribution::Single {model_configuration_id,profile_digest,..} if model_configuration_id == "actual-model" && profile_digest == "actual-profile")
        );
        let visible = serde_json::to_string(&snapshot.visible_conversation).unwrap();
        assert!(visible.contains("completed"));
        assert!(visible.contains("A DONE"));
        assert!(!visible.contains("actual planning"));
        assert!(!visible.contains("real tool result"));
        store.abort(&b);
    }
}
