//! Archive input identity must not depend on successful response association.
use super::*;

fn admit(
    store: &AgentTurnHistoryStore,
    scope: &AgentTurnHistoryKey,
    input: &ModelRequestIRV1,
    replay: &ReplayStore,
) -> AgentTurnBegin {
    let analyzed = store.prepare_scoped_request(scope, input, replay).unwrap();
    store
        .begin_with_admission(
            scope.clone(),
            plan(1),
            analyzed,
            TurnDecisionInputs {
                native_turn: Some([1; 32]),
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

#[test]
fn exact_carried_input_retains_archive_source_across_response_association_loss() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store.scope_key("workspace", "archive-source").unwrap();
    let mut messages = vec![text(MessageRole::User, "Original input")];
    let (first, history) = new_turn(admit(&store, &scope, &request(messages.clone()), &replay));
    drop(history);
    accepted_output::text_output(&store, &first, "Accepted output");
    close_turn(
        &store,
        &first,
        "regular",
        "model",
        "profile",
        "first",
        Instant::now(),
    );
    for index in 0..3 {
        // Deliberately different from accepted output: no execution reuse is legal.
        messages.push(text(
            MessageRole::Assistant,
            format!("Unmatched output {index}"),
        ));
        let (next, history) = new_turn(admit(&store, &scope, &request(messages.clone()), &replay));
        assert_ne!(next.agent_turn_id, first.agent_turn_id);
        assert_eq!(next.capture_input_start, None);
        assert_eq!(next.capture_turn_anchor, first.capture_turn_anchor);
        assert!(history.previous_decision.is_none());
        drop(history);
        accepted_output::text_output(&store, &next, "Accepted output");
        close_turn(
            &store,
            &next,
            "regular",
            "model",
            "profile",
            &format!("next-{index}"),
            Instant::now(),
        );
    }
}

#[test]
fn rewritten_shortened_and_new_inputs_never_alias_the_old_source() {
    for case in [
        "rewrite",
        "shorten",
        "new_input",
        "unavailable",
        "window_changed",
    ] {
        let (_directory, replay) = replay();
        let store = AgentTurnHistoryStore::default();
        let scope = store.scope_key("workspace", case).unwrap();
        let original = vec![
            text(MessageRole::User, "Context"),
            text(MessageRole::User, "Input"),
        ];
        let (first, history) = new_turn(admit(&store, &scope, &request(original.clone()), &replay));
        drop(history);
        accepted_output::text_output(&store, &first, "Accepted");
        close_turn(
            &store,
            &first,
            "regular",
            "model",
            "profile",
            "a",
            Instant::now(),
        );
        let mut messages = original;
        match case {
            "rewrite" => {
                messages[0] = text(MessageRole::User, "Rewritten context");
                messages.push(text(MessageRole::Assistant, "Unmatched output"));
            }
            "shorten" => {
                messages.remove(0);
            }
            "new_input" => messages.extend([
                text(MessageRole::Assistant, "Accepted"),
                text(MessageRole::User, "Input"),
            ]),
            "window_changed" | "unavailable" => {
                messages.push(text(MessageRole::Assistant, "Unmatched output"));
            }
            _ => {}
        }
        let input = request(messages);
        let mut analyzed = store
            .prepare_scoped_request(&scope, &input, &replay)
            .unwrap();
        if case == "unavailable" {
            analyzed.anchors = None;
        } else if case == "window_changed" {
            analyzed
                .select_current_input(
                    &input.messages[1].content,
                    Some(crate::agent_turn_history::CurrentInputPosition::Window {
                        first: 0,
                        last: 1,
                    }),
                    &input,
                    &replay,
                )
                .unwrap();
        }
        let begin = store
            .begin_with_admission(
                scope,
                plan(1),
                analyzed,
                TurnDecisionInputs {
                    native_turn: Some([1; 32]),
                    history_continuity: crate::context_hold::HistoryContinuity::Continuing,
                    reselect_on_user_message: true,
                },
                Instant::now(),
                || Some(()),
            )
            .unwrap()
            .unwrap()
            .0;
        let (next, history) = new_turn(begin);
        assert_ne!(
            next.capture_turn_anchor, first.capture_turn_anchor,
            "{case}"
        );
        drop(history);
        store.abort(&next);
    }
}

#[test]
fn ambiguous_prefix_sources_never_choose_an_arbitrary_input_occurrence() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store.scope_key("workspace", "ambiguous-source").unwrap();
    let user = text(MessageRole::User, "Identical concurrent input");
    let input = request(vec![user.clone()]);
    let (first, first_history) = new_turn(admit(&store, &scope, &input, &replay));
    let (second, second_history) = new_turn(admit(&store, &scope, &input, &replay));
    assert_ne!(first.capture_turn_anchor, second.capture_turn_anchor);
    drop((first_history, second_history));
    for (ticket, id) in [(&first, "a"), (&second, "b")] {
        accepted_output::text_output(&store, ticket, "Accepted output");
        close_turn(
            &store,
            ticket,
            "regular",
            "model",
            "profile",
            id,
            Instant::now(),
        );
    }
    let input = request(vec![user, text(MessageRole::Assistant, "Unmatched output")]);
    let (next, history) = new_turn(admit(&store, &scope, &input, &replay));
    assert_ne!(next.capture_turn_anchor, first.capture_turn_anchor);
    assert_ne!(next.capture_turn_anchor, second.capture_turn_anchor);
    assert_eq!(next.capture_turn_anchor, next.agent_turn_id);
    drop(history);
    store.abort(&next);
}
