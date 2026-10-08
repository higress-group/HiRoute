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
    let prepared = request_template(&authority(), &diagnostic_request(), &history(true)).unwrap();
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
    let prepared = request_template(&a, &diagnostic_request(), &history(false)).unwrap();
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
        request_template(&a, &request, &history(false)),
        Err(CallFailure::RejectedInput)
    ));
    let mut h = history(true);
    let mut oversized = (*h.visible_conversation[0]).clone();
    oversized.user = vec![VisibleContentPart::Text {
        text: "x".repeat(MAX_REQUEST_BYTES),
    }];
    h.visible_conversation.insert(0, Arc::new(oversized));
    let prepared = request_template(&a, &diagnostic_request(), &h).unwrap();
    assert!(prepared.has_target && prepared.target_partial);
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
