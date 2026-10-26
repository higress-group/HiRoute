//! Routing-stage closure is independent of transport termination and capture coverage.
//! Exercise measured production admission; caller-supplied continuity is not proof.
use super::*;
use crate::server::core_runtime::model_ir::{ModelEvent, ModelStreamEventV1, ResponseItemStatus};

fn measured(
    store: &AgentTurnHistoryStore,
    scope: &AgentTurnHistoryKey,
    input: &ModelRequestIRV1,
    replay: &ReplayStore,
    source_unverified: bool,
) -> AgentTurnBegin {
    let mut analyzed = store.prepare_scoped_request(scope, input, replay).unwrap();
    if source_unverified {
        analyzed.mark_user_origin_unverified();
    }
    store
        .begin_with_admission(
            scope.clone(),
            plan(1),
            analyzed,
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
}

pub(super) fn accepted_tool(store: &AgentTurnHistoryStore, ticket: &AgentTurnTicket, id: &str) {
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
                arguments: json!({"must_not_be_projected": "secret detail"}),
                status: ResponseItemStatus::Completed,
            },
        )],
    );
}

fn finish(
    store: &AgentTurnHistoryStore,
    ticket: &AgentTurnTicket,
    request_id: &str,
    status: AgentTurnStatus,
    terminal: bool,
) -> Option<CompletedAgentTurn> {
    if ticket.new_turn {
        store.commit_decision(ticket, decision("regular")).unwrap();
    }
    store
        .finish_request(
            ticket,
            request_id.into(),
            status,
            vec![execution("model", "profile", "regular", request_id)],
            terminal,
            Instant::now(),
        )
        .unwrap()
}

fn target_with_members<'a>(
    history: &'a AgentTurnHistorySnapshot,
    requests: &[&str],
) -> &'a AssessmentTarget {
    let target = history
        .assessment_target
        .as_ref()
        .expect("captured actual execution must remain available for assessment");
    let membership = target.history_membership.as_ref().unwrap();
    assert_eq!(
        membership
            .request_ids
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        requests,
        "assessment must bind the complete actual request set"
    );
    assert!(!membership.clipped);
    assert!(
        !membership.capture_partial,
        "an unproved closure must not fabricate missing accepted content"
    );
    target
}

#[test]
fn tool_stage_association_loss_freezes_actual_unknown_execution_as_partial() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store
        .scope_key("workspace", "closure-tool-mismatch")
        .unwrap();
    let user = text(MessageRole::User, "Look up the record");
    let (a, _) = new_turn(measured(
        &store,
        &scope,
        &request(vec![user.clone()]),
        &replay,
        false,
    ));
    accepted_tool(&store, &a, "call-1");
    assert!(finish(&store, &a, "a-tool", AgentTurnStatus::Completed, false).is_none());

    // Same real activation and failed tool result, but the echoed assistant
    // arguments no longer match what the Gateway actually delivered.
    let mut rewritten_call = tool_call("call-1", "lookup");
    if let ContentPart::ToolCall { arguments, .. } = &mut rewritten_call.content[0] {
        *arguments = json!({"must_not_be_projected": "rewritten detail"});
    }
    let AgentTurnBegin::NewTurn {
        ticket,
        history,
        completed,
        ..
    } = measured(
        &store,
        &scope,
        &request(vec![user, rewritten_call, tool_result("call-1", true)]),
        &replay,
        false,
    )
    else {
        panic!("an unproved edge must reclassify without inheriting the old decision")
    };
    assert_ne!(ticket.key, a.key);
    assert!(history.previous_decision.is_none());
    let closed = completed.unwrap();
    assert_eq!(closed.status, AgentTurnStatus::Unknown);
    assert!(!closed.history_partial);
    assert!(!closed.history_membership.as_ref().unwrap().capture_partial);
    let target = target_with_members(&history, &["a-tool"]);
    assert_eq!(target.through_turn_id, a.agent_turn_id);
    assert!(
        target
            .history_membership
            .as_ref()
            .unwrap()
            .association_unverified
    );
    assert!(
        target.target_partial,
        "freezing is not proof that the tool stage ended"
    );
    assert_eq!(
        history.visible_conversation[0].status,
        AgentTurnStatus::Unknown
    );
    assert_eq!(
        history.visible_conversation[0].steps,
        vec![vec![VisibleContentPart::ToolActivity {
            tool: "functions.lookup".into(),
            status: ToolStatus::Unknown,
        }]],
        "an unproved client result cannot rewrite the actual old tool outcome"
    );
    store.abort(&ticket);
}

