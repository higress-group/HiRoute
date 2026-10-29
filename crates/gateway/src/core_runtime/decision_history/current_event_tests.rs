//! Native input positions are capture events without becoming portable Users.
use super::*;

fn peer() -> Value {
    json!({"type":"agent_message", "author":"child", "recipient":"parent",
        "content":[{"type":"input_text", "text":"actual peer result"}]})
}

fn begin(
    fixture: &Fixture,
    request: &model_ir::ModelRequestIRV1,
    current: Option<(&[model_ir::ContentPart], usize)>,
) -> DecisionHistory {
    begin_input(
        fixture,
        request,
        current.map(|(parts, _)| parts),
        current.map(|(_, index)| CurrentInputPosition::Event(index)),
        false,
    )
}

pub(super) fn begin_input(
    fixture: &Fixture,
    request: &model_ir::ModelRequestIRV1,
    current: Option<&[model_ir::ContentPart]>,
    index: Option<CurrentInputPosition>,
    unavailable: bool,
) -> DecisionHistory {
    fixture
        .runtime
        .begin_authorized_decision_history(
            &super::super::super::observation::accepted_request_for_runtime_test(),
            PlanSnapshot {
                plan_id: "plan".into(),
                plan_revision: 1,
            },
            Some(CaptureScope {
                scope_id: "native-event-test".into(),
                native_turn: None,
                branch_cursor: (None, None),
                origin_unverified: true,
                missing_prefix: false,
                unavailable_input: false,
                continuity: HistoryContinuity::NoCheckpoint,
            }),
            None,
            HistoryInput {
                request,
                replay: &fixture.replay,
                current_input: current,
                current_input_position: index,
                current_input_unavailable: unavailable,
            },
        )
        .unwrap()
}

#[test]
fn opaque_native_event_captures_output_without_copying_ancestor_intent_and_recovers() {
    let fixture = Fixture::new();
    let input = json!([
        {"role":"user", "content":"ancestor is not current intent"},
        {"type":"agent_message", "author":"child", "recipient":"parent",
         "content":[{"type":"encrypted_content", "encrypted_content":"opaque ciphertext"}]}
    ]);
    let request = Fixture::request(input);
    let mut opaque = begin_input(
        &fixture,
        &request,
        None,
        Some(CurrentInputPosition::Event(1)),
        true,
    );
    assert!(opaque.guard.is_some());
    fixture.finish(
        &mut opaque,
        &[text_event("actual opaque-stage output")],
        true,
    );
    drop(opaque);
    let b_request = Fixture::request(json!([{"role":"user", "content":"actual fresh B"}]));
    let mut b = begin(&fixture, &b_request, None);
    let visible = serde_json::to_string(&b.history.visible_conversation).unwrap();
    assert!(visible.contains("native_event_input"));
    assert!(visible.contains("actual opaque-stage output"));
    assert!(!visible.contains("ancestor is not current intent"));
    assert!(!visible.contains("opaque ciphertext"));
    fixture.finish(&mut b, &[text_event("actual fresh B output")], true);
    drop(b);
    let c_request = Fixture::request(json!([{"role":"user", "content":"actual fresh C"}]));
    let c = begin(&fixture, &c_request, None);
    let target = c.history.assessment_target.as_ref().unwrap();
    assert!(!target.target_partial);
    let visible = serde_json::to_string(&c.history.visible_conversation).unwrap();
    assert!(visible.contains("actual fresh B"));
    assert!(visible.contains("actual fresh B output"));

    let unknown = Fixture::new();
    let missed = begin_input(&unknown, &request, None, None, true);
    assert!(
        missed.guard.is_none(),
        "unknown event position cannot select an ancestor"
    );
    assert!(begin(&unknown, &b_request, None).guard.is_some());
}

#[test]
fn peer_only_and_peer_after_prior_output_capture_the_actual_event_for_next_assessment() {
    for earlier_history in [false, true] {
        let fixture = Fixture::new();
        let mut items = Vec::new();
        if earlier_history {
            items.extend([
                json!({"role":"user", "content":"uncaptured original task"}),
                json!({"role":"assistant", "content":"uncaptured earlier output"}),
            ]);
        }
        items.push(peer());
        let request = Fixture::request(Value::Array(items.clone()));
        let original = serde_json::to_value(&request).unwrap();
        let original_native = request.native_body.clone();
        let parts = [model_ir::ContentPart::Text {
            text: "actual peer result".into(),
        }];
        let index = request.messages.len() - 1;
        let mut a = begin(&fixture, &request, Some((&parts, index)));
        assert!(a.guard.is_some());
        assert!(a.history.visible_conversation.is_empty());
        fixture.finish(
            &mut a,
            &[text_event("accepted peer processing output")],
            true,
        );
        drop(a);
        assert_eq!(serde_json::to_value(&request).unwrap(), original);
        assert_eq!(request.native_body, original_native);
        assert_eq!(
            request.messages[index].role,
            model_ir::MessageRole::Assistant
        );
        assert!(request.messages[index].content.is_empty());

        items.extend([
            json!({"role":"assistant", "content":"accepted peer processing output"}),
            json!({"role":"user", "content":"real new question B"}),
        ]);
        let b_request = Fixture::request(Value::Array(items));
        let b = begin(&fixture, &b_request, None);
        let target = b.history.assessment_target.as_ref().unwrap();
        assert!(
            !target.target_partial,
            "old output is before the current event"
        );
        let visible = serde_json::to_string(&b.history.visible_conversation).unwrap();
        assert!(visible.contains("actual peer result"));
        assert!(visible.contains("accepted peer processing output"));
        assert!(visible.contains("unverified_request_user_origin"));
        assert!(!visible.contains("uncaptured original task"));
        assert!(!visible.contains("uncaptured earlier output"));
        assert!(!visible.contains("real new question B"));
    }
}

#[test]
fn output_after_native_event_remains_partial_and_invalid_positions_are_optional_misses() {
    let fixture = Fixture::new();
    let request = Fixture::request(json!([peer(),
        {"role":"assistant", "content":"missed peer execution"}
    ]));
    let parts = [model_ir::ContentPart::Text {
        text: "actual peer result".into(),
    }];
    let mut a = begin(&fixture, &request, Some((&parts, 0)));
    fixture.finish(&mut a, &[text_event("accepted later suffix")], true);
    drop(a);
    let b_request = Fixture::request(json!([{"role":"user", "content":"real next B"}]));
    let b = begin(&fixture, &b_request, None);
    assert!(b.history.assessment_target.as_ref().unwrap().target_partial);

    let isolated = Fixture::new();
    let invalid = Fixture::request(json!([peer(), {"role":"user", "content":"later user"}]));
    for index in [0, 2, usize::MAX] {
        let missed = begin(&isolated, &invalid, Some((&parts, index)));
        assert!(missed.guard.is_none());
    }
    let mut recovered = begin(&isolated, &b_request, None);
    assert!(recovered.guard.is_some());
    isolated.finish(
        &mut recovered,
        &[text_event("captured after optional fault")],
        true,
    );
    drop(recovered);
    let c_request = Fixture::request(json!([{"role":"user", "content":"real next C"}]));
    let c = begin(&isolated, &c_request, None);
    assert!(c.history.assessment_target.is_some());
}
