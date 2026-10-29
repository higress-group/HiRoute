use super::*;
use crate::server::core_runtime::adapters::decode_ingress_request;
use crate::server::core_runtime::model_ir::ResponsesReasoningHistoryV1;
use crate::server::request_plan::IngressProtocol;
use serde_json::json;

fn request(role: &str) -> ModelRequestIRV1 {
    let mut request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"route","stream":false,"input":[
            {"role":role,"content":"first instructions"},
            {"role":"user","content":"actual task"},
            {"role":"assistant","content":"accepted output"},
            {"type":"function_call","call_id":"call","name":"lookup","arguments":"{}"},
            {"type":"function_call_output","call_id":"call","output":"tool receipt"}
        ]}),
    )
    .unwrap();
    // These tests mutate canonical history directly. Native layout integrity
    // has separate ingress-based tests and cannot describe these edited values.
    request.native_body = None;
    request.native_history = None;
    request
}

fn measure(request: &ModelRequestIRV1, prefix: usize) -> HistoryDigests {
    visible_history(request, &[37; 32])
        .unwrap()
        .measure(Some(prefix))
        .unwrap()
}

#[test]
fn opaque_observed_capture_does_not_establish_strict_provider_prefix_affinity() {
    let mut opaque = request("system");
    opaque.native_only = true;
    opaque.native_body = Some(json!({"input":[{"type":"agent_message","content":"opaque peer"}]}));
    assert!(visible_history(&opaque, &[37; 32]).is_none());
    let observed = observed_history_with_replay(&opaque, &[37; 32], None)
        .unwrap()
        .measure(Some(opaque.messages.len()));
    // A fabricated/missing layout cannot establish either strict or semantic
    // continuity. Request-owned execution capture has its independent scope.
    assert!(observed.is_none());
    assert!(branch_anchors(&opaque, &[37; 32], None, &[]).is_none());
}

#[test]
fn only_same_position_system_or_developer_text_is_omitted_from_semantic_digest() {
    for role in ["system", "developer"] {
        let original = request(role);
        let before = measure(&original, original.messages.len());
        let mut changed = original.clone();
        changed.messages[0].content[0] = ContentPart::Text {
            text: "new native instructions and skills".into(),
        };
        let after = measure(&changed, original.messages.len());
        assert_ne!(before.complete_digest, after.complete_digest);
        assert_eq!(
            before.semantic_complete_digest,
            after.semantic_complete_digest
        );
        assert_eq!(
            Some(before.semantic_complete_digest),
            after.semantic_prefix_digest
        );
        assert_ne!(Some(before.complete_digest), after.prefix_digest);
    }
}

#[test]
fn semantic_digest_keeps_roles_content_tools_metadata_and_message_positions_strict() {
    let original = request("system");
    let baseline = measure(&original, original.messages.len());
    let mut variants = Vec::new();
    for index in [1, 2] {
        let mut value = original.clone();
        value.messages[index].content[0] = ContentPart::Text {
            text: "rewritten".into(),
        };
        variants.push(value);
    }
    let mut role = original.clone();
    role.messages[0].role = MessageRole::Developer;
    variants.push(role);
    let mut name = original.clone();
    name.messages[0].name = Some("changed".into());
    variants.push(name);
    let mut part_count = original.clone();
    part_count.messages[0].content.push(ContentPart::Text {
        text: "extra block".into(),
    });
    variants.push(part_count);
    let mut arguments = original.clone();
    let ContentPart::ToolCall {
        arguments: value, ..
    } = &mut arguments.messages[3].content[0]
    else {
        panic!()
    };
    *value = json!({"changed":true});
    variants.push(arguments);
    let mut output = original.clone();
    let ContentPart::ToolResult { output: value, .. } = &mut output.messages[4].content[0] else {
        panic!()
    };
    *value = ToolOutput::Text("changed receipt".into());
    variants.push(output);
    let mut metadata = original.clone();
    metadata.responses_reasoning_history.insert(
        0,
        ResponsesReasoningHistoryV1 {
            native_fields: serde_json::Map::from_iter([(
                "summary".into(),
                json!("native sidecar"),
            )]),
            encrypted_content: ResponsesReasoningEncryptedContentV1::Absent,
        },
    );
    variants.push(metadata);
    let mut shifted = original.clone();
    shifted.messages.insert(0, original.messages[0].clone());
    variants.push(shifted);
    for changed in variants {
        let measured = measure(&changed, original.messages.len());
        assert_ne!(
            Some(baseline.semantic_complete_digest),
            measured.semantic_prefix_digest
        );
    }

    // Non-text instruction content is also strict, including a tool part in an
    // instruction envelope; the exception is never a role-wide content skip.
    let mut with_tool = original.clone();
    with_tool.messages[0]
        .content
        .push(original.messages[3].content[0].clone());
    let before = measure(&with_tool, with_tool.messages.len());
    let ContentPart::ToolCall { name, .. } = &mut with_tool.messages[0].content[1] else {
        panic!()
    };
    *name = "different_tool".into();
    assert_ne!(
        before.semantic_complete_digest,
        measure(&with_tool, with_tool.messages.len()).semantic_complete_digest
    );
}

