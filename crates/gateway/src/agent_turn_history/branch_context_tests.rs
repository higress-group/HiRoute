use super::*;
use hiroute_domain::{BranchExecutionPolicyV1, BranchExecutionV1};

#[test]
fn continuation_inherits_actual_primary_without_rewriting_the_opening_decision() {
    for scope in [hiroute_domain::SMART_SAVING_SCOPE_ID, "code"] {
        let (_directory, replay) = replay();
        let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
        let key = store.scope_key("workspace", scope).unwrap();
        let now = Instant::now();
        let user = text(MessageRole::User, "look up the record");
        let (ticket, _) = new_turn(
            store
                .begin(
                    key.clone(),
                    plan(1),
                    &request(vec![user.clone()]),
                    &replay,
                    now,
                )
                .unwrap(),
        );
        let policy = hiroute_domain::JudgmentSettingsV1::default().execution_policy(scope);
        let mut selected = decision(scope);
        selected.policy = Some(policy.clone());
        selected.execution_group = hiroute_domain::ExecutionGroupV1::Regular;
        selected.selection_reason = hiroute_domain::ModelGroupReasonV1::SimpleTask;
        store.commit_decision(&ticket, selected.clone()).unwrap();
        let mut accepted = execution("primary", "profile", scope, "request");
        accepted.branch_execution = Some(BranchExecutionV1 {
            policy,
            group: hiroute_domain::ExecutionGroupV1::Primary,
            candidate_index: 0,
        });
        store
            .finish_request(
                &ticket,
                "request".into(),
                AgentTurnStatus::Completed,
                vec![accepted],
                false,
                now,
            )
            .unwrap();
        let continued = request(vec![
            user,
            tool_call("lookup-1", "lookup"),
            tool_result("lookup-1", false),
        ]);
        let AgentTurnBegin::Continuation { decision, .. } = store
            .begin(key.clone(), plan(1), &continued, &replay, now)
            .unwrap()
        else {
            panic!("tool result must continue the current turn")
        };
        assert_eq!(
            decision.execution_group,
            hiroute_domain::ExecutionGroupV1::Primary
        );
        assert_eq!(
            decision.selection_reason,
            hiroute_domain::ModelGroupReasonV1::AvailabilityRelay
        );
        assert_eq!(
            store.inner.lock().entries[&key].active.decision.as_ref(),
            Some(&selected)
        );
    }
}

fn close_branch_turn(
    store: &AgentTurnHistoryStore,
    ticket: &AgentTurnTicket,
    now: Instant,
) -> CompletedAgentTurn {
    let policy = BranchExecutionPolicyV1 {
        name: "Code".into(),
        floor_millis: 500,
        criteria_digest: None,
    };
    let mut selected = decision("code");
    selected.policy = Some(policy.clone());
    store.commit_decision(ticket, selected).unwrap();
    let mut executed = execution("same-model", "same-profile", "code", "request");
    executed.branch_execution = Some(BranchExecutionV1 {
        policy,
        group: hiroute_domain::ExecutionGroupV1::Regular,
        candidate_index: 0,
    });
    store
        .finish_request(
            ticket,
            "request".into(),
            AgentTurnStatus::Completed,
            vec![executed],
            true,
            now,
        )
        .unwrap()
        .unwrap()
}

#[test]
fn branch_context_reset_starts_a_new_competence_segment_on_the_same_model() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let key = store.scope_key("workspace", "branch-reset").unwrap();
    let now = Instant::now();
    let first = request(vec![text(MessageRole::User, "old task")]);
    let (ticket, _) = new_turn(
        store
            .begin(key.clone(), plan(1), &first, &replay, now)
            .unwrap(),
    );
    let old = close_branch_turn(&store, &ticket, now);
    let new_user = text(MessageRole::User, "new task");
    let (ticket, history) = new_turn(
        store
            .begin_with_context(
                key.clone(),
                plan(1),
                &request(vec![new_user.clone()]),
                &replay,
                TurnDecisionInputs {
                    native_turn: None,
                    history_continuity: crate::context_hold::HistoryContinuity::Rewritten,
                    reselect_on_user_message: true,
                },
                now,
            )
            .unwrap(),
    );
    assert!(history.assessment_target.is_none());
    assert!(history.assessment_from.is_none());
    assert!(history.previous_decision.is_none());
    drop(history);
    let current = close_branch_turn(&store, &ticket, now);
    assert_ne!(
        current.segment_id, old.segment_id,
        "the same model cannot merge two context scopes"
    );
    let (_, history) = new_turn(
        store
            .begin(
                key,
                plan(1),
                &request(vec![
                    new_user,
                    text(MessageRole::Assistant, "done"),
                    text(MessageRole::User, "please improve this"),
                ]),
                &replay,
                now,
            )
            .unwrap(),
    );
    let target = history.assessment_target.as_ref().unwrap();
    assert_eq!(target.segment_id, current.segment_id);
    assert_eq!(target.first_ordinal, 2);
    assert_eq!(history.assessment_from, Some(1));
}

