use std::sync::Arc;

use super::*;
use crate::agent_turn_history::{
    AgentTurnHistorySnapshot, AgentTurnStatus, VisibleAgentTurn, VisibleContentPart,
};
use crate::server::core_runtime::model_ir::{
    CanonicalMessage, MODEL_REQUEST_IR_SCHEMA, RequestedReasoningControl, ToolChoice,
};
use crate::server::request_plan::IngressProtocol;

pub(crate) fn request(text: &str) -> ModelRequestIRV1 {
    ModelRequestIRV1 {
        native_body: None,
        native_only: false,
        schema_version: MODEL_REQUEST_IR_SCHEMA.into(),
        ingress_protocol: IngressProtocol::Responses,
        served_model_id: "smart-route".into(),
        stream: false,
        instructions: Vec::new(),
        messages: vec![CanonicalMessage {
            role: MessageRole::User,
            content: vec![ContentPart::Text { text: text.into() }],
            name: None,
        }],
        tools: Vec::new(),
        tool_namespaces: Vec::new(),
        responses_tool_order: Vec::new(),
        web_search: None,
        responses_search_history: Default::default(),
        responses_annotations: Default::default(),
        tool_choice: ToolChoice::Auto,
        parallel_tool_calls: false,
        requested_reasoning: RequestedReasoningControl::absent(),
        requested_max_output_tokens: None,
        provider_state: Vec::new(),
        responses_options: None,
        responses_item_ids: Default::default(),
        responses_item_statuses: Default::default(),
        responses_message_phases: Default::default(),
        responses_internal_chat_message_metadata: Default::default(),
        responses_reasoning_history: Default::default(),
    }
}

pub(crate) fn history(target: bool) -> AgentTurnHistorySnapshot {
    AgentTurnHistorySnapshot {
        previous_decision: None,
        visible_conversation: vec![Arc::new(VisibleAgentTurn {
            branch_id: Some("smart_saving_simple".into()),
            executed_branch_id: None,
            user: vec![VisibleContentPart::Text {
                text: "Fix the test".into(),
            }],
            status: AgentTurnStatus::Completed,
            steps: vec![vec![VisibleContentPart::Text {
                text: "Done".into(),
            }]],
        })],
        history_partial: false,
        assessment_from: target.then_some(0),
        assessment_target: None,
        _pin: None,
    }
}

fn authority() -> std::sync::Arc<RestBranchClassifierAuthorityV1> {
    crate::server::publication::compile_classifier_mode_authority(
        &hiroute_domain::ComplexityClassifierModeV1::DecisionService {
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
        },
        1,
    )
    .unwrap()
}
fn definition() -> DecisionDefinitionV1 {
    hiroute_domain::DegreePolicyV1::default()
        .definition()
        .into()
}
fn categories() -> DecisionDefinitionV1 {
    DecisionDefinitionV1::Categorical {
        instructions: "Main intent; default review".into(),
        options: vec![
            hiroute_domain::CategoryOptionV1 {
                id: "write".into(),
                criterion: "Write".into(),
                refinement: Some(hiroute_domain::DegreePolicyV1::default().definition()),
            },
            hiroute_domain::CategoryOptionV1 {
                id: "review".into(),
                criterion: "Review".into(),
                refinement: None,
            },
        ],
    }
}
#[test]
fn latest_user_streams_full_spilled_cjk_and_marker_literal_without_a_size_fallback() {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use hiroute_gateway_core::runtime::body::BudgetTree;

    use crate::content_ref::{externalize_model_request, model_content_refs};
    use crate::replay::{ReplayConfig, ReplayManager};
    use crate::server::core_runtime::adapters::{decode_ingress_request, sequential_replay_body};

    let latest_user = format!(
        "{} literal-marker=__hiroute_content_ref_v2_0_0_1_1__",
        "界".repeat(400_000)
    );
    let document = json!({
        "model": "smart-route",
        "instructions": "must not be sent to the classifier",
        "input": latest_user,
        "stream": false,
    });
    let mut request =
        decode_ingress_request(IngressProtocol::Responses, &document).expect("decode request");
    let root = std::env::temp_dir().join(format!(
        "hiroute-classifier-protocol-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let manager = ReplayManager::open(ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 64,
        orphan_ttl: Duration::from_secs(60),
    })
    .expect("open replay manager");
    let tree = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024).expect("budget tree");
    let budget = tree.stream(8 * 1024 * 1024).expect("stream budget");
    let store = manager
        .begin_request(budget.clone())
        .expect("begin replay request");
    externalize_model_request(&mut request, &store, 8 * 1024).expect("externalize request");
    store
        .prevalidate(&model_content_refs(&request))
        .expect("prevalidate replay");

    let prepared = classifier_request_template(&request, &history(false), &authority())
        .expect("classification request template");
    let template = prepared.template;
    assert!(template.wire_len > 1024 * 1024);
    let mut reader = sequential_replay_body(template, store.clone(), &budget, 16 * 1024)
        .expect("classification request reader");
    let mut bytes = Vec::new();
    while let Some(chunk) = reader.next_chunk().expect("request chunk") {
        bytes.extend_from_slice(chunk.bytes());
    }
    reader.release();
    let body: Value = serde_json::from_slice(&bytes).expect("valid classifier JSON");
    assert_eq!(body["latest_user"][0]["text"], document["input"]);
    assert_eq!(body["visible_conversation"].as_array().unwrap().len(), 1);
    assert!(
        !String::from_utf8(bytes)
            .unwrap()
            .contains("must not be sent")
    );

    drop(request);
    drop(store);
    drop(manager);
    assert_eq!(budget.snapshot().unwrap().live, 0);
    std::fs::remove_dir_all(root).expect("remove replay fixture");
}

