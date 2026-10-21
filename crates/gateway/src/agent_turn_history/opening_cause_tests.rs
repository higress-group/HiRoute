use super::*;
use crate::context_hold::HistoryContinuity;
use hiroute_domain::AgentTurnOpeningCauseV1 as Cause;

#[test]
fn user_message_cause_does_not_depend_on_decision_inheritance() {
    let (_directory, replay) = replay();
    let now = Instant::now();
    for (has_decision, revision, append_user, expected) in [
        (true, 2, true, Cause::UserMessage),
        (false, 1, true, Cause::UserMessage),
        (false, 2, true, Cause::UserMessage),
        (true, 2, false, Cause::Other),
        (false, 1, false, Cause::Other),
    ] {
        let store = AgentTurnHistoryStore::default();
        let key = store.scope_key("workspace", "opening").unwrap();
        let mut messages = vec![text(MessageRole::User, "first")];
        let (first, history) = new_turn(
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
        close_turn(&store, &first, "smart_saving_simple", "m", "p", "r1", now);
        if !has_decision {
            store
                .inner
                .lock()
                .entries
                .get_mut(&key)
                .unwrap()
                .active
                .decision = None;
        }
        if append_user {
            messages.push(text(MessageRole::User, "follow-up"));
        }
        let begin = store
            .begin_with_context(
                key,
                plan(revision),
                &request(messages),
                &replay,
                TurnDecisionInputs {
                    native_turn: None,
                    history_continuity: HistoryContinuity::Continuing,
                    reselect_on_user_message: false,
                },
                now,
            )
            .unwrap();
        assert!(
            matches!(
                &begin,
                AgentTurnBegin::NewTurn {
                    inherited_decision: None,
                    ..
                }
            ),
            "a changed Plan or absent decision must still trigger fresh judgment",
        );
        let (ticket, history) = new_turn(begin);
        drop(history);
        let completed = close_turn(&store, &ticket, "smart_saving_simple", "m", "p", "r2", now);
        assert_eq!(completed.opening_cause, expected);
    }
}
