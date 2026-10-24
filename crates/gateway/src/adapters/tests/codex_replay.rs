use super::*;
use crate::content_ref::ContentValueExt;

#[test]
fn codex_subscription_replay_omits_only_completed_reasoning_output_status() {
    let body = json!({"model":"route", "stream":true, "input":[
        {"type":"reasoning","id":"rs-native","status":"completed",
         "summary":[{"type":"summary_text","text":"retained thought"}],
         "encrypted_content":"retained-state"},
        {"type":"message","id":"msg-native","role":"assistant","status":"completed",
         "phase":"final_answer","content":[{"type":"output_text","text":"answer"}]},
        {"type":"message","role":"user","content":"next task"}
    ]});
    let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    let original = request.clone();
    for connector in [
        "connector.cpa.codex",
        "connector.openai.p0",
        "connector.cpa.claude",
    ] {
        let mut profile =
            exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses);
        profile.connector.connector_id = connector.into();
        let mut expected = body["input"].clone();
        if connector == "connector.cpa.codex" {
            expected[0].as_object_mut().unwrap().remove("status");
        }
        assert_eq!(
            project_candidate_request(&request, &profile).unwrap().body["input"],
            expected,
            "normalization belongs only to the Codex subscription endpoint"
        );
        let template = project_candidate_request_template(&request, &profile).unwrap();
        let rendered: serde_json::Value = serde_json::from_slice(&template.bytes).unwrap();
        assert_eq!(rendered["input"], expected);
        assert_eq!(
            request, original,
            "each attempt starts from immutable client history"
        );
    }
}

#[test]
fn codex_subscription_replay_omits_null_reasoning_output_status() {
    // Responses providers may serialize an absent output lifecycle as null.
    // The Codex subscription input dialect rejects this output-only field.
    let body = json!({"model":"route", "stream":true, "input":[
        {"type":"reasoning","id":"msg-fixture-reasoning","status":null,
         "summary":[],"content":[{"type":"reasoning_text","text":"retained thought"}],
         "encrypted_content":null},
        {"type":"message","id":"msg-fixture-final","role":"assistant","status":"completed",
         "content":[{"type":"output_text","text":"answer"}]},
        {"type":"message","role":"user","content":"next task"}
    ]});
    let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    let original = request.clone();
    assert!(!request.responses_item_statuses.contains_key(&0));
    for connector in [
        "connector.cpa.codex",
        "connector.openai.p0",
        "connector.cpa.claude",
    ] {
        let mut profile =
            exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses);
        profile.connector.connector_id = connector.into();
        let mut expected = body["input"].clone();
        if connector == "connector.cpa.codex" {
            expected[0].as_object_mut().unwrap().remove("status");
        }
        assert_eq!(
            project_candidate_request(&request, &profile).unwrap().body["input"],
            expected,
            "a nullable output status is omitted only for the Codex subscription input dialect"
        );
        let template = project_candidate_request_template(&request, &profile).unwrap();
        let rendered: serde_json::Value = serde_json::from_slice(&template.bytes).unwrap();
        assert_eq!(rendered["input"], expected);
        assert_eq!(request, original);
    }
}

#[test]
fn codex_subscription_replay_does_not_normalize_unfinished_reasoning_status() {
    for status in ["in_progress", "incomplete"] {
        let body = json!({"model":"route","input":[
            {"type":"reasoning","id":"rs-unfinished","status":status,"summary":[]},
            {"role":"user","content":"next task"}
        ]});
        let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
        let original = request.clone();
        let mut profile =
            exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses);
        profile.connector.connector_id = "connector.cpa.codex".into();
        assert_eq!(
            project_candidate_request(&request, &profile).unwrap().body["input"],
            body["input"],
            "the dialect normalization cannot manufacture completed history"
        );
        assert_eq!(request, original);
    }
}

#[test]
fn codex_subscription_normalization_survives_externalized_status_and_sequential_replay() {
    assert_codex_replay_wire(false, None);
}

#[test]
fn codex_subscription_replay_indexes_skip_additional_tool_declarations() {
    assert_codex_replay_wire(true, None);
}

