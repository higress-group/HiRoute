use super::*;
use crate::context_hold::{NativeCheckpoint, NativeUserOrigin};

fn native_hold(
    fixture: &Fixture,
    request: &model_ir::ModelRequestIRV1,
    turn: u8,
    checkpoint: Option<u8>,
    user_origin: NativeUserOrigin,
) -> PreparedHold {
    let mut hold = fixture.prepare(request);
    hold.set_native(NativeCheckpoint {
        turn: Some([turn; 32]),
        checkpoint: checkpoint.map(|value| [value; 32]),
        requires_checkpoint: true,
        user_origin,
        ..Default::default()
    });
    hold
}

fn assert_detached(history: &DecisionHistory) {
    assert!(history.guard.is_none());
    assert!(history.hold_ticket.is_none());
    assert!(history.correlated.is_none());
    assert!(history.history.history_partial);
    assert!(history.history.visible_conversation.is_empty());
    assert!(history.history.assessment_target.is_none());
}

fn visible_users(history: &DecisionHistory) -> Vec<Vec<VisibleContentPart>> {
    history
        .history
        .visible_conversation
        .iter()
        .map(|turn| turn.user.clone())
        .collect()
}

fn user(text: &str) -> Vec<VisibleContentPart> {
    vec![VisibleContentPart::Text { text: text.into() }]
}

#[test]
fn native_same_turn_appended_synthetic_user_keeps_the_original_user_and_ordinal() {
    let fixture = Fixture::new();
    let first = Fixture::request(json!([{"role":"user","content":"original task"}]));
    let mut admitted = fixture.begin(
        &first,
        native_hold(&fixture, &first, 1, None, NativeUserOrigin::CurrentUser),
    );
    let original_turn = admitted
        .guard
        .as_ref()
        .unwrap()
        .ticket()
        .agent_turn_id
        .clone();
    fixture.finish(&mut admitted, &[text_event("accepted prefix")], true);
    drop(admitted);

    let continued = Fixture::request(json!([
        {"role":"user","content":"original task"},
        {"role":"assistant","content":"accepted prefix"},
        {"role":"user","content":"synthetic continue"}
    ]));
    let hold = native_hold(
        &fixture,
        &continued,
        1,
        Some(7),
        NativeUserOrigin::ExistingTurn,
    );
    assert_eq!(hold.history_continuity(), HistoryContinuity::NativeSameTurn);
    assert!(hold.history_is_proven());
    assert!(hold.native_user_unavailable());
    let mut continuation = fixture.begin(&continued, hold);
    let ticket = continuation.guard.as_ref().unwrap().ticket();
    assert!(!ticket.new_turn);
    assert_eq!(ticket.agent_turn_id, original_turn);
    assert!(continuation.correlated.is_some());
    fixture.finish(&mut continuation, &[text_event("accepted final")], true);
    drop(continuation);

    let next = Fixture::request(json!([
        {"role":"user","content":"original task"},
        {"role":"assistant","content":"accepted prefix"},
        {"role":"user","content":"synthetic continue"},
        {"role":"assistant","content":"accepted final"},
        {"role":"user","content":"next real task"}
    ]));
    let mut admitted = fixture.begin(
        &next,
        native_hold(&fixture, &next, 2, Some(7), NativeUserOrigin::CurrentUser),
    );
    assert!(admitted.guard.as_ref().unwrap().ticket().new_turn);
    assert_eq!(visible_users(&admitted), vec![user("original task")]);
    let target = admitted.history.assessment_target.as_ref().unwrap();
    assert_eq!(target.first_ordinal, 1);
    assert_eq!(target.through_ordinal, 1);
    let visible = serde_json::to_string(&admitted.history.visible_conversation).unwrap();
    assert!(visible.contains("accepted prefix") && visible.contains("accepted final"));
    assert!(!visible.contains("synthetic continue"));
    fixture.finish(&mut admitted, &[text_event("next answer")], true);
    drop(admitted);

    let third = Fixture::request(json!([
        {"role":"user","content":"original task"},
        {"role":"assistant","content":"accepted prefix"},
        {"role":"user","content":"synthetic continue"},
        {"role":"assistant","content":"accepted final"},
        {"role":"user","content":"next real task"},
        {"role":"assistant","content":"next answer"},
        {"role":"user","content":"third real task"}
    ]));
    let admitted = fixture.begin(
        &third,
        native_hold(&fixture, &third, 3, Some(7), NativeUserOrigin::CurrentUser),
    );
    assert_eq!(
        visible_users(&admitted),
        vec![user("original task"), user("next real task")]
    );
    assert_eq!(
        admitted
            .history
            .assessment_target
            .as_ref()
            .unwrap()
            .through_ordinal,
        2
    );
}

