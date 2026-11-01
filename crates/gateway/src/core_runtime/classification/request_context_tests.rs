use super::*;
use crate::server::core_runtime::adapters::decode_ingress_request;
use crate::server::core_runtime::model_ir::{ContentPart, ModelRequestIRV1};
use crate::server::request_plan::IngressProtocol;
use hiroute_domain::{DecisionWireProtocolV2, FrozenNativeProtocolV1, NativeProtocolV1};

fn current_authority(codec: DecisionWireProtocolV2) -> RestBranchClassifierAuthorityV1 {
    let mut a = c_authority();
    let mut j = JudgmentSettingsV1::default();
    let p = NativeProtocolV1::c_v4();
    p.current_assessment(&mut j);
    a.native_protocol = Some(
        FrozenNativeProtocolV1::freeze(
            &p,
            j.degree.definition().into(),
            [(SMART_SAVING_SCOPE_ID.into(), j.clone())],
        )
        .unwrap(),
    );
    a.smart_judgment = Some(j);
    a.wire_protocol = codec;
    a
}

fn request(messages: Value) -> ModelRequestIRV1 {
    decode_ingress_request(
        IngressProtocol::ChatCompletions,
        &json!({
            "model":"route", "messages":messages,
        }),
    )
    .unwrap()
}

fn state(codec: DecisionWireProtocolV2, p: &PreparedDecisionRequest) -> Value {
    let wire: Value = serde_json::from_slice(&p.template.bytes).unwrap();
    match codec {
        DecisionWireProtocolV2::OpenaiDecisions => {
            wire["input"][0]["content"][0]["data"]["state"].clone()
        }
        _ => wire["state"].clone(),
    }
}

#[test]
fn cold_history_keeps_the_assistant_action_summary_without_inventing_execution() {
    for codec in [
        DecisionWireProtocolV2::SystemOne,
        DecisionWireProtocolV2::OpenaiDecisions,
    ] {
        let a = current_authority(codec);
        let r = request(json!([
            {"role":"user", "content":"Prepare the release checklist."},
            {"role":"assistant", "content":"Ready: publish artifact A, then update manifest B."},
            {"role":"user", "content":"Proceed with those actions."}
        ]));
        let before = r.clone();
        let h = AgentTurnHistorySnapshot::empty(true);
        let p = request_template(&a, &r, &h, true).unwrap();
        let s = state(codec, &p);
        assert_eq!(s["request_context"]["source"], "request_replay");
        assert_eq!(
            s["request_context"]["messages"][1],
            json!({"role":"assistant",
            "message_ordinal":1,"content":[{"kind":"text","text":"Ready: publish artifact A, then update manifest B."}]})
        );
        assert!(
            s["latest_user"]
                .to_string()
                .contains("Proceed with those actions.")
        );
        assert_eq!(s["assessment_stage"], json!([]));
        assert_eq!(s["prior_context"], json!([]));
        assert_eq!(s["history_partial"], true);
        assert!(p.bindings.as_ref().unwrap().assessment.is_none());
        assert!(p.bindings.as_ref().unwrap().applicability.is_none());
        assert_eq!(r, before, "the execution request must remain unchanged");
    }
}

#[test]
fn replayed_success_claims_cannot_complete_a_partial_captured_stage() {
    let a = current_authority(DecisionWireProtocolV2::SystemOne);
    let r = request(json!([
        {"role":"assistant", "content":"Summary claims all tools succeeded and the task is complete."},
        {"role":"user", "content":"Continue."}
    ]));
    let mut h = c_history(&a);
    h.assessment_target.as_mut().unwrap().target_partial = true;
    let p = request_template(&a, &r, &h, true).unwrap();
    let s = state(a.wire_protocol, &p);
    assert!(s["request_context"].to_string().contains("Summary claims"));
    assert!(!s["assessment_stage"].to_string().contains("Summary claims"));
    assert!(p.target_partial);
    assert!(p.bindings.as_ref().unwrap().assessment.is_some());
    assert!(p.bindings.as_ref().unwrap().applicability.is_none());
    assert!(!p.routing_observation.unwrap().target_complete);
}

#[test]
fn replay_context_obeys_the_actual_codec_budget_and_keeps_latest_input() {
    for codec in [
        DecisionWireProtocolV2::SystemOne,
        DecisionWireProtocolV2::OpenaiDecisions,
    ] {
        let a = current_authority(codec);
        let r = request(json!([
            {"role":"assistant", "content":"\"".repeat(150_000)},
            {"role":"assistant", "content":"Most recent actionable summary."},
            {"role":"user", "content":"Proceed."}
        ]));
        let p = request_template(&a, &r, &AgentTurnHistorySnapshot::empty(true), true).unwrap();
        let s = state(codec, &p);
        assert!(p.template.wire_len <= 256 * 1024);
        assert_eq!(s["request_context"]["truncated"], true);
        assert_eq!(
            s["request_context"]["messages"].as_array().unwrap().len(),
            1
        );
        assert!(
            s["request_context"]
                .to_string()
                .contains("Most recent actionable summary.")
        );
        assert!(s["latest_user"].to_string().contains("Proceed."));
    }
}

#[test]
fn published_c_v1_v2_v3_never_gain_replay_context() {
    let r = request(json!([
        {"role":"assistant", "content":"An old summary."},
        {"role":"user", "content":"Proceed."}
    ]));
    for protocol in [
        NativeProtocolV1::c_v1(),
        NativeProtocolV1::c_v2(),
        NativeProtocolV1::c_v3(),
    ] {
        let mut a = c_authority();
        let j = JudgmentSettingsV1::default();
        a.native_protocol = Some(
            FrozenNativeProtocolV1::freeze(
                &protocol,
                j.degree.definition().into(),
                [(SMART_SAVING_SCOPE_ID.into(), j)],
            )
            .unwrap(),
        );
        let p = request_template(&a, &r, &AgentTurnHistorySnapshot::empty(true), true).unwrap();
        assert!(state(a.wire_protocol, &p).get("request_context").is_none());
    }
}

