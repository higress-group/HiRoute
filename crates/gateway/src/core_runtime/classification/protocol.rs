use serde::Deserialize;
use serde_json::{Map, Value, json, value::RawValue};

use super::super::adapters::{
    PreparedReplayTemplate, ProtocolAdapterError, ReplacementEncoding, RequestedReplacement,
    prepare_replay_json_template,
};
use super::super::model_ir::{ContentPart, ImageSource, MessageRole, ModelRequestIRV1};
use super::CallFailure;
use crate::agent_turn_history::AgentTurnHistorySnapshot;
use crate::content_ref::ContentValueExt;
use crate::server::request_plan::ClassifierBranchAuthorityV1;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ClassifierAssessment {
    pub(super) score: f64,
    pub(super) partial: bool,
    pub(super) reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ClassifierResponse {
    pub(super) branch_id: String,
    pub(super) assessment: Option<ClassifierAssessment>,
    pub(super) invalid_assessment: bool,
}

pub(super) fn classifier_request_template(
    request: &ModelRequestIRV1,
    history: &AgentTurnHistorySnapshot,
    branches: &[ClassifierBranchAuthorityV1],
) -> Result<PreparedReplayTemplate, ProtocolAdapterError> {
    let mut refs = Vec::new();
    let latest_user = latest_user_wire_value(request, &mut refs)?;
    let mut branch_map = Map::new();
    for branch in branches {
        branch_map.insert(
            branch.id.to_string(),
            Value::String(branch.description.to_string()),
        );
    }
    let visible_conversation = serde_json::to_value(&history.visible_conversation)
        .map_err(|error| ProtocolAdapterError::Serialization(error.to_string()))?;
    let body = json!({
        "branches": Value::Object(branch_map),
        "latest_user": latest_user,
        "visible_conversation": visible_conversation,
        "history_partial": history.history_partial,
        "assessment_from": history.assessment_from,
    });
    prepare_replay_json_template(&body, refs)
}

fn latest_user_wire_value(
    request: &ModelRequestIRV1,
    refs: &mut Vec<RequestedReplacement>,
) -> Result<Value, ProtocolAdapterError> {
    let message = request
        .messages
        .iter()
        .rev()
        .find(|message| {
            message.role == MessageRole::User
                && message.content.iter().any(|part| {
                    matches!(part, ContentPart::Text { .. } | ContentPart::Image { .. })
                })
        })
        .ok_or_else(|| {
            ProtocolAdapterError::Serialization(
                "classification requires a current user message".into(),
            )
        })?;
    let values = message
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(project_text(text, refs)),
            ContentPart::Image {
                source: ImageSource::Url { .. },
            } => Some(json!({"kind":"unavailable","source_kind":"image_url"})),
            ContentPart::Image {
                source: ImageSource::Base64 { .. },
            } => Some(json!({"kind":"unavailable","source_kind":"image_base64"})),
            ContentPart::ToolCall { .. }
            | ContentPart::ToolResult { .. }
            | ContentPart::ProviderState { .. } => None,
        })
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Err(ProtocolAdapterError::Serialization(
            "classification current user message is empty".into(),
        ));
    }
    Ok(Value::Array(values))
}

