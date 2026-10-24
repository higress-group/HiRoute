//! Input-boundary evidence must not silently promote a response association.
use super::*;
use crate::agent_turn_history::{CurrentInputPosition, projection::AnalyzedRequest};
use crate::context_hold::HistoryContinuity;
use hiroute_domain::AgentTurnOpeningCauseV1 as Cause;

fn admit(
    store: &AgentTurnHistoryStore,
    scope: &AgentTurnHistoryKey,
    analyzed: AnalyzedRequest,
    native_turn: Option<[u8; 32]>,
) -> AgentTurnBegin {
    store
        .begin_with_admission(
            scope.clone(),
            plan(1),
            analyzed,
            TurnDecisionInputs {
                native_turn,
                history_continuity: HistoryContinuity::Continuing,
                reselect_on_user_message: true,
            },
            Instant::now(),
            || Some(()),
        )
        .unwrap()
        .unwrap()
        .0
}

fn interrupted(
    store: &AgentTurnHistoryStore,
    scope: &AgentTurnHistoryKey,
    input: &ModelRequestIRV1,
    replay: &ReplayStore,
) -> (AgentTurnTicket, CompletedAgentTurn) {
    let analyzed = store.prepare_scoped_request(scope, input, replay).unwrap();
    let (ticket, history) = new_turn(admit(store, scope, analyzed, Some([1; 32])));
    drop(history);
    store.bind_request(&ticket, "a").unwrap();
    store.commit_decision(&ticket, decision("regular")).unwrap();
    accepted_output::text_output(store, &ticket, "delivered text");
    accepted_output::tool(store, &ticket, "unfinished", "lookup");
    store.mark_output_partial(&ticket);
    let finished = store
        .finish_request(
            &ticket,
            "a".into(),
            AgentTurnStatus::Interrupted,
            vec![execution("model", "profile", "regular", "a")],
            true,
            Instant::now(),
        )
        .unwrap()
        .unwrap();
    (ticket, finished)
}

#[test]
fn new_input_after_interrupted_output_keeps_independent_boundary_and_frozen_partial_target() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store.scope_key("workspace", "interrupted-input").unwrap();
    let user = text(MessageRole::User, "A");
    let (a, old) = interrupted(&store, &scope, &request(vec![user.clone()]), &replay);
    let mut messages = vec![
        user,
        text(MessageRole::Assistant, "delivered text"),
        text(MessageRole::User, "B"),
    ];
    let input = request(messages.clone());
    let analyzed = store
        .prepare_scoped_request(&scope, &input, &replay)
        .unwrap();
    let (b, history) = new_turn(admit(&store, &scope, analyzed, Some([2; 32])));
    assert_ne!(
        a.key, b.key,
        "an incomplete output edge cannot reuse the old branch"
    );
    assert!(history.previous_decision.is_none());
    let target = history.assessment_target.as_ref().unwrap();
    assert!(target.target_partial);
    let membership = target.history_membership.as_ref().unwrap();
    assert_eq!(membership.request_ids, ["a"]);
    assert!(membership.capture_partial && membership.association_unverified);
    assert_eq!(target.through_turn_id, a.agent_turn_id);
    assert_eq!(
        store.bind_request(&b, "b").unwrap().opening_cause,
        Cause::UserMessage
    );
    accepted_output::text_output(&store, &b, "B completed");
    let closed_b = close_turn(
        &store,
        &b,
        "regular",
        "model",
        "profile",
        "b",
        Instant::now(),
    );
    assert!(!closed_b.history_partial);
    assert_ne!(closed_b.segment_id, old.segment_id);
    let inner = store.inner.lock();
    let retained = &inner.entries[&a.key].closed.back().unwrap().completed;
    assert_eq!(retained.status, AgentTurnStatus::Interrupted);
    assert!(retained.history_partial);
    assert_eq!(retained.history_membership, old.history_membership);
    assert_eq!(retained.segment_id, old.segment_id);
    assert_eq!(retained.attribution, old.attribution);
    drop(inner);
    drop(history);
    messages.extend([
        text(MessageRole::Assistant, "B completed"),
        text(MessageRole::User, "C"),
    ]);
    let input = request(messages);
    let analyzed = store
        .prepare_scoped_request(&scope, &input, &replay)
        .unwrap();
    let (c, history) = new_turn(admit(&store, &scope, analyzed, Some([3; 32])));
    let target = history.assessment_target.as_ref().unwrap();
    assert!(!target.target_partial);
    assert_eq!(
        target.history_membership.as_ref().unwrap().request_ids,
        ["b"]
    );
    assert_eq!(target.through_turn_id, b.agent_turn_id);
    assert_eq!(target.segment_id, closed_b.segment_id);
    store.abort(&c);
}

