use super::*;
use crate::server::core_runtime::classification::{CallFailure, adaptive_request};
use hiroute_domain::{
    CanonicalDigest, DECISION_CONTEXT_ALGORITHM_V1, DECISION_CONTEXT_RESERVE_V1,
    DecisionContextSourceV1, ResolvedDecisionContextV1,
};

fn authority(
    codec: DecisionWireProtocolV2,
    window: u32,
    question: u32,
) -> RestBranchClassifierAuthorityV1 {
    let mut a = current_authority(codec);
    let mut context = ResolvedDecisionContextV1 {
        request_window_tokens: window,
        per_question_window_tokens: question,
        request_source: DecisionContextSourceV1::User,
        per_question_source: DecisionContextSourceV1::User,
        catalog_revision: 1,
        catalog_digest: CanonicalDigest::of_bytes(b"catalog"),
        metadata: None,
        typesafe_long_context_header: false,
        accounting_algorithm: DECISION_CONTEXT_ALGORITHM_V1.into(),
        reserve_policy: DECISION_CONTEXT_RESERVE_V1.into(),
        context_digest: CanonicalDigest::of_bytes(&[]),
    };
    context.context_digest = context.calculate_digest().unwrap();
    a.native_protocol = Some(
        a.native_protocol
            .take()
            .unwrap()
            .with_context(Some(&context))
            .unwrap(),
    );
    a.context_budget = Some(context);
    a
}

fn turn(user: &str, steps: Vec<String>) -> Arc<VisibleAgentTurn> {
    Arc::new(VisibleAgentTurn {
        branch_id: Some(SMART_SAVING_SCOPE_ID.into()),
        executed_branch_id: Some(SMART_SAVING_SCOPE_ID.into()),
        user: vec![VisibleContentPart::Text { text: user.into() }],
        status: AgentTurnStatus::Completed,
        steps: steps
            .into_iter()
            .map(|text| vec![VisibleContentPart::Text { text }])
            .collect(),
    })
}

#[test]
fn adaptive_replay_retains_cold_summaries_but_never_completes_a_partial_target() {
    for codec in [
        DecisionWireProtocolV2::SystemOne,
        DecisionWireProtocolV2::OpenaiDecisions,
    ] {
        let summary = "Ready: publish artifact A, then update manifest B.";
        let r = request(json!([
            {"role":"assistant","content":summary},
            {"role":"user","content":"Proceed."}
        ]));
        let original = r.clone();
        let a = authority(codec, 32_000, 32_000);
        for (partial, captured_execution) in [(false, false), (true, false), (true, true)] {
            let mut h = if partial {
                c_history(&a)
            } else {
                AgentTurnHistorySnapshot::empty(true)
            };
            if partial {
                h.assessment_target.as_mut().unwrap().target_partial = true;
                if captured_execution {
                    h.visible_conversation[0] = turn(
                        "Previous work",
                        vec!["A genuinely captured earlier execution step.".into()],
                    );
                }
            }
            let p = request_template(&a, &r, &h, true).unwrap();
            assert_wire_budgets(codec, &a, &p);
            let s = state(codec, &p);
            assert_eq!(
                s["request_context"]["messages"][0]["content"][0]["text"],
                summary
            );
            assert!(!s["assessment_stage"].to_string().contains(summary));
            assert!(!s["prior_context"].to_string().contains(summary));
            assert!(p.bindings.as_ref().unwrap().applicability.is_none());
            assert_eq!(
                p.bindings.as_ref().unwrap().assessment.is_some(),
                captured_execution
            );
            assert!(!p.routing_observation.as_ref().unwrap().target_complete);
            assert_eq!(r, original);
        }
    }
}

