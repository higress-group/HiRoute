use super::*;
use crate::context_hold::visible_history_with_replay;

fn body() -> serde_json::Value {
    json!({"model":"alias","messages":[
        {"role":"system","content":"Use the available tools."},
        {"role":"user","content":"Read the value."},
        {"role":"assistant","content":null,"reasoning_content":"Read before answering.",
            "tool_calls":[{"id":"call-1","type":"function", "function":{"name":"read","arguments":"{}"}}]},
        {"role":"tool","tool_call_id":"call-1","content":"42"}
    ]})
}

#[test]
fn chat_reasoning_aliases_share_canonical_history_and_preserve_native_payload() {
    let original = body();
    let canonical = decode_ingress_request(IngressProtocol::ChatCompletions, &original).unwrap();
    let expected = visible_history_with_replay(&canonical, &[7; 32], None)
        .unwrap()
        .measure(None)
        .unwrap();
    for duplicate in [false, true] {
        let mut body = original.clone();
        body["messages"][2]["reasoning"] = body["messages"][2]["reasoning_content"].clone();
        if !duplicate {
            body["messages"][2]
                .as_object_mut()
                .unwrap()
                .remove("reasoning_content");
        }
        let request = decode_ingress_request(IngressProtocol::ChatCompletions, &body).unwrap();
        assert!(!request.native_only);
        assert_eq!(request.messages, canonical.messages);
        assert_eq!(
            visible_history_with_replay(&request, &[7; 32], None)
                .unwrap()
                .measure(None)
                .unwrap()
                .complete_digest,
            expected.complete_digest
        );
        let profile = exact_state_profile(
            IngressProtocol::ChatCompletions,
            IngressProtocol::ChatCompletions,
        );
        assert_eq!(
            project_candidate_request(&request, &profile).unwrap().body["messages"],
            body["messages"]
        );
        body["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({"role":"assistant","content":"42",
            "reasoning":"new thinking","reasoning_content":"new thinking"}));
        let request = decode_ingress_request(IngressProtocol::ChatCompletions, &body).unwrap();
        let cleaned =
            project_candidate_request_template_with_cleanup(&request, &profile, Some(2)).unwrap();
        let cleaned: serde_json::Value = serde_json::from_slice(&cleaned.bytes).unwrap();
        assert!(cleaned["messages"][2].get("reasoning").is_none());
        assert!(cleaned["messages"][2].get("reasoning_content").is_none());
        assert_eq!(cleaned["messages"][3], body["messages"][3]);
        assert_eq!(cleaned["messages"][4], body["messages"][4]);
        assert_eq!(request.native_body.as_ref(), Some(&body));
    }
}

#[test]
fn conflicting_or_opaque_chat_reasoning_still_prevents_history_reuse() {
    for alias in [
        json!("different thinking"),
        json!({"encrypted":"opaque"}),
        json!(null),
        json!(["thinking"]),
    ] {
        let mut body = body();
        body["messages"][2]["reasoning"] = alias;
        let request = decode_ingress_request(IngressProtocol::ChatCompletions, &body).unwrap();
        assert!(request.native_only);
        assert!(visible_history_with_replay(&request, &[7; 32], None).is_none());
        let profile = exact_state_profile(
            IngressProtocol::ChatCompletions,
            IngressProtocol::ChatCompletions,
        );
        assert_eq!(
            project_candidate_request(&request, &profile).unwrap().body["messages"],
            body["messages"]
        );
        assert!(
            project_candidate_request_template_with_cleanup(&request, &profile, Some(2)).is_err()
        );
    }
    for index in [0, 1, 3] {
        let mut body = body();
        body["messages"][index]["reasoning"] = json!("unexpected role");
        let request = decode_ingress_request(IngressProtocol::ChatCompletions, &body).unwrap();
        assert!(request.native_only);
    }
    let mut body = body();
    body["messages"][2]["reasoning"] = body["messages"][2]["reasoning_content"].clone();
    body["messages"][2]["unknown_state"] = json!("must not be ignored");
    let request = decode_ingress_request(IngressProtocol::ChatCompletions, &body).unwrap();
    assert!(visible_history_with_replay(&request, &[7; 32], None).is_none());
}