#[test]
fn matching_failed_tool_result_and_retry_stay_in_one_actual_stage() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store.scope_key("workspace", "closure-tool-retry").unwrap();
    let mut messages = vec![text(MessageRole::User, "Look up the record")];
    let (a, _) = new_turn(measured(
        &store,
        &scope,
        &request(messages.clone()),
        &replay,
        false,
    ));
    accepted_tool(&store, &a, "call-1");
    finish(&store, &a, "a-tool-1", AgentTurnStatus::Completed, false);
    messages.extend([tool_call("call-1", "lookup"), tool_result("call-1", true)]);
    let AgentTurnBegin::Continuation { ticket: retry, .. } =
        measured(&store, &scope, &request(messages.clone()), &replay, false)
    else {
        panic!("a failed result for the actual delivered call is still a tool continuation")
    };
    assert_eq!(retry.agent_turn_id, a.agent_turn_id);
    assert_eq!(retry.key, a.key);
    accepted_tool(&store, &retry, "call-2");
    finish(
        &store,
        &retry,
        "a-tool-2",
        AgentTurnStatus::Completed,
        false,
    );
    messages.extend([tool_call("call-2", "lookup"), tool_result("call-2", false)]);
    let AgentTurnBegin::Continuation {
        ticket: final_ticket,
        ..
    } = measured(&store, &scope, &request(messages.clone()), &replay, false)
    else {
        panic!("the retry result must continue the same activation")
    };
    assert_eq!(final_ticket.agent_turn_id, a.agent_turn_id);
    assert!(store.inner.lock().entries[&a.key].closed.is_empty());
    accepted_output::text_output(&store, &final_ticket, "The record is ready");
    finish(
        &store,
        &final_ticket,
        "a-final",
        AgentTurnStatus::Completed,
        true,
    );
    messages.extend([
        text(MessageRole::Assistant, "The record is ready"),
        text(MessageRole::User, "Assess the preceding work"),
    ]);
    let (b, history) = new_turn(measured(&store, &scope, &request(messages), &replay, false));
    let target = target_with_members(&history, &["a-tool-1", "a-tool-2", "a-final"]);
    assert!(!target.target_partial);
    assert_eq!(history.visible_conversation.len(), 1);
    let tool_statuses = history.visible_conversation[0]
        .steps
        .iter()
        .flatten()
        .filter_map(|part| match part {
            VisibleContentPart::ToolActivity { status, .. } => Some(*status),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        tool_statuses,
        vec![ToolStatus::Failed, ToolStatus::Completed]
    );
    store.abort(&b);
}

#[test]
fn witnessed_new_activation_closes_routing_stage_without_inventing_task_completion() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store
        .scope_key("workspace", "closure-new-activation")
        .unwrap();
    let mut messages = vec![text(MessageRole::User, "Look up the first record")];
    let (a, _) = new_turn(measured(
        &store,
        &scope,
        &request(messages.clone()),
        &replay,
        false,
    ));
    accepted_tool(&store, &a, "call-1");
    finish(&store, &a, "a-tool", AgentTurnStatus::Completed, false);
    messages.extend([
        tool_call("call-1", "lookup"),
        tool_result("call-1", true),
        text(MessageRole::User, "Now look up a different record"),
    ]);
    let AgentTurnBegin::NewTurn {
        ticket: b,
        history,
        completed,
        ..
    } = measured(&store, &scope, &request(messages.clone()), &replay, false)
    else {
        panic!("a measured appended activation must open a new decision")
    };
    let closed = completed.unwrap();
    assert_eq!(closed.status, AgentTurnStatus::Unknown);
    assert!(!closed.history_partial);
    assert_eq!(
        history.visible_conversation[0].status,
        AgentTurnStatus::Unknown
    );
    assert!(!target_with_members(&history, &["a-tool"]).target_partial);
    assert_eq!(
        history.visible_conversation[0].steps,
        vec![vec![VisibleContentPart::ToolActivity {
            tool: "functions.lookup".into(),
            status: ToolStatus::Failed,
        }]]
    );
    accepted_output::text_output(&store, &b, "The different record is ready");
    let b_closed = finish(&store, &b, "b-final", AgentTurnStatus::Completed, true).unwrap();
    assert_eq!(b_closed.segment_id, closed.segment_id);
    messages.extend([
        text(MessageRole::Assistant, "The different record is ready"),
        text(MessageRole::User, "Assess the complete preceding stage"),
    ]);
    let (c, all) = new_turn(measured(&store, &scope, &request(messages), &replay, false));
    assert!(!target_with_members(&all, &["a-tool", "b-final"]).target_partial);
    assert_eq!(all.visible_conversation[0].status, AgentTurnStatus::Unknown);
    store.abort(&c);
}