#[test]
fn native_compaction_rewrite_keeps_saved_user_without_applying_unproven_tool_results() {
    let fixture = Fixture::new();
    let first = Fixture::request(json!([{"role":"user","content":"original tool task"}]));
    let mut admitted = fixture.begin(
        &first,
        native_hold(&fixture, &first, 1, None, NativeUserOrigin::CurrentUser),
    );
    let original_turn = admitted
        .guard
        .as_ref()
        .unwrap()
        .ticket()
        .agent_turn_id
        .clone();
    fixture.finish(&mut admitted, &[tool_event()], false);
    drop(admitted);

    let rewritten = Fixture::request(json!([
        {"role":"user","content":"synthetic summary"},
        {"type":"function_call","call_id":"tool-1","name":"read","arguments":"{}"},
        {"type":"function_call_output","call_id":"tool-1","output":"unproven rewritten receipt"}
    ]));
    let hold = native_hold(
        &fixture,
        &rewritten,
        1,
        Some(7),
        NativeUserOrigin::ExistingTurn,
    );
    assert_eq!(
        hold.history_continuity(),
        HistoryContinuity::NativeContinuation
    );
    assert!(hold.history_is_proven());
    assert!(hold.native_user_unavailable());
    let mut admitted = fixture.begin(&rewritten, hold);
    assert_eq!(
        admitted.guard.as_ref().unwrap().ticket().agent_turn_id,
        original_turn
    );
    assert!(!admitted.guard.as_ref().unwrap().ticket().new_turn);
    assert!(admitted.correlated.is_some());
    fixture.finish(&mut admitted, &[text_event("accepted completion")], true);
    drop(admitted);

    let next = Fixture::request(json!([
        {"role":"user","content":"synthetic summary"},
        {"type":"function_call","call_id":"tool-1","name":"read","arguments":"{}"},
        {"type":"function_call_output","call_id":"tool-1","output":"unproven rewritten receipt"},
        {"role":"assistant","content":"accepted completion"},
        {"role":"user","content":"next real task"}
    ]));
    let admitted = fixture.begin(
        &next,
        native_hold(&fixture, &next, 2, Some(7), NativeUserOrigin::CurrentUser),
    );
    assert_eq!(visible_users(&admitted), vec![user("original tool task")]);
    let parts = admitted
        .history
        .visible_conversation
        .iter()
        .flat_map(|turn| &turn.steps)
        .flatten()
        .collect::<Vec<_>>();
    assert!(parts.iter().any(|part| matches!(
        part,
        VisibleContentPart::ToolActivity {
            status: ToolStatus::Unknown,
            ..
        }
    )));
    assert!(!parts.iter().any(|part| matches!(
        part,
        VisibleContentPart::ToolActivity {
            status: ToolStatus::Completed,
            ..
        }
    )));
    let visible = serde_json::to_string(&admitted.history.visible_conversation).unwrap();
    assert!(!visible.contains("synthetic summary"));
    assert!(!visible.contains("unproven rewritten receipt"));
    assert!(visible.contains("accepted completion"));
}

#[test]
fn cold_native_existing_turn_cannot_capture_a_summary_or_publish_a_checkpoint() {
    let fixture = Fixture::new();
    let synthetic = Fixture::request(json!([{"role":"user","content":"cold synthetic summary"}]));
    let hold = native_hold(
        &fixture,
        &synthetic,
        1,
        Some(7),
        NativeUserOrigin::ExistingTurn,
    );
    assert_eq!(hold.history_continuity(), HistoryContinuity::NoCheckpoint);
    assert!(!hold.history_is_proven());
    assert!(hold.native_user_unavailable());
    let rejected = fixture.begin(&synthetic, hold);
    assert_detached(&rejected);
    drop(rejected);
    assert_eq!(
        fixture.prepare(&synthetic).history_continuity(),
        HistoryContinuity::NoCheckpoint
    );

    let current = Fixture::request(json!([
        {"role":"user","content":"cold synthetic summary"},
        {"role":"assistant","content":"uncaptured continuation"},
        {"role":"user","content":"first real captured task"}
    ]));
    let hold = native_hold(
        &fixture,
        &current,
        2,
        Some(7),
        NativeUserOrigin::CurrentUser,
    );
    assert!(hold.history_is_proven());
    assert!(!hold.native_user_unavailable());
    let mut admitted = fixture.begin(&current, hold);
    assert!(admitted.guard.as_ref().unwrap().ticket().new_turn);
    assert!(admitted.history.visible_conversation.is_empty());
    assert!(admitted.history.assessment_target.is_none());
    fixture.finish(&mut admitted, &[text_event("real accepted answer")], true);
    drop(admitted);

    let next = Fixture::request(json!([
        {"role":"user","content":"cold synthetic summary"},
        {"role":"assistant","content":"uncaptured continuation"},
        {"role":"user","content":"first real captured task"},
        {"role":"assistant","content":"real accepted answer"},
        {"role":"user","content":"next real task"}
    ]));
    let admitted = fixture.begin(
        &next,
        native_hold(&fixture, &next, 3, Some(7), NativeUserOrigin::CurrentUser),
    );
    assert_eq!(
        visible_users(&admitted),
        vec![user("first real captured task")]
    );
    let visible = serde_json::to_string(&admitted.history.visible_conversation).unwrap();
    assert!(
        !visible.contains("cold synthetic summary") && !visible.contains("uncaptured continuation")
    );
}