#[test]
fn chat_alias_normalization_does_not_hide_tool_history_changes() {
    let original = body();
    let request = decode_ingress_request(IngressProtocol::ChatCompletions, &original).unwrap();
    let previous = visible_history_with_replay(&request, &[7; 32], None)
        .unwrap()
        .measure(None)
        .unwrap();
    let mut continuation = original.clone();
    continuation["messages"][2]["reasoning"] =
        continuation["messages"][2]["reasoning_content"].clone();
    continuation["messages"]
        .as_array_mut()
        .unwrap()
        .push(json!({"role":"user","content":"Add eight."}));
    for mutation in 0..5 {
        let mut body = continuation.clone();
        match mutation {
            1 => {
                body["messages"][2]["tool_calls"][0]["function"]["arguments"] =
                    json!("{\"other\":true}")
            }
            2 => body["messages"][3]["tool_call_id"] = json!("different-call"),
            3 => body["messages"][3]["content"] = json!("43"),
            4 => body["messages"][2]["reasoning_content"] = json!("changed thinking"),
            _ => {}
        }
        let request = decode_ingress_request(IngressProtocol::ChatCompletions, &body).unwrap();
        let digest = visible_history_with_replay(&request, &[7; 32], None)
            .and_then(|history| history.measure(Some(previous.message_count)))
            .and_then(|digests| digests.prefix_digest);
        assert_eq!(digest == Some(previous.complete_digest), mutation == 0);
    }
}

#[test]
fn replay_backed_chat_aliases_compare_bytes_instead_of_reference_locations() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-chat-alias-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = ReplayManager::open(ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 4096,
        record_bytes: 4096,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .unwrap();
    let tree = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024).unwrap();
    let budget = tree.stream(8 * 1024 * 1024).unwrap();
    let store = manager.begin_request(budget.clone()).unwrap();
    let text = "推理 🧠 \"quoted\"\n".repeat(4096);
    // Alias agreement concerns text bytes. Strict history additionally needs
    // the JSON producer's provenance; a manually stored raw range is not it.
    let mut cases = Vec::new();
    for canonical_proof in [false, true] {
        let encode = |text: &str| {
            if canonical_proof {
                let mut pool = store.begin_content_pool().unwrap();
                let reference = pool.append_canonical_json(&json!(text)).unwrap();
                pool.seal().unwrap();
                reference
            } else {
                store
                    .store_content(&serde_json::to_vec(text).unwrap())
                    .unwrap()
            }
        };
        let first = encode(&text);
        let second = encode(&text);
        let different = encode(&text.replace("推", "理"));
        cases.push((canonical_proof, first, second, different));
    }
    for (canonical_proof, first, second, different) in cases {
        assert_ne!(first, second);
        store
            .prevalidate(&[first.clone(), second.clone(), different.clone()])
            .unwrap();
        for (alias, matches) in [
            (second.json_marker(), true),
            (json!(text), true),
            (different.json_marker(), false),
        ] {
            let mut body = body();
            body["messages"][2]["reasoning_content"] = first.json_marker();
            body["messages"][2]["reasoning"] = alias;
            let request = decode_ingress_request_with_replay(
                IngressProtocol::ChatCompletions,
                &body,
                Some(&store),
            )
            .unwrap();
            assert_eq!(!request.native_only, matches);
            if matches {
                assert_eq!(
                    visible_history_with_replay(&request, &[7; 32], Some(&store))
                        .unwrap()
                        .measure(None)
                        .is_some(),
                    canonical_proof
                );
                let profile = exact_state_profile(
                    IngressProtocol::ChatCompletions,
                    IngressProtocol::ChatCompletions,
                );
                let template = project_candidate_request_template(&request, &profile).unwrap();
                let mut reader =
                    sequential_attempt_body(template, store.clone(), &budget, 257).unwrap();
                let mut bytes = Vec::new();
                while let Some(chunk) = reader.next_chunk().unwrap() {
                    bytes.extend_from_slice(chunk.bytes());
                }
                let output: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(output["messages"][2]["reasoning"], text);
                assert_eq!(output["messages"][2]["reasoning_content"], text);
            }
            assert!(
                decode_ingress_request(IngressProtocol::ChatCompletions, &body)
                    .unwrap()
                    .native_only
            );
        }
    }
    drop(store);
    drop(manager);
    fs_err_remove_dir(&root);
}
