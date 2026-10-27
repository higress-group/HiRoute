//! Native plaintext replay after cancellation must retain actual tool membership.
use super::*;
use crate::agent_turn_history::boundary::{BoundaryCapture, incoming_evidence};
use crate::context_hold::fingerprint::FingerprintBudget;
use crate::server::core_runtime::model_ir::{ModelEvent, ModelStreamEventV1, ResponseItemStatus};

fn reasoning(text: &str) -> serde_json::Value {
    json!({"type":"reasoning","id":"reasoning-item","status":null,
        "summary":[],"content":[{"type":"reasoning_text","text":text}],
        "encrypted_content":null})
}

fn begin(
    store: &AgentTurnHistoryStore,
    scope: &AgentTurnHistoryKey,
    replay: &ReplayStore,
    wire: &serde_json::Value,
    allowance: usize,
) -> AgentTurnBegin {
    let request = reasoning_boundary::production_request_for(
        IngressProtocol::Responses,
        wire,
        replay,
        allowance,
    );
    store
        .begin_with_admission(
            scope.clone(),
            plan(1),
            store
                .prepare_scoped_request(scope, &request, replay)
                .unwrap(),
            TurnDecisionInputs {
                native_turn: Some([7; 32]),
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

fn finish(
    store: &AgentTurnHistoryStore,
    ticket: &AgentTurnTicket,
    request: &str,
    status: AgentTurnStatus,
    terminal: bool,
) -> Option<CompletedAgentTurn> {
    store
        .finish_request(
            ticket,
            request.into(),
            status,
            vec![execution("actual-model", "profile", "regular", request)],
            terminal,
            Instant::now(),
        )
        .unwrap()
}

fn tool_step(
    store: &AgentTurnHistoryStore,
    ticket: &AgentTurnTicket,
    wire: &mut serde_json::Value,
    id: &str,
    path: &str,
    result: &str,
) {
    store.bind_request(ticket, id);
    store.accept_output(
        ticket,
        &[ModelStreamEventV1::new(
            0,
            ModelEvent::ReasoningFinished {
                index: 0,
                text: "Read the requested file.".into(),
                status: ResponseItemStatus::Completed,
            },
        )],
    );
    accepted_output::tool(store, ticket, id, "read");
    store.accept_output(
        ticket,
        &[ModelStreamEventV1::new(
            1,
            ModelEvent::ToolCallFinished {
                index: 1,
                logical_id: id.into(),
                tool_kind: ToolKindV1::Function,
                namespace: Some("functions".into()),
                name: "read".into(),
                arguments: json!({"path":path}),
                encrypted_function_args: None,
                status: ResponseItemStatus::Completed,
            },
        )],
    );
    assert!(finish(store, ticket, id, AgentTurnStatus::Completed, false).is_none());
    wire["input"].as_array_mut().unwrap().extend([
        reasoning("Read the requested file."),
        json!({"type":"function_call","call_id":id,"namespace":"functions","name":"read",
            "arguments":serde_json::to_string(&json!({"path":path})).unwrap()}),
        json!({"type":"function_call_output","call_id":id,"output":result}),
    ]);
}

fn continuation(begin: AgentTurnBegin, prior: &AgentTurnTicket) -> AgentTurnTicket {
    let AgentTurnBegin::Continuation { ticket, decision } = begin else {
        panic!("an exact accepted plaintext/tool edge must inherit the frozen decision")
    };
    assert_eq!(ticket.agent_turn_id, prior.agent_turn_id);
    assert_eq!(ticket.key, prior.key);
    assert_eq!(decision.branch_id, "regular");
    ticket
}

#[test]
fn cancelled_turn_then_plaintext_error_recovery_keeps_one_complete_stage() {
    for allowance in [0, 8 * 1024, 64 * 1024] {
        let (_dir, replay) = replay();
        let store = AgentTurnHistoryStore::default();
        let scope = store.scope_key("workspace", "plaintext-recovery").unwrap();
        let mut wire = json!({"model":"route","instructions":"x".repeat(8*1024),
            "input":[{"role":"user","content":"Read two files and analyze."}]});
        let (a, history) = new_turn(begin(&store, &scope, &replay, &wire, allowance));
        drop(history);
        store.commit_decision(&a, decision("regular")).unwrap();
        tool_step(&store, &a, &mut wire, "a1", "first.txt", "first file");
        let a2 = continuation(begin(&store, &scope, &replay, &wire, allowance), &a);
        tool_step(&store, &a2, &mut wire, "a2", "second.txt", "second file");
        let a3 = continuation(begin(&store, &scope, &replay, &wire, allowance), &a);
        store.bind_request(&a3, "a3");
        accepted_output::text_output(&store, &a3, "An unfinished analysis");
        // The production capture owner marks an interrupted open output as
        // partial; changing the terminal status alone is not capture evidence.
        store.mark_output_partial(&a3);
        let stopped = finish(&store, &a3, "a3", AgentTurnStatus::Interrupted, true).unwrap();
        assert_eq!(stopped.status, AgentTurnStatus::Interrupted);
        assert!(stopped.history_partial);
        wire["input"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":"An unfinished analysis"}),
            json!({"role":"user","content":"Read missing.txt; if absent read README.md."}),
        ]);
        let (b, a_target) = new_turn(begin(&store, &scope, &replay, &wire, allowance));
        assert_ne!(a.agent_turn_id, b.agent_turn_id);
        assert!(a_target.assessment_target.as_ref().unwrap().target_partial);
        drop(a_target);
        store.commit_decision(&b, decision("regular")).unwrap();
        tool_step(
            &store,
            &b,
            &mut wire,
            "b1",
            "missing.txt",
            "Error: not found",
        );
        let b2 = continuation(begin(&store, &scope, &replay, &wire, allowance), &b);
        tool_step(
            &store,
            &b2,
            &mut wire,
            "b2",
            "README.md",
            "Synthetic project",
        );
        let b3 = continuation(begin(&store, &scope, &replay, &wire, allowance), &b);
        store.bind_request(&b3, "b3");
        accepted_output::text_output(&store, &b3, "Recovered by reading README.");
        let completed = finish(&store, &b3, "b3", AgentTurnStatus::Completed, true).unwrap();
        let members = completed.history_membership.as_ref().unwrap();
        assert_eq!(completed.status, AgentTurnStatus::Completed);
        assert_eq!(members.request_ids, ["b1", "b2", "b3"]);
        assert!(!members.capture_partial);
        wire["input"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":"Recovered by reading README."}),
            json!({"role":"user","content":"Now summarize the purpose."}),
        ]);
        let (c, frozen) = new_turn(begin(&store, &scope, &replay, &wire, allowance));
        let target = frozen.assessment_target.as_ref().unwrap();
        assert!(!target.target_partial);
        assert_eq!(
            target.history_membership.as_ref().unwrap().request_ids,
            ["b1", "b2", "b3"]
        );
        assert_eq!(target.through_turn_id, b.agent_turn_id);
        store.abort(&c);
    }
}

#[test]
fn plaintext_content_requires_exact_accepted_text_and_closed_shape() {
    for allowance in [0, 8 * 1024, 64 * 1024] {
        let (_dir, replay) = replay();
        let mut capture = BoundaryCapture::default();
        capture.accept(&ModelEvent::ReasoningFinished {
            index: 0,
            text: "Accepted thought.".into(),
            status: ResponseItemStatus::Completed,
        });
        capture.accept(&ModelEvent::TextFinished {
            index: 1,
            text: "READY".into(),
            annotations: vec![],
            status: ResponseItemStatus::Completed,
        });
        let wire = json!({"model":"route","instructions":"x".repeat(8*1024),
            "input":[reasoning("Accepted thought."),{"role":"assistant","content":"READY"}]});
        let matches = |body: &serde_json::Value| {
            let request = reasoning_boundary::production_request_for(
                IngressProtocol::Responses,
                body,
                &replay,
                allowance,
            );
            incoming_evidence(&request, 0, &replay, &mut FingerprintBudget::default())
                .and_then(|incoming| capture.evidence().unwrap().matches(incoming))
        };
        assert_eq!(matches(&wire), Some(false));
        for (pointer, value) in [
            ("/input/0/content/0/text", json!("changed")),
            ("/input/0/content/0/type", json!("unknown")),
            (
                "/input/0/content",
                json!([{"type":"reasoning_text","text":"Accepted thought."},{"type":"reasoning_text","text":"hidden"}]),
            ),
            (
                "/input/0/summary",
                json!([{"type":"summary_text","text":"Accepted thought."}]),
            ),
            ("/input/0/encrypted_content", json!("uncaptured ciphertext")),
            ("/input/1/content", json!("changed answer")),
        ] {
            let mut changed = wire.clone();
            *changed.pointer_mut(pointer).unwrap() = value;
            assert!(matches(&changed).is_none(), "changed {pointer}");
        }
        let mut unknown = wire.clone();
        unknown["input"][0]["content"][0]["extension"] = json!(null);
        assert!(matches(&unknown).is_none());

        let mut tool_capture = BoundaryCapture::default();
        tool_capture.accept(&ModelEvent::ReasoningFinished {
            index: 0,
            text: "Accepted thought.".into(),
            status: ResponseItemStatus::Completed,
        });
        tool_capture.accept(&ModelEvent::ToolCallStarted {
            index: 1,
            logical_id: "call-1".into(),
            native_id: "call-1".into(),
            tool_kind: ToolKindV1::Function,
            namespace: Some("functions".into()),
            name: "read".into(),
            owner: Box::new(accepted_output::owner()),
            item_id: None,
        });
        tool_capture.accept(&ModelEvent::ToolCallFinished {
            index: 1,
            logical_id: "call-1".into(),
            tool_kind: ToolKindV1::Function,
            namespace: Some("functions".into()),
            name: "read".into(),
            arguments: json!({"path":"README.md"}),
            encrypted_function_args: None,
            status: ResponseItemStatus::Completed,
        });
        let mut tool_wire = wire.clone();
        tool_wire["input"][1] = json!({"type":"function_call", "call_id":"call-1",
            "namespace":"functions", "name":"read", "arguments":"{\"path\":\"README.md\"}"});
        let tool_matches = |body: &serde_json::Value| {
            let request = reasoning_boundary::production_request_for(
                IngressProtocol::Responses,
                body,
                &replay,
                allowance,
            );
            incoming_evidence(&request, 0, &replay, &mut FingerprintBudget::default())
                .and_then(|incoming| tool_capture.evidence().unwrap().matches(incoming))
        };
        assert_eq!(tool_matches(&tool_wire), Some(false));
        for (field, value) in [
            ("call_id", json!("other-call")),
            ("name", json!("write")),
            ("arguments", json!("{\"path\":\"other.txt\"}")),
        ] {
            let mut changed = tool_wire.clone();
            changed["input"][1][field] = value;
            assert!(tool_matches(&changed).is_none(), "changed tool {field}");
        }
    }
}
