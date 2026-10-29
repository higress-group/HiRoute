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

fn visible(history: &DecisionHistory) -> String {
    serde_json::to_string(&history.history.visible_conversation).unwrap()
}

fn assert_recoverable_pair(fixture: &Fixture, prefix: Value) {
    let mut items = prefix.as_array().unwrap().clone();
    items.push(json!({"role":"user","content":"fresh actual A"}));
    let a = Fixture::request(Value::Array(items.clone()));
    let mut first = fixture.begin(
        &a,
        native_hold(fixture, &a, 9, Some(7), NativeUserOrigin::CurrentUser),
    );
    assert!(first.guard.as_ref().unwrap().ticket().new_turn);
    assert!(first.correlated.is_none());
    let a_id = first.guard.as_ref().unwrap().ticket().agent_turn_id.clone();
    fixture.finish(&mut first, &[text_event("actual A output")], true);
    drop(first);
    items.push(json!({"role":"assistant","content":"actual A output"}));
    items.push(json!({"role":"user","content":"fresh actual B"}));
    let b = Fixture::request(Value::Array(items));
    let second = fixture.begin(
        &b,
        native_hold(fixture, &b, 10, Some(7), NativeUserOrigin::CurrentUser),
    );
    assert!(second.guard.as_ref().unwrap().ticket().new_turn);
    assert!(second.correlated.is_none());
    let target = second.history.assessment_target.as_ref().unwrap();
    assert_eq!(target.through_turn_id, a_id);
    assert!(
        !target.target_partial,
        "a complete new A must recover evaluation"
    );
    assert!(visible(&second).contains("actual A output"));
    assert!(!visible(&second).contains("synthetic summary"));
    assert!(!visible(&second).contains("synthetic continue"));
}

#[test]
fn cold_native_continuation_captures_actual_output_without_promoting_a_summary() {
    let fixture = Fixture::new();
    let synthetic = Fixture::request(json!([{"role":"user","content":"cold synthetic summary"}]));
    let hold = native_hold(
        &fixture,
        &synthetic,
        1,
        Some(7),
        NativeUserOrigin::ExistingTurn,
    );
    assert!(hold.native_user_unavailable());
    let mut captured = fixture.begin(&synthetic, hold);
    assert!(captured.guard.is_some());
    assert!(captured.correlated.is_none());
    assert!(captured.hold_ticket.is_none());
    fixture.finish(
        &mut captured,
        &[text_event("accepted continuation output")],
        true,
    );
    drop(captured);
    assert_recoverable_pair(
        &fixture,
        json!([
            {"role":"user","content":"cold synthetic summary"},
            {"role":"assistant","content":"accepted continuation output"}
        ]),
    );
}

#[test]
fn same_native_turn_does_not_turn_synthetic_input_into_a_new_user_fact() {
    let fixture = Fixture::new();
    let first = Fixture::request(json!([{"role":"user","content":"original task"}]));
    let mut captured = fixture.begin(
        &first,
        native_hold(&fixture, &first, 1, None, NativeUserOrigin::CurrentUser),
    );
    fixture.finish(&mut captured, &[text_event("accepted prefix")], true);
    drop(captured);
    let input = json!([
        {"role":"user","content":"original task"},
        {"role":"assistant","content":"accepted prefix"},
        {"role":"user","content":"synthetic continue"}
    ]);
    let request = Fixture::request(input.clone());
    let mut continuation = fixture.begin(
        &request,
        native_hold(
            &fixture,
            &request,
            1,
            Some(7),
            NativeUserOrigin::ExistingTurn,
        ),
    );
    // An optional affinity-cache miss may lose inheritance, never accepted output.
    assert!(continuation.guard.is_some());
    fixture.finish(&mut continuation, &[text_event("accepted final")], true);
    drop(continuation);
    let mut prefix = input.as_array().unwrap().clone();
    prefix.push(json!({"role":"assistant","content":"accepted final"}));
    assert_recoverable_pair(&fixture, Value::Array(prefix));
}

#[test]
fn compaction_link_alone_cannot_join_execution_or_apply_unproven_tool_results() {
    let fixture = Fixture::new();
    let first = Fixture::request(json!([{"role":"user","content":"original tool task"}]));
    let mut captured = fixture.begin(
        &first,
        native_hold(&fixture, &first, 1, None, NativeUserOrigin::CurrentUser),
    );
    let old_id = captured
        .guard
        .as_ref()
        .unwrap()
        .ticket()
        .agent_turn_id
        .clone();
    fixture.finish(&mut captured, &[tool_event()], false);
    drop(captured);
    let input = json!([
        {"role":"user","content":"synthetic summary"},
        {"type":"function_call","call_id":"tool-1","name":"read","arguments":"{}"},
        {"type":"function_call_output","call_id":"tool-1","output":"unproven rewritten receipt"}
    ]);
    let request = Fixture::request(input.clone());
    let mut continuation = fixture.begin(
        &request,
        native_hold(
            &fixture,
            &request,
            1,
            Some(7),
            NativeUserOrigin::ExistingTurn,
        ),
    );
    assert_ne!(
        continuation.guard.as_ref().unwrap().ticket().agent_turn_id,
        old_id
    );
    assert!(continuation.correlated.is_none());
    let prior = visible(&continuation);
    assert!(prior.contains("original tool task"));
    assert!(prior.contains("unknown"));
    assert!(!prior.contains("unproven rewritten receipt"));
    fixture.finish(
        &mut continuation,
        &[text_event("accepted completion")],
        true,
    );
    drop(continuation);
    let mut prefix = input.as_array().unwrap().clone();
    prefix.push(json!({"role":"assistant","content":"accepted completion"}));
    assert_recoverable_pair(&fixture, Value::Array(prefix));
}

