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
        "same_activation",
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
        let native = if case == "same_activation" { 1 } else { 2 };
        let (b, _) = new_turn(admit(&store, &scope, analyzed, Some([native; 32])));
        assert_ne!(a.key, b.key, "{case}");
        let expected = if case == "changed" {
            Cause::HistoryRewrite
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