#[test]
fn borrowed_semantic_view_cannot_replace_strict_execution_evidence_or_positions() {
    let original = request("system");
    let mut semantic = original.clone();
    semantic.messages[1].content[0] = ContentPart::Text {
        text: "verified native user projection".into(),
    };
    let strict = measure(&original, original.messages.len());
    let view = measure(&semantic, semantic.messages.len());
    let borrowed = visible_history(&original, &[37; 32])
        .unwrap()
        .with_semantic_request(Some(&semantic))
        .unwrap()
        .measure(Some(original.messages.len()))
        .unwrap();
    assert_eq!(borrowed.complete_digest, strict.complete_digest);
    assert_eq!(borrowed.prefix_digest, strict.prefix_digest);
    assert_eq!(
        borrowed.semantic_complete_digest,
        view.semantic_complete_digest
    );
    semantic.messages.pop();
    assert!(
        visible_history(&original, &[37; 32])
            .unwrap()
            .with_semantic_request(Some(&semantic))
            .is_none()
    );
}

#[test]
fn complete_history_fingerprints_detect_same_length_edits_and_part_boundaries() {
    let mut original = request("system");
    let text = "x".repeat(128 * 1024);
    original.messages[1].content = vec![ContentPart::Text { text: text.clone() }];
    let baseline = measure(&original, original.messages.len());
    let anchors = branch_anchors(&original, &[37; 32], None, &[2]).unwrap();
    for index in [0, text.len() / 2, text.len() - 1] {
        let mut changed = original.clone();
        let mut bytes = text.as_bytes().to_vec();
        bytes[index] = b'y';
        changed.messages[1].content = vec![ContentPart::Text {
            text: String::from_utf8(bytes).unwrap(),
        }];
        let actual = measure(&changed, changed.messages.len());
        assert_ne!(actual.complete_digest, baseline.complete_digest);
        assert_ne!(
            actual.semantic_complete_digest,
            baseline.semantic_complete_digest
        );
        assert_ne!(
            branch_anchors(&changed, &[37; 32], None, &[2])
                .unwrap()
                .prefixes,
            anchors.prefixes
        );
    }
    let mut split = original.clone();
    split.messages[1].content = vec![
        ContentPart::Text {
            text: text[..text.len() / 2].into(),
        },
        ContentPart::Text {
            text: text[text.len() / 2..].into(),
        },
    ];
    assert_ne!(
        measure(&split, split.messages.len()).complete_digest,
        baseline.complete_digest
    );
    assert_eq!(
        measure(&original, original.messages.len()).complete_digest,
        baseline.complete_digest
    );
}

#[test]
fn strict_semantic_and_branch_evidence_reuse_full_request_content() {
    use crate::content_ref::externalize_model_request;
    use crate::replay::{ReplayConfig, ReplayManager};
    use hiroute_gateway_core::runtime::body::BudgetTree;
    for threshold in [31, 1024 * 1024] {
        let root = std::env::temp_dir().join(format!(
            "hiroute-history-fingerprints-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = ReplayManager::open(ReplayConfig {
            root: root.clone(),
            memory_threshold_bytes: threshold,
            record_bytes: 31,
            orphan_ttl: std::time::Duration::from_secs(60),
        })
        .unwrap();
        let budget = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024)
            .unwrap()
            .stream(8 * 1024 * 1024)
            .unwrap();
        let store = manager.begin_request(budget.clone()).unwrap();
        let mut input = request("system");
        input.messages[1].content = vec![ContentPart::Text {
            text: "full content 中文".repeat(4096),
        }];
        let plain = measure(&input, input.messages.len());
        externalize_model_request(&mut input, &store, 0).unwrap();
        let history = visible_history_with_replay(&input, &[37; 32], Some(&store)).unwrap();
        let first = history.measure(Some(input.messages.len())).unwrap();
        assert_eq!(first.complete_digest, plain.complete_digest);
        assert_eq!(
            first.semantic_complete_digest,
            plain.semantic_complete_digest
        );
        let measured = store.runtime_fingerprint_computations();
        assert!(measured > 0);
        for _ in 0..3 {
            assert_eq!(
                history
                    .measure(Some(input.messages.len()))
                    .unwrap()
                    .complete_digest,
                first.complete_digest
            );
            branch_anchors(&input, &[37; 32], Some(&store), &[2]).unwrap();
            assert_eq!(store.runtime_fingerprint_computations(), measured);
        }
        drop((input, store, manager));
        assert_eq!(budget.snapshot().unwrap().live, 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