#[test]
fn independent_boundary_requires_actual_comparable_input_evidence() {
    for case in [
        "changed",
        "no_input",
        "anchors_absent",
        "position_absent",
        "same_id_new_input",
        "explicit_unavailable",
        "window_overlap",
        "retry",
    ] {
        let (_directory, replay) = replay();
        let store = AgentTurnHistoryStore::default();
        let scope = store.scope_key("workspace", case).unwrap();
        let user = text(MessageRole::User, "A");
        let (a, _) = interrupted(&store, &scope, &request(vec![user.clone()]), &replay);
        let mut messages = vec![user, text(MessageRole::Assistant, "delivered text")];
        if case != "no_input" {
            messages.push(text(MessageRole::User, "B"));
        }
        if case == "changed" {
            messages[0] = text(MessageRole::User, "different old input");
        }
        if case == "retry" {
            messages.truncate(1);
        }
        let input = request(messages);
        let mut analyzed = store
            .prepare_scoped_request(&scope, &input, &replay)
            .unwrap();
        if case == "anchors_absent" {
            analyzed.anchors = None;
        } else if case == "position_absent" {
            analyzed.anchors.as_mut().unwrap().prefixes.clear();
        } else if case == "explicit_unavailable" {
            analyzed.mark_current_input_unavailable();
        } else if case == "window_overlap" {
            analyzed
                .select_current_input(
                    &input.messages[2].content,
                    Some(CurrentInputPosition::Window { first: 0, last: 2 }),
                    &input,
                    &replay,
                )
                .unwrap();
        }
        let native = if matches!(case, "same_id_new_input" | "explicit_unavailable") {
            1
        } else {
            2
        };
        let (b, _) = new_turn(admit(&store, &scope, analyzed, Some([native; 32])));
        assert_ne!(a.key, b.key, "{case}");
        let expected = if case == "changed" {
            Cause::HistoryRewrite
        } else if case == "same_id_new_input" {
            Cause::UserMessage
        } else {
            Cause::Other
        };
        assert_eq!(
            store.bind_request(&b, "b").unwrap().opening_cause,
            expected,
            "{case}"
        );
        store.abort(&b);
    }
}

#[test]
fn concurrent_ingress_retry_cannot_use_an_older_prefix_to_claim_new_input() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store.scope_key("workspace", "concurrent-retry").unwrap();
    let user = text(MessageRole::User, "A");
    interrupted(&store, &scope, &request(vec![user.clone()]), &replay);
    let input = request(vec![
        user,
        text(MessageRole::Assistant, "delivered text"),
        text(MessageRole::User, "B"),
    ]);
    let analyzed = store
        .prepare_scoped_request(&scope, &input, &replay)
        .unwrap();
    let (b, _) = new_turn(admit(&store, &scope, analyzed, Some([2; 32])));
    assert_eq!(
        store.bind_request(&b, "b").unwrap().opening_cause,
        Cause::UserMessage
    );
    let analyzed = store
        .prepare_scoped_request(&scope, &input, &replay)
        .unwrap();
    let (retry, _) = new_turn(admit(&store, &scope, analyzed, Some([2; 32])));
    assert_ne!(retry.key, b.key);
    assert_eq!(
        store.bind_request(&retry, "retry").unwrap().opening_cause,
        Cause::Other
    );
    store.abort(&retry);
    store.abort(&b);
}

#[test]
fn checkpoint_advancing_past_measured_positions_is_unavailable_not_rewritten() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store.scope_key("workspace", "advanced-checkpoint").unwrap();
    let user = text(MessageRole::User, "A");
    let (a, _) = interrupted(&store, &scope, &request(vec![user.clone()]), &replay);
    let input = request(vec![
        user,
        text(MessageRole::Assistant, "delivered text"),
        text(MessageRole::User, "B"),
    ]);
    let analyzed = store
        .prepare_scoped_request(&scope, &input, &replay)
        .unwrap();
    // A concurrent admission can advance this count after projection releases
    // the lock. The new position was never hashed by this request.
    store
        .inner
        .lock()
        .entries
        .get_mut(&a.key)
        .unwrap()
        .checkpoint
        .message_count = 2;
    let (b, _) = new_turn(admit(&store, &scope, analyzed, Some([2; 32])));
    assert_eq!(
        store.bind_request(&b, "b").unwrap().opening_cause,
        Cause::Other
    );
    store.abort(&b);
}

