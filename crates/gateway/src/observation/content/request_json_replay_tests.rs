//! Attach under observation::tests with #[path = "content/request_json_replay_tests.rs"].
//! Exercises the public request-capture seam, including the production ingress
//! path that can leave a Replay string leaf inside a ProviderState JSON value.
use super::*;
use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::Duration;

use crate::content_ref::{ContentValueExt, externalize_model_request};
use crate::replay::{ReplayConfig, ReplayManager, ReplayStore};
use crate::server::core_runtime::model_ir::{ContentPart, ModelRequestIRV1};
use hiroute_gateway_core::runtime::body::BudgetTree;

struct ReplayFixture {
    store: ReplayStore,
    _root: TemporaryRoot,
}

struct TemporaryRoot(std::path::PathBuf);

impl Drop for TemporaryRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl ReplayFixture {
    fn new(threshold: usize) -> Self {
        let root = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "hiroute-content-json-{}-{}",
                std::process::id(),
                super::super::unix_nanos()
            ));
        let manager = ReplayManager::open(ReplayConfig {
            root: root.clone(),
            memory_threshold_bytes: threshold,
            record_bytes: 31,
            orphan_ttl: Duration::from_secs(60),
        })
        .unwrap();
        let tree = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024).unwrap();
        Self {
            store: manager
                .begin_request(tree.stream(8 * 1024 * 1024).unwrap())
                .unwrap(),
            _root: TemporaryRoot(root),
        }
    }
}

struct RecordSink(mpsc::Sender<Value>);
impl ObservationRecordSink for RecordSink {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        self.0
            .send(serde_json::from_slice(record.payload()).unwrap())
            .unwrap();
        Ok(accounted_acknowledgement(record))
    }
}

struct Captured {
    key: [u8; 32],
    records: Vec<Value>,
}

fn capture(request_ir: &ModelRequestIRV1, store: &ReplayStore) -> Captured {
    let (sender, receiver) = mpsc::channel();
    let mut sinks = GatewayObservationSinks::discard();
    sinks.conversation_content = Arc::new(RecordSink(sender));
    let observation = request_with_sinks(
        OtelContentPolicy::Disabled,
        DiagnosticsPort::default(),
        true,
        sinks,
    );
    let backing_before = store.snapshot();
    let bytes_before = store.budget().snapshot().unwrap().live;
    observation.capture_request(request_ir, store);
    assert_eq!(
        store.snapshot(),
        backing_before,
        "observation must not create Replay streams"
    );
    assert_eq!(
        store.budget().snapshot().unwrap().live,
        bytes_before,
        "observation must not retain a new Replay range or per-value backing"
    );
    let mut records = Vec::new();
    loop {
        let record = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("content terminal barrier");
        let terminal = matches!(record["phase"].as_str(), Some("finish" | "abort"));
        assert!(
            record["loss_watermark"].is_null(),
            "unexpected observation loss: {record}"
        );
        records.push(record);
        if terminal {
            break;
        }
    }
    Captured {
        key: observation.inner.key,
        records,
    }
}

fn decoded_base64(value: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = Vec::new();
    let mut pending = 0_u32;
    let mut bits = 0_u32;
    for byte in value.bytes().take_while(|byte| *byte != b'=') {
        let digit = ALPHABET
            .iter()
            .position(|candidate| *candidate == byte)
            .expect("base64 digit") as u32;
        pending = (pending << 6) | digit;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            result.push((pending >> bits) as u8);
        }
    }
    result
}