#[test]
fn adaptive_replay_uses_real_reference_lengths_and_never_slices_a_large_summary() {
    use crate::content_ref::ContentRef;
    for codec in [
        DecisionWireProtocolV2::SystemOne,
        DecisionWireProtocolV2::OpenaiDecisions,
    ] {
        let mut r = request(json!([
            {"role":"assistant","content":"placeholder"},
            {"role":"user","content":"Proceed."}
        ]));
        r.messages[0].content = vec![ContentPart::Text {
            text: ContentRef::new(1, 0, 100_000, 300_000).wire_marker(),
        }];
        for window in [32_000, 1_050_000] {
            let a = authority(codec, window, window);
            let p = request_template(&a, &r, &AgentTurnHistorySnapshot::empty(true), true).unwrap();
            let s = state(codec, &p);
            let large = window > 300_000;
            assert_eq!(
                s["request_context"]["messages"].as_array().unwrap().len(),
                usize::from(large)
            );
            assert_eq!(s["request_context"]["truncated"], !large);
            assert!(
                p.template.wire_len
                    <= a.context_budget.as_ref().unwrap().request_usable_tokens() as usize
            );
            assert!(
                p.template.bytes.len() < 16 * 1024,
                "preparation must not load replay text"
            );
            if large {
                assert!(p.template.wire_len > 300_000);
            }
            let observation = p.routing_observation.unwrap();
            assert!(observation.validate());
            assert_eq!(
                observation.context_budget.unwrap().replay_budget_omitted,
                !large
            );
        }
    }
}

#[test]
fn adaptive_native_input_window_excludes_current_and_trailing_context() {
    use crate::agent_turn_history::CurrentInputPosition::{Event, Window};
    use crate::server::core_runtime::classification::DecisionInput;
    for codec in [
        DecisionWireProtocolV2::SystemOne,
        DecisionWireProtocolV2::OpenaiDecisions,
    ] {
        let a = authority(codec, 32_000, 32_000);
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
            assert_wire_budgets(codec, &a, &p);
            let s = state(codec, &p);
            assert_eq!(s["latest_user"][0]["text"], "CURRENT_NATIVE_INPUT");
            let replay = &s["request_context"];
            assert_eq!(replay["boundary_unavailable"], position.is_none());
            assert_eq!(
                replay.to_string().contains("PRIOR_SUMMARY"),
                position.is_some()
            );
            for excluded in [
                "CURRENT_NATIVE_INPUT",
                "TRAILING_ASSISTANT",
                "TRAILING_SYNTHETIC_USER",
            ] {
                assert!(!replay.to_string().contains(excluded));
            }
        }
    }
}

fn assert_wire_budgets(
    codec: DecisionWireProtocolV2,
    a: &RestBranchClassifierAuthorityV1,
    prepared: &PreparedDecisionRequest,
) {
    let context = a.context_budget.as_ref().unwrap();
    assert!(prepared.template.wire_len <= context.request_usable_tokens() as usize);
    assert_eq!(prepared.template.wire_len, prepared.template.bytes.len());
    let wire: Value = serde_json::from_slice(&prepared.template.bytes).unwrap();
    let singles = match codec {
        DecisionWireProtocolV2::SystemOne => wire["questions"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, q)| {
                let mut single = wire.clone();
                single["questions"] =
                    Value::Object(serde_json::Map::from_iter([(name.clone(), q.clone())]));
                single
            })
            .collect::<Vec<_>>(),
        DecisionWireProtocolV2::OpenaiDecisions => wire["questions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|q| {
                let mut single = wire.clone();
                single["questions"] = json!([q]);
                single
            })
            .collect(),
    };
    for single in singles {
        assert!(
            serde_json::to_vec(&single).unwrap().len()
                <= context.per_question_usable_tokens() as usize
        );
    }
    assert!(prepared.routing_observation.as_ref().unwrap().validate());
}