#[test]
fn native_saved_user_requires_the_same_plan_and_a_valid_captured_decision() {
    for missing_decision in [false, true] {
        let fixture = Fixture::new();
        let first = Fixture::request(json!([{"role":"user","content":"original task"}]));
        let mut admitted = fixture.begin(
            &first,
            native_hold(&fixture, &first, 1, None, NativeUserOrigin::CurrentUser),
        );
        let ticket = admitted.guard.as_ref().unwrap().ticket().clone();
        fixture.finish(&mut admitted, &[text_event("accepted answer")], true);
        drop(admitted);
        if missing_decision {
            fixture
                .runtime
                .agent_turn_history
                .clear_decision_for_runtime_test(&ticket);
        }
        let synthetic = Fixture::request(json!([{"role":"user","content":"synthetic summary"}]));
        let hold = native_hold(
            &fixture,
            &synthetic,
            1,
            Some(7),
            NativeUserOrigin::ExistingTurn,
        );
        assert!(hold.history_is_proven());
        assert_eq!(
            hold.history_continuity(),
            HistoryContinuity::NativeContinuation
        );
        let rejected = fixture
            .runtime
            .begin_decision_history(
                &super::super::super::observation::accepted_request_for_runtime_test(),
                PlanSnapshot {
                    plan_id: "plan".into(),
                    plan_revision: if missing_decision { 1 } else { 2 },
                },
                &synthetic,
                &fixture.replay,
                Some(hold),
            )
            .unwrap();
        assert_detached(&rejected);
        drop(rejected);
        // Rejected history admission cannot replace the last accepted native checkpoint.
        assert_eq!(
            native_hold(
                &fixture,
                &synthetic,
                1,
                Some(7),
                NativeUserOrigin::ExistingTurn
            )
            .history_continuity(),
            HistoryContinuity::NativeContinuation
        );
    }
}

#[test]
fn a_retained_context_checkpoint_cannot_recreate_a_missing_native_agent_turn() {
    let mut fixture = Fixture::new();
    let first = Fixture::request(json!([{"role":"user","content":"original task"}]));
    let mut admitted = fixture.begin(
        &first,
        native_hold(&fixture, &first, 1, None, NativeUserOrigin::CurrentUser),
    );
    fixture.finish(&mut admitted, &[text_event("accepted answer")], true);
    drop(admitted);
    // Agent history and prefix checkpoints have independent eviction/lifetime boundaries.
    fixture.runtime.agent_turn_history = Arc::new(Default::default());
    let synthetic = Fixture::request(json!([{"role":"user","content":"synthetic summary"}]));
    let hold = native_hold(
        &fixture,
        &synthetic,
        1,
        Some(7),
        NativeUserOrigin::ExistingTurn,
    );
    assert_eq!(
        hold.history_continuity(),
        HistoryContinuity::NativeContinuation
    );
    assert!(hold.history_is_proven());
    let rejected = fixture.begin(&synthetic, hold);
    assert_detached(&rejected);
    drop(rejected);
    assert_eq!(
        native_hold(
            &fixture,
            &synthetic,
            1,
            Some(7),
            NativeUserOrigin::ExistingTurn
        )
        .history_continuity(),
        HistoryContinuity::NativeContinuation
    );

    let current = Fixture::request(json!([
        {"role":"user","content":"synthetic summary"},
        {"role":"assistant","content":"uncaptured continuation"},
        {"role":"user","content":"new real task"}
    ]));
    let mut admitted = fixture.begin(
        &current,
        native_hold(
            &fixture,
            &current,
            2,
            Some(7),
            NativeUserOrigin::CurrentUser,
        ),
    );
    assert!(admitted.guard.as_ref().unwrap().ticket().new_turn);
    assert!(admitted.history.visible_conversation.is_empty());
    fixture.finish(&mut admitted, &[text_event("new real answer")], true);
    drop(admitted);
    let next = Fixture::request(json!([
        {"role":"user","content":"synthetic summary"},
        {"role":"assistant","content":"uncaptured continuation"},
        {"role":"user","content":"new real task"},
        {"role":"assistant","content":"new real answer"},
        {"role":"user","content":"next real task"}
    ]));
    let admitted = fixture.begin(
        &next,
        native_hold(&fixture, &next, 3, Some(7), NativeUserOrigin::CurrentUser),
    );
    assert_eq!(visible_users(&admitted), vec![user("new real task")]);
}