fn captured_json(capture: &Captured, kind: &str) -> Vec<Value> {
    assert_eq!(capture.records.last().unwrap()["phase"], "finish");
    let mut parts: BTreeMap<(u64, u64), Vec<&Value>> = BTreeMap::new();
    for record in &capture.records {
        if record["phase"] == "append" && record["content_kind"] == kind {
            parts
                .entry((
                    record["message_ordinal"].as_u64().unwrap(),
                    record["part_ordinal"].as_u64().unwrap(),
                ))
                .or_default()
                .push(record);
        }
    }
    parts
        .values()
        .map(|records| {
            let bytes: Vec<u8> = records
                .iter()
                .flat_map(|record| {
                    let bytes = decoded_base64(record["canonical_bytes_base64"].as_str().unwrap());
                    assert!(bytes.len() <= 64 * 1024);
                    bytes
                })
                .collect();
            let mut digest = super::super::crypto::WorkspaceHmac::new(
                &capture.key,
                b"conversation-content-blob",
            );
            digest.update(b"hiroute.model-request-ir/v1");
            digest.update(b"application/json");
            digest.update_stream(&bytes);
            let digest = format!("blob-{}", digest.finish().trim_start_matches("sha256:"));
            for record in records {
                assert_eq!(record["canonical_media_type"], "application/json");
                assert_eq!(record["content_blob_digest"], digest);
                assert_eq!(record["content_id"], records[0]["content_id"]);
                if !record["content_ref"].is_null() {
                    assert_eq!(
                        record["content_ref"]["byte_count"].as_u64(),
                        Some(bytes.len() as u64)
                    );
                    assert_eq!(record["content_ref"]["digest"], digest);
                }
            }
            serde_json::from_slice(&bytes).expect("captured semantic JSON")
        })
        .collect()
}

fn production_request(wire: &Value, replay: &ReplayStore, inline_limit: usize) -> ModelRequestIRV1 {
    use crate::content_ref::{
        compact_ingress_document_with_markers, parse_ingress_document, scan_ingress_document,
    };
    use crate::server::core_runtime::adapters::decode_ingress_request_with_replay;
    let mut raw = replay.begin_raw().unwrap();
    raw.append(&serde_json::to_vec(wire).unwrap()).unwrap();
    let raw = raw.seal().unwrap();
    let stats = scan_ingress_document(replay.reader(&raw).unwrap()).unwrap();
    let (mut document, mut workspace) =
        parse_ingress_document(IngressProtocol::Responses, replay, &raw, stats).unwrap();
    compact_ingress_document_with_markers(
        IngressProtocol::Responses,
        &mut document,
        replay,
        workspace.generated_markers(),
    )
    .unwrap();
    let mut request =
        decode_ingress_request_with_replay(IngressProtocol::Responses, &document, Some(replay))
            .unwrap();
    externalize_model_request(&mut request, replay, inline_limit).unwrap();
    request
}

fn state_request(values: impl IntoIterator<Item = Value>) -> ModelRequestIRV1 {
    use crate::server::core_runtime::adapters::decode_ingress_request;
    use crate::server::core_runtime::model_ir::OpaqueProviderState;
    let mut request = decode_ingress_request(IngressProtocol::Responses, &json!({
        "model":"route", "input":[{"type":"reasoning","summary":[],"encrypted_content":"placeholder"}]
    })).unwrap();
    request.messages[0].content = values
        .into_iter()
        .map(|value| ContentPart::ProviderState {
            state: Box::new(OpaqueProviderState {
                block_index: None,
                kind: "encrypted_content".into(),
                value,
            }),
        })
        .collect();
    request
}

#[test]
fn request_json_capture_expands_production_reasoning_string_leaves() {
    for threshold in [64, 2 * 1024 * 1024] {
        for inline_limit in [0, 8 * 1024, 64 * 1024] {
            let fixture = ReplayFixture::new(threshold);
            let opaque = "opaque-\"\\\n\t中文".repeat(160);
            let request = production_request(
                &json!({
                    "model":"route", "instructions":"x".repeat(8 * 1024),
                    "input":[{"type":"reasoning","id":"rs-captured","summary":[],"encrypted_content":opaque}]
                }),
                &fixture.store,
                inline_limit,
            );
            let captured = capture(&request, &fixture.store);
            assert_eq!(
                captured_json(&captured, "provider_state"),
                vec![json!(opaque)]
            );
        }
    }
}

