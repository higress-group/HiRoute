//! Input content and its native window must identify the same capture boundary.
use super::current_event_tests::begin_input;
use super::*;
use crate::server::core_runtime::native_routing_view::NativeRoutingView;
use sha2::{Digest, Sha256};

fn begin_window(fixture: &Fixture, items: Value, selected: &[usize]) -> DecisionHistory {
    let hashes = items
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["role"] == "user")
        .map(|item| {
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&[item["content"].as_str().unwrap()]).unwrap())
            )
        })
        .collect::<Vec<_>>();
    let mut headers = http::HeaderMap::new();
    headers.insert("x-hiroute-agent", "dsh".parse().unwrap());
    headers.insert("x-deepseek-harness-session-id", "session".parse().unwrap());
    headers.insert("x-hiroute-routing-context", json!({
        "version":2, "agent":"dsh", "session_id":"session", "purpose":"main",
        "input":{"kind":"native_messages", "event_id":"event", "coverage":"window",
            "messages":selected.iter().map(|index| json!({"index":index,"sha256":hashes[*index]})).collect::<Vec<_>>(),
            "scope_digest":format!("{:x}", Sha256::digest(serde_json::to_vec(&hashes).unwrap()))}
    }).to_string().parse().unwrap());
    let request = Fixture::request(items);
    let original = serde_json::to_value(&request).unwrap();
    let budget = BudgetTree::new(1024 * 1024, 1024 * 1024)
        .unwrap()
        .stream(1024 * 1024)
        .unwrap();
    let view = NativeRoutingView::prepare(&headers, &request, &fixture.replay, &budget);
    assert!(view.current_input().is_some());
    let history = begin_input(
        fixture,
        &request,
        view.current_input(),
        view.current_input_position(),
        false,
    );
    assert_eq!(serde_json::to_value(&request).unwrap(), original);
    history
}

#[test]
fn native_reasoning_replay_keeps_new_input_and_tool_steps_in_their_actual_turns() {
    use model_ir::ResponseItemStatus;
    let fixture = Fixture::new();
    let mut items = vec![json!({"role":"user","content":"first task"})];
    let mut a = begin_window(&fixture, json!(items), &[0]);
    let a_id = a.guard.as_ref().unwrap().ticket().agent_turn_id.clone();
    fixture.finish(
        &mut a,
        &[
            ModelStreamEventV1::new(
                0,
                ModelEvent::ReasoningFinished {
                    index: 0,
                    text: String::new(),
                    status: ResponseItemStatus::Completed,
                },
            ),
            ModelStreamEventV1::new(
                1,
                ModelEvent::TextFinished {
                    index: 1,
                    text: "READY".into(),
                    annotations: vec![],
                    status: ResponseItemStatus::Completed,
                    final_phase: None,
                },
            ),
        ],
        true,
    );
    drop(a);
    items.extend([
        json!({"type":"reasoning","id":"rs_0","summary":[{"type":"summary_text","text":""}]}),
        json!({"role":"assistant","content":"READY"}),
        json!({"role":"user","content":"read the file"}),
    ]);
    let mut b = begin_window(&fixture, json!(items), &[1]);
    assert!(b.correlated.is_none(), "a new input must be judged again");
    let target = b.history.assessment_target.as_ref().unwrap();
    assert_eq!(target.through_turn_id, a_id);
    assert!(!target.target_partial);
    assert!(
        !target
            .history_membership
            .as_ref()
            .unwrap()
            .association_unverified
    );
    let b_id = b.guard.as_ref().unwrap().ticket().agent_turn_id.clone();
    let mut call = tool_event();
    call.sequence = 1;
    if let ModelEvent::ToolCallStarted { index, .. } = &mut call.event {
        *index = 1;
    }
    fixture.finish(
        &mut b,
        &[
            ModelStreamEventV1::new(
                0,
                ModelEvent::ReasoningFinished {
                    index: 0,
                    text: "Read before answering.".into(),
                    status: ResponseItemStatus::Completed,
                },
            ),
            call,
            ModelStreamEventV1::new(
                2,
                ModelEvent::ToolCallFinished {
                    encrypted_function_args: None,
                    index: 1,
                    logical_id: "tool-1".into(),
                    status: ResponseItemStatus::Completed,
                    tool_kind: ToolKindV1::Function,
                    namespace: None,
                    name: "read".into(),
                    arguments: json!({}),
                },
            ),
        ],
        false,
    );
    drop(b);
    items.extend([
        json!({"type":"reasoning","id":"rs_0","summary":[{"type":"summary_text","text":"Read before answering."}]}),
        json!({"role":"assistant","content":""}),
        json!({"type":"function_call","call_id":"tool-1","name":"read","arguments":"{}"}),
        json!({"type":"function_call_output","call_id":"tool-1","output":"actual file"}),
    ]);
    let continued = begin_window(&fixture, json!(items), &[1]);
    assert!(!continued.guard.as_ref().unwrap().ticket().new_turn);
    assert_eq!(
        continued.guard.as_ref().unwrap().ticket().agent_turn_id,
        b_id
    );
    assert!(
        continued.correlated.is_some(),
        "tool continuation inherits the executed group"
    );
}