#[test]
fn terminal_request_with_unknown_outcome_does_not_prove_stage_closure() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store
        .scope_key("workspace", "closure-provider-unknown")
        .unwrap();
    let (a, _) = new_turn(measured(
        &store,
        &scope,
        &request(vec![text(MessageRole::User, "A")]),
        &replay,
        false,
    ));
    accepted_output::text_output(&store, &a, "Actually delivered partial output");
    let closed = finish(&store, &a, "a-unknown", AgentTurnStatus::Unknown, true).unwrap();
    assert_eq!(closed.status, AgentTurnStatus::Unknown);
    assert!(!closed.history_partial);
    let (b, history) = new_turn(measured(
        &store,
        &scope,
        &request(vec![text(MessageRole::User, "rewritten input")]),
        &replay,
        false,
    ));
    let target = target_with_members(&history, &["a-unknown"]);
    assert!(
        target.target_partial,
        "no outstanding HTTP request is not a final model outcome"
    );
    assert_eq!(
        history.visible_conversation[0].status,
        AgentTurnStatus::Unknown
    );
    store.abort(&b);
}

#[test]
fn new_activation_cannot_turn_a_provider_unknown_tool_response_into_complete_evidence() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store
        .scope_key("workspace", "closure-tool-provider-unknown")
        .unwrap();
    let mut messages = vec![text(MessageRole::User, "Look up the record")];
    let (a, _) = new_turn(measured(
        &store,
        &scope,
        &request(messages.clone()),
        &replay,
        false,
    ));
    accepted_tool(&store, &a, "call-1");
    // Unlike a known Completed tool response, the provider outcome itself is
    // unresolved. The emitted call and a new user input cannot prove that
    // the old response ended; keep the actual accepted capture intact.
    finish(
        &store,
        &a,
        "a-provider-unknown",
        AgentTurnStatus::Unknown,
        false,
    );
    messages.extend([
        tool_call("call-1", "lookup"),
        tool_result("call-1", false),
        text(MessageRole::User, "Independent B"),
    ]);
    let AgentTurnBegin::NewTurn {
        ticket: b,
        history,
        completed,
        ..
    } = measured(&store, &scope, &request(messages), &replay, false)
    else {
        panic!("the real new activation must still be independently routable")
    };
    assert_eq!(completed.unwrap().status, AgentTurnStatus::Unknown);
    assert!(
        target_with_members(&history, &["a-provider-unknown"]).target_partial,
        "a routing boundary cannot supply the missing provider outcome"
    );
    store.abort(&b);
}

#[test]
fn later_complete_new_input_recovers_without_washing_an_unclosed_segment() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store
        .scope_key("workspace", "closure-new-input-recovery")
        .unwrap();
    let mut messages = vec![text(MessageRole::User, "A")];
    let (a, _) = new_turn(measured(
        &store,
        &scope,
        &request(messages.clone()),
        &replay,
        false,
    ));
    accepted_output::text_output(&store, &a, "Delivered A output");
    let a_closed = finish(&store, &a, "a-unknown", AgentTurnStatus::Unknown, true).unwrap();
    messages.extend([
        text(MessageRole::Assistant, "Delivered A output"),
        text(MessageRole::User, "Independent B"),
    ]);
    let (b, frozen) = new_turn(measured(
        &store,
        &scope,
        &request(messages.clone()),
        &replay,
        false,
    ));
    assert!(target_with_members(&frozen, &["a-unknown"]).target_partial);
    accepted_output::text_output(&store, &b, "B is complete");
    let b_closed = finish(&store, &b, "b-final", AgentTurnStatus::Completed, true).unwrap();
    assert_ne!(a_closed.segment_id, b_closed.segment_id);
    messages.extend([
        text(MessageRole::Assistant, "B is complete"),
        text(MessageRole::User, "Assess B"),
    ]);
    let (c, recovered) = new_turn(measured(&store, &scope, &request(messages), &replay, false));
    let target = target_with_members(&recovered, &["b-final"]);
    assert_eq!(target.segment_id, b_closed.segment_id);
    assert!(
        !target.target_partial,
        "an earlier unproved boundary cannot poison a new complete stage"
    );
    assert!(target_with_members(&frozen, &["a-unknown"]).target_partial);
    assert_eq!(
        frozen.visible_conversation[0].status,
        AgentTurnStatus::Unknown
    );
    store.abort(&c);
}

