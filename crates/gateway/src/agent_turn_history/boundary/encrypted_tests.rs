use super::*;
use crate::agent_turn_history::boundary::{BoundaryCapture, incoming_evidence};
use crate::context_hold::fingerprint::FingerprintBudget;
use crate::server::core_runtime::model_ir::{ModelEvent, ModelStreamEventV1};

fn events(cipher: &str, summary: &str) -> Vec<ModelStreamEventV1> {
    use crate::server::core_runtime::adapters::NativeResponseDecoder;
    use crate::server::core_runtime::profiles::{
        CandidateProtocolProfile, NativeProviderStateEmission, fixed_reasoning,
    };
    let mut profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    profile.capability.native_provider_state = NativeProviderStateEmission::Native;
    let output = vec![
        json!({"type":"reasoning","id":"rs-edit","status":"completed","summary": if summary.is_empty() {vec![]} else {vec![json!({"type":"summary_text","text":summary})]},"encrypted_content":cipher}),
        json!({"type":"message","id":"msg-edit","status":"completed","role":"assistant", "content":[{"type":"output_text","text":"Apply the edit.","annotations":[]}]}),
        json!({"type":"function_call","id":"fc-edit","status":"completed","call_id":"call-edit","name":"edit","arguments":"{\"justification\":\"\",\"value\":1}"}),
    ];
    let mut wire = format!(
        "data: {}\n\n",
        json!({"type":"response.created","response":{"id":"response-edit","model":"physical","status":"in_progress","output":[]}})
    );
    for (index, item) in output.iter().enumerate() {
        let mut start = item.clone();
        start["status"] = json!("in_progress");
        match item["type"].as_str().unwrap() {
            "reasoning" => {
                start.as_object_mut().unwrap().remove("encrypted_content");
                start["summary"] = json!([]);
            }
            "message" => {
                start["content"] = json!([]);
            }
            "function_call" => {
                start["arguments"] = json!("");
            }
            _ => unreachable!(),
        }
        let mut frames =
            vec![json!({"type":"response.output_item.added","output_index":index,"item":start})];
        if item["type"] == "message" {
            let text = &item["content"][0]["text"];
            frames.extend([
                json!({"type":"response.output_text.delta","output_index":index,"item_id":item["id"],"content_index":0,"delta":text}),
                json!({"type":"response.output_text.done","output_index":index,"item_id":item["id"],"content_index":0,"text":text}),
            ]);
        } else if item["type"] == "function_call" {
            frames.extend([
                json!({"type":"response.function_call_arguments.delta","output_index":index,"item_id":item["id"],"delta":item["arguments"]}),
                json!({"type":"response.function_call_arguments.done","output_index":index,"item_id":item["id"],"arguments":item["arguments"]}),
            ]);
        }
        frames.push(json!({"type":"response.output_item.done","output_index":index,"item":item}));
        for frame in frames {
            wire.push_str(&format!("data: {frame}\n\n"));
        }
    }
    wire.push_str(&format!("data: {}\n\n", json!({"type":"response.completed","response":{"id":"response-edit","model":"physical","status":"completed","output":output,"usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}})));
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    decoder.feed(wire.as_bytes(), true).unwrap();
    let events = decoder.finish().unwrap().events;
    let provider = events
        .iter()
        .position(|e| matches!(e.event, ModelEvent::ProviderState { .. }))
        .unwrap();
    let finished = events
        .iter()
        .position(|e| matches!(e.event, ModelEvent::ReasoningFinished { .. }))
        .unwrap();
    assert!(
        provider < finished,
        "use actual empty-summary lifecycle order"
    );
    events
}

fn suffix(cipher: &str, summary: &str) -> Vec<serde_json::Value> {
    vec![
        json!({"type":"reasoning","id":"rs-edit","summary": if summary.is_empty() {vec![]} else {vec![json!({"type":"summary_text","text":summary})]},"encrypted_content":cipher}),
        json!({"role":"assistant","content":"Apply the edit."}),
        json!({"type":"function_call","call_id":"call-edit","name":"edit","arguments":"{\"value\":1,\"justification\":\"\"}"}),
        json!({"type":"function_call_output","call_id":"call-edit","output":"Error: invalid justification: expected a non-empty sentence"}),
    ]
}

#[test]
fn encrypted_boundary_requires_the_actual_accepted_state_across_replay_layouts() {
    for summary in ["", "Use the requested tool."] {
        let cipher = "opaque-state-".repeat(256);
        let mut captured = BoundaryCapture::default();
        for event in events(&cipher, summary) {
            captured.accept(&event.event);
        }
        let accepted = captured
            .evidence()
            .expect("known delivered ciphertext is comparable");
        for (inline_limit, prefix_bytes) in [0, 8 * 1024, 64 * 1024]
            .into_iter()
            .flat_map(|limit| [0, 8 * 1024].map(|prefix| (limit, prefix)))
        {
            for variant in ["same", "changed", "missing", "uncaptured"] {
                let (_directory, replay) = replay();
                let mut items = suffix(&cipher, summary);
                if variant == "changed" {
                    items[0]["encrypted_content"] = json!(format!("{cipher}different"));
                } else if variant == "missing" {
                    items[0]
                        .as_object_mut()
                        .unwrap()
                        .remove("encrypted_content");
                }
                let wire =
                    json!({"model":"route","instructions":"x".repeat(prefix_bytes),"input":items});
                let request = reasoning_boundary::production_request_for(
                    IngressProtocol::Responses,
                    &wire,
                    &replay,
                    inline_limit,
                );
                let proof =
                    incoming_evidence(&request, 0, &replay, &mut FingerprintBudget::default());
                let result = if variant == "uncaptured" {
                    let mut visible_only = BoundaryCapture::default();
                    for event in events(&cipher, summary) {
                        if !matches!(event.event, ModelEvent::ProviderState { .. }) {
                            visible_only.accept(&event.event);
                        }
                    }
                    proof.and_then(|p| visible_only.evidence().unwrap().matches(p))
                } else {
                    proof.and_then(|p| accepted.matches(p))
                };
                assert_eq!(
                    result,
                    (variant == "same").then_some(true),
                    "{variant}, allowance {inline_limit}, prefix {prefix_bytes}"
                );
            }
        }
    }
}

#[test]
fn encrypted_tool_error_continues_actual_stage_and_next_input_assesses_every_request() {
    for inline_limit in [0, 8 * 1024, 64 * 1024] {
        let (_directory, replay) = replay();
        let store = AgentTurnHistoryStore::default();
        let scope = store.scope_key("workspace", "encrypted-retry").unwrap();
        let begin = |wire: &serde_json::Value| {
            let request = reasoning_boundary::production_request_for(
                IngressProtocol::Responses,
                wire,
                &replay,
                inline_limit,
            );
            store
                .begin_with_admission(
                    scope.clone(),
                    plan(1),
                    store
                        .prepare_scoped_request(&scope, &request, &replay)
                        .unwrap(),
                    TurnDecisionInputs {
                        native_turn: None,
                        history_continuity: crate::context_hold::HistoryContinuity::Continuing,
                        reselect_on_user_message: true,
                    },
                    Instant::now(),
                    || Some(()),
                )
                .unwrap()
                .unwrap()
                .0
        };
        let mut wire = json!({"model":"route","input":[{"role":"user","content":"edit the file"}]});
        let (a, history) = new_turn(begin(&wire));
        drop(history);
        store.commit_decision(&a, decision("regular")).unwrap();
        store.bind_request(&a, "tool-request");
        let cipher = "opaque-state-".repeat(256);
        store.accept_output(&a, &events(&cipher, ""));
        store
            .finish_request(
                &a,
                "tool-request".into(),
                AgentTurnStatus::Completed,
                vec![execution("model", "profile", "regular", "tool-request")],
                false,
                Instant::now(),
            )
            .unwrap();
        wire["input"]
            .as_array_mut()
            .unwrap()
            .extend(suffix(&cipher, ""));
        // Assert the evidence contracts independently of branch selection so a
        // regression identifies the failed boundary, not only its final routing.
        let request = reasoning_boundary::production_request_for(
            IngressProtocol::Responses,
            &wire,
            &replay,
            inline_limit,
        );
        let analyzed = store
            .prepare_scoped_request(&scope, &request, &replay)
            .unwrap();
        {
            let inner = store.inner.lock();
            let entry = inner.entries.get(&a.key).unwrap();
            assert!(
                !entry.active.capture_partial,
                "accepted output capture must be complete"
            );
            let accepted = entry
                .response_boundary
                .expect("accepted response has a boundary");
            let anchors = analyzed
                .anchors
                .as_ref()
                .expect("actual input is fingerprintable");
            assert!(
                anchors.prefixes.contains(&(
                    entry.checkpoint.message_count,
                    entry
                        .checkpoint
                        .digest
                        .expect("original input has a fingerprint")
                )),
                "the original input prefix is unchanged"
            );
            let incoming = analyzed
                .boundaries
                .iter()
                .find(|(position, _)| *position == entry.checkpoint.message_count)
                .expect("actual replay exposes the accepted response boundary")
                .1;
            assert_eq!(
                accepted.matches(incoming),
                Some(true),
                "actual replay matches accepted opaque and visible output"
            );
        }
        let AgentTurnBegin::Continuation {
            ticket,
            decision: inherited,
        } = begin(&wire)
        else {
            panic!("the tool error is a continuation, not a new routing activation")
        };
        assert_eq!(ticket.agent_turn_id, a.agent_turn_id);
        assert_eq!(inherited.branch_id, "regular");
        assert!(
            !ticket.allow_affinity,
            "opaque equality cannot authorize provider affinity"
        );
        store.bind_request(&ticket, "recovery-request");
        accepted_output::text_output(&store, &ticket, "Recovered.");
        let completed = store
            .finish_request(
                &ticket,
                "recovery-request".into(),
                AgentTurnStatus::Completed,
                vec![execution("model", "profile", "regular", "recovery-request")],
                true,
                Instant::now(),
            )
            .unwrap()
            .unwrap();
        assert!(!completed.history_partial);
        wire["input"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":"Recovered."}),
            json!({"role":"user","content":"next actual input"}),
        ]);
        let (b, frozen) = new_turn(begin(&wire));
        assert!(
            b.allow_affinity,
            "later exact plaintext output recovers affinity"
        );
        let target = frozen.assessment_target.as_ref().unwrap();
        assert!(!target.target_partial);
        let membership = target.history_membership.as_ref().unwrap();
        assert_eq!(membership.request_ids, ["tool-request", "recovery-request"]);
        assert!(membership.association_unverified);
        assert!(!membership.capture_partial);
        assert!(
            !serde_json::to_string(&frozen.visible_conversation)
                .unwrap()
                .contains(&cipher)
        );
        store.abort(&b);
    }
}
