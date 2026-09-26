use super::*;
use crate::server::core_runtime::model_ir::{
    ExactProviderPathV1, ModelEvent, ModelStreamEventV1, ResponseItemStatus, WebSearchAction,
    WebSearchCallV1, WebSearchPhase, WebSearchStatus,
};

pub(super) fn text_output(store: &AgentTurnHistoryStore, ticket: &AgentTurnTicket, text: &str) {
    store.accept_output(
        ticket,
        &[ModelStreamEventV1::new(
            0,
            ModelEvent::TextDelta {
                index: 0,
                text: text.into(),
            },
        )],
    );
}

pub(super) fn tool(store: &AgentTurnHistoryStore, ticket: &AgentTurnTicket, id: &str, name: &str) {
    store.accept_output(
        ticket,
        &[ModelStreamEventV1::new(
            0,
            ModelEvent::ToolCallStarted {
                index: 1,
                logical_id: id.into(),
                native_id: id.into(),
                tool_kind: ToolKindV1::Function,
                namespace: Some("functions".into()),
                name: name.into(),
                owner: Box::new(owner()),
                item_id: None,
            },
        )],
    );
}

fn owner() -> ExactProviderPathV1 {
    ExactProviderPathV1 {
        provider_id: "p".into(),
        endpoint_id: "e".into(),
        entitlement_id: "e".into(),
        connector_id: "c".into(),
        connector_revision: "1".into(),
        capability_id: "c".into(),
        capability_revision: "1".into(),
        model_configuration_id: "m".into(),
        native_model: "m".into(),
        upstream_protocol: IngressProtocol::Responses,
        adapter_revision: "1".into(),
        serializer_revision: "1".into(),
        decoder_revision: "1".into(),
    }
}

