use super::*;

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