#[test]
fn request_has_exact_five_fields_and_no_execution_identity() {
    let prepared =
        classifier_request_template(&request("next"), &history(false), &authority()).unwrap();
    let body: Value = serde_json::from_slice(&prepared.template.bytes).unwrap();
    assert_eq!(body.as_object().unwrap().len(), 5);
    assert_eq!(body["decision"]["kind"], "ordinal");
    assert_eq!(body["latest_user"][0]["text"], "next");
    assert!(body["assessment_target"].is_null());
    assert_eq!(
        body["visible_conversation"][0].as_object().unwrap().len(),
        3
    );
    for absent in [
        "plan_id",
        "model_configuration_id",
        "branch_id",
        "assessment_from",
    ] {
        assert!(!String::from_utf8_lossy(&prepared.template.bytes).contains(absent));
    }
}
#[test]
fn valid_category_survives_invalid_selected_degree_and_optional_assessment() {
    let output = parse_classifier_response(br#"{"decision":{"kind":"categorical","choice":"write","refinement":{"kind":"ordinal","probabilities":{"simple":1}}},"assessment":{"score":2,"partial":false}}"#, &categories(), true).unwrap();
    assert_eq!(output.branch_id, "write");
    assert!(output.degree_failed && output.probabilities.is_none());
    assert!(output.invalid_assessment && output.assessment.is_none());
    let single = parse_classifier_response(
        br#"{"decision":{"kind":"categorical","choice":"review"}}"#,
        &categories(),
        false,
    )
    .unwrap();
    assert!(!single.degree_failed && single.probabilities.is_none());
}
#[test]
fn exact_probability_keys_sum_and_duplicates_are_checked() {
    for probabilities in [
        r#"{"simple":0.8,"complex":0.1}"#,
        r#"{"simple":1}"#,
        r#"{"simple":1,"complex":0,"extra":0}"#,
        r#"{"simple":0.8,"simple":0.8,"complex":0.2}"#,
        r#"{"simple":-0.1,"complex":1.1}"#,
    ] {
        let body =
            format!(r#"{{"decision":{{"kind":"ordinal","probabilities":{probabilities}}}}}"#);
        assert!(
            parse_classifier_response(body.as_bytes(), &definition(), false)
                .unwrap()
                .degree_failed
        );
    }
    let output = parse_classifier_response(
        br#"{"decision":{"kind":"ordinal","probabilities":{"simple":0.8,"complex":0.2000001}}}"#,
        &definition(),
        false,
    )
    .unwrap();
    assert!(!output.degree_failed);
    assert!((output.probabilities.unwrap().values().sum::<f64>() - 1.0).abs() < 1e-12);
}
#[test]
fn response_rejects_unknown_duplicate_category_and_chat_envelopes() {
    for body in [
        r#"{"decision":{"kind":"categorical","choice":"unknown"}}"#,
        r#"{"decision":{"kind":"categorical","choice":"review","choice":"write"}}"#,
        r#"{"decision":{"kind":"categorical","choice":"review"},"extra":1}"#,
        r#"{"choices":[{"message":{"content":"review"}}]}"#,
    ] {
        assert_eq!(
            parse_classifier_response(body.as_bytes(), &categories(), false),
            Err(CallFailure::InvalidOutput)
        );
    }
}
#[test]
fn assessment_requires_a_target_and_preserves_unicode_reason() {
    let body = json!({"decision":{"kind":"ordinal","probabilities":{"simple":0.8,"complex":0.2}},"assessment":{"score":0.5,"partial":false,"reason":"文".repeat(1024)}});
    let bytes = serde_json::to_vec(&body).unwrap();
    assert_eq!(
        parse_classifier_response(&bytes, &definition(), true)
            .unwrap()
            .assessment
            .unwrap()
            .score,
        0.5
    );
    assert!(
        parse_classifier_response(&bytes, &definition(), false)
            .unwrap()
            .invalid_assessment
    );
    let duplicate = br#"{"decision":{"kind":"ordinal","probabilities":{"simple":1,"complex":0}},"assessment":{"score":0.4,"score":0.5,"partial":true}}"#;
    assert!(
        parse_classifier_response(duplicate, &definition(), true)
            .unwrap()
            .invalid_assessment
    );
}