#[test]
fn current_window_retains_completed_a_for_b_but_fences_a_real_intervening_gap() {
    for multipart in [false, true] {
        for missed in [false, true] {
            let fixture = Fixture::new();
            let mut items = vec![
                json!({"role":"user","content":"uncaptured summary"}),
                json!({"role":"assistant","content":"old client output"}),
                json!({"role":"user","content":"actual A"}),
            ];
            let mut a = begin_window(&fixture, json!(items), &[1]);
            let a_id = a.guard.as_ref().unwrap().ticket().agent_turn_id.clone();
            fixture.finish(&mut a, &[text_event("accepted A output")], true);
            drop(a);
            items.push(json!({"role":"assistant","content":"accepted A output"}));
            let start = if missed {
                items.extend([
                    json!({"role":"user","content":"uncaptured intervening task"}),
                    json!({"role":"assistant","content":"uncaptured execution"}),
                ]);
                3
            } else {
                2
            };
            items.push(json!({"role":"user","content":"actual B"}));
            let selected = if multipart {
                items.push(json!({"role":"user","content":"B second input"}));
                vec![start, start + 1]
            } else {
                vec![start]
            };
            items.push(json!({"role":"user","content":"trailing runtime context"}));
            let mut b = begin_window(&fixture, json!(items), &selected);
            assert_eq!(
                b.history.assessment_target.is_some(),
                !missed,
                "multipart={multipart}, actual capture gap={missed}"
            );
            if let Some(target) = b.history.assessment_target.as_ref() {
                assert_eq!(target.through_turn_id, a_id);
                assert!(!target.target_partial);
                let visible = serde_json::to_string(&b.history.visible_conversation).unwrap();
                assert!(visible.contains("actual A") && visible.contains("accepted A output"));
                assert!(
                    !visible.contains("uncaptured summary")
                        && !visible.contains("old client output")
                );
            }
            let b_id = b.guard.as_ref().unwrap().ticket().agent_turn_id.clone();
            fixture.finish(&mut b, &[text_event("accepted B output")], true);
            drop(b);
            items.extend([
                json!({"role":"assistant","content":"accepted B output"}),
                json!({"role":"user","content":"actual C"}),
            ]);
            let last = items.iter().filter(|m| m["role"] == "user").count() - 1;
            let c = begin_window(&fixture, json!(items), &[last]);
            let target = c
                .history
                .assessment_target
                .as_ref()
                .expect("later complete input recovers");
            assert_eq!(target.through_turn_id, b_id);
            assert!(!target.target_partial);
            let visible = serde_json::to_string(&c.history.visible_conversation).unwrap();
            assert!(visible.contains("actual B") && visible.contains("accepted B output"));
            assert!(!visible.contains("trailing runtime context"));
        }
    }
}

#[test]
fn current_window_does_not_erase_uncaptured_output_after_its_input() {
    let fixture = Fixture::new();
    let mut a = begin_window(
        &fixture,
        json!([
            {"role":"user","content":"actual A"},
            {"role":"assistant","content":"uncaptured A execution"},
            {"role":"user","content":"trailing runtime context"}
        ]),
        &[0],
    );
    fixture.finish(&mut a, &[text_event("accepted later A suffix")], true);
    drop(a);
    let b = begin_window(
        &fixture,
        json!([{"role":"user","content":"actual B"}]),
        &[0],
    );
    assert!(b.history.assessment_target.as_ref().unwrap().target_partial);
    let visible = serde_json::to_string(&b.history.visible_conversation).unwrap();
    assert!(visible.contains("accepted later A suffix"));
    assert!(!visible.contains("uncaptured A execution"));
}
