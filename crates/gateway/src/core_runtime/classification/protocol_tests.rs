use std::sync::Arc;

use super::super::extension::{
    parse_response as parse_classifier_response, request_template as classifier_request_template,
};
use super::*;
use crate::agent_turn_history::{
    AgentTurnHistorySnapshot, AgentTurnStatus, VisibleAgentTurn, VisibleContentPart,
};
use crate::server::core_runtime::model_ir::{
    CanonicalMessage, MODEL_REQUEST_IR_SCHEMA, RequestedReasoningControl, ToolChoice,
};
use crate::server::core_runtime::model_ir::{MessageRole, ModelRequestIRV1};
use crate::server::request_plan::IngressProtocol;
use hiroute_domain::{DecisionDefinitionV1, ExecutionGroupV1};

pub(crate) fn request(text: &str) -> ModelRequestIRV1 {
    ModelRequestIRV1 {
        native_body: None,
        native_only: false,
        native_history: None,
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

    let prepared = classifier_request_template(&request, &history(false), &authority(), true)
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
    assert_eq!(
        body["latest_user"][1],
        json!({
            "kind":"unavailable", "source_kind":"unverified_request_user_origin"
        })
    );
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
fn optional_hold_preparation_miss_discloses_origin_without_disabling_classification() {
    use crate::context_hold::{
        ContextHoldStore, ContextRequest, NativeUserOrigin, prepare_context,
    };
    use crate::replay::{ReplayConfig, ReplayManager};
    use crate::server::core_runtime::native_routing_view::NativeRoutingView;
    use hiroute_gateway_core::runtime::body::BudgetTree;
    use http::HeaderMap;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    let root = std::env::temp_dir().join(format!(
        "hiroute-classifier-origin-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = ReplayManager::open(ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 4096,
        record_bytes: 4096,
        orphan_ttl: Duration::from_secs(60),
    })
    .unwrap();
    let budget = BudgetTree::new(1024 * 1024, 1024 * 1024)
        .unwrap()
        .stream(1024 * 1024)
        .unwrap();
    let replay = manager.begin_request(budget.clone()).unwrap();
    let authorized = crate::server::publication::authorized_native_consumer_fixture(None);
    for generation in [0, 1] {
        for busy in [false, true] {
            let request = request("native task or compacted summary");
            let metadata = json!({"session_id":"session", "thread_id":"thread",
                "turn_id":"turn", "request_kind":"turn",
                "window_id":format!("thread:{generation}")});
            let mut headers = HeaderMap::new();
            headers.insert("session-id", "session".parse().unwrap());
            headers.insert("thread-id", "thread".parse().unwrap());
            headers.insert("user-agent", "codex_exec/0.147.0".parse().unwrap());
            headers.insert(
                "x-codex-turn-metadata",
                metadata.to_string().parse().unwrap(),
            );
            let view = NativeRoutingView::prepare(&headers, &request, &replay, &budget);
            assert!(!view.origin_unverified());
            let holds = ContextHoldStore::default();
            let lock = busy.then(|| holds.lock_for_runtime_test());
            let (identity, prepared, unavailable) = prepare_context(
                &holds,
                &[17; 32],
                &authorized,
                ContextRequest {
                    ingress: IngressProtocol::Responses,
                    headers: &headers,
                    document: &json!({}),
                    request: &request,
                    semantic_request: Some(view.request(&request)),
                    user_origin_unverified: view.origin_unverified(),
                    replay: &replay,
                },
                Instant::now(),
            );
            assert_eq!(
                identity.native_checkpoint().user_origin,
                NativeUserOrigin::NativeTurn
            );
            assert_eq!(
                prepared.is_none(),
                busy,
                "real optional preparation must miss under lock"
            );
            assert!(
                !unavailable,
                "origin uncertainty must leave classification available"
            );
            let unverified =
                view.origin_unverified() || identity.input_origin_unverified(prepared.as_ref());
            assert!(
                unverified,
                "native activation metadata never proves human input, even with an empty cache"
            );
            let encoded =
                classifier_request_template(&request, &history(false), &authority(), unverified)
                    .expect("ordinary input remains classifiable");
            let body: Value = serde_json::from_slice(&encoded.template.bytes).unwrap();
            let mut expected =
                vec![json!({"kind":"text", "text":"native task or compacted summary"})];
            if unverified {
                expected.push(
                    json!({"kind":"unavailable", "source_kind":"unverified_request_user_origin"}),
                );
            }
            assert_eq!(body["latest_user"], json!(expected));
            assert!(body["preceding_stage"].is_null());
            assert_eq!(body["visible_conversation"].as_array().unwrap().len(), 1);
            drop(lock);
        }
    }
    drop(replay);
    drop(manager);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn request_has_exact_five_fields_and_no_execution_identity() {
    let prepared =
        classifier_request_template(&request("next"), &history(false), &authority(), false)
            .unwrap();
    let body: Value = serde_json::from_slice(&prepared.template.bytes).unwrap();
    assert_eq!(body.as_object().unwrap().len(), 5);
    assert_eq!(body["route"]["kind"], "smart_saving");
    assert_eq!(body["latest_user"][0]["text"], "next");
    assert!(body["preceding_stage"].is_null());
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
fn valid_category_survives_invalid_group_and_optional_assessment() {
    let output = parse_classifier_response(
        br#"{"category":"write","group":"other","assessment":{"score":2,"partial":false}}"#,
        &categories(),
        Some("write"),
    )
    .unwrap();
    assert_eq!(output.branch_id, "write");
    assert!(output.degree_failed && output.probabilities.is_none());
    assert!(output.invalid_assessment && output.assessment.is_none());
    for body in [
        br#"{"category":"review"}"#.as_slice(),
        br#"{"category":"review","group":{"invalid":true}}"#,
    ] {
        let single = parse_classifier_response(body, &categories(), None).unwrap();
        assert_eq!(single.extension_group, Some(ExecutionGroupV1::Regular));
        assert!(!single.degree_failed);
    }
}
#[test]
fn direct_decision_rejects_old_unknown_duplicate_and_chat_envelopes() {
    for body in [
        r#"{"category":"unknown","group":"regular"}"#,
        r#"{"category":"review","category":"write","group":"regular"}"#,
        r#"{"category":"review","group":"regular","extra":1}"#,
        r#"{"decision":{"kind":"ordinal","probabilities":{"simple":1,"complex":0}}}"#,
        r#"{"choices":[{"message":{"content":"review"}}]}"#,
    ] {
        assert_eq!(
            parse_classifier_response(body.as_bytes(), &categories(), None),
            Err(CallFailure::InvalidOutput)
        );
    }
    assert!(
        parse_classifier_response(
            br#"{"category":"smart_saving","group":"regular"}"#,
            &definition(),
            None
        )
        .is_err()
    );
    assert!(
        parse_classifier_response(
            br#"{"group":"regular","group":"primary"}"#,
            &definition(),
            None
        )
        .is_err()
    );
}
#[test]
fn assessments_require_same_category_and_reasons_are_bounded_separately() {
    let body = json!({"category":"write","group":"regular","reason":"文".repeat(1024),"assessment":{"score":0.5,"partial":false}});
    let bytes = serde_json::to_vec(&body).unwrap();
    let response = parse_classifier_response(&bytes, &categories(), Some("write")).unwrap();
    assert_eq!(response.assessment.unwrap().score, 0.5);
    assert_eq!(response.extension_group, Some(ExecutionGroupV1::Regular));
    assert_eq!(response.extension_reason.unwrap().chars().count(), 1024);
    for target in [None, Some("review")] {
        let response = parse_classifier_response(&bytes, &categories(), target).unwrap();
        assert!(response.invalid_assessment && response.assessment.is_none());
        assert_eq!(response.extension_group, Some(ExecutionGroupV1::Regular));
    }
    let duplicate = br#"{"group":"regular","assessment":{"score":0.4,"score":0.5,"partial":true}}"#;
    assert!(
        parse_classifier_response(duplicate, &definition(), Some("smart_saving"))
            .unwrap()
            .invalid_assessment
    );
    let extra =
        br#"{"group":"regular","assessment":{"score":0.4,"partial":true,"reason":"extra"}}"#;
    assert!(
        parse_classifier_response(extra, &definition(), Some("smart_saving"))
            .unwrap()
            .invalid_assessment
    );
    for reason in [String::new(), "文".repeat(1025)] {
        assert!(
            parse_classifier_response(
                &serde_json::to_vec(&json!({"group":"regular","reason":reason})).unwrap(),
                &definition(),
                None
            )
            .is_err()
        );
    }
}
#[test]
fn builtin_probability_normalization_keeps_strict_keys_and_sum() {
    for probabilities in [
        r#"{"simple":0.8,"complex":0.1}"#,
        r#"{"simple":1}"#,
        r#"{"simple":1,"complex":0,"extra":0}"#,
        r#"{"simple":-0.1,"complex":1.1}"#,
    ] {
        let map: UniqueMap<f64> = serde_json::from_str(probabilities).unwrap();
        assert!(normalize_probabilities(map.0, ["simple", "complex"].into_iter()).is_err());
    }
    assert!(
        serde_json::from_str::<UniqueMap<f64>>(r#"{"simple":0.8,"simple":0.8,"complex":0.2}"#)
            .is_err()
    );
}

#[test]
fn preceding_stage_requires_exact_plan_connection_and_actual_category() {
    use crate::agent_turn_history::{AssessmentTarget, ExecutionAttribution, PlanSnapshot};
    let mut authority = (*authority()).clone();
    authority.decision = categories();
    authority.default_category = Some("review".into());
    authority.plan_identity = Some(("plan/test".into(), 7));
    let mut history = history(true);
    history.assessment_target = Some(AssessmentTarget {
        evaluation_scope: None,
        segment_id: "private-stage".into(),
        history_membership: None,
        first_turn_id: "turn-1".into(),
        through_turn_id: "turn-1".into(),
        first_ordinal: 1,
        through_ordinal: 1,
        target_partial: false,
        plan: PlanSnapshot {
            plan_id: "plan/test".into(),
            plan_revision: 7,
        },
        attribution: ExecutionAttribution::Single {
            selected_branch_id: "write".into(),
            executed_branch_id: "write".into(),
            model_configuration_id: "private-model".into(),
            profile_digest: "private-profile".into(),
        },
        branch_execution: Some(hiroute_domain::BranchExecutionV1 {
            policy: authority
                .extension_identity
                .as_ref()
                .unwrap()
                .execution_policy("Write"),
            group: ExecutionGroupV1::Primary,
            candidate_index: 0,
        }),
    });
    for variant in 0..5 {
        let mut candidate = history.clone();
        let target = candidate.assessment_target.as_mut().unwrap();
        match variant {
            1 => target.plan.plan_revision = 6,
            2 => {
                target
                    .branch_execution
                    .as_mut()
                    .unwrap()
                    .policy
                    .extension
                    .as_mut()
                    .unwrap()
                    .revision = 2
            }
            3 => target.attribution = ExecutionAttribution::Mixed,
            4 => {
                if let ExecutionAttribution::Single {
                    executed_branch_id, ..
                } = &mut target.attribution
                {
                    *executed_branch_id = "review".into();
                }
            }
            _ => (),
        }
        let request =
            classifier_request_template(&request("Review now"), &candidate, &authority, false)
                .unwrap();
        let body: Value = serde_json::from_slice(&request.template.bytes).unwrap();
        if variant == 0 {
            assert_eq!(
                body["preceding_stage"],
                json!({"from":0,"category":"write","group":"primary","complete":true})
            );
        } else {
            assert!(body["preceding_stage"].is_null());
        }
        let wire = String::from_utf8(request.template.bytes.to_vec()).unwrap();
        for private in [
            "private-stage",
            "private-model",
            "private-profile",
            "plan/test",
            "floor_millis",
            "simple_threshold",
        ] {
            assert!(!wire.contains(private));
        }
    }
}

#[test]
fn explicit_null_is_not_absence_for_category_or_reason() {
    for body in [
        br#"{"category":null,"group":"regular"}"#.as_slice(),
        br#"{"group":"regular","reason":null}"#,
    ] {
        assert_eq!(
            parse_classifier_response(body, &definition(), None),
            Err(CallFailure::InvalidOutput)
        );
    }
}