#[test]
fn codex_subscription_replay_normalizes_before_prefix_cleanup_reindexes_items() {
    for additional_tools in [false, true] {
        assert_codex_replay_wire(additional_tools, Some(4));
    }
}

fn assert_codex_replay_wire(additional_tools: bool, cleanup: Option<usize>) {
    let root = std::env::temp_dir().join(format!(
        "hiroute-codex-status-replay-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = ReplayManager::open(ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    let budget = BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024)
        .unwrap()
        .stream(4 * 1024 * 1024)
        .unwrap();
    let store = manager.begin_request(budget.clone()).unwrap();
    let mut body = json!({"model":"route","stream":true,"input":[
        {"role":"system","content":"long instruction ".repeat(500)},
        {"type":"reasoning","id":"rs-old-first","status":"completed","summary":[{"type":"summary_text","text":"old thought"}]},
        {"type":"reasoning","id":"rs-old-second","status":"completed","summary":[]},
        {"type":"message","id":"msg-old","role":"assistant","content":[{"type":"output_text","text":"old answer"}]},
        {"role":"user","content":"next task"},
        {"type":"reasoning","id":"rs-native","status":"completed","summary":[{"type":"summary_text","text":"retained thought"}],"encrypted_content":"opaque-native-state"},
        {"type":"message","id":"msg-native","role":"assistant","status":"completed","phase":"final_answer","content":[{"type":"output_text","text":"answer"}]},
        {"type":"function_call","id":"fc-native","call_id":"call-native","name":"read","arguments":"{}","status":"completed"},
        {"type":"function_call_output","call_id":"call-native","output":"result","status":"incomplete"},
        {"type":"reasoning","id":"rs-no-status","summary":[]},
        {"type":"reasoning","id":"rs-null-status","status":null,"summary":[]},
        {"role":"user","content":"final task"}
    ]});
    if additional_tools {
        body["input"].as_array_mut().unwrap().insert(
            0,
            json!({
                "type":"additional_tools","tools":[{
                    "type":"function","name":"read","parameters":{"type":"object"}
                }]
            }),
        );
    }
    let mut request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    externalize_model_request(&mut request, &store, 0).unwrap();
    assert!(
        request.native_body.as_ref().unwrap()["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["type"] == "reasoning")
            .nth(2)
            .unwrap()["status"]
            .as_str()
            .unwrap()
            .content_ref()
            .is_some()
    );
    let original = request.clone();
    store.prevalidate(&model_content_refs(&request)).unwrap();
    for connector in ["connector.cpa.codex", "connector.openai.p0"] {
        let mut profile =
            exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses);
        profile.connector.connector_id = connector.into();
        let mut expected = body["input"].clone();
        if cleanup.is_some() {
            expected.as_array_mut().unwrap().retain(|item| {
                !matches!(item["id"].as_str(), Some("rs-old-first" | "rs-old-second"))
            });
        }
        if connector == "connector.cpa.codex" {
            for item in expected.as_array_mut().unwrap() {
                if item["type"] == "reasoning"
                    && (item["status"] == "completed"
                        || item.get("status").is_some_and(serde_json::Value::is_null))
                {
                    item.as_object_mut().unwrap().remove("status");
                }
            }
        }
        for quantum in [1, 127, 8192] {
            let template =
                project_candidate_request_template_with_cleanup(&request, &profile, cleanup)
                    .unwrap();
            let wire_len = template.wire_len;
            let mut reader =
                sequential_attempt_body(template, store.clone(), &budget, quantum).unwrap();
            let mut bytes = Vec::new();
            while let Some(chunk) = reader.next_chunk().unwrap() {
                bytes.extend_from_slice(chunk.bytes());
            }
            assert_eq!(bytes.len(), wire_len);
            let rendered: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                rendered["input"], expected,
                "actual Replay wire must preserve the endpoint dialect with additional_tools={additional_tools} and cleanup={cleanup:?}"
            );
            assert_eq!(request, original);
        }
    }
    drop((store, manager));
    fs_err_remove_dir(&root);
}
