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
fn codex_subscription_normalization_survives_externalized_status_and_sequential_replay() {
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
    let body = json!({"model":"route","stream":true,"input":[
        {"role":"system","content":"long instruction ".repeat(500)},
        {"type":"reasoning","id":"rs-native","status":"completed","summary":[{"type":"summary_text","text":"retained thought"}],"encrypted_content":"opaque-native-state"},
        {"type":"message","id":"msg-native","role":"assistant","status":"completed","phase":"final_answer","content":[{"type":"output_text","text":"answer"}]},
        {"type":"function_call","id":"fc-native","call_id":"call-native","name":"read","arguments":"{}","status":"completed"},
        {"type":"function_call_output","call_id":"call-native","output":"result","status":"incomplete"},
        {"role":"user","content":"next task"}
    ]});
    let mut request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    externalize_model_request(&mut request, &store, 0).unwrap();
    assert!(
        request.native_body.as_ref().unwrap()["input"][1]["status"]
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
        if connector == "connector.cpa.codex" {
            expected[1].as_object_mut().unwrap().remove("status");
        }
        for quantum in [1, 127, 8192] {
            let template = project_candidate_request_template(&request, &profile).unwrap();
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
                "actual request-reader bytes must apply the endpoint dialect after Replay externalization"
            );
            assert_eq!(request, original);
        }
    }
    drop((store, manager));
    fs_err_remove_dir(&root);
}