#[test]
fn larger_window_admits_above_legacy_ceiling_and_small_window_keeps_whole_latest_steps() {
    for codec in [
        DecisionWireProtocolV2::SystemOne,
        DecisionWireProtocolV2::OpenaiDecisions,
    ] {
        let r = request(
            json!([{ "role":"user", "content":"Execute the current requirement exactly." }]),
        );
        let original = r.clone();
        let mut steps = (0..23)
            .map(|i| format!("step {i}: {}", "中文😀\\\"".repeat(1200)))
            .collect::<Vec<_>>();
        // Keep the history above the legacy ceiling, with a complete short tail
        // that fits beside the current frozen questions in the small window.
        // Large latest steps that cannot fit are covered separately below.
        steps.push("Latest complete execution step: 中文😀\\\"".repeat(8));
        let large = authority(codec, 1_050_000, 1_050_000);
        let mut h = c_history(&large);
        h.visible_conversation = vec![turn("Full prior user requirement", steps.clone())];
        let p = request_template(&large, &r, &h, false).unwrap();
        assert_wire_budgets(codec, &large, &p);
        assert!(p.template.wire_len > 262_144);
        assert!(!p.target_partial);
        assert_eq!(
            state(codec, &p)["assessment_stage"][0]["steps"]
                .as_array()
                .unwrap()
                .len(),
            24
        );
        let small = authority(codec, 32_000, 32_000);
        let mut small_history = c_history(&small);
        small_history.visible_conversation = h.visible_conversation.clone();
        let p = request_template(&small, &r, &small_history, false).unwrap();
        assert_wire_budgets(codec, &small, &p);
        let s = state(codec, &p);
        assert!(p.target_partial);
        assert_eq!(
            s["assessment_stage"][0]["user"][0]["text"],
            "Full prior user requirement"
        );
        assert_eq!(
            s["assessment_stage"][0]["steps"][0][0]["source_kind"],
            "context_budget"
        );
        let retained = s["assessment_stage"][0]["steps"].as_array().unwrap();
        let retained_execution = &retained[1..];
        assert!(!retained_execution.is_empty());
        assert!(retained_execution.len() < steps.len());
        assert_eq!(
            Value::Array(retained_execution.to_vec()),
            serde_json::to_value(
                &h.visible_conversation[0].steps[steps.len() - retained_execution.len()..]
            )
            .unwrap(),
            "{codec:?}: retained steps must be an exact whole suffix"
        );
        assert_eq!(
            s["latest_user"][0]["text"],
            "Execute the current requirement exactly."
        );
        assert!(p.bindings.as_ref().unwrap().applicability.is_none());
        assert!(p.bindings.as_ref().unwrap().assessment.is_some());
        assert_eq!(h.visible_conversation[0].steps.len(), 24);
        assert_eq!(r, original);
    }
}

#[test]
fn independent_question_window_limits_shared_state_and_shape_can_trim_optional_steps() {
    for codec in [
        DecisionWireProtocolV2::SystemOne,
        DecisionWireProtocolV2::OpenaiDecisions,
    ] {
        let a = authority(codec, 100_000, 20_000);
        let mut h = c_history(&a);
        h.visible_conversation = vec![turn("user", vec!["a".repeat(9000), "b".repeat(9000)])];
        let p = request_template(&a, &diagnostic_request(), &h, false).unwrap();
        assert_wire_budgets(codec, &a, &p);
        assert!(p.target_partial);
        assert_eq!(
            p.routing_observation
                .as_ref()
                .unwrap()
                .context_budget
                .as_ref()
                .unwrap()
                .limiting_constraint,
            hiroute_domain::DecisionContextConstraintV1::Question
        );
    }
    let codec = DecisionWireProtocolV2::OpenaiDecisions;
    let a = authority(codec, 1_050_000, 1_050_000);
    let mut h = c_history(&a);
    h.visible_conversation = vec![turn("user", vec!["small".into(); 4000])];
    let p = request_template(&a, &diagnostic_request(), &h, false).unwrap();
    assert_wire_budgets(codec, &a, &p);
    assert!(p.target_partial);
    assert!(
        crate::server::core_runtime::classification::openai_decisions::state_value_count(&state(
            codec, &p
        ))
        .is_ok()
    );
    assert_eq!(
        p.routing_observation
            .as_ref()
            .unwrap()
            .context_budget
            .as_ref()
            .unwrap()
            .limiting_constraint,
        hiroute_domain::DecisionContextConstraintV1::JsonShape
    );
}