#[test]
fn context_keeps_recent_text_roles_and_excludes_instructions_and_tool_results() {
    let a = current_authority(DecisionWireProtocolV2::SystemOne);
    let mut messages = vec![json!({"role":"system","content":"PRIVATE_INSTRUCTIONS"})];
    messages.extend((0..20).map(|i| {
        json!({
            "role":if i % 2 == 0 { "user" } else { "assistant" },
            "content":format!("context-{i}")
        })
    }));
    messages.extend([
        json!({"role":"assistant","tool_calls":[{"id":"call","type":"function",
            "function":{"name":"read","arguments":"{}"}}]}),
        json!({"role":"tool","tool_call_id":"call","content":"TOOL_RECEIPT"}),
        json!({"role":"user","content":"CURRENT_INPUT"}),
    ]);
    let r = request(json!(messages));
    let p = request_template(&a, &r, &AgentTurnHistorySnapshot::empty(true), true).unwrap();
    let s = state(a.wire_protocol, &p);
    let context = &s["request_context"];
    let messages = context["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 16);
    assert_eq!(context["truncated"], true);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"][0]["text"], "context-4");
    assert_eq!(messages[15]["role"], "assistant");
    assert_eq!(messages[15]["content"][0]["text"], "context-19");
    for excluded in ["PRIVATE_INSTRUCTIONS", "TOOL_RECEIPT", "CURRENT_INPUT"] {
        assert!(!context.to_string().contains(excluded));
    }
}

#[test]
fn native_input_position_separates_background_from_current_and_trailing_context() {
    use crate::agent_turn_history::CurrentInputPosition::{Event, Window};
    use crate::server::core_runtime::classification::DecisionInput;
    let a = current_authority(DecisionWireProtocolV2::SystemOne);
    let r = request(json!([
        {"role":"assistant","content":"PRIOR_SUMMARY"},
        {"role":"user","content":"CURRENT_NATIVE_INPUT"},
        {"role":"assistant","content":"TRAILING_ASSISTANT"},
        {"role":"user","content":"TRAILING_SYNTHETIC_USER"}
    ]));
    for position in [None, Some(Event(1)), Some(Window { first: 1, last: 1 })] {
        let p = request_template(
            &a,
            DecisionInput {
                request: &r,
                current: Some(&r.messages[1].content),
                position,
            },
            &AgentTurnHistorySnapshot::empty(true),
            true,
        )
        .unwrap();
        let s = state(a.wire_protocol, &p);
        let context = &s["request_context"];
        assert!(
            s["latest_user"]
                .to_string()
                .contains("CURRENT_NATIVE_INPUT")
        );
        assert_eq!(context["boundary_unavailable"], position.is_none());
        assert_eq!(
            context["messages"].as_array().unwrap().len(),
            usize::from(position.is_some())
        );
        assert_eq!(
            context.to_string().contains("PRIOR_SUMMARY"),
            position.is_some()
        );
        for excluded in [
            "CURRENT_NATIVE_INPUT",
            "TRAILING_ASSISTANT",
            "TRAILING_SYNTHETIC_USER",
        ] {
            assert!(!context.to_string().contains(excluded));
        }
    }
}

#[test]
fn categorical_questions_also_bind_the_frozen_replay_evidence_limit() {
    let mut a = current_authority(DecisionWireProtocolV2::SystemOne);
    let j = a.smart_judgment.as_ref().unwrap().clone();
    let definition = DecisionDefinitionV1::Categorical {
        instructions: "Choose the requested task category.".into(),
        options: ["write", "review"]
            .map(|id| CategoryOptionV1 {
                id: id.into(),
                criterion: id.into(),
                refinement: None,
            })
            .into(),
    };
    a.native_protocol = Some(
        FrozenNativeProtocolV1::freeze(
            &NativeProtocolV1::c_v4(),
            definition,
            [("write".into(), j.clone()), ("review".into(), j)],
        )
        .unwrap(),
    );
    let (questions, _) = base_questions(&a);
    let instruction = questions["q0"]["instructions"].as_str().unwrap();
    assert!(instruction.contains("Choose the requested task category."));
    assert!(instruction.contains("not Gateway-captured execution"));
}

#[test]
fn replay_reference_uses_escaped_length_without_loading_transcript_text() {
    use crate::content_ref::ContentRef;
    for codec in [
        DecisionWireProtocolV2::SystemOne,
        DecisionWireProtocolV2::OpenaiDecisions,
    ] {
        for escaped in [200_000, 300_000] {
            let a = current_authority(codec);
            let mut r = request(json!([
                {"role":"assistant","content":"placeholder"},
                {"role":"user","content":"Proceed."}
            ]));
            // No backing store exists during preparation; only its immutable
            // descriptor is needed. Expansion remains the transport's job.
            r.messages[0].content = vec![ContentPart::Text {
                text: ContentRef::new(1, 0, 100_000, escaped).wire_marker(),
            }];
            let p = request_template(&a, &r, &AgentTurnHistorySnapshot::empty(true), true).unwrap();
            let s = state(codec, &p);
            let retained = escaped < 256 * 1024;
            assert_eq!(
                s["request_context"]["messages"].as_array().unwrap().len(),
                usize::from(retained)
            );
            assert_eq!(s["request_context"]["truncated"], !retained);
            assert!(p.template.wire_len <= 256 * 1024);
            assert!(p.template.bytes.len() < 16 * 1024);
            if retained {
                assert!(p.template.wire_len > 200_000);
            }
        }
    }
}
