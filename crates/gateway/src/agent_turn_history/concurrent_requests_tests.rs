use super::*;

fn scoped(
    store: &AgentTurnHistoryStore,
    key: &AgentTurnHistoryKey,
    input: &ModelRequestIRV1,
    replay: &ReplayStore,
) -> Result<AgentTurnBegin, AgentTurnHistoryError> {
    store
        .begin_with_admission(
            key.clone(),
            plan(1),
            store.prepare_scoped_request(key, input, replay)?,
            TurnDecisionInputs {
                native_turn: None,
                history_continuity: crate::context_hold::HistoryContinuity::Continuing,
                reselect_on_user_message: true,
            },
            Instant::now(),
            || Some(()),
        )
        .map(|result| result.unwrap().0)
}

#[test]
fn overlapping_same_scope_requests_capture_distinct_models_and_immutable_targets() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let key = store.scope_key("workspace", "parallel").unwrap();
    let a = request(vec![text(MessageRole::User, "A")]);
    let (a_ticket, _) = new_turn(scoped(&store, &key, &a, &replay).unwrap());
    close_turn(
        &store,
        &a_ticket,
        "regular",
        "model-a",
        "profile-a",
        "request-a",
        Instant::now(),
    );
    let slow = request(vec![
        text(MessageRole::User, "A"),
        text(MessageRole::Assistant, "answer A"),
        text(MessageRole::User, "slow C"),
    ]);
    let (slow_ticket, slow_snapshot) = new_turn(scoped(&store, &key, &slow, &replay).unwrap());
    let fast = request(vec![
        text(MessageRole::User, "A"),
        text(MessageRole::Assistant, "answer A"),
        text(MessageRole::User, "fast B"),
    ]);
    let (fast_ticket, fast_snapshot) = new_turn(scoped(&store, &key, &fast, &replay).unwrap());
    assert_ne!(fast_ticket.key, slow_ticket.key);
    assert_ne!(fast_ticket.request_token, slow_ticket.request_token);
    let target = fast_snapshot.assessment_target.as_ref().unwrap();
    assert_eq!(target.through_turn_id, a_ticket.agent_turn_id);
    close_turn(
        &store,
        &slow_ticket,
        "regular",
        "model-c",
        "profile-c",
        "request-c",
        Instant::now(),
    );
    assert_eq!(
        fast_snapshot
            .assessment_target
            .as_ref()
            .unwrap()
            .through_turn_id,
        a_ticket.agent_turn_id
    );
    let fast_completed = close_turn(
        &store,
        &fast_ticket,
        "primary",
        "model-b",
        "profile-b",
        "request-b",
        Instant::now(),
    );
    assert!(
        matches!(fast_completed.attribution, ExecutionAttribution::Single { model_configuration_id, .. } if model_configuration_id == "model-b")
    );
    assert_eq!(slow_snapshot.visible_conversation.len(), 1);
    assert_eq!(fast_snapshot.visible_conversation.len(), 1);
}

#[test]
fn live_scope_limit_is_optional_and_recovers_without_a_new_session() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let key = store.scope_key("workspace", "capacity").unwrap();
    let input = request(vec![text(
        MessageRole::User,
        "identical input is not deduplication",
    )]);
    let mut tickets = Vec::new();
    for _ in 0..32 {
        let (ticket, snapshot) = new_turn(scoped(&store, &key, &input, &replay).unwrap());
        drop(snapshot);
        tickets.push(ticket);
    }
    assert!(matches!(
        scoped(&store, &key, &input, &replay),
        Err(AgentTurnHistoryError::Resource)
    ));
    store.abort(&tickets.pop().unwrap());
    let (recovered, _) = new_turn(scoped(&store, &key, &input, &replay).unwrap());
    assert!(
        tickets
            .iter()
            .all(|ticket| ticket.request_token != recovered.request_token)
    );
    store.abort(&recovered);
    for ticket in tickets {
        store.abort(&ticket);
    }
}