#[test]
fn failed_historical_reservation_omits_evidence_without_refilling_and_mandatory_no_fit_is_local() {
    for codec in [
        DecisionWireProtocolV2::SystemOne,
        DecisionWireProtocolV2::OpenaiDecisions,
    ] {
        let r = request(json!([{ "role":"user", "content":"Current" }]));
        let large = authority(codec, 100_000, 100_000);
        let baseline =
            request_template(&large, &r, &AgentTurnHistorySnapshot::empty(false), false).unwrap();
        let window = u32::try_from(baseline.template.wire_len).unwrap() + 1024;
        let a = authority(codec, window, window);
        let mut h = c_history(&a);
        h.visible_conversation = vec![turn("prior requirement", vec!["actual execution".into()])];
        let p = request_template(&a, &r, &h, false).unwrap();
        assert_wire_budgets(codec, &a, &p);
        assert!(p.target_partial);
        assert_eq!(state(codec, &p)["assessment_stage"], json!([]));
        assert!(p.bindings.as_ref().unwrap().assessment.is_none());
        assert!(p.bindings.as_ref().unwrap().applicability.is_none());
        let tiny = authority(codec, 1024, 1024);
        let error = request_template(&tiny, &r, &h, false).err().unwrap();
        let CallFailure::BudgetRejected(rejection) = error else {
            panic!("expected local budget rejection")
        };
        let o = adaptive_request::rejected_observation(&tiny, &h, rejection).unwrap();
        assert!(o.validate());
        assert!(o.context_budget.unwrap().encoded_bytes.is_none());
    }
}

#[test]
fn newest_oversized_step_is_not_skipped_and_user_only_capture_cannot_request_assessment() {
    for codec in [
        DecisionWireProtocolV2::SystemOne,
        DecisionWireProtocolV2::OpenaiDecisions,
    ] {
        let a = authority(codec, 32_000, 32_000);
        let r = request(json!([{ "role":"user", "content":"Keep the current requirement." }]));
        let original = r.clone();
        let mut h = c_history(&a);
        h.visible_conversation = vec![turn(
            "whole user",
            vec!["small older step".into(), "x".repeat(100_000)],
        )];
        let p = request_template(&a, &r, &h, false).unwrap();
        assert_wire_budgets(codec, &a, &p);
        assert!(p.target_partial);
        assert_eq!(state(codec, &p)["assessment_stage"], json!([]));
        assert!(p.bindings.as_ref().unwrap().assessment.is_none());
        assert!(p.bindings.as_ref().unwrap().applicability.is_none());
        assert_eq!(
            state(codec, &p)["latest_user"][0]["text"],
            "Keep the current requirement."
        );
        assert_eq!(h.visible_conversation[0].steps.len(), 2);
        assert_eq!(r, original);
        h.visible_conversation = vec![turn("user-only is not execution", vec![])];
        let p = request_template(&a, &r, &h, false).unwrap();
        assert!(p.bindings.as_ref().unwrap().assessment.is_none());
        assert!(p.bindings.as_ref().unwrap().applicability.is_none());
        assert!(!p.routing_observation.as_ref().unwrap().target_complete);
    }
}

#[test]
fn target_has_no_eight_turn_cap_and_any_target_loss_excludes_captured_background() {
    for codec in [
        DecisionWireProtocolV2::SystemOne,
        DecisionWireProtocolV2::OpenaiDecisions,
    ] {
        let a = authority(codec, 262_144, 262_144);
        let mut h = c_history(&a);
        h.visible_conversation = (0..20)
            .map(|i| turn(&format!("turn {i}"), vec!["execution".into()]))
            .collect();
        h.assessment_from = Some(10);
        let p = request_template(&a, &diagnostic_request(), &h, false).unwrap();
        assert_wire_budgets(codec, &a, &p);
        let s = state(codec, &p);
        assert_eq!(s["assessment_stage"].as_array().unwrap().len(), 10);
        assert_eq!(s["prior_context"].as_array().unwrap().len(), 8);
        h.visible_conversation[10] = turn(&"user".repeat(100_000), vec!["execution".into()]);
        let p = request_template(&a, &diagnostic_request(), &h, false).unwrap();
        assert_wire_budgets(codec, &a, &p);
        assert!(p.target_partial);
        assert_eq!(state(codec, &p)["prior_context"], json!([]));
        assert_eq!(
            state(codec, &p)["assessment_stage"]
                .as_array()
                .unwrap()
                .len(),
            9
        );
    }
}