#[test]
fn budget_trimmed_original_user_cannot_authorize_a_native_continuation() {
    let mut fixture = Fixture::new();
    fixture.runtime.agent_turn_history = Arc::new(
        crate::agent_turn_history::AgentTurnHistoryStore::new(8 * 1024, Duration::from_secs(60)),
    );
    let original = "large-original-user-".repeat(2_000);
    let first = Fixture::request(json!([{"role":"user","content":original}]));
    let mut admitted = fixture.begin(
        &first,
        native_hold(&fixture, &first, 1, None, NativeUserOrigin::CurrentUser),
    );
    assert!(
        admitted.guard.is_some(),
        "the original request is admitted with its oversized history trimmed"
    );
    fixture.finish(&mut admitted, &[text_event("accepted answer")], true);
    drop(admitted);
    let synthetic = Fixture::request(json!([{"role":"user","content":"synthetic summary"}]));
    let hold = native_hold(
        &fixture,
        &synthetic,
        1,
        Some(7),
        NativeUserOrigin::ExistingTurn,
    );
    assert!(hold.history_is_proven());
    let rejected = fixture.begin(&synthetic, hold);
    assert_detached(&rejected);
    drop(rejected);
    assert_eq!(
        native_hold(
            &fixture,
            &synthetic,
            1,
            Some(7),
            NativeUserOrigin::ExistingTurn
        )
        .history_continuity(),
        HistoryContinuity::NativeContinuation
    );

    let next = Fixture::request(json!([
        {"role":"user","content":original},
        {"role":"assistant","content":"accepted answer"},
        {"role":"user","content":"next real task"}
    ]));
    let admitted = fixture.begin(
        &next,
        native_hold(&fixture, &next, 2, Some(7), NativeUserOrigin::CurrentUser),
    );
    assert!(admitted.guard.is_some());
    assert!(admitted.history.history_partial);
    assert_eq!(admitted.history.visible_conversation.len(), 1);
    assert!(
        admitted.history.visible_conversation[0].user.is_empty(),
        "the retained entry must not masquerade as a complete original User"
    );
}

#[test]
fn output_capture_gap_preserves_complete_user_and_exact_prefix_tool_continuation() {
    let fixture = Fixture::new();
    let first = Fixture::request(json!([{"role":"user","content":"first"}]));
    let mut admitted = fixture.begin(
        &first,
        native_hold(&fixture, &first, 1, None, NativeUserOrigin::CurrentUser),
    );
    let ticket = admitted.guard.as_ref().unwrap().ticket().clone();
    fixture.finish(&mut admitted, &[tool_event()], false);
    drop(admitted);
    fixture
        .runtime
        .agent_turn_history
        .note_capture_gap(&ticket.key);

    let continued = tool_continuation(ToolResultStatusV1::Completed);
    let hold = native_hold(
        &fixture,
        &continued,
        1,
        None,
        NativeUserOrigin::ExistingTurn,
    );
    assert_eq!(hold.history_continuity(), HistoryContinuity::NativeSameTurn);
    let mut admitted = fixture.begin(&continued, hold);
    assert_eq!(
        admitted.guard.as_ref().unwrap().ticket().agent_turn_id,
        ticket.agent_turn_id
    );
    assert!(admitted.correlated.is_some());
    fixture.finish(&mut admitted, &[text_event("main completed")], true);
    drop(admitted);
    let mut next = continued.messages.clone();
    next.extend(
        Fixture::request(json!([
            {"role":"assistant","content":"main completed"},
            {"role":"user","content":"next real task"}
        ]))
        .messages,
    );
    let mut request = continued;
    request.messages = next;
    let admitted = fixture.begin(
        &request,
        native_hold(&fixture, &request, 2, None, NativeUserOrigin::CurrentUser),
    );
    assert!(admitted.history.history_partial);
    assert_eq!(visible_users(&admitted), vec![user("first")]);
    assert!(
        admitted
            .history
            .visible_conversation
            .iter()
            .flat_map(|turn| &turn.steps)
            .flatten()
            .any(|part| matches!(
                part,
                VisibleContentPart::ToolActivity {
                    status: ToolStatus::Completed,
                    ..
                }
            ))
    );
    if let Some(target) = admitted.history.assessment_target.as_ref() {
        assert!(target.target_partial);
    }
}