#[test]
fn complete_actual_stage_remains_assessable_with_unverified_context_and_source() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store
        .scope_key("workspace", "closure-complete-contextual")
        .unwrap();
    let (a, _) = new_turn(measured(
        &store,
        &scope,
        &request(vec![text(MessageRole::User, "Actual native input A")]),
        &replay,
        true,
    ));
    accepted_output::text_output(&store, &a, "Actual accepted A answer");
    let closed = finish(&store, &a, "a-final", AgentTurnStatus::Completed, true).unwrap();
    let (b, history) = new_turn(measured(
        &store,
        &scope,
        &request(vec![text(MessageRole::User, "Rewritten native input B")]),
        &replay,
        true,
    ));
    assert!(history.previous_decision.is_none());
    let target = target_with_members(&history, &["a-final"]);
    assert_eq!(target.segment_id, closed.segment_id);
    assert!(
        !target.target_partial,
        "uncertain association alone must not disable actual-stage assessment"
    );
    let membership = target.history_membership.as_ref().unwrap();
    assert!(membership.association_unverified);
    assert!(membership.source_unverified);
    assert_eq!(
        membership.relation,
        hiroute_domain::HistoryRelationV1::Contextual
    );
    let visible = serde_json::to_string(&history.visible_conversation).unwrap();
    assert!(visible.contains("Actual accepted A answer"));
    assert!(visible.contains("unverified_request_user_origin"));
    store.abort(&b);
}

#[test]
fn same_turn_reopen_preserves_provider_outcomes_and_frozen_snapshots() {
    for provider_status in [AgentTurnStatus::Completed, AgentTurnStatus::Unknown] {
        let (_directory, replay) = replay();
        let store = AgentTurnHistoryStore::default();
        let scope = store
            .scope_key("workspace", "closure-same-turn-reopen")
            .unwrap();
        let mut messages = vec![text(MessageRole::User, "A")];
        let (a, _) = new_turn(measured(
            &store,
            &scope,
            &request(messages.clone()),
            &replay,
            false,
        ));
        accepted_tool(&store, &a, "call-1");
        finish(&store, &a, "a-first", provider_status, false);
        let (fork, frozen) = new_turn(measured(
            &store,
            &scope,
            &request(vec![text(MessageRole::User, "Unrelated rewrite")]),
            &replay,
            false,
        ));
        assert!(target_with_members(&frozen, &["a-first"]).target_partial);
        store.abort(&fork);

        messages.extend([tool_call("call-1", "lookup"), tool_result("call-1", false)]);
        let AgentTurnBegin::Continuation {
            ticket: resumed, ..
        } = measured(&store, &scope, &request(messages.clone()), &replay, false)
        else {
            panic!("the exact accepted prefix must reopen the original execution")
        };
        assert_eq!(resumed.key, a.key);
        assert_eq!(resumed.agent_turn_id, a.agent_turn_id);
        assert!(store.inner.lock().entries[&a.key].closed.is_empty());
        accepted_output::text_output(&store, &resumed, "A is finally complete");
        finish(
            &store,
            &resumed,
            "a-final",
            AgentTurnStatus::Completed,
            true,
        );
        messages.extend([
            text(MessageRole::Assistant, "A is finally complete"),
            text(MessageRole::User, "Assess all of A"),
        ]);
        let (b, history) = new_turn(measured(&store, &scope, &request(messages), &replay, false));
        let target = target_with_members(&history, &["a-first", "a-final"]);
        assert_eq!(
            target.target_partial,
            provider_status == AgentTurnStatus::Unknown,
            "a later final cannot invent the earlier provider outcome"
        );
        assert_eq!(history.visible_conversation.len(), 1);
        let visible = serde_json::to_string(&history.visible_conversation).unwrap();
        assert!(visible.contains("functions.lookup"));
        assert!(visible.contains("A is finally complete"));
        assert!(target_with_members(&frozen, &["a-first"]).target_partial);
        assert_eq!(
            frozen.visible_conversation[0].status,
            AgentTurnStatus::Unknown
        );
        store.abort(&b);
    }
}
