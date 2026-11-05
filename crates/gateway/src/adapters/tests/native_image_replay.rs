use super::*;

#[test]
fn native_image_replay_preserves_the_complete_data_uri_and_detail() {
    let payload = "QUJD".repeat(3_000);
    let image_url = format!("data:image/png;base64,{payload}");
    for protocol in [IngressProtocol::Responses, IngressProtocol::ChatCompletions] {
        for inline_limit in [0, 8 * 1024] {
            let mut document = match protocol {
                IngressProtocol::Responses => json!({
                    "model":"alias","input":[{"role":"user","content":[
                        {"type":"input_image","image_url":image_url,"detail":"high"}
                    ]}]
                }),
                IngressProtocol::ChatCompletions => json!({
                    "model":"alias","messages":[{"role":"user","content":[
                        {"type":"image_url","image_url":{"url":image_url,"detail":"high"}}
                    ]}]
                }),
                _ => unreachable!(),
            };
            let root = std::env::temp_dir().join(format!(
                "hiroute-native-image-replay-{}-{}",
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
            let tree = BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024).unwrap();
            let budget = tree.stream(4 * 1024 * 1024).unwrap();
            let store = manager.begin_request(budget.clone()).unwrap();
            compact_ingress_document(protocol, &mut document, &store).unwrap();
            let mut request = decode_ingress_request(protocol, &document).unwrap();
            externalize_model_request(&mut request, &store, inline_limit).unwrap();
            let profile = CandidateProtocolProfile::exact_portable_path(
                protocol,
                protocol,
                "physical",
                fixed_reasoning("fixed"),
            );
            let template = project_candidate_request_template(&request, &profile).unwrap();
            store.prevalidate(&model_content_refs(&request)).unwrap();
            let mut reader =
                sequential_attempt_body(template, store.clone(), &budget, 257).unwrap();
            let mut wire = Vec::new();
            while let Some(chunk) = reader.next_chunk().unwrap() {
                wire.extend_from_slice(chunk.bytes());
            }
            let body: serde_json::Value = serde_json::from_slice(&wire).unwrap();
            let (url, detail) = match protocol {
                IngressProtocol::Responses => (
                    &body["input"][0]["content"][0]["image_url"],
                    &body["input"][0]["content"][0]["detail"],
                ),
                IngressProtocol::ChatCompletions => (
                    &body["messages"][0]["content"][0]["image_url"]["url"],
                    &body["messages"][0]["content"][0]["image_url"]["detail"],
                ),
                _ => unreachable!(),
            };
            assert_eq!(url, &image_url, "{protocol:?}, inline={inline_limit}");
            assert_eq!(detail, "high");
            assert!(!url.as_str().unwrap().contains("__hiroute_content_ref_"));
            drop((reader, store, manager));
            assert_eq!(budget.snapshot().unwrap().live, 0);
            fs_err_remove_dir(&root);
        }
    }
}

#[test]
fn native_image_replay_does_not_expand_literal_client_markers_recursively() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-native-image-literal-{}-{}",
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
    let tree = BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024).unwrap();
    let budget = tree.stream(4 * 1024 * 1024).unwrap();
    let store = manager.begin_request(budget.clone()).unwrap();
    let mut writer = store.begin_raw().unwrap();
    writer.append(b"unrelated request-owned bytes").unwrap();
    let unrelated = writer.seal().unwrap();
    let literal_url = format!("data:image/png;base64,{}", unrelated.wire_marker());
    let mut document = json!({
        "model":"alias","input":[{"role":"user","content":[
            {"type":"input_image","image_url":literal_url}
        ]}]
    });
    compact_ingress_document(IngressProtocol::Responses, &mut document, &store).unwrap();
    let mut request = decode_ingress_request(IngressProtocol::Responses, &document).unwrap();
    externalize_model_request(&mut request, &store, 0).unwrap();
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let template = project_candidate_request_template(&request, &profile).unwrap();
    store.prevalidate(&model_content_refs(&request)).unwrap();
    let mut reader = sequential_attempt_body(template, store.clone(), &budget, 31).unwrap();
    let mut wire = Vec::new();
    while let Some(chunk) = reader.next_chunk().unwrap() {
        wire.extend_from_slice(chunk.bytes());
    }
    let body: serde_json::Value = serde_json::from_slice(&wire).unwrap();
    assert_eq!(body["input"][0]["content"][0]["image_url"], literal_url);
    assert!(
        !String::from_utf8(wire)
            .unwrap()
            .contains("unrelated request-owned bytes")
    );
    drop((reader, store, manager));
    assert_eq!(budget.snapshot().unwrap().live, 0);
    fs_err_remove_dir(&root);
}