#[test]
fn request_json_capture_preserves_typed_nested_replay_and_all_json_consumers() {
    use crate::server::core_runtime::adapters::decode_ingress_request;
    use crate::server::core_runtime::model_ir::ToolOutput;
    for threshold in [64, 2 * 1024 * 1024] {
        let fixture = ReplayFixture::new(threshold);
        let text = "quoted \"text\"\n中文\\tail";
        let leaf = fixture.store.store_content(text.as_bytes()).unwrap();
        let nested = json!({"z":2,"a":[true,null,"nested"]});
        let mut pool = fixture.store.begin_content_pool().unwrap();
        let nested_ref = pool.append_canonical_json(&nested).unwrap();
        pool.seal().unwrap();
        let value = json!({"leaf":leaf.wire_marker(),"nested":nested_ref.json_marker(),"number":7});
        let expected = json!({"leaf":text,"nested":nested,"number":7});
        let mut request = decode_ingress_request(
            IngressProtocol::Responses,
            &json!({
                "model":"route", "input":[
                    {"type":"function_call","call_id":"call","name":"tool","arguments":"{}"},
                    {"type":"function_call_output","call_id":"call","output":{"ok":true}}
                ], "tools":[{"type":"function","name":"tool","parameters":{"type":"object"}}]
            }),
        )
        .unwrap();
        let ContentPart::ToolCall { arguments, .. } = &mut request.messages[0].content[0] else {
            panic!("tool call")
        };
        *arguments = value.clone();
        let ContentPart::ToolResult { output, .. } = &mut request.messages[1].content[0] else {
            panic!("tool result")
        };
        *output = ToolOutput::Json(value.clone());
        request.tools[0].input_schema = Some(value.clone());
        let captured = capture(&request, &fixture.store);
        for kind in ["tool_call_arguments", "tool_result_json", "tool_schema"] {
            assert_eq!(
                captured_json(&captured, kind),
                vec![expected.clone()],
                "{kind}"
            );
        }
        let captured = capture(
            &state_request([value, nested_ref.json_marker()]),
            &fixture.store,
        );
        assert_eq!(
            captured_json(&captured, "provider_state"),
            vec![expected, nested]
        );
    }
}

#[test]
fn request_json_capture_keeps_small_inline_values_without_new_replay_ranges() {
    let fixture = ReplayFixture::new(64 * 1024);
    let expected: Vec<Value> = (0..16)
        .map(|n| json!({"value":n,"text":"small","ok":true}))
        .collect();
    assert_eq!(fixture.store.snapshot().live_streams, 0);
    let captured = capture(&state_request(expected.clone()), &fixture.store);
    assert_eq!(captured_json(&captured, "provider_state"), expected);
    assert_eq!(fixture.store.snapshot().live_streams, 0);
}

#[test]
fn request_json_capture_preserves_a_literal_locator_after_production_ingress_isolation() {
    let fixture = ReplayFixture::new(64);
    let live = fixture
        .store
        .store_content(b"must-not-substitute-this-secret")
        .unwrap();
    let literal = live.wire_marker();
    let request = production_request(
        &json!({"model":"route","input":[
            {"type":"reasoning","summary":[],"encrypted_content":literal}
        ]}),
        &fixture.store,
        0,
    );
    let captured = capture(&request, &fixture.store);
    assert_eq!(
        captured_json(&captured, "provider_state"),
        vec![json!(literal)]
    );
}

#[test]
fn request_json_capture_aborts_when_a_replay_string_leaf_is_unavailable() {
    let fixture = ReplayFixture::new(64);
    let reference = fixture
        .store
        .store_content(b"unavailable opaque bytes")
        .unwrap();
    let value = Value::String(reference.wire_marker());
    assert!(value.as_str().unwrap().content_ref().is_some());
    fixture.store.release_stream(&reference).unwrap();
    let captured = capture(&state_request([value]), &fixture.store);
    let terminal = captured.records.last().unwrap();
    assert_eq!(terminal["phase"], "abort");
    assert_eq!(terminal["abort_reason"], "canonical_content_unavailable");
    assert!(
        !captured
            .records
            .iter()
            .any(|record| record["phase"] == "finish")
    );
}
