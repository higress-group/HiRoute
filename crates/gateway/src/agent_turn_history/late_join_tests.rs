use super::*;
use crate::server::core_runtime::adapters::decode_ingress_request;
use crate::server::core_runtime::model_ir::{ModelEvent, ModelStreamEventV1, ResponseItemStatus};
use serde_json::Value;

const TOOL_RESPONSE: &str = "Use the tool.";
const TOOL_ARGUMENTS: &str = r#"{"id":7}"#;

fn wire_request(protocol: IngressProtocol, transcript: &[Value]) -> ModelRequestIRV1 {
    let body = match protocol {
        IngressProtocol::ChatCompletions => json!({"model":"route","messages":transcript}),
        IngressProtocol::Messages => {
            json!({"model":"route","max_tokens":128,"messages":transcript})
        }
        IngressProtocol::Responses => json!({"model":"route","input":transcript}),
    };
    decode_ingress_request(protocol, &body).unwrap()
}

fn responses_text(text: &str) -> Value {
    json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]})
}

fn late_join_prefix(protocol: IngressProtocol) -> Vec<Value> {
    let old_output = "Output from before this branch opened.";
    let assistant = if protocol == IngressProtocol::Responses {
        responses_text(old_output)
    } else {
        json!({"role":"assistant","content":old_output})
    };
    vec![
        json!({"role":"user","content":"Look up an item."}),
        assistant,
    ]
}

fn responses_call(id: &str) -> Value {
    json!({"type":"function_call","id":format!("fc_{id}"),"call_id":id,
        "name":"lookup","arguments":TOOL_ARGUMENTS,"status":"completed"})
}

fn responses_result(id: &str) -> Value {
    json!({"type":"function_call_output","call_id":id,"output":"found"})
}

fn tool_exchange(protocol: IngressProtocol, id: &str) -> Vec<Value> {
    match protocol {
        IngressProtocol::ChatCompletions => vec![
            json!({"role":"assistant","content":TOOL_RESPONSE,"tool_calls":[{"type":"function",
                "id":id,"function":{"name":"lookup","arguments":TOOL_ARGUMENTS}}]}),
            json!({"role":"tool","tool_call_id":id,"content":"found"}),
        ],
        IngressProtocol::Messages => vec![
            json!({"role":"assistant","content":[{"type":"text","text":TOOL_RESPONSE},
                {"type":"tool_use","id":id,"name":"lookup","input":{"id":7}}]}),
            json!({"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":"found"}]}),
        ],
        IngressProtocol::Responses => vec![
            responses_text(TOOL_RESPONSE),
            responses_call(id),
            responses_result(id),
        ],
    }
}

fn accept_tool_response(
    store: &AgentTurnHistoryStore,
    ticket: &AgentTurnTicket,
    protocol: IngressProtocol,
    id: &str,
) {
    accepted_output::text_output(store, ticket, TOOL_RESPONSE);
    let mut owner = accepted_output::owner();
    owner.upstream_protocol = protocol;
    store.accept_output(
        ticket,
        &[
            ModelStreamEventV1::new(
                1,
                ModelEvent::ToolCallStarted {
                    index: 1,
                    logical_id: id.into(),
                    native_id: id.into(),
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
                    index: 1,
                    logical_id: id.into(),
                    tool_kind: ToolKindV1::Function,
                    namespace: None,
                    name: "lookup".into(),
                    arguments: serde_json::from_str(TOOL_ARGUMENTS).unwrap(),
                    encrypted_function_args: None,
                    status: ResponseItemStatus::Completed,
                },
            ),
        ],
    );
}

fn finish(
    store: &AgentTurnHistoryStore,
    ticket: &AgentTurnTicket,
    request_id: &str,
    completion: impl Into<RequestCompletion>,
) -> Option<CompletedAgentTurn> {
    store
        .finish_request(
            ticket,
            request_id.into(),
            AgentTurnStatus::Completed,
            vec![execution("model", "profile", "regular", request_id)],
            completion,
            Instant::now(),
        )
        .unwrap()
}