#[test]
fn changed_native_id_without_new_input_keeps_exact_tool_stage_until_real_new_input() {
    use crate::server::core_runtime::model_ir::{
        ModelEvent, ModelStreamEventV1, ResponseItemStatus,
    };

    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store.scope_key("workspace", "native-id-only").unwrap();
    let user = text(MessageRole::User, "A: look up the fixture");
    let mut messages = vec![user.clone()];
    let input = request(messages.clone());
    let mut analyzed = store
        .prepare_scoped_request(&scope, &input, &replay)
        .unwrap();
    analyzed.mark_user_origin_unverified();
    let (a, history) = new_turn(admit(&store, &scope, analyzed, Some([1; 32])));
    drop(history);
    let original = store.bind_request(&a, "a-tool").unwrap();
    store.commit_decision(&a, decision("regular")).unwrap();
    accepted_output::tool(&store, &a, "call-1", "lookup");
    store.accept_output(
        &a,
        &[ModelStreamEventV1::new(
            1,
            ModelEvent::ToolCallFinished {
                encrypted_function_args: None,
                index: 1,
                logical_id: "call-1".into(),
                tool_kind: ToolKindV1::Function,
                namespace: Some("functions".into()),
                name: "lookup".into(),
                arguments: json!({"must_not_be_projected":"secret detail"}),
                status: ResponseItemStatus::Completed,
            },
        )],
    );
    assert!(
        store
            .finish_request(
                &a,
                "a-tool".into(),
                AgentTurnStatus::Completed,
                vec![execution("model", "profile", "regular", "a-tool")],
                false,
                Instant::now()
            )
            .unwrap()
            .is_none()
    );

    messages.extend([tool_call("call-1", "lookup"), tool_result("call-1", false)]);
    let input = request(messages.clone());
    let mut analyzed = store
        .prepare_scoped_request(&scope, &input, &replay)
        .unwrap();
    analyzed
        .select_current_input(
            &user.content,
            Some(CurrentInputPosition::Window { first: 0, last: 0 }),
            &input,
            &replay,
        )
        .unwrap();
    analyzed.mark_user_origin_unverified();
    let AgentTurnBegin::Continuation {
        ticket: continued,
        decision: inherited,
    } = admit(&store, &scope, analyzed, Some([2; 32]))
    else {
        panic!(
            "an exact tool edge plus a changed native ID is not a new input or assessment boundary"
        )
    };
    assert!(!continued.new_turn);
    assert_eq!(continued.key, a.key);
    assert_eq!(continued.agent_turn_id, a.agent_turn_id);
    assert_eq!(inherited.branch_id, "regular");
    let next = store.bind_request(&continued, "a-final").unwrap();
    assert_eq!(next.opening_cause, original.opening_cause);
    {
        let inner = store.inner.lock();
        let entry = &inner.entries[&a.key];
        assert!(
            entry.closed.is_empty(),
            "tool continuation cannot close or score A"
        );
        assert!(!entry.active.finalized);
        assert!(entry.current_segment.is_none());
    }
    accepted_output::text_output(&store, &continued, "A completed");
    let closed = store
        .finish_request(
            &continued,
            "a-final".into(),
            AgentTurnStatus::Completed,
            vec![execution("model", "profile", "regular", "a-final")],
            true,
            Instant::now(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(closed.agent_turn_id, a.agent_turn_id);
    assert_eq!(closed.status, AgentTurnStatus::Completed);
    assert!(!closed.history_partial);
    assert_eq!(
        closed.history_membership.as_ref().unwrap().request_ids,
        ["a-tool", "a-final"]
    );

    messages.extend([
        text(MessageRole::Assistant, "A completed"),
        text(MessageRole::User, "B: inspect the next independent fixture"),
    ]);
    let input = request(messages);
    let mut analyzed = store
        .prepare_scoped_request(&scope, &input, &replay)
        .unwrap();
    analyzed
        .select_current_input(
            &input.messages[4].content,
            Some(CurrentInputPosition::Window { first: 4, last: 4 }),
            &input,
            &replay,
        )
        .unwrap();
    analyzed.mark_user_origin_unverified();
    let AgentTurnBegin::NewTurn {
        ticket: b,
        history,
        inherited_decision,
        completed,
    } = admit(&store, &scope, analyzed, Some([3; 32]))
    else {
        panic!("a genuinely appended input must still request a fresh decision")
    };
    assert!(inherited_decision.is_none());
    assert!(
        completed.is_none(),
        "A already closed on its actual final response"
    );
    assert_ne!(b.agent_turn_id, a.agent_turn_id);
    assert_eq!(
        store.bind_request(&b, "b").unwrap().opening_cause,
        Cause::UserMessage
    );
    let target = history.assessment_target.as_ref().unwrap();
    assert!(!target.target_partial);
    assert_eq!(target.through_turn_id, a.agent_turn_id);
    assert_eq!(target.segment_id, closed.segment_id);
    assert_eq!(target.history_membership, closed.history_membership);
    assert_eq!(history.visible_conversation.len(), 1);
    assert_eq!(
        history.visible_conversation[0].status,
        AgentTurnStatus::Completed
    );
    assert_eq!(
        history.visible_conversation[0].steps,
        vec![
            vec![VisibleContentPart::ToolActivity {
                tool: "functions.lookup".into(),
                status: ToolStatus::Completed
            }],
            vec![VisibleContentPart::Text {
                text: "A completed".into()
            }],
        ]
    );
    store.abort(&b);
}

fn finish_tool_step(store: &AgentTurnHistoryStore, ticket: &AgentTurnTicket, id: &str) {
    use crate::server::core_runtime::model_ir::{
        ModelEvent, ModelStreamEventV1, ResponseItemStatus,
    };
    store.bind_request(ticket, id).unwrap();
    if ticket.new_turn {
        store.commit_decision(ticket, decision("regular")).unwrap();
    }
    accepted_output::tool(store, ticket, id, "lookup");
    store.accept_output(
        ticket,
        &[ModelStreamEventV1::new(
            1,
            ModelEvent::ToolCallFinished {
                encrypted_function_args: None,
                index: 1,
                logical_id: id.into(),
                tool_kind: ToolKindV1::Function,
                namespace: Some("functions".into()),
                name: "lookup".into(),
                arguments: json!({"must_not_be_projected":"secret detail"}),
                status: ResponseItemStatus::Completed,
            },
        )],
    );
    assert!(
        store
            .finish_request(
                ticket,
                id.into(),
                AgentTurnStatus::Completed,
                vec![execution("model", "profile", "regular", id)],
                false,
                Instant::now()
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn selected_input_position_controls_exact_admission_independently_of_native_id() {
    for appended in [false, true] {
        for changed_id in [false, true] {
            let (_directory, replay) = replay();
            let store = AgentTurnHistoryStore::default();
            let scope = store.scope_key("workspace", "input-id-matrix").unwrap();
            let user = text(MessageRole::User, "actual A input");
            let input = request(vec![user.clone()]);
            let analyzed = store
                .prepare_scoped_request(&scope, &input, &replay)
                .unwrap();
            let (a, history) = new_turn(admit(&store, &scope, analyzed, Some([1; 32])));
            drop(history);
            finish_tool_step(&store, &a, "a-tool");

            let mut messages = vec![
                user,
                tool_call("a-tool", "lookup"),
                tool_result("a-tool", false),
            ];
            if appended {
                messages.push(text(MessageRole::User, "actual B input"));
            }
            let input = request(messages);
            let mut analyzed = store
                .prepare_scoped_request(&scope, &input, &replay)
                .unwrap();
            let index = if appended { 3 } else { 0 };
            analyzed
                .select_current_input(
                    &input.messages[index].content,
                    Some(CurrentInputPosition::Window {
                        first: index,
                        last: index,
                    }),
                    &input,
                    &replay,
                )
                .unwrap();
            let native = if changed_id { 2 } else { 1 };
            let begin = admit(&store, &scope, analyzed, Some([native; 32]));
            match begin {
                AgentTurnBegin::NewTurn {
                    ticket,
                    completed,
                    inherited_decision,
                    ..
                } => {
                    assert!(
                        appended,
                        "changed_id={changed_id}: an ID cannot invent input"
                    );
                    assert_eq!(ticket.key, a.key, "the accepted response edge is exact");
                    assert_ne!(ticket.agent_turn_id, a.agent_turn_id);
                    assert!(
                        inherited_decision.is_none(),
                        "new input requests a fresh decision"
                    );
                    let completed = completed.unwrap();
                    assert_eq!(completed.agent_turn_id, a.agent_turn_id);
                    assert_eq!(
                        completed.history_membership.unwrap().request_ids,
                        ["a-tool"]
                    );
                    assert_eq!(
                        store.bind_request(&ticket, "b").unwrap().opening_cause,
                        Cause::UserMessage
                    );
                    store.abort(&ticket);
                }
                AgentTurnBegin::Continuation { ticket, decision } => {
                    assert!(
                        !appended,
                        "changed_id={changed_id}: an ID cannot hide input"
                    );
                    assert_eq!(ticket.key, a.key);
                    assert_eq!(ticket.agent_turn_id, a.agent_turn_id);
                    assert_eq!(decision.branch_id, "regular");
                    {
                        let inner = store.inner.lock();
                        let entry = &inner.entries[&a.key];
                        assert!(entry.closed.is_empty(), "no closure or assessment target");
                        assert!(!entry.active.finalized);
                        assert!(entry.current_segment.is_none());
                    }
                    store.abort(&ticket);
                }
            }
        }
    }
}

#[test]
fn changed_native_id_is_retained_for_later_explicit_existing_turn_input() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store.scope_key("workspace", "latest-native-id").unwrap();
    let mut messages = vec![text(MessageRole::User, "actual original A input")];
    let input = request(messages.clone());
    let analyzed = store
        .prepare_scoped_request(&scope, &input, &replay)
        .unwrap();
    let (a, history) = new_turn(admit(&store, &scope, analyzed, Some([1; 32])));
    drop(history);
    finish_tool_step(&store, &a, "a-1");

    messages.extend([tool_call("a-1", "lookup"), tool_result("a-1", false)]);
    let input = request(messages.clone());
    let analyzed = store
        .prepare_scoped_request(&scope, &input, &replay)
        .unwrap();
    let AgentTurnBegin::Continuation { ticket: a2, .. } =
        admit(&store, &scope, analyzed, Some([2; 32]))
    else {
        panic!("ID1 to ID2 must retain the exact current tool stage")
    };
    assert_eq!(a2.agent_turn_id, a.agent_turn_id);
    finish_tool_step(&store, &a2, "a-2");

    // ExistingTurn explicitly says the wire User is continuation context.
    // Production clears the selected position; ID equality alone cannot do so.
    let synthetic = "synthetic continuation notification, not a new task";
    messages.extend([
        tool_call("a-2", "lookup"),
        tool_result("a-2", false),
        text(MessageRole::User, synthetic),
    ]);
    let input = request(messages);
    let mut analyzed = store
        .prepare_scoped_request(&scope, &input, &replay)
        .unwrap();
    analyzed.mark_current_input_unavailable();
    assert_eq!(analyzed.current_input_start, None);
    let AgentTurnBegin::Continuation {
        ticket: a3,
        decision: inherited,
    } = admit(&store, &scope, analyzed, Some([2; 32]))
    else {
        panic!("explicit ExistingTurn context must retain the captured original input")
    };
    assert!(!a3.new_turn);
    assert_eq!(a3.key, a.key);
    assert_eq!(a3.agent_turn_id, a.agent_turn_id);
    assert_eq!(inherited.branch_id, "regular");
    store.bind_request(&a3, "a-3").unwrap();
    {
        let inner = store.inner.lock();
        let entry = &inner.entries[&a.key];
        assert!(entry.closed.is_empty());
        assert!(!entry.active.finalized);
        assert_eq!(
            entry.active.user,
            vec![VisibleContentPart::Text {
                text: "actual original A input".into(),
            }]
        );
    }
    accepted_output::text_output(&store, &a3, "actual final A output");
    let completed = store
        .finish_request(
            &a3,
            "a-3".into(),
            AgentTurnStatus::Completed,
            vec![execution("model", "profile", "regular", "a-3")],
            true,
            Instant::now(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(completed.agent_turn_id, a.agent_turn_id);
    assert!(!completed.history_partial);
    assert_eq!(
        completed.history_membership.as_ref().unwrap().request_ids,
        ["a-1", "a-2", "a-3"]
    );
    let inner = store.inner.lock();
    let closed = &inner.entries[&a.key].closed;
    assert_eq!(closed.len(), 1);
    assert!(
        !serde_json::to_string(&closed[0].wire.user)
            .unwrap()
            .contains(synthetic)
    );
    assert_eq!(closed[0].wire.steps.len(), 3);
}
