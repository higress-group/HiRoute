use super::*;
use crate::server::core_runtime::model_ir::{ModelEvent, ModelStreamEventV1};

#[test]
fn known_main_capture_gaps_are_scoped_and_never_wait_for_the_store_lock() {
    for contended in [false, true] {
        let (_directory, replay) = replay();
        let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
        let now = Instant::now();
        let first = request(vec![text(MessageRole::User, "first")]);
        let keys = [
            store.scope_key("workspace", "affected").unwrap(),
            store.scope_key("workspace", "independent").unwrap(),
        ];
        for key in &keys {
            let (ticket, _) = new_turn(
                store
                    .begin(key.clone(), plan(1), &first, &replay, now)
                    .unwrap(),
            );
            store.accept_output(
                &ticket,
                &[ModelStreamEventV1::new(
                    0,
                    ModelEvent::TextDelta {
                        index: 0,
                        text: "accepted first answer".into(),
                    },
                )],
            );
            close_turn(&store, &ticket, "native", "model", "profile", "first", now);
        }
        if contended {
            let _lock = store.inner.lock();
            store.note_capture_gap(&keys[0]);
        } else {
            store.note_capture_gap(&keys[0]);
        }
        let same_stripe = keys[0].0[0] == keys[1].0[0];
        for (index, key) in keys.into_iter().enumerate() {
            let mut messages = vec![
                text(MessageRole::User, "first"),
                text(MessageRole::Assistant, "accepted first answer"),
            ];
            if index == 0 {
                messages.push(text(MessageRole::System, "late native relay"));
                messages.push(text(MessageRole::Assistant, "uncaptured relay answer"));
            }
            messages.push(text(MessageRole::User, "fresh user"));
            let next = request(messages);
            let (_, history) = new_turn(store.begin(key, plan(1), &next, &replay, now).unwrap());
            let gap = contended && same_stripe || index == 0;
            assert_eq!(history.history_partial, gap);
            assert_eq!(history.assessment_target.is_none(), gap);
            assert_eq!(history.visible_conversation.len(), 1);
            let visible = serde_json::to_string(&history.visible_conversation).unwrap();
            assert!(visible.contains("accepted first answer"));
            assert!(!visible.contains("uncaptured relay answer"));
        }
    }
}

#[test]
fn first_history_admission_cannot_acknowledge_a_concurrent_uncaptured_main() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let now = Instant::now();
    let key = store
        .scope_key("workspace", "concurrent-first-main")
        .unwrap();
    let first = request(vec![text(MessageRole::User, "first")]);
    let (begin, ()) = store
        .begin_with_admission(
            key.clone(),
            plan(1),
            AgentTurnHistoryStore::prepare_request(&first, &replay).unwrap(),
            TurnDecisionInputs {
                native_turn: None,
                history_continuity: crate::context_hold::HistoryContinuity::Continuing,
                reselect_on_user_message: true,
            },
            now,
            || {
                // The first history entry does not exist yet, but its admission owns
                // the lock. A parallel detached main must survive its initialization.
                store.note_capture_gap(&key);
                Some(())
            },
        )
        .unwrap()
        .unwrap();
    let (ticket, _) = new_turn(begin);
    close_turn(&store, &ticket, "native", "model", "profile", "first", now);
    let next = request(vec![
        text(MessageRole::User, "first"),
        text(MessageRole::System, "native relay"),
        text(MessageRole::Assistant, "uncaptured relay answer"),
        text(MessageRole::User, "fresh user"),
    ]);
    let (_, history) = new_turn(store.begin(key, plan(1), &next, &replay, now).unwrap());
    assert!(history.history_partial);
    assert!(history.assessment_target.is_none());
}