fn assert_late_join_tool_rounds_keep_one_partial_stage(protocol: IngressProtocol) {
    let (_dir, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store.scope_key("workspace", "late-join").unwrap();
    let mut transcript = late_join_prefix(protocol);
    let input = wire_request(protocol, &transcript);
    let (mut ticket, history) = new_turn(scoped(&store, &scope, &input, &replay).unwrap());
    assert!(history.history_partial);
    let branch = ticket.key.clone();
    let turn = ticket.agent_turn_id.clone();
    store.commit_decision(&ticket, decision("regular")).unwrap();

    for (request_id, call_id) in [("first-request", "first"), ("second-request", "second")] {
        accept_tool_response(&store, &ticket, protocol, call_id);
        assert!(finish(&store, &ticket, request_id, false).is_none());
        transcript.extend(tool_exchange(protocol, call_id));
        let next = wire_request(protocol, &transcript);
        let AgentTurnBegin::Continuation {
            ticket: next,
            decision,
        } = scoped(&store, &scope, &next, &replay).unwrap()
        else {
            panic!("{protocol:?} must continue after {request_id}");
        };
        assert_eq!(next.key, branch);
        assert_eq!(next.agent_turn_id, turn);
        assert_eq!(decision.branch_id, "regular");
        ticket = next;
    }

    accepted_output::text_output(&store, &ticket, "Finished both lookups.");
    let completed = finish(&store, &ticket, "final-request", true).unwrap();
    assert!(completed.history_partial);
    assert_eq!(completed.agent_turn_id, turn);
    let membership = completed.history_membership.unwrap();
    assert!(membership.capture_partial);
    assert_eq!(
        membership.request_ids,
        ["first-request", "second-request", "final-request"]
    );
}

#[test]
fn late_join_chat_completions_tool_rounds_keep_one_partial_stage() {
    assert_late_join_tool_rounds_keep_one_partial_stage(IngressProtocol::ChatCompletions);
}

#[test]
fn late_join_messages_tool_rounds_keep_one_partial_stage() {
    assert_late_join_tool_rounds_keep_one_partial_stage(IngressProtocol::Messages);
}

#[test]
fn late_join_responses_tool_rounds_keep_one_partial_stage() {
    assert_late_join_tool_rounds_keep_one_partial_stage(IngressProtocol::Responses);
}

#[test]
fn late_join_responses_preserves_full_boundary_priority_over_tool_batch() {
    let protocol = IngressProtocol::Responses;
    for (replayed_text, expected_affinity) in [
        (Some(TOOL_RESPONSE), Some(true)),
        (None, Some(false)),
        (Some("Rewritten response."), None),
    ] {
        let (_dir, replay) = replay();
        let store = AgentTurnHistoryStore::default();
        let scope = store.scope_key("workspace", "late-join-responses").unwrap();
        let mut transcript = late_join_prefix(protocol);
        let input = wire_request(protocol, &transcript);
        let (first, _) = new_turn(scoped(&store, &scope, &input, &replay).unwrap());
        store.commit_decision(&first, decision("regular")).unwrap();
        accept_tool_response(&store, &first, protocol, "lookup");
        let call = responses_call("lookup");
        let batch = Arc::new(
            DeliveredToolBatch::from_completed_output(std::slice::from_ref(&call), |_| true)
                .unwrap(),
        );
        assert!(
            finish(
                &store,
                &first,
                "first-request",
                RequestCompletion {
                    terminal: false,
                    tool_batch: Some(Arc::clone(&batch))
                },
            )
            .is_none()
        );
        if let Some(text) = replayed_text {
            transcript.extend([responses_text(text), call]);
        }
        transcript.push(responses_result("lookup"));
        let next = wire_request(protocol, &transcript);
        let analyzed = store
            .prepare_scoped_request(&scope, &next, &replay)
            .unwrap();
        assert!(
            analyzed
                .tool_suffixes
                .iter()
                .any(|(_, suffix)| batch.matches(suffix))
        );
        assert_eq!(analyzed.boundaries.is_empty(), replayed_text.is_none());

        let ticket = match (
            scoped(&store, &scope, &next, &replay).unwrap(),
            expected_affinity,
        ) {
            (AgentTurnBegin::Continuation { ticket, decision }, Some(affinity)) => {
                assert_eq!(ticket.key, first.key);
                assert_eq!(ticket.agent_turn_id, first.agent_turn_id);
                assert_eq!(decision.branch_id, "regular");
                assert_eq!(ticket.allow_affinity, affinity, "{replayed_text:?}");
                ticket
            }
            (
                AgentTurnBegin::NewTurn {
                    ticket,
                    inherited_decision,
                    ..
                },
                None,
            ) => {
                assert_ne!(ticket.key, first.key);
                assert!(inherited_decision.is_none());
                ticket
            }
            _ => panic!("unexpected admission for replay {replayed_text:?}"),
        };
        store.abort(&ticket);
    }
}

#[test]
fn late_join_real_capture_gap_blocks_response_boundary() {
    for protocol in [
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
        IngressProtocol::Responses,
    ] {
        let (_dir, replay) = replay();
        let store = AgentTurnHistoryStore::default();
        let scope = store
            .scope_key("workspace", "late-join-capture-gap")
            .unwrap();
        let mut transcript = late_join_prefix(protocol);
        let input = wire_request(protocol, &transcript);
        let (first, _) = new_turn(scoped(&store, &scope, &input, &replay).unwrap());
        store.commit_decision(&first, decision("regular")).unwrap();
        accept_tool_response(&store, &first, protocol, "lookup");
        assert!(
            store.inner.lock().entries[&first.key]
                .active
                .boundary
                .evidence()
                .is_some()
        );
        store.mark_output_partial(&first);
        assert!(finish(&store, &first, "first-request", false).is_none());
        assert!(
            store.inner.lock().entries[&first.key]
                .response_boundary
                .is_none(),
            "{protocol:?}"
        );

        transcript.extend(tool_exchange(protocol, "lookup"));
        let next = wire_request(protocol, &transcript);
        let (fork, _) = new_turn(scoped(&store, &scope, &next, &replay).unwrap());
        assert_ne!(fork.key, first.key, "{protocol:?}");
        store.abort(&fork);
    }
}
