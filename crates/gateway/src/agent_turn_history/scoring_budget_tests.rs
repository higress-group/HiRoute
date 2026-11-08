use super::*;

#[test]
fn execution_record_growth_leaves_room_for_the_next_tool_request() {
    use crate::server::core_runtime::adapters::decode_ingress_request;

    let (_directory, replay) = replay();
    let mut store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let scope = store
        .scope_key("workspace", "scoring-global-pressure")
        .unwrap();
    let decode = |messages: &[serde_json::Value]| {
        decode_ingress_request(
            IngressProtocol::Responses,
            &json!({"model":"route","input":messages}),
        )
        .unwrap()
    };
    let mut messages = vec![json!({"role":"user","content":"Run sequential tools"})];
    let (first, _) = new_turn(scoped(&store, &scope, &decode(&messages), &replay).unwrap());
    store.commit_decision(&first, decision("regular")).unwrap();
    let turn_id = first.agent_turn_id.clone();
    let mut ticket = first;
    for step in 0..=128 {
        let id = format!("request-{step:064x}");
        store.bind_request(&ticket, &id).unwrap();
        let call_id = format!("call_{step}");
        let call = json!({"type":"function_call","id":format!("fc_{step}"),
            "call_id":call_id,"name":"lookup","arguments":"{}","status":"completed"});
        let batch = Arc::new(
            DeliveredToolBatch::from_completed_output(std::slice::from_ref(&call), |_| true)
                .unwrap(),
        );
        if step == 128 {
            // Request bookkeeping and the delivered batch fit in the remaining
            // global budget, while doubling the execution record array does not.
            let accounted = store.inner.lock().accounted_bytes;
            store.max_memory_bytes = accounted + 8 * 1024;
        }
        store
            .finish_request(
                &ticket,
                id.clone(),
                AgentTurnStatus::Completed,
                vec![execution("model", "profile", "regular", &id)],
                RequestCompletion {
                    terminal: false,
                    tool_batch: Some(batch),
                },
                Instant::now(),
            )
            .unwrap();
        messages.push(call);
        messages.push(json!({"type":"function_call_output","call_id":call_id,"output":"done"}));
        let AgentTurnBegin::Continuation {
            ticket: next,
            decision,
        } = scoped(&store, &scope, &decode(&messages), &replay).unwrap()
        else {
            panic!("the delivered tool batch must keep request {step} in the same turn");
        };
        assert_eq!(next.agent_turn_id, turn_id);
        assert_eq!(decision.branch_id, "regular");
        ticket = next;
    }
    let inner = store.inner.lock();
    let entry = &inner.entries[&scope];
    assert!(entry.active.request_ids_clipped);
    assert_eq!(entry.active.executions.len(), 128);
    assert!(inner.accounted_bytes <= store.max_memory_bytes);
    drop(inner);
    store.abort(&ticket);
}

#[test]
fn oversized_output_keeps_closed_evidence_when_eviction_cannot_help() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(32 * 1024 * 1024, Duration::from_secs(60));
    let scope = store
        .scope_key("workspace", "scoring-unfittable-output")
        .unwrap();
    let now = Instant::now();
    let first = request(vec![text(MessageRole::User, "A")]);
    let (a, _) = new_turn(
        store
            .begin(scope.clone(), plan(1), &first, &replay, now)
            .unwrap(),
    );
    accepted_output::text_output(&store, &a, &"a".repeat(150_000));
    close_turn(&store, &a, "regular", "model", "profile", "a", now);

    let second = request(vec![
        text(MessageRole::User, "A"),
        text(MessageRole::User, "B"),
    ]);
    let (b, snapshot) = new_turn(
        store
            .begin(scope.clone(), plan(1), &second, &replay, now)
            .unwrap(),
    );
    drop(snapshot);
    store.commit_decision(&b, decision("regular")).unwrap();
    accepted_output::text_output(&store, &b, &"b".repeat(4_800_000));
    let before = store.inner.lock().scope_history_bytes(&scope);
    accepted_output::text_output(&store, &b, &"c".repeat(300_000));

    let inner = store.inner.lock();
    let entry = &inner.entries[&scope];
    assert!(entry.active.scope_partial);
    assert_eq!(inner.scope_history_bytes(&scope), before);
    assert_eq!(entry.closed.len(), 1);
    assert_eq!(entry.closed[0].completed.agent_turn_id, a.agent_turn_id);
    drop(inner);
    store.abort(&b);
}
