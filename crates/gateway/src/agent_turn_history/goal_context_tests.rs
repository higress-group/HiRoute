use super::*;
use crate::server::core_runtime::adapters::decode_ingress_request;
use crate::server::core_runtime::model_ir::{ModelEvent, ModelStreamEventV1, ResponseItemStatus};
use hiroute_gateway_core::runtime::body::MemoryRole;
use serde_json::Value;

fn goal() -> Value {
    json!({"type":"function_call_output","id":"goal-context","name":"goal_objective",
        "namespace":"agent","output":"Complete the synthetic goal."})
}

fn call() -> Value {
    json!({"type":"function_call","id":"fc-goal","call_id":"call-goal",
        "namespace":"functions","name":"create_goal","arguments":"{}","status":"completed"})
}

fn decode(input: Vec<Value>) -> ModelRequestIRV1 {
    decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"route","input":input}),
    )
    .unwrap()
}

fn prepare(
    store: &AgentTurnHistoryStore,
    scope: &AgentTurnHistoryKey,
    replay: &ReplayStore,
) -> (AgentTurnTicket, Arc<ToolDeliveryHandoff>) {
    let input = decode(vec![json!({"role":"user","content":"Complete the task."})]);
    let (ticket, _) = new_turn(native_retry::begin(store, scope, &input, replay, Some(1)));
    store.commit_decision(&ticket, decision("regular")).unwrap();
    let batch = Arc::new(DeliveredToolBatch::from_completed_output(&[call()], |_| true).unwrap());
    let charge = replay
        .budget()
        .reserve(MemoryRole::SemanticState, batch.retained_bytes())
        .unwrap();
    let handoff = store.register_tool_handoff(&ticket).unwrap();
    handoff.prepare((batch, charge));
    (ticket, handoff)
}

fn callback(goal_before_result: bool) -> Vec<Value> {
    let mut items = vec![
        json!({"role":"user","content":"Complete the task."}),
        call(),
        json!({"type":"function_call_output","call_id":"call-goal","output":"active"}),
    ];
    items.insert(if goal_before_result { 2 } else { 3 }, goal());
    items
}

fn pending(
    store: &AgentTurnHistoryStore,
    scope: &AgentTurnHistoryKey,
    request: &ModelRequestIRV1,
    replay: &ReplayStore,
    plan_revision: u64,
    turn: u8,
) -> Option<Arc<ToolDeliveryHandoff>> {
    let analyzed = store
        .prepare_scoped_request(scope, request, replay)
        .unwrap();
    store.pending_tool_predecessor(
        scope,
        &plan(plan_revision),
        &analyzed,
        &TurnDecisionInputs {
            native_turn: Some([turn; 32]),
            history_continuity: crate::context_hold::HistoryContinuity::Continuing,
            reselect_on_user_message: true,
        },
    )
}

#[test]
fn goal_context_handoff_keeps_the_delivered_tool_turn_and_frozen_decision() {
    for (goal_before_result, externalize) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let (_directory, replay) = replay();
        let store = AgentTurnHistoryStore::default();
        let scope = store.scope_key("workspace", "goal-handoff").unwrap();
        let (first, handoff) = prepare(&store, &scope, &replay);
        let mut request = decode(callback(goal_before_result));
        if externalize {
            crate::content_ref::externalize_model_request(&mut request, &replay, 0).unwrap();
        }
        assert!(request.native_only);
        let layout = request.native_history_layout().unwrap();
        let goal_index = if goal_before_result { 2 } else { 3 };
        assert!(layout[goal_index].goal_context);
        assert!(!layout[goal_index].modeled);
        assert!(
            layout
                .iter()
                .enumerate()
                .all(|(index, item)| item.goal_context == (index == goal_index))
        );
        let matched = pending(&store, &scope, &request, &replay, 1, 1)
            .expect("known goal context must not hide a genuine tool callback");
        assert!(Arc::ptr_eq(&matched, &handoff));

        accepted_output::tool(&store, &first, "call-goal", "create_goal");
        store.accept_output(
            &first,
            &[ModelStreamEventV1::new(
                1,
                ModelEvent::ToolCallFinished {
                    index: 1,
                    logical_id: "call-goal".into(),
                    tool_kind: ToolKindV1::Function,
                    namespace: Some("functions".into()),
                    name: "create_goal".into(),
                    arguments: json!({}),
                    encrypted_function_args: None,
                    status: ResponseItemStatus::Completed,
                },
            )],
        );
        store.mark_output_partial(&first);
        handoff.confirm();
        store
            .finish_request(
                &first,
                "first".into(),
                AgentTurnStatus::Completed,
                vec![execution("model", "profile", "regular", "first")],
                RequestCompletion {
                    terminal: false,
                    tool_batch: handoff.delivered_batch(),
                },
                Instant::now(),
            )
            .unwrap();
        let AgentTurnBegin::Continuation { ticket, decision } =
            native_retry::begin(&store, &scope, &request, &replay, Some(1))
        else {
            panic!("matched goal callback must inherit the original decision");
        };
        assert_eq!(ticket.agent_turn_id, first.agent_turn_id);
        assert_eq!(ticket.key, first.key);
        assert_eq!(decision, super::decision("regular"));
        assert!(!ticket.allow_affinity);
        store.abort(&ticket);
    }
}

#[test]
fn goal_context_handoff_requires_complete_tool_evidence_and_a_unique_predecessor() {
    for case in [
        "missing_result",
        "wrong_id",
        "duplicate_result",
        "extra_result",
        "arguments",
        "new_user",
        "changed_prefix",
        "unknown_history",
        "unknown_goal_fields",
        "unfinished_goal",
        "goal_with_call_id",
        "different_plan",
        "different_turn",
        "ambiguous",
    ] {
        let (_directory, replay) = replay();
        let store = AgentTurnHistoryStore::default();
        let scope = store.scope_key("workspace", case).unwrap();
        let (first, _handoff) = prepare(&store, &scope, &replay);
        let mut input = callback(false);
        let mut sibling = None;
        match case {
            "missing_result" => { input.remove(2); }
            "wrong_id" => input[2]["call_id"] = json!("other-call"),
            "duplicate_result" => input.push(input[2].clone()),
            "extra_result" => input.push(json!({"type":"function_call_output","call_id":"extra-call","output":"extra"})),
            "arguments" => input[1]["arguments"] = json!("{\"changed\":true}"),
            "new_user" => input.push(json!({"role":"user","content":"A new task."})),
            "changed_prefix" => input[0]["content"] = json!("A different task."),
            "unknown_history" => input.push(json!({"type":"web_search_call","id":"ws-future","status":"completed","action":{"type":"future_action"}})),
            "unknown_goal_fields" => input[3]["provider_options"] = json!({"future":true}),
            "unfinished_goal" => input[3]["status"] = json!("in_progress"),
            "goal_with_call_id" => input[3]["call_id"] = json!("another-call"),
            "ambiguous" => sibling = Some(prepare(&store, &scope, &replay)),
            _ => {}
        }
        let request = decode(input);
        assert!(
            pending(
                &store,
                &scope,
                &request,
                &replay,
                if case == "different_plan" { 2 } else { 1 },
                if case == "different_turn" { 2 } else { 1 },
            )
            .is_none(),
            "{case} must not release a predecessor's routing ticket"
        );
        store.abort(&first);
        if let Some((ticket, _handoff)) = sibling {
            store.abort(&ticket);
        }
    }
}
