use super::*;
use crate::agent_turn_history::{
    AgentTurnStatus, AssessmentTarget, ExecutionAttribution, PlanSnapshot, VisibleAgentTurn,
    VisibleContentPart,
};
use crate::server::core_runtime::classification::diagnostic_request;
use hiroute_domain::{BranchExecutionV1, CategoryOptionV1, ExecutionGroupV1, JudgmentSettingsV1};
use std::sync::Arc;

fn authority() -> RestBranchClassifierAuthorityV1 {
    let mode = hiroute_domain::ComplexityClassifierModeV1::DecisionService {
        service: Box::new(hiroute_domain::DecisionServiceV1 {
            id: "decision-fixture".into(),
            revision: 1,
            name: "Fixture extension".into(),
            connection: hiroute_domain::DecisionConnectionV1::Custom {
                endpoint: "http://127.0.0.1:8081/v1/decisions".into(),
                timeout_ms: 3000,
                auth_header: None,
            },
        }),
    };
    let mut a =
        (*crate::server::publication::compile_classifier_mode_authority(&mode, 1).unwrap()).clone();
    a.system_one_model = Some("decision-model-preview".into());
    a
}
fn history(with_target: bool) -> AgentTurnHistorySnapshot {
    let judgment = JudgmentSettingsV1::default();
    AgentTurnHistorySnapshot {
        previous_decision: None,
        history_partial: false,
        _pin: None,
        visible_conversation: vec![Arc::new(VisibleAgentTurn {
            branch_id: Some(SMART_SAVING_SCOPE_ID.into()),
            executed_branch_id: Some(SMART_SAVING_SCOPE_ID.into()),
            user: vec![VisibleContentPart::Text {
                text: "Previous work".into(),
            }],
            status: AgentTurnStatus::Completed,
            steps: vec![],
        })],
        assessment_from: with_target.then_some(0),
        assessment_target: with_target.then(|| AssessmentTarget {
            evaluation_scope: None,
            history_membership: None,
            branch_execution: Some(BranchExecutionV1 {
                policy: judgment.execution_policy("Smart saving"),
                group: ExecutionGroupV1::Regular,
                candidate_index: 0,
            }),
            segment_id: "stage-1".into(),
            first_turn_id: "turn-1".into(),
            through_turn_id: "turn-1".into(),
            first_ordinal: 1,
            through_ordinal: 1,
            target_partial: false,
            plan: PlanSnapshot {
                plan_id: "plan-1".into(),
                plan_revision: 1,
            },
            attribution: ExecutionAttribution::Single {
                selected_branch_id: SMART_SAVING_SCOPE_ID.into(),
                executed_branch_id: SMART_SAVING_SCOPE_ID.into(),
                model_configuration_id: "secret-model".into(),
                profile_digest: "secret-profile".into(),
            },
        }),
    }
}
#[test]
fn questions_are_opaque_and_degree_uses_probabilities_but_competence_uses_raw_score() {
    let prepared =
        request_template(&authority(), &diagnostic_request(), &history(true), false).unwrap();
    let body: Value = serde_json::from_slice(&prepared.template.bytes).unwrap();
    assert_eq!(body["questions"].as_object().unwrap().len(), 2);
    assert_eq!(
        body["questions"]["q0"]["criteria"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        body["questions"]["q1"]["criteria"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(body["state"]["assessment_from"], 0);
    assert!(!String::from_utf8_lossy(&prepared.template.bytes).contains("secret-model"));
    let output = parse_response(br#"{"answers":{"q0":{"type":"score","score":0.1,"confidence":0.1,"probabilities":{"0":0.8,"1":0.2}},"q1":{"type":"score","score":1.2,"confidence":0.1,"probabilities":{"0":1,"1":0,"2":0}}}}"#, prepared.bindings.as_ref().unwrap()).unwrap();
    assert_eq!(output.probabilities.unwrap()["simple"], 0.8);
    assert_eq!(output.assessment.unwrap().score, 0.6);
}
#[test]
fn categorical_reduction_ignores_unselected_malformed_degree() {
    let mut a = authority();
    a.decision = DecisionDefinitionV1::Categorical {
        instructions: "Main intent".into(),
        options: vec![
            CategoryOptionV1 {
                id: "write".into(),
                criterion: "Write".into(),
                refinement: Some(JudgmentSettingsV1::default().degree.definition()),
            },
            CategoryOptionV1 {
                id: "review".into(),
                criterion: "Review".into(),
                refinement: Some(JudgmentSettingsV1::default().degree.definition()),
            },
        ],
    };
    let prepared = request_template(&a, &diagnostic_request(), &history(false), false).unwrap();
    let body: Value = serde_json::from_slice(&prepared.template.bytes).unwrap();
    assert_eq!(body["questions"]["q0"]["type"], "choice");
    assert_eq!(body["questions"].as_object().unwrap().len(), 3);
    let bytes = br#"{"answers":{"q0":{"type":"choice","choice":"review"},"q1":{"garbage":true},"q2":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#;
    let output = parse_response(bytes, prepared.bindings.as_ref().unwrap()).unwrap();
    assert_eq!(output.branch_id, "review");
    assert_eq!(output.probabilities.unwrap()["simple"], 0.9);
    let failed = parse_response(
        br#"{"answers":{"q0":{"type":"choice","choice":"review"},"q1":null}}"#,
        prepared.bindings.as_ref().unwrap(),
    )
    .unwrap();
    assert_eq!(failed.branch_id, "review");
    assert!(failed.degree_failed);
    assert!(
        parse_response(
            br#"{"answers":{"q0":{"type":"choice","choice":"unknown"}}}"#,
            prepared.bindings.as_ref().unwrap()
        )
        .is_err()
    );
}
#[test]
fn oversized_current_input_is_rejected_and_history_removal_marks_only_affected_target() {
    let a = authority();
    let mut request = diagnostic_request();
    request.messages[0].content = vec![crate::server::core_runtime::model_ir::ContentPart::Text {
        text: "界".repeat(MAX_REQUEST_BYTES),
    }];
    assert!(matches!(
        request_template(&a, &request, &history(false), false),
        Err(CallFailure::RejectedInput)
    ));
    let mut h = history(true);
    let mut oversized = (*h.visible_conversation[0]).clone();
    oversized.user = vec![VisibleContentPart::Text {
        text: "x".repeat(MAX_REQUEST_BYTES),
    }];
    h.visible_conversation.insert(0, Arc::new(oversized));
    let prepared = request_template(&a, &diagnostic_request(), &h, false).unwrap();
    assert!(prepared.bindings.as_ref().unwrap().assessment.is_some() && prepared.target_partial);
    let body: Value = serde_json::from_slice(&prepared.template.bytes).unwrap();
    assert_eq!(
        body["state"]["visible_conversation"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(body["state"]["assessment_from"], 0);
    assert_eq!(body["state"]["history_partial"], true);
}

fn c_authority() -> RestBranchClassifierAuthorityV1 {
    let mut a = authority();
    let p = hiroute_domain::NativeProtocolV1::c_v1();
    let mut j = JudgmentSettingsV1::default();
    p.stock_assessment(&mut j);
    a.decision = j.degree.definition().into();
    a.smart_judgment = Some(j.clone());
    a.native_protocol = Some(
        hiroute_domain::FrozenNativeProtocolV1::freeze(
            &p,
            a.decision.clone(),
            [(SMART_SAVING_SCOPE_ID.into(), j)],
        )
        .unwrap(),
    );
    a
}

#[test]
fn c_composite_stage_keeps_quality_question_without_single_model_protection() {
    let authority = c_authority();
    let mut history = c_history(&authority);
    let target = history.assessment_target.as_mut().unwrap();
    let policy = target.branch_execution.take().unwrap().policy;
    target.attribution = ExecutionAttribution::Mixed;
    target.evaluation_scope = Some(hiroute_domain::StageEvaluationScopeV1 {
        version: 1,
        executed_branch_id: SMART_SAVING_SCOPE_ID.into(),
        policy,
    });
    let prepared = request_template(&authority, &diagnostic_request(), &history, true).unwrap();
    let bindings = prepared.bindings.as_ref().unwrap();
    assert!(
        bindings.assessment.is_some(),
        "joint work is still evaluated"
    );
    assert!(
        bindings.applicability.is_none(),
        "joint quality is not single-model protection"
    );
    let body: Value = serde_json::from_slice(&prepared.template.bytes).unwrap();
    assert_eq!(
        body["state"]["assessment_stage"].as_array().unwrap().len(),
        1
    );
    assert_eq!(body["state"]["history_partial"], false);
}
fn c_history(a: &RestBranchClassifierAuthorityV1) -> AgentTurnHistorySnapshot {
    let mut h = history(true);
    h.assessment_target
        .as_mut()
        .unwrap()
        .branch_execution
        .as_mut()
        .unwrap()
        .policy = a.native_protocol.as_ref().unwrap().execution_policy(
        SMART_SAVING_SCOPE_ID,
        a.smart_judgment.as_ref().unwrap(),
        "Smart saving",
    );
    h
}
#[test]
fn c_stock_questions_exactly_match_the_appendix_and_isolate_invalid_q3() {
    let a = c_authority();
    let h = c_history(&a);
    let p = request_template(&a, &diagnostic_request(), &h, false).unwrap();
    let wire: Value = serde_json::from_slice(&p.template.bytes).unwrap();
    assert_eq!(wire["questions"], hiroute_domain::stock_c_questions());
    assert!(wire["state"].get("visible_conversation").is_none());
    assert!(wire["state"].get("assessment_from").is_none());
    let valid = r#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}},"q1":{"type":"score","score":0.2},"q2":{"type":"choice","choice":"not_established","probabilities":{"applicable":0.5,"not_established":0.5}}}}"#;
    let response = parse_response(valid.as_bytes(), p.bindings.as_ref().unwrap()).unwrap();
    assert_eq!(response.applicability, Some(0.5));
    assert_eq!(response.assessment.unwrap().score, 0.1);
    for q3 in [
        "null",
        r#"{"type":"choice","choice":"not_established"}"#,
        r#"{"type":"choice","probabilities":{"applicable":0.2,"applicable":0.3,"not_established":0.5}}"#,
        r#"{"type":"choice","probabilities":{"applicable":-0.1,"not_established":1.1}}"#,
        r#"{"type":"choice","probabilities":{"other":0.2,"not_established":0.8}}"#,
        r#"{"type":"choice","probabilities":{"applicable":0.2,"not_established":0.2}}"#,
        r#"{"type":"choice","probabilities":{"applicable":1e999,"not_established":0}}"#,
        r#"{"type":"score","probabilities":{"applicable":0.5,"not_established":0.5}}"#,
        r#"{"type":"choice","probabilities":{"applicable":0.5,"not_established":0.5,"other":0}}"#,
    ] {
        let mut body: Value = serde_json::from_str(valid).unwrap();
        body["answers"].as_object_mut().unwrap().remove("q2");
        // Insert the raw value to retain duplicate probability keys for the negative case.
        let text = serde_json::to_string(&body).unwrap();
        let end = text.rfind('}').unwrap() - 1;
        let text = format!("{},\"q2\":{q3}{}}}", &text[..end], "}");
        let response = parse_response(text.as_bytes(), p.bindings.as_ref().unwrap()).unwrap();
        assert!(response.applicability.is_none(), "{q3}");
        assert_eq!(response.assessment.unwrap().score, 0.1);
        assert_eq!(response.probabilities.unwrap()["simple"], 0.9);
    }
}

#[test]
fn c_request_uses_the_frozen_version_and_never_substitutes_current_q2() {
    let mut wires = Vec::new();
    let mut policies = Vec::new();
    for protocol in [
        hiroute_domain::NativeProtocolV1::c_v1(),
        hiroute_domain::NativeProtocolV1::c_v2(),
    ] {
        let mut authority = c_authority();
        let mut judgment = JudgmentSettingsV1::default();
        protocol.current_assessment(&mut judgment);
        judgment.competence.standard = Some(hiroute_domain::AssessmentStandardV1::Default);
        let frozen = hiroute_domain::FrozenNativeProtocolV1::freeze(
            &protocol,
            judgment.degree.definition().into(),
            [(SMART_SAVING_SCOPE_ID.into(), judgment.clone())],
        )
        .unwrap();
        policies.push(frozen.execution_policy(SMART_SAVING_SCOPE_ID, &judgment, "same"));
        authority.smart_judgment = Some(judgment);
        authority.native_protocol = Some(frozen.clone());
        let prepared = request_template(
            &authority,
            &diagnostic_request(),
            &c_history(&authority),
            false,
        )
        .unwrap();
        let wire: Value = serde_json::from_slice(&prepared.template.bytes).unwrap();
        assert_eq!(
            wire["questions"]["q1"],
            frozen.scopes[SMART_SAVING_SCOPE_ID].assessment
        );
        assert_eq!(wire["questions"]["q2"], protocol.fixed.applicability);
        assert_eq!(
            prepared.routing_observation.unwrap().protocol,
            protocol.schema
        );
        wires.push(wire);
    }
    assert_eq!(wires[0]["state"], wires[1]["state"]);
    assert_eq!(wires[0]["questions"]["q0"], wires[1]["questions"]["q0"]);
    assert_eq!(wires[0]["questions"]["q1"], wires[1]["questions"]["q1"]);
    assert_ne!(wires[0]["questions"]["q2"], wires[1]["questions"]["q2"]);
    assert_ne!(policies[0], policies[1]);
}

#[test]
fn c_request_keeps_feedback_separate_and_sends_the_sealed_questions() {
    let mut authority = c_authority();
    let protocol = hiroute_domain::NativeProtocolV1::default();
    let mut judgment = JudgmentSettingsV1::default();
    protocol.current_assessment(&mut judgment);
    judgment.competence.standard = Some(hiroute_domain::AssessmentStandardV1::Default);
    let frozen = hiroute_domain::FrozenNativeProtocolV1::freeze(
        &protocol,
        judgment.degree.definition().into(),
        [(SMART_SAVING_SCOPE_ID.into(), judgment.clone())],
    )
    .unwrap();
    authority.smart_judgment = Some(judgment);
    authority.native_protocol = Some(frozen.clone());
    let history = c_history(&authority);
    for feedback in [
        "This result is not satisfactory; please improve it.",
        "这个结果不太满意，请继续改进。",
    ] {
        let mut request = diagnostic_request();
        request.messages[0].content =
            vec![crate::server::core_runtime::model_ir::ContentPart::Text {
                text: feedback.into(),
            }];
        let prepared = request_template(&authority, &request, &history, false).unwrap();
        let wire: Value = serde_json::from_slice(&prepared.template.bytes).unwrap();
        assert_eq!(
            wire["questions"]["q1"],
            frozen.scopes[SMART_SAVING_SCOPE_ID].assessment
        );
        assert_eq!(wire["questions"]["q2"], frozen.protocol.fixed.applicability);
        assert!(wire["state"]["latest_user"].to_string().contains(feedback));
        assert!(
            wire["state"]["assessment_stage"]
                .to_string()
                .contains("Previous work")
        );
        assert!(
            !wire["state"]["assessment_stage"]
                .to_string()
                .contains(feedback)
        );
        assert!(prepared.bindings.as_ref().unwrap().assessment.is_some());
        assert!(prepared.bindings.as_ref().unwrap().applicability.is_some());
    }
}

#[test]
fn observed_input_is_disclosed_without_dropping_native_assessment_or_applicability() {
    for native_c in [false, true] {
        let a = if native_c { c_authority() } else { authority() };
        let h = if native_c {
            c_history(&a)
        } else {
            history(true)
        };
        let original = request_template(&a, &diagnostic_request(), &h, false).unwrap();
        let observed = request_template(&a, &diagnostic_request(), &h, true).unwrap();
        let original_wire: Value = serde_json::from_slice(&original.template.bytes).unwrap();
        let observed_wire: Value = serde_json::from_slice(&observed.template.bytes).unwrap();
        let mut expected = original_wire.clone();
        expected["state"]["latest_user"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "kind":"unavailable", "source_kind":"unverified_request_user_origin"
            }));
        assert_eq!(observed_wire, expected);
        assert!(observed.bindings.as_ref().unwrap().assessment.is_some());
        assert_eq!(observed.target_partial, original.target_partial);
        if native_c {
            let observed = observed.routing_observation.unwrap();
            let original = original.routing_observation.unwrap();
            assert_eq!(observed.target_complete, original.target_complete);
            assert_eq!(
                observed.applicability_requested,
                original.applicability_requested
            );
            assert!(observed.applicability_requested);
        }
    }
}

#[test]
fn c_rejects_oversized_current_input_without_truncating_it() {
    let a = c_authority();
    let mut request = diagnostic_request();
    request.messages[0].content = vec![crate::server::core_runtime::model_ir::ContentPart::Text {
        // Escaping must count toward the actual wire budget too.
        text: "\"".repeat(140_000),
    }];
    assert!(matches!(
        request_template(&a, &request, &c_history(&a), false),
        Err(CallFailure::RejectedInput)
    ));
}
#[test]
fn c_partitions_cap_only_background_and_preserve_a_ten_turn_target() {
    for background in [0, 2, 8, 12] {
        let a = c_authority();
        let mut h = c_history(&a);
        h.visible_conversation = (0..background + 10)
            .map(|i| {
                let mut turn = (*h.visible_conversation[0]).clone();
                turn.user = vec![VisibleContentPart::Text {
                    text: format!("turn {i}"),
                }];
                Arc::new(turn)
            })
            .collect();
        h.assessment_from = Some(background);
        let prepared = request_template(&a, &diagnostic_request(), &h, false).unwrap();
        let wire: Value = serde_json::from_slice(&prepared.template.bytes).unwrap();
        assert_eq!(
            wire["state"]["prior_context"].as_array().unwrap().len(),
            background.min(8)
        );
        assert_eq!(
            wire["state"]["assessment_stage"].as_array().unwrap().len(),
            10
        );
        assert!(!prepared.target_partial);
        assert!(prepared.routing_observation.unwrap().target_complete);
        h.assessment_target.as_mut().unwrap().target_partial = true;
        let partial = request_template(&a, &diagnostic_request(), &h, false).unwrap();
        assert!(partial.bindings.as_ref().unwrap().applicability.is_none());
        assert!(partial.target_partial);
        h.assessment_target = None;
        h.assessment_from = None;
        let no_target = request_template(&a, &diagnostic_request(), &h, false).unwrap();
        assert!(no_target.bindings.as_ref().unwrap().assessment.is_none());
    }
}
#[test]
fn c_reserves_q3_then_never_refills_a_budget_partial_target() {
    let a = c_authority();
    let mut h = c_history(&a);
    let base = request_template(&a, &diagnostic_request(), &h, false)
        .unwrap()
        .template
        .wire_len;
    let mut large = (*h.visible_conversation[0]).clone();
    large.user = vec![VisibleContentPart::Text {
        text: "x".repeat(256 * 1024 - base + "Previous work".len() - 100),
    }];
    let mut first = (*h.visible_conversation[0]).clone();
    first.user = vec![VisibleContentPart::Text {
        text: "a".repeat(1024),
    }];
    h.visible_conversation = vec![Arc::new(first), Arc::new(large)];
    let p = request_template(&a, &diagnostic_request(), &h, false).unwrap();
    let repeat = request_template(&a, &diagnostic_request(), &h, false).unwrap();
    assert_eq!(p.template.bytes, repeat.template.bytes);
    let o = p.routing_observation.as_ref().unwrap();
    assert!(p.target_partial);
    assert!(!o.applicability_requested);
    assert_eq!(o.stage_removed, 1);
    assert_eq!(o.stage_retained, 1);
    assert!(p.template.wire_len + 1024 < 256 * 1024);
}

#[test]
fn c_background_keeps_bounded_interrupted_and_unknown_turns_in_chronology() {
    for status in [AgentTurnStatus::Interrupted, AgentTurnStatus::Unknown] {
        for position in [1, 8] {
            for with_target in [false, true] {
                let a = c_authority();
                let mut h = c_history(&a);
                let original = h.visible_conversation[0].clone();
                h.visible_conversation = (0..10)
                    .map(|i| {
                        let mut turn = (*original).clone();
                        turn.user = vec![VisibleContentPart::Text {
                            text: format!("Requirement {i}"),
                        }];
                        if i == position {
                            turn.status = status;
                        }
                        Arc::new(turn)
                    })
                    .collect();
                h.assessment_from = with_target.then_some(9);
                if !with_target {
                    h.assessment_target = None;
                }
                let p = request_template(&a, &diagnostic_request(), &h, false).unwrap();
                let wire: Value = serde_json::from_slice(&p.template.bytes).unwrap();
                let background = wire["state"]["prior_context"].as_array().unwrap();
                let start = if with_target { 1 } else { 2 };
                assert_eq!(background.len(), 8, "{status:?}, {position}, {with_target}");
                for (i, turn) in background.iter().enumerate() {
                    assert_eq!(
                        turn["user"][0]["text"],
                        format!("Requirement {}", start + i)
                    );
                }
                if position >= start {
                    assert_eq!(
                        background[position - start]["status"],
                        serde_json::to_value(status).unwrap()
                    );
                }
                assert_eq!(p.routing_observation.unwrap().target_complete, with_target);
                assert_eq!(p.bindings.unwrap().applicability.is_some(), with_target);
            }
        }
    }
}

#[test]
fn c_reopened_frozen_questions_ignore_changed_current_authority_defaults() {
    for smart in [true, false] {
        let mut a = c_authority();
        if !smart {
            let j = a.smart_judgment.clone().unwrap();
            a.branch_policies = ["writing", "general"]
                .into_iter()
                .enumerate()
                .map(|(i, id)| hiroute_domain::MaterializedBranchV1 {
                    id: id.into(),
                    name: id.into(),
                    condition: format!("{id} work"),
                    group: hiroute_domain::MaterializedGroupId::Branch(i as u16),
                    primary_group: Some(hiroute_domain::MaterializedGroupId::BranchPrimary(
                        i as u16,
                    )),
                    judgment: Some(j.clone()),
                })
                .collect();
            let definition =
                hiroute_domain::branch_decision_definition(&a.branch_policies, "general");
            a.native_protocol = Some(
                hiroute_domain::FrozenNativeProtocolV1::freeze(
                    &hiroute_domain::NativeProtocolV1::default(),
                    definition,
                    a.branch_policies
                        .iter()
                        .map(|b| (b.id.clone(), b.judgment.clone().unwrap())),
                )
                .unwrap(),
            );
        }
        let bytes = serde_json::to_vec(a.native_protocol.as_ref().unwrap()).unwrap();
        let original = request_template(&a, &diagnostic_request(), &history(false), false).unwrap();
        // Model a later binary's independently changed current category/degree defaults.
        a.decision = DecisionDefinitionV1::Categorical {
            instructions: "Changed current default; never execute for this publication".into(),
            options: vec![CategoryOptionV1 {
                id: "new-default".into(),
                criterion: "New default".into(),
                refinement: None,
            }],
        };
        a.native_protocol = Some(serde_json::from_slice(&bytes).unwrap());
        let frozen = a.native_protocol.as_ref().unwrap();
        if smart {
            frozen
                .validate_smart(a.smart_judgment.as_ref().unwrap())
                .unwrap();
        } else {
            frozen
                .validate_branches(&a.branch_policies, "general")
                .unwrap();
        }
        let reopened = request_template(&a, &diagnostic_request(), &history(false), false).unwrap();
        assert_eq!(reopened.template.bytes, original.template.bytes);
        assert_eq!(
            reopened.bindings.unwrap().definition,
            a.native_protocol.unwrap().decision
        );
    }
}

#[test]
fn c_omitted_target_distinguishes_absent_evidence_from_policy_mismatch() {
    use hiroute_domain::ProtectionEligibilityV1 as E;
    for (variant, expected) in [
        (0, E::Unrated),
        (1, E::DifferentCategory),
        (2, E::DifferentPolicy),
    ] {
        let a = c_authority();
        let mut h = c_history(&a);
        let target = h.assessment_target.as_mut().unwrap();
        match variant {
            0 => target.branch_execution = None,
            1 => {
                if let ExecutionAttribution::Single {
                    executed_branch_id, ..
                } = &mut target.attribution
                {
                    *executed_branch_id = "removed-category".into();
                }
            }
            _ => {
                target
                    .branch_execution
                    .as_mut()
                    .unwrap()
                    .policy
                    .floor_millis = Some(501)
            }
        }
        let prepared = request_template(&a, &diagnostic_request(), &h, false).unwrap();
        assert_eq!(prepared.routing_observation.unwrap().eligibility, expected);
        assert!(prepared.bindings.unwrap().assessment.is_none());
    }
}
