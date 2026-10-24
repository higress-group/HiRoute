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
        extension: None,
        floor_millis: Some(500),
        criteria_digest: None,
    };
    let mut selected = decision("code");
    selected.policy = Some(policy.clone());
    if ticket.new_turn {
        store.commit_decision(ticket, selected).unwrap();
    }
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
    assert!(
        history
            .assessment_target
            .as_ref()
            .unwrap()
            .history_membership
            .as_ref()
            .unwrap()
            .association_unverified
    );
    assert_eq!(history.assessment_from, Some(0));
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
fn complete_new_input_recovers_assessment_after_a_partial_stage_on_the_same_model() {
    for current_output_missing in [false, true] {
        let (_directory, replay) = replay();
        let store = AgentTurnHistoryStore::default();
        let key = store
            .scope_key("workspace", "recover-partial-stage")
            .unwrap();
        let now = Instant::now();
        let mut messages = vec![
            text(MessageRole::User, "task begun before Gateway restart"),
            text(MessageRole::Assistant, "uncaptured execution"),
        ];
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
        drop(history);
        accepted_output::text_output(&store, &ticket, "captured suffix");
        let partial = close_branch_turn(&store, &ticket, now);
        assert!(partial.history_partial);
        messages.push(text(MessageRole::Assistant, "captured suffix"));
        messages.push(text(MessageRole::User, "fresh A"));
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
        let old_target = history.assessment_target.as_ref().unwrap();
        assert_eq!(old_target.segment_id, partial.segment_id);
        assert!(
            old_target.target_partial,
            "A cannot repair missing old execution"
        );
        drop(history);
        if current_output_missing {
            store.mark_output_partial(&ticket);
        }
        accepted_output::text_output(&store, &ticket, "actual A execution");
        let a = close_branch_turn(&store, &ticket, now);
        assert_eq!(a.history_partial, current_output_missing);
        assert_eq!(a.segment_id == partial.segment_id, current_output_missing);
        messages.push(text(MessageRole::Assistant, "actual A execution"));
        messages.push(text(MessageRole::User, "fresh B"));
        let (_, history) = new_turn(
            store
                .begin(key, plan(1), &request(messages), &replay, now)
                .unwrap(),
        );
        assert!(
            history.history_partial,
            "global missing history remains disclosed"
        );
        let target = history.assessment_target.as_ref().unwrap();
        assert_eq!(target.segment_id, a.segment_id);
        assert_eq!(target.target_partial, current_output_missing);
        assert_eq!(target.through_ordinal, 2);
        assert_eq!(
            target.first_ordinal,
            if current_output_missing { 1 } else { 2 }
        );
        assert_eq!(
            history.assessment_from,
            Some(if current_output_missing { 0 } else { 1 })
        );
    }
}

#[test]
fn retrying_or_reclassifying_a_turn_cannot_launder_a_partial_segment() {
    use hiroute_domain::AgentTurnOpeningCauseV1 as Cause;
    for new_execution_id in [false, true] {
        let (_directory, replay) = replay();
        let store = AgentTurnHistoryStore::default();
        let key = store.scope_key("workspace", "partial-retry").unwrap();
        let now = Instant::now();
        let user = text(MessageRole::User, "same task");
        let earlier = text(MessageRole::User, "earlier task on the same model");
        let (ticket, history) = new_turn(
            store
                .begin(
                    key.clone(),
                    plan(1),
                    &request(vec![earlier.clone()]),
                    &replay,
                    now,
                )
                .unwrap(),
        );
        drop(history);
        close_branch_turn(&store, &ticket, now);
        let messages = vec![
            earlier,
            text(MessageRole::Assistant, "earlier output"),
            user,
        ];
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
        drop(history);
        accepted_output::text_output(&store, &ticket, "accepted execution");
        let original = close_branch_turn(&store, &ticket, now);
        // Evict actual earlier execution without losing the current turn's
        // output. A fully captured retry must not wash that missing prefix away.
        assert!(trim_oldest_closed_turn(&mut store.inner.lock(), &key).unwrap());
        let AgentTurnBegin::Continuation { ticket, .. } = store
            .begin(key.clone(), plan(1), &request(messages), &replay, now)
            .unwrap()
        else {
            panic!("same input must resume the existing turn")
        };
        let ticket = if new_execution_id {
            // A reclassification can allocate an execution ID while retaining
            // the input boundary. The ID is not fresh-user evidence.
            let mut inner = store.inner.lock();
            let entry = inner.entries.get_mut(&key).unwrap();
            entry.active.agent_turn_id = "reclassified-same-input".into();
            entry.active.opening_cause = Cause::Other;
            AgentTurnTicket {
                agent_turn_id: entry.active.agent_turn_id.clone(),
                ..ticket
            }
        } else {
            ticket
        };
        accepted_output::text_output(&store, &ticket, "retry output");
        let retry = close_branch_turn(&store, &ticket, now);
        assert!(retry.history_partial);
        assert_eq!(retry.segment_id, original.segment_id);
        assert_eq!(
            retry.agent_turn_id != original.agent_turn_id,
            new_execution_id
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
