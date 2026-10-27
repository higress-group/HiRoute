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
