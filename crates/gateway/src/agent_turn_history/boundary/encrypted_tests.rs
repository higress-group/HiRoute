use super::*;
use crate::agent_turn_history::boundary::{BoundaryCapture, incoming_evidence};
use crate::context_hold::fingerprint::FingerprintBudget;
use crate::server::core_runtime::model_ir::{
    ModelEvent, ModelStreamEventV1, OpaqueProviderState, ResponseItemStatus,
};

fn events(cipher: &str) -> Vec<ModelStreamEventV1> {
    vec![
        ModelEvent::ReasoningFinished {
            index: 0,
            text: String::new(),
            status: ResponseItemStatus::Completed,
        },
        ModelEvent::ProviderState {
            state: Box::new(OpaqueProviderState {
                block_index: Some(0),
                kind: "encrypted_content".into(),
                value: json!(cipher),
            }),
        },
        ModelEvent::TextFinished {
            index: 1,
            text: "Apply the edit.".into(),
            annotations: vec![],
            status: ResponseItemStatus::Completed,
        },
        ModelEvent::ToolCallStarted {
            index: 2,
            logical_id: "call-edit".into(),
            native_id: "call-edit".into(),
            tool_kind: ToolKindV1::Function,
            namespace: None,
            name: "edit".into(),
            owner: Box::new(accepted_output::owner()),
            item_id: None,
        },
        ModelEvent::ToolCallFinished {
            index: 2,
            logical_id: "call-edit".into(),
            tool_kind: ToolKindV1::Function,
            namespace: None,
            name: "edit".into(),
            arguments: json!({"justification":"", "value":1}),
            status: ResponseItemStatus::Completed,
        },
    ]
    .into_iter()
    .enumerate()
    .map(|(i, event)| ModelStreamEventV1::new(i as u64, event))
    .collect()
}

fn suffix(cipher: &str) -> Vec<serde_json::Value> {
    vec![
        json!({"type":"reasoning","id":"rs-edit","summary":[],"encrypted_content":cipher}),
        json!({"role":"assistant","content":"Apply the edit."}),
        json!({"type":"function_call","call_id":"call-edit","name":"edit","arguments":"{\"value\":1,\"justification\":\"\"}"}),
        json!({"type":"function_call_output","call_id":"call-edit","output":"Error: invalid justification: expected a non-empty sentence"}),
    ]
}

#[test]
fn encrypted_boundary_requires_the_actual_accepted_state_across_replay_layouts() {
    let cipher = "opaque-state-".repeat(256);
    let mut captured = BoundaryCapture::default();
    for event in events(&cipher) {
        captured.accept(&event.event);
    }
    let accepted = captured
        .evidence()
        .expect("known delivered ciphertext is comparable");
    for inline_limit in [0, 8 * 1024, 64 * 1024] {
        for variant in ["same", "changed", "missing", "uncaptured"] {
            let (_directory, replay) = replay();
            let mut items = suffix(&cipher);
            if variant == "changed" {
                items[0]["encrypted_content"] = json!(format!("{cipher}different"));
            } else if variant == "missing" {
                items[0]
                    .as_object_mut()
                    .unwrap()
                    .remove("encrypted_content");
            }
            let wire = json!({"model":"route","instructions":"x".repeat(8 * 1024),"input":items});
            let request = reasoning_boundary::production_request_for(
                IngressProtocol::Responses,
                &wire,
                &replay,
                inline_limit,
            );
            let proof = incoming_evidence(&request, 0, &replay, &mut FingerprintBudget::default());
            let result = if variant == "uncaptured" {
                let mut visible_only = BoundaryCapture::default();
                for event in events(&cipher) {
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
                "{variant}, allowance {inline_limit}"
            );
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
        store.accept_output(&a, &events(&cipher));
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
            .extend(suffix(&cipher));
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