#[test]
fn changed_plan_or_missing_decision_never_authorizes_inheritance() {
    for missing_decision in [false, true] {
        let fixture = Fixture::new();
        let first = Fixture::request(json!([{"role":"user","content":"original task"}]));
        let mut captured = fixture.begin(
            &first,
            native_hold(&fixture, &first, 1, None, NativeUserOrigin::CurrentUser),
        );
        let ticket = captured.guard.as_ref().unwrap().ticket().clone();
        fixture.finish(&mut captured, &[text_event("accepted answer")], true);
        drop(captured);
        if missing_decision {
            fixture
                .runtime
                .agent_turn_history
                .clear_decision_for_runtime_test(&ticket);
        }
        let synthetic = Fixture::request(json!([{"role":"user","content":"synthetic summary"}]));
        let history = fixture
            .runtime
            .begin_decision_history(
                &super::super::super::observation::accepted_request_for_runtime_test(),
                PlanSnapshot {
                    plan_id: "plan".into(),
                    plan_revision: if missing_decision { 1 } else { 2 },
                },
                &synthetic,
                &fixture.replay,
                Some(native_hold(
                    &fixture,
                    &synthetic,
                    1,
                    Some(7),
                    NativeUserOrigin::ExistingTurn,
                )),
            )
            .unwrap();
        assert!(history.guard.is_some());
        assert!(history.correlated.is_none());
        assert!(history.hold_ticket.is_none());
        if !missing_decision {
            assert!(history.history.assessment_target.is_none());
        }
        assert!(!visible(&history).contains("synthetic summary"));
    }
}

#[test]
fn restart_does_not_recreate_missing_execution_and_new_pair_recovers() {
    let mut fixture = Fixture::new();
    let first = Fixture::request(json!([{"role":"user","content":"old original task"}]));
    let mut captured = fixture.begin(
        &first,
        native_hold(&fixture, &first, 1, None, NativeUserOrigin::CurrentUser),
    );
    fixture.finish(
        &mut captured,
        &[text_event("discarded pre-restart output")],
        true,
    );
    drop(captured);
    fixture.runtime.agent_turn_history = Arc::new(Default::default());
    let synthetic = Fixture::request(json!([{"role":"user","content":"synthetic summary"}]));
    let mut captured = fixture.begin(
        &synthetic,
        native_hold(
            &fixture,
            &synthetic,
            1,
            Some(7),
            NativeUserOrigin::ExistingTurn,
        ),
    );
    assert!(captured.history.visible_conversation.is_empty());
    assert!(captured.history.assessment_target.is_none());
    fixture.finish(&mut captured, &[text_event("post-restart output")], true);
    drop(captured);
    assert_recoverable_pair(
        &fixture,
        json!([
            {"role":"user","content":"synthetic summary"},
            {"role":"assistant","content":"post-restart output"}
        ]),
    );
}

#[test]
fn trimmed_user_is_not_recovered_from_summary_and_short_inputs_remain_available() {
    let mut fixture = Fixture::new();
    fixture.runtime.agent_turn_history = Arc::new(
        crate::agent_turn_history::AgentTurnHistoryStore::new(64 * 1024, Duration::from_secs(60)),
    );
    let first =
        Fixture::request(json!([{"role":"user","content":"large original user ".repeat(10000)}]));
    let mut captured = fixture.begin(
        &first,
        native_hold(&fixture, &first, 1, None, NativeUserOrigin::CurrentUser),
    );
    fixture.finish(&mut captured, &[text_event("accepted answer")], true);
    drop(captured);
    let synthetic = Fixture::request(json!([{"role":"user","content":"synthetic summary"}]));
    let mut captured = fixture.begin(
        &synthetic,
        native_hold(
            &fixture,
            &synthetic,
            1,
            Some(7),
            NativeUserOrigin::ExistingTurn,
        ),
    );
    assert!(captured.correlated.is_none());
    assert!(!visible(&captured).contains("synthetic summary"));
    fixture.finish(&mut captured, &[text_event("accepted continuation")], true);
    drop(captured);
    assert_recoverable_pair(&fixture, json!([]));
}

#[test]
fn a_capture_gap_cannot_be_repaired_by_native_turn_id_or_tool_receipt() {
    let fixture = Fixture::new();
    let first = Fixture::request(json!([{"role":"user","content":"first"}]));
    let mut captured = fixture.begin(
        &first,
        native_hold(&fixture, &first, 1, None, NativeUserOrigin::CurrentUser),
    );
    let ticket = captured.guard.as_ref().unwrap().ticket().clone();
    fixture.finish(&mut captured, &[tool_event()], false);
    drop(captured);
    fixture
        .runtime
        .agent_turn_history
        .note_capture_gap(&ticket.key);
    let request = tool_continuation(ToolResultStatusV1::Completed);
    let mut captured = fixture.begin(
        &request,
        native_hold(&fixture, &request, 1, None, NativeUserOrigin::ExistingTurn),
    );
    assert!(captured.correlated.is_none());
    assert!(!visible(&captured).contains("completed"));
    fixture.finish(&mut captured, &[text_event("new accepted output")], true);
    drop(captured);
    assert_recoverable_pair(&fixture, json!([]));
}
