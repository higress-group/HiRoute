//! Measured retries use a native activation and complete request evidence, not a session lock.
use super::*;

pub(super) fn begin(
    store: &AgentTurnHistoryStore,
    scope: &AgentTurnHistoryKey,
    request: &ModelRequestIRV1,
    replay: &ReplayStore,
    native_turn: Option<u8>,
) -> AgentTurnBegin {
    let mut analyzed = store
        .prepare_scoped_request(scope, request, replay)
        .unwrap();
    analyzed.mark_user_origin_unverified();
    store
        .begin_with_admission(
            scope.clone(),
            plan(1),
            analyzed,
            TurnDecisionInputs {
                native_turn: native_turn.map(|value| [value; 32]),
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

fn finish_failed(
    store: &AgentTurnHistoryStore,
    ticket: &AgentTurnTicket,
    id: &str,
) -> CompletedAgentTurn {
    store.commit_decision(ticket, decision("regular")).unwrap();
    store
        .finish_request(
            ticket,
            id.into(),
            AgentTurnStatus::Failed,
            vec![],
            true,
            Instant::now(),
        )
        .unwrap()
        .unwrap()
}

#[test]
fn failed_native_retry_reuses_decision_and_captures_both_actual_requests_for_next_input() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store.scope_key("workspace", "retry").unwrap();
    let input = request(vec![text(MessageRole::User, "A")]);
    let (first, _) = new_turn(begin(&store, &scope, &input, &replay, Some(1)));
    let failed = finish_failed(&store, &first, "failed-before-output");
    let AgentTurnBegin::Continuation {
        ticket: retry,
        decision: inherited,
    } = begin(&store, &scope, &input, &replay, Some(1))
    else {
        panic!("an exact native retry before delivery must not reclassify the same activation")
    };
    assert_eq!(retry.key, first.key);
    assert_eq!(retry.agent_turn_id, first.agent_turn_id);
    assert_eq!(inherited.branch_id, "regular");
    assert!(
        !retry.allow_affinity,
        "a failed request proves no provider cache state"
    );
    accepted_output::text_output(&store, &retry, "actual answer A");
    let completed = store
        .finish_request(
            &retry,
            "successful-retry".into(),
            AgentTurnStatus::Completed,
            vec![execution("model", "profile", "regular", "successful-retry")],
            true,
            Instant::now(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(completed.agent_turn_id, failed.agent_turn_id);
    assert_eq!(
        completed.history_membership.as_ref().unwrap().request_ids,
        ["failed-before-output", "successful-retry"]
    );
    assert_eq!(
        failed.history_membership.as_ref().unwrap().request_ids,
        ["failed-before-output"]
    );
    assert_eq!(failed.status, AgentTurnStatus::Failed);
    let next = request(vec![
        text(MessageRole::User, "A"),
        text(MessageRole::Assistant, "actual answer A"),
        text(MessageRole::User, "B"),
    ]);
    let (b, history) = new_turn(begin(&store, &scope, &next, &replay, Some(2)));
    let target = history
        .assessment_target
        .as_ref()
        .expect("actual retry execution remains evaluable");
    let membership = target.history_membership.as_ref().unwrap();
    assert_eq!(
        membership.request_ids,
        ["failed-before-output", "successful-retry"]
    );
    assert!(membership.source_unverified && membership.association_unverified);
    assert_eq!(history.visible_conversation.len(), 1);
    assert_eq!(
        history.visible_conversation[0].status,
        AgentTurnStatus::Completed
    );
    assert!(
        serde_json::to_string(&history.visible_conversation)
            .unwrap()
            .contains("actual answer A")
    );
    store.abort(&b);
}

#[test]
fn equal_input_does_not_merge_new_activations_changed_setup_or_unproved_delivery() {
    for case in [
        "new-turn",
        "no-turn",
        "changed-input",
        "changed-setup",
        "partial-output",
        "capture-gap",
        "pending",
        "unknown",
        "cancelled",
        "completed",
    ] {
        let (_directory, replay) = replay();
        let store = AgentTurnHistoryStore::default();
        let scope = store.scope_key("workspace", case).unwrap();
        let mut input = request(vec![text(MessageRole::User, "A")]);
        let (first, _) = new_turn(begin(&store, &scope, &input, &replay, Some(1)));
        store.commit_decision(&first, decision("regular")).unwrap();
        if case == "partial-output" {
            accepted_output::text_output(&store, &first, "partial delivery");
        }
        if case != "pending" {
            let status = match case {
                "unknown" => AgentTurnStatus::Unknown,
                "cancelled" => AgentTurnStatus::Interrupted,
                "completed" => AgentTurnStatus::Completed,
                _ => AgentTurnStatus::Failed,
            };
            store
                .finish_request(
                    &first,
                    "original".into(),
                    status,
                    vec![],
                    true,
                    Instant::now(),
                )
                .unwrap();
        }
        if case == "capture-gap" {
            store.note_capture_gap(&scope);
        }
        if case == "changed-input" {
            input = request(vec![text(MessageRole::User, "B")]);
        }
        if case == "changed-setup" {
            input.parallel_tool_calls = !input.parallel_tool_calls;
        }
        let turn = match case {
            "no-turn" => None,
            "new-turn" => Some(2),
            _ => Some(1),
        };
        let AgentTurnBegin::NewTurn { ticket, .. } = begin(&store, &scope, &input, &replay, turn)
        else {
            panic!("{case} must remain an independent classification")
        };
        assert_ne!(ticket.key, first.key, "{case}");
        store.abort(&ticket);
        if case == "pending" {
            store.abort(&first);
        }
    }
}

#[test]
fn duplicate_inflight_native_requests_remain_ambiguous_after_both_fail() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store.scope_key("workspace", "duplicate").unwrap();
    let input = request(vec![text(MessageRole::User, "A")]);
    let (first, _) = new_turn(begin(&store, &scope, &input, &replay, Some(1)));
    let (sibling, _) = new_turn(begin(&store, &scope, &input, &replay, Some(1)));
    assert_ne!(first.key, sibling.key);
    finish_failed(&store, &first, "first");
    finish_failed(&store, &sibling, "sibling");
    let (third, _) = new_turn(begin(&store, &scope, &input, &replay, Some(1)));
    assert_ne!(third.key, first.key);
    assert_ne!(third.key, sibling.key);
    store.abort(&third);
}