#[test]
fn changed_plan_never_grades_an_old_stage_using_the_new_rubric() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let key = store
        .scope_key("workspace", "branch-plan-revision")
        .unwrap();
    let now = Instant::now();
    let user = text(MessageRole::User, "first");
    let (ticket, _) = new_turn(
        store
            .begin(
                key.clone(),
                plan(1),
                &request(vec![user.clone()]),
                &replay,
                now,
            )
            .unwrap(),
    );
    close_branch_turn(&store, &ticket, now);
    let (_, history) = new_turn(
        store
            .begin(
                key,
                plan(2),
                &request(vec![user, text(MessageRole::User, "next")]),
                &replay,
                now,
            )
            .unwrap(),
    );
    assert!(history.assessment_target.is_none());
    assert!(history.assessment_from.is_none());
    assert_eq!(
        history.visible_conversation.len(),
        1,
        "visible context remains available for classification"
    );
}

#[test]
fn uncaptured_prefix_does_not_make_a_fully_captured_stage_partial() {
    for missed_current_output in [false, true] {
        let (_directory, replay) = replay();
        let store = AgentTurnHistoryStore::default();
        let key = store.scope_key("workspace", "late-history-join").unwrap();
        let now = Instant::now();
        let mut messages = vec![
            text(MessageRole::User, "earlier context supplied by the client"),
            text(MessageRole::User, "current task"),
        ];
        if missed_current_output {
            messages.push(text(MessageRole::Assistant, "output before HiRoute joined"));
        }
        let (ticket, history) = new_turn(
            store
                .begin(
                    key.clone(),
                    plan(1),
                    &request(messages.clone()),
                    &replay,
                    now,
                )
                .unwrap(),
        );
        assert!(
            history.history_partial,
            "the unseen prefix remains explicit"
        );
        drop(history);
        accepted_output::text_output(&store, &ticket, "captured answer");
        let completed = close_branch_turn(&store, &ticket, now);
        assert_eq!(completed.history_partial, missed_current_output);
        messages.push(text(MessageRole::Assistant, "captured answer"));
        messages.push(text(MessageRole::User, "please assess the current task"));
        let (_, history) = new_turn(
            store
                .begin(key, plan(1), &request(messages), &replay, now)
                .unwrap(),
        );
        assert!(history.history_partial);
        assert_eq!(history.assessment_from, Some(0));
        assert_eq!(
            history.assessment_target.as_ref().unwrap().target_partial,
            missed_current_output,
            "only evidence missing inside the scored stage can invalidate its completeness"
        );
    }
}

#[test]
fn observed_rewrites_assess_captured_execution_without_merging_segments_or_reopening_gaps() {
    use crate::context_hold::HistoryContinuity::{Rewritten, Shortened};
    for continuity in [Rewritten, Shortened] {
        for (gap, revision) in [(false, 1), (true, 1), (false, 2)] {
            let (_directory, replay) = replay();
            let store = AgentTurnHistoryStore::default();
            let key = store.scope_key("workspace", "observed-rewrite").unwrap();
            let now = Instant::now();
            let observed = |user: &str, revision| {
                let mut analyzed = AgentTurnHistoryStore::prepare_request(
                    &request(vec![text(MessageRole::User, user)]),
                    &replay,
                )
                .unwrap();
                analyzed.mark_user_origin_unverified();
                new_turn(
                    store
                        .begin_with_admission(
                            key.clone(),
                            plan(revision),
                            analyzed,
                            TurnDecisionInputs {
                                native_turn: None,
                                history_continuity: continuity,
                                reselect_on_user_message: true,
                            },
                            now,
                            || Some(()),
                        )
                        .unwrap()
                        .unwrap()
                        .0,
                )
            };
            let (ticket, history) = observed("actual A", 1);
            drop(history);
            accepted_output::text_output(&store, &ticket, "accepted A output");
            let a = close_branch_turn(&store, &ticket, now);
            if gap {
                store.note_capture_gap(&key);
            }
            let (ticket, history) = observed("actual B after a rewritten prefix", revision);
            assert!(
                history.previous_decision.is_none(),
                "rewrite must reclassify"
            );
            let expected = !gap && revision == 1;
            assert_eq!(history.assessment_target.is_some(), expected);
            if expected {
                let target = history.assessment_target.as_ref().unwrap();
                assert_eq!(target.segment_id, a.segment_id);
                assert_eq!((target.first_ordinal, target.through_ordinal), (1, 1));
                assert!(!target.target_partial);
                let visible = serde_json::to_string(&history.visible_conversation).unwrap();
                assert!(visible.contains("accepted A output"));
                assert!(visible.contains("unverified_request_user_origin"));
            }
            drop(history);
            accepted_output::text_output(&store, &ticket, "accepted B output");
            let b = close_branch_turn(&store, &ticket, now);
            assert_ne!(a.segment_id, b.segment_id, "rewrites cannot merge stages");
            let (_, history) = observed("actual C", revision);
            assert_eq!(
                history.assessment_target.as_ref().unwrap().segment_id,
                b.segment_id,
                "a complete new stage must recover after a gap or publication change"
            );
        }
    }
}