#[test]
fn accepted_search_is_one_activity_without_details_or_reasoning() {
    let (_dir, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let key = store.scope_key("w", "search").unwrap();
    let now = Instant::now();
    let (ticket, history) = new_turn(
        store
            .begin(
                key.clone(),
                plan(1),
                &request(vec![text(MessageRole::User, "search")]),
                &replay,
                now,
            )
            .unwrap(),
    );
    drop(history);
    for status in [WebSearchStatus::Searching, WebSearchStatus::Failed] {
        store.accept_output(
            &ticket,
            &[
                ModelStreamEventV1::new(
                    0,
                    ModelEvent::WebSearch {
                        index: 1,
                        phase: WebSearchPhase::Done,
                        native_id: "native".into(),
                        owner: Box::new(owner()),
                        item: WebSearchCallV1 {
                            id: "search".into(),
                            status,
                            action: Some(WebSearchAction::OpenPage {
                                url: "private-url".into(),
                            }),
                        },
                    },
                ),
                ModelStreamEventV1::new(
                    1,
                    ModelEvent::ReasoningDelta {
                        index: 2,
                        text: "private reasoning".into(),
                    },
                ),
            ],
        );
    }
    assert_eq!(
        store.inner.lock().entries[&key].active.steps,
        vec![vec![VisibleContentPart::ToolActivity {
            tool: "web_search".into(),
            status: crate::agent_turn_history::ToolStatus::Failed
        }]]
    );
}

#[test]
fn accepted_text_survives_client_rewrite_and_history_rebuild_without_terminal_duplication() {
    for rebuilt in [false, true] {
        let (_dir, replay) = replay();
        let store = AgentTurnHistoryStore::default();
        let key = store.scope_key("w", "s").unwrap();
        let now = Instant::now();
        let (ticket, history) = new_turn(
            store
                .begin(
                    key.clone(),
                    plan(1),
                    &request(vec![text(MessageRole::User, "first")]),
                    &replay,
                    now,
                )
                .unwrap(),
        );
        drop(history);
        text_output(&store, &ticket, "actual ");
        text_output(&store, &ticket, "answer");
        store.accept_output(
            &ticket,
            &[ModelStreamEventV1::new(
                2,
                ModelEvent::TextFinished {
                    index: 0,
                    text: "actual answer".into(),
                    annotations: Vec::new(),
                    status: ResponseItemStatus::Completed,
                },
            )],
        );
        close_turn(&store, &ticket, "simple", "m", "p", "r", now);
        let mut messages = if rebuilt {
            Vec::new()
        } else {
            vec![
                text(MessageRole::User, "first"),
                text(MessageRole::Assistant, "client fabricated answer"),
            ]
        };
        messages.push(text(MessageRole::User, "second"));
        let input = request(messages);
        let boundary = if rebuilt {
            store.begin_with_context(
                key,
                plan(1),
                &input,
                &replay,
                TurnDecisionInputs {
                    message_history_continues: false,
                    reselect_on_user_message: true,
                },
                now,
            )
        } else {
            store.begin(key, plan(1), &input, &replay, now)
        };
        let (_, history) = new_turn(boundary.unwrap());
        assert_eq!(
            history.visible_conversation[0].steps,
            vec![vec![VisibleContentPart::Text {
                text: "actual answer".into()
            }]]
        );
        assert!(
            !history.history_partial,
            "a ContextHold boundary does not erase locally accepted history"
        );
    }
}

#[test]
fn interrupted_output_is_preserved_and_late_events_cannot_touch_the_next_request() {
    let (_dir, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let key = store.scope_key("w", "s").unwrap();
    let now = Instant::now();
    let (ticket, history) = new_turn(
        store
            .begin(
                key.clone(),
                plan(1),
                &request(vec![text(MessageRole::User, "first")]),
                &replay,
                now,
            )
            .unwrap(),
    );
    drop(history);
    store.commit_decision(&ticket, decision("simple")).unwrap();
    text_output(&store, &ticket, "delivered prefix");
    store.mark_output_partial(&ticket);
    store
        .finish_request(
            &ticket,
            "r".into(),
            AgentTurnStatus::Interrupted,
            vec![execution("m", "p", "simple", "r")],
            true,
            now,
        )
        .unwrap();
    let (next, history) = new_turn(
        store
            .begin_with_context(
                key.clone(),
                plan(1),
                &request(vec![text(MessageRole::User, "second")]),
                &replay,
                TurnDecisionInputs {
                    message_history_continues: false,
                    reselect_on_user_message: true,
                },
                now,
            )
            .unwrap(),
    );
    text_output(&store, &ticket, "must not arrive");
    assert!(history.history_partial);
    assert_eq!(
        history.visible_conversation[0].steps,
        vec![vec![VisibleContentPart::Text {
            text: "delivered prefix".into()
        }]]
    );
    assert!(store.inner.lock().entries[&key].active.steps.is_empty());
    store.abort(&next);
}

#[test]
fn dropped_request_marks_its_accepted_prefix_partial_and_rejects_queued_events() {
    let (_dir, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let key = store.scope_key("w", "drop").unwrap();
    let now = Instant::now();
    let (ticket, history) = new_turn(
        store
            .begin(
                key.clone(),
                plan(1),
                &request(vec![text(MessageRole::User, "first")]),
                &replay,
                now,
            )
            .unwrap(),
    );
    drop(history);
    store.commit_decision(&ticket, decision("simple")).unwrap();
    text_output(&store, &ticket, "accepted prefix");
    store.abort(&ticket);
    text_output(&store, &ticket, "queued after drop");
    let (_, history) = new_turn(
        store
            .begin_with_context(
                key,
                plan(1),
                &request(vec![text(MessageRole::User, "next")]),
                &replay,
                TurnDecisionInputs {
                    message_history_continues: false,
                    reselect_on_user_message: true,
                },
                now,
            )
            .unwrap(),
    );
    assert!(history.history_partial);
    assert_eq!(
        history.visible_conversation[0].status,
        AgentTurnStatus::Unknown
    );
    assert_eq!(
        history.visible_conversation[0].steps,
        vec![vec![VisibleContentPart::Text {
            text: "accepted prefix".into()
        }]]
    );
}

#[test]
fn retry_of_finalized_request_retains_original_user_and_both_accepted_answers() {
    let (_dir, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let key = store.scope_key("w", "retry").unwrap();
    let now = Instant::now();
    let input = request(vec![text(MessageRole::User, "original question")]);
    let (ticket, history) = new_turn(
        store
            .begin(key.clone(), plan(1), &input, &replay, now)
            .unwrap(),
    );
    drop(history);
    text_output(&store, &ticket, "first accepted answer");
    close_turn(&store, &ticket, "simple", "m", "p", "r1", now);
    let AgentTurnBegin::Continuation { ticket: retry, .. } = store
        .begin(key.clone(), plan(1), &input, &replay, now)
        .unwrap()
    else {
        panic!("exact retry must retain the frozen turn")
    };
    text_output(&store, &retry, "retry accepted answer");
    store
        .finish_request(
            &retry,
            "r2".into(),
            AgentTurnStatus::Completed,
            vec![execution("m", "p", "simple", "r2")],
            true,
            now,
        )
        .unwrap();
    let (_, history) = new_turn(
        store
            .begin_with_context(
                key,
                plan(1),
                &request(vec![text(MessageRole::User, "new question")]),
                &replay,
                TurnDecisionInputs {
                    message_history_continues: false,
                    reselect_on_user_message: true,
                },
                now,
            )
            .unwrap(),
    );
    assert_eq!(history.visible_conversation.len(), 1);
    assert!(!history.history_partial);
    assert_eq!(
        history.visible_conversation[0].user,
        vec![VisibleContentPart::Text {
            text: "original question".into()
        }]
    );
    assert_eq!(
        history.visible_conversation[0].steps,
        vec![
            vec![VisibleContentPart::Text {
                text: "first accepted answer".into()
            }],
            vec![VisibleContentPart::Text {
                text: "retry accepted answer".into()
            }],
        ]
    );
}

#[test]
fn output_capture_is_bounded_and_never_imports_client_assistant_on_cache_miss() {
    let (_dir, replay) = replay();
    let store = AgentTurnHistoryStore::new(32 * 1024, Duration::from_secs(60));
    let key = store.scope_key("w", "s").unwrap();
    let (ticket, history) = new_turn(
        store
            .begin(
                key.clone(),
                plan(1),
                &request(vec![
                    text(MessageRole::User, "first"),
                    text(MessageRole::Assistant, "untrusted history"),
                ]),
                &replay,
                Instant::now(),
            )
            .unwrap(),
    );
    assert!(
        history.history_partial,
        "unobserved assistant content must not be presented as complete"
    );
    drop(history);
    text_output(&store, &ticket, "a small accepted prefix");
    text_output(&store, &ticket, &"x".repeat(64 * 1024));
    let inner = store.inner.lock();
    let entry = &inner.entries[&key];
    assert!(entry.active.capture_partial);
    assert!(inner.accounted_bytes <= 32 * 1024);
    assert_eq!(
        entry.active.steps,
        vec![vec![VisibleContentPart::Text {
            text: "a small accepted prefix".into()
        }]]
    );
}