fn project_text(value: &str, refs: &mut Vec<RequestedReplacement>) -> Value {
    if let Some(content) = value.content_ref() {
        refs.push(RequestedReplacement {
            content,
            encoding: ReplacementEncoding::JsonString,
        });
    }
    json!({"kind":"text","text":value})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassifierOutput {
    branch_id: String,
    #[serde(default)]
    assessment: Option<Box<RawValue>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssessmentOutput {
    score: f64,
    partial: bool,
    #[serde(default)]
    reason: Option<String>,
}

pub(super) fn parse_classifier_response(
    body: &[u8],
    allowed: &[&str],
    has_assessment_target: bool,
) -> Result<ClassifierResponse, CallFailure> {
    let output: ClassifierOutput =
        serde_json::from_slice(body).map_err(|_| CallFailure::InvalidOutput)?;
    if !allowed.iter().any(|allowed| *allowed == output.branch_id) {
        return Err(CallFailure::InvalidOutput);
    }
    let mut invalid_assessment = false;
    let assessment = output.assessment.and_then(|raw| {
        let parsed = serde_json::from_str::<AssessmentOutput>(raw.get()).ok();
        let valid = has_assessment_target
            && parsed.as_ref().is_some_and(|assessment| {
                assessment.score.is_finite()
                    && (0.0..=1.0).contains(&assessment.score)
                    && assessment
                        .reason
                        .as_ref()
                        .is_none_or(|reason| !reason.is_empty() && reason.chars().count() <= 1_024)
            });
        if !valid {
            invalid_assessment = true;
            return None;
        }
        parsed.map(|assessment| ClassifierAssessment {
            score: assessment.score,
            partial: assessment.partial,
            reason: assessment.reason,
        })
    });
    Ok(ClassifierResponse {
        branch_id: output.branch_id,
        assessment,
        invalid_assessment,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::agent_turn_history::{
        AgentTurnHistorySnapshot, AgentTurnStatus, VisibleAgentTurn, VisibleContentPart,
    };
    use crate::server::core_runtime::model_ir::{
        CanonicalMessage, MODEL_REQUEST_IR_SCHEMA, RequestedReasoningControl, ToolChoice,
    };
    use crate::server::request_plan::IngressProtocol;

    fn request(text: &str) -> ModelRequestIRV1 {
        ModelRequestIRV1 {
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

    fn history(target: bool) -> AgentTurnHistorySnapshot {
        AgentTurnHistorySnapshot {
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

    fn branches() -> [ClassifierBranchAuthorityV1; 2] {
        [
            ClassifierBranchAuthorityV1 {
                id: "smart_saving_simple".into(),
                description: "Economy".into(),
            },
            ClassifierBranchAuthorityV1 {
                id: "smart_saving_complex".into(),
                description: "Primary".into(),
            },
        ]
    }

    #[test]
    fn latest_user_streams_full_spilled_cjk_and_marker_literal_without_a_size_fallback() {
        use std::time::{Duration, SystemTime, UNIX_EPOCH};

        use hiroute_gateway_core::runtime::body::BudgetTree;

        use crate::content_ref::{externalize_model_request, model_content_refs};
        use crate::replay::{ReplayConfig, ReplayManager};
        use crate::server::core_runtime::adapters::{
            decode_ingress_request, sequential_replay_body,
        };

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

        let template = classifier_request_template(&request, &history(false), &branches())
            .expect("classification request template");
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
    fn request_has_exact_five_fields_and_no_plan_or_tool_details() {
        let template =
            classifier_request_template(&request("next question"), &history(true), &branches())
                .unwrap();
        let body: Value = serde_json::from_slice(&template.bytes).unwrap();
        assert_eq!(body.as_object().unwrap().len(), 5);
        assert_eq!(body["latest_user"][0]["text"], "next question");
        assert_eq!(body["assessment_from"], 0);
        let encoded = String::from_utf8(template.bytes.to_vec()).unwrap();
        for absent in [
            "schema",
            "instructions",
            "plan_id",
            "model_configuration_id",
        ] {
            assert!(!encoded.contains(absent));
        }
    }

    #[test]
    fn response_keeps_valid_branch_when_optional_assessment_is_invalid() {
        let allowed = ["smart_saving_simple", "smart_saving_complex"];
        let output = parse_classifier_response(
            br#"{"branch_id":"smart_saving_complex","assessment":{"score":2,"partial":false}}"#,
            &allowed,
            true,
        )
        .unwrap();
        assert_eq!(output.branch_id, "smart_saving_complex");
        assert!(output.assessment.is_none());
        assert!(output.invalid_assessment);
    }

    #[test]
    fn response_rejects_unknown_duplicate_and_vendor_envelopes() {
        let allowed = ["smart_saving_simple"];
        for body in [
            br#"{"branch_id":"smart_saving_simple","extra":1}"#.as_slice(),
            br#"{"branch_id":"smart_saving_simple","branch_id":"smart_saving_simple"}"#.as_slice(),
            br#"{"choices":[{"message":{"content":"smart_saving_simple"}}]}"#.as_slice(),
        ] {
            assert_eq!(
                parse_classifier_response(body, &allowed, false),
                Err(CallFailure::InvalidOutput)
            );
        }
    }

    #[test]
    fn assessment_reason_is_optional_and_duplicate_nested_fields_are_dropped() {
        let allowed = ["smart_saving_simple"];
        let valid = parse_classifier_response(
            br#"{"branch_id":"smart_saving_simple","assessment":{"score":0.4,"partial":true}}"#,
            &allowed,
            true,
        )
        .unwrap();
        assert_eq!(valid.assessment.unwrap().score, 0.4);
        let duplicate = parse_classifier_response(
            br#"{"branch_id":"smart_saving_simple","assessment":{"score":0.4,"score":0.5,"partial":true}}"#,
            &allowed,
            true,
        )
        .unwrap();
        assert!(duplicate.assessment.is_none());
        assert!(duplicate.invalid_assessment);
    }
}
