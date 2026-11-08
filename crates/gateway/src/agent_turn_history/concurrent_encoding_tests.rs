//! A lowered encoding proof cannot choose between concurrent response owners.
use super::*;
use crate::server::core_runtime::model_ir::{ModelEvent, ModelStreamEventV1, ResponseItemStatus};

#[test]
fn plaintext_encoding_ambiguity_keeps_pending_and_completed_branches_separate() {
    for finished in [false, true] {
        let (_dir, replay) = replay();
        let store = AgentTurnHistoryStore::default();
        let scope = store.scope_key("workspace", "encoding-ambiguity").unwrap();
        let input = request(vec![text(MessageRole::User, "same A")]);
        let (a, _) = new_turn(scoped(&store, &scope, &input, &replay).unwrap());
        let (b, _) = new_turn(scoped(&store, &scope, &input, &replay).unwrap());
        for (ticket, id) in [(&a, "a"), (&b, "b")] {
            store.commit_decision(ticket, decision("regular")).unwrap();
            store.bind_request(ticket, id).unwrap();
            accepted_output::tool(&store, ticket, "call", "lookup");
            store.accept_output(
                ticket,
                &[ModelStreamEventV1::new(
                    1,
                    ModelEvent::ToolCallFinished {
                        encrypted_function_args: Some(vec![]),
                        index: 1,
                        logical_id: "call".into(),
                        tool_kind: ToolKindV1::Function,
                        namespace: Some("functions".into()),
                        name: "lookup".into(),
                        arguments: json!({"id":7}),
                        status: ResponseItemStatus::Completed,
                    },
                )],
            );
            if finished {
                store
                    .finish_request(
                        ticket,
                        id.into(),
                        AgentTurnStatus::Completed,
                        vec![execution("model", "profile", "regular", id)],
                        false,
                        Instant::now(),
                    )
                    .unwrap();
            }
        }
        let mut call = tool_call("call", "lookup");
        let ContentPart::ToolCall { arguments, .. } = &mut call.content[0] else {
            unreachable!()
        };
        *arguments = json!({"id":7});
        let next = request(vec![
            text(MessageRole::User, "same A"),
            call,
            tool_result("call", false),
        ]);
        let (c, _) = new_turn(scoped(&store, &scope, &next, &replay).unwrap());
        assert_ne!(c.key, a.key);
        assert_ne!(c.key, b.key);
        store.abort(&c);
        store.abort(&a);
        store.abort(&b);
    }
}

#[test]
fn exact_response_edge_skips_a_siblings_pending_tool_handoff() {
    use crate::server::core_runtime::adapters::decode_ingress_request;
    use hiroute_gateway_core::runtime::body::MemoryRole;

    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store
        .scope_key("workspace", "full-before-tool-handoff")
        .unwrap();
    let user = json!({"role":"user","content":"same input"});
    let decode = |input: Vec<serde_json::Value>| {
        decode_ingress_request(
            IngressProtocol::Responses,
            &json!({"model":"route","input":input}),
        )
        .unwrap()
    };
    let input = decode(vec![user.clone()]);
    let (a, _) = new_turn(scoped(&store, &scope, &input, &replay).unwrap());
    let (b, _) = new_turn(scoped(&store, &scope, &input, &replay).unwrap());
    for ticket in [&a, &b] {
        store.commit_decision(ticket, decision("regular")).unwrap();
    }
    let call = json!({"type":"function_call","id":"fc_shared","call_id":"shared",
        "namespace":"functions","name":"lookup","arguments":"{\"id\":7}","status":"completed"});
    let batch = Arc::new(
        DeliveredToolBatch::from_completed_output(std::slice::from_ref(&call), |_| true).unwrap(),
    );
    let charge = replay
        .budget()
        .reserve(MemoryRole::SemanticState, batch.retained_bytes())
        .unwrap();
    let handoff = store.register_tool_handoff(&a).unwrap();
    handoff.prepare((Arc::clone(&batch), charge));

    accepted_output::text_output(&store, &b, "B accepted output");
    accepted_output::tool(&store, &b, "shared", "lookup");
    store.accept_output(
        &b,
        &[ModelStreamEventV1::new(
            2,
            ModelEvent::ToolCallFinished {
                index: 1,
                logical_id: "shared".into(),
                tool_kind: ToolKindV1::Function,
                namespace: Some("functions".into()),
                name: "lookup".into(),
                arguments: json!({"id": 7}),
                encrypted_function_args: None,
                status: ResponseItemStatus::Completed,
            },
        )],
    );
    store
        .finish_request(
            &b,
            "b".into(),
            AgentTurnStatus::Completed,
            vec![execution("model", "profile", "regular", "b")],
            RequestCompletion {
                terminal: false,
                tool_batch: Some(batch),
            },
            Instant::now(),
        )
        .unwrap();

    let callback = |answer| {
        decode(vec![
            user.clone(),
            json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":answer}]}),
            call.clone(),
            json!({"type":"function_call_output","call_id":"shared","output":"result"}),
        ])
    };
    let lookup = |request: &ModelRequestIRV1| {
        let analyzed = store
            .prepare_scoped_request(&scope, request, &replay)
            .unwrap();
        store.pending_tool_predecessor(
            &scope,
            &plan(1),
            &analyzed,
            &TurnDecisionInputs {
                native_turn: None,
                history_continuity: crate::context_hold::HistoryContinuity::Continuing,
                reselect_on_user_message: true,
            },
        )
    };
    assert!(
        lookup(&callback("A still pending")).is_some_and(|pending| Arc::ptr_eq(&pending, &handoff))
    );
    let exact = callback("B accepted output");
    assert!(lookup(&exact).is_none(), "B's exact edge must never wake A");
    let AgentTurnBegin::Continuation { ticket, .. } =
        scoped(&store, &scope, &exact, &replay).unwrap()
    else {
        panic!("B's exact edge continues its branch");
    };
    assert_eq!(ticket.key, b.key);
    store.abort(&ticket);
    store.abort(&a);
}
