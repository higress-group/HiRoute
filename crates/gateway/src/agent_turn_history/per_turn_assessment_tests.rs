//! A user turn is the scoring unit, independently of executor continuity.
use super::*;

#[test]
fn same_executor_scores_each_turn_and_cancellation_cannot_poison_other_turns() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let key = store.scope_key("workspace", "independent-scores").unwrap();
    let mut messages = Vec::new();
    let mut completed = Vec::<CompletedAgentTurn>::new();
    let mut frozen = Vec::new();
    for ordinal in 1..=5 {
        messages.push(text(MessageRole::User, format!("work {ordinal}")));
        let (ticket, history) = new_turn(
            store
                .begin(
                    key.clone(),
                    plan(1),
                    &request(messages.clone()),
                    &replay,
                    Instant::now(),
                )
                .unwrap(),
        );
        if ordinal > 1 {
            let target = history.assessment_target.as_ref().unwrap();
            assert_eq!(target.first_turn_id, target.through_turn_id);
            assert_eq!(target.first_ordinal, target.through_ordinal);
            assert_eq!(history.assessment_from, Some(ordinal - 2));
            assert_eq!(target.first_ordinal, (ordinal - 1) as u64);
            assert_eq!(target.first_turn_id, completed[ordinal - 2].agent_turn_id);
            assert_eq!(target.target_partial, ordinal == 4);
            assert_eq!(
                target.history_membership.as_ref().unwrap().request_ids,
                vec![format!("request-{}", ordinal - 1)]
            );
            frozen.push(target.clone());
        }
        drop(history);
        if ordinal == 5 {
            store.abort(&ticket);
            break;
        }
        let answer = format!("answer {ordinal}");
        let request_id = format!("request-{ordinal}");
        store.commit_decision(&ticket, decision("code")).unwrap();
        accepted_output::text_output(&store, &ticket, &answer);
        if ordinal == 3 {
            store.mark_output_partial(&ticket);
        }
        let turn = store
            .finish_request(
                &ticket,
                request_id.clone(),
                if ordinal == 3 {
                    AgentTurnStatus::Interrupted
                } else {
                    AgentTurnStatus::Completed
                },
                vec![execution("same-model", "same-profile", "code", &request_id)],
                true,
                Instant::now(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(turn.history_partial, ordinal == 3);
        assert!(
            completed
                .iter()
                .all(|previous| previous.segment_id != turn.segment_id)
        );
        completed.push(turn);
        messages.push(text(MessageRole::Assistant, &answer));
    }
    assert_eq!(frozen.len(), 4);
    assert!(!frozen[0].target_partial);
    assert!(!frozen[1].target_partial);
    assert!(frozen[2].target_partial);
    assert!(!frozen[3].target_partial);
}

#[test]
fn an_unattributed_latest_turn_cannot_reuse_an_older_assessment_target() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let key = store.scope_key("workspace", "no-stale-score").unwrap();
    let mut messages = vec![text(MessageRole::User, "first")];
    let (first, _) = new_turn(
        store
            .begin(
                key.clone(),
                plan(1),
                &request(messages.clone()),
                &replay,
                Instant::now(),
            )
            .unwrap(),
    );
    close_turn(
        &store,
        &first,
        "code",
        "model",
        "profile",
        "r1",
        Instant::now(),
    );
    {
        let mut inner = store.inner.lock();
        let entry = inner.entries.get_mut(&key).unwrap();
        entry.referenced_segment = entry.current_segment.clone();
    }
    messages.extend([
        text(MessageRole::Assistant, "first answer"),
        text(MessageRole::User, "second"),
    ]);
    let (second, history) = new_turn(
        store
            .begin(
                key.clone(),
                plan(1),
                &request(messages.clone()),
                &replay,
                Instant::now(),
            )
            .unwrap(),
    );
    assert!(history.assessment_target.is_some());
    drop(history);
    store.commit_decision(&second, decision("code")).unwrap();
    store
        .finish_request(
            &second,
            "r2".into(),
            AgentTurnStatus::Completed,
            vec![],
            true,
            Instant::now(),
        )
        .unwrap();
    messages.extend([
        text(MessageRole::Assistant, "unattributed"),
        text(MessageRole::User, "third"),
    ]);
    let (third, history) = new_turn(
        store
            .begin(key, plan(1), &request(messages), &replay, Instant::now())
            .unwrap(),
    );
    assert!(history.assessment_target.is_none());
    assert!(history.assessment_from.is_none());
    store.abort(&third);
}
