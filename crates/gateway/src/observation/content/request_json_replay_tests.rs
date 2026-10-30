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
    assert_capture_ids(&observation, &records);
    Captured {
        key: observation.inner.key,
        records,
    }
}

// Recompute the published identities from their original formula, independently
// of the private chunk fields and the reused fork state.
fn assert_capture_ids(observation: &RequestObservation, records: &[Value]) {
    use super::super::crypto::{WorkspaceHmac, stable_id};
    let key = &observation.inner.key;
    let metadata = observation.metadata();
    let fork = stable_id(
        "fork",
        key,
        b"conversation-fork",
        &[metadata.request_id.as_bytes(), b"request_input", b"request"],
    );
    let mut transcript = WorkspaceHmac::new(key, b"transcript-root");
    for part in [
        b"hiroute.model-request-ir/v1".as_slice(),
        b"no-parent-root",
        b"append",
        metadata.request_id.as_bytes(),
        b"request_input",
        fork.as_bytes(),
    ] {
        transcript.update(part);
    }
    let mut previous = None;
    for record in records {
        assert_eq!(record["fork_id"], fork);
        if record["phase"] != "append" {
            continue;
        }
        let message = record["message_ordinal"].as_u64().unwrap() as u32;
        let part = record["part_ordinal"].as_u64().unwrap() as u32;
        let message_id = stable_id(
            "message",
            key,
            b"conversation-message-instance",
            &[
                metadata.conversation_id.as_bytes(),
                b"no-parent-root",
                metadata.request_id.as_bytes(),
                b"request_input",
                &message.to_be_bytes(),
                record["message_role"].as_str().unwrap().as_bytes(),
            ],
        );
        let content_id = stable_id(
            "content",
            key,
            b"conversation-content-id",
            &[
                message_id.as_bytes(),
                &part.to_be_bytes(),
                record["content_blob_digest"].as_str().unwrap().as_bytes(),
            ],
        );
        assert_eq!(record["message_instance_id"], message_id);
        assert_eq!(record["content_id"], content_id);
        if !record["content_ref"].is_null() {
            assert_eq!(record["content_ref"]["content_id"], content_id);
        }
        if previous != Some((message, part)) {
            transcript.update(message_id.as_bytes());
            transcript.update(content_id.as_bytes());
            previous = Some((message, part));
        }
    }
    if records.last().unwrap()["phase"] == "finish" {
        assert_eq!(
            records.last().unwrap()["result_transcript_root"],
            format!(
                "transcript-{}",
                transcript.finish().trim_start_matches("sha256:")
            )
        );
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

fn asynchronous_observation() -> (
    RequestObservation,
    mpsc::Receiver<Value>,
    super::super::content_work::ContentGate,
) {
    let (sender, receiver) = mpsc::channel();
    let mut observation = request_with_sinks_capacity(
        OtelContentPolicy::Disabled,
        DiagnosticsPort::default(),
        true,
        GatewayObservationSinks {
            conversation_content: Arc::new(RecordSink(sender)),
            ..GatewayObservationSinks::discard()
        },
        16 * 1024 * 1024,
    );
    let (worker, gate) = super::super::content_work::ContentWorkProducer::paused();
    Arc::get_mut(&mut observation.inner)
        .unwrap()
        .channels
        .content_work = worker;
    (observation, receiver, gate)
}

fn content_attempt() -> super::super::request::AttemptObservation {
    super::super::request::AttemptObservation {
        reasoning_fields_removed: 0,
        response_cleanup_reported: false,
        ordinal: 1,
        attempt_id: "attempt:test".into(),
        stable_binding_id: "binding:test".into(),
        credential_ref: "credential:test".into(),
        key_id: "key:test".into(),
        provider_name: "provider:test".into(),
        request_model: "actual-model".into(),
        upstream_protocol: "responses".into(),
        model_configuration_id: "model:test".into(),
        adapter_revision: "adapter:test".into(),
        candidate_id: "candidate:test".into(),
        profile_digest: "profile:test".into(),
        start_reason: "initial".into(),
        previous_attempt_id: None,
        started_at: std::time::Instant::now(),
        effective_cost_micros: None,
        cost_class: None,
        credential_generation: 1,
    }
}

#[test]
fn async_content_keeps_the_request_root_before_fast_response_after_foreground_drop() {
    use super::super::content::CanonicalFrameDelivery;
    use crate::content_ref::model_content_refs;
    use crate::server::core_runtime::model_ir::{ModelEvent, ModelStreamEventV1};
    for threshold in [31, 1024 * 1024] {
        let ReplayFixture { store, _root } = ReplayFixture::new(threshold);
        let budget = store.budget().clone();
        let mut input = state_request([json!({"blob":"complete text 中文".repeat(1024)})]);
        externalize_model_request(&mut input, &store, 0).unwrap();
        store.prevalidate(&model_content_refs(&input)).unwrap();
        let oracle = capture(&input, &store);
        let (mut observation, receiver, gate) = asynchronous_observation();
        observation.content_occurred_at = Some(123_456);
        Arc::get_mut(&mut observation.inner).unwrap().key = oracle.key;
        observation.capture_request_async(&input, &store);
        let attempt = content_attempt();
        observation.lock_state().accepted_attempt = Some(attempt.clone());
        observation.begin_response_content(&attempt);
        observation.emit_canonical_response_delivery(
            &attempt,
            "accepted-frame",
            CanonicalFrameDelivery {
                events: vec![ModelStreamEventV1::new(
                    1,
                    ModelEvent::TextDelta {
                        index: 0,
                        text: "delivered answer".into(),
                    },
                )],
                ..CanonicalFrameDelivery::default()
            },
        );
        observation.finish_response_content("finish", None);
        // No encoding has run, yet the request path and actual response
        // handoff have completed. The original IR/backing can now disappear.
        assert!(receiver.try_recv().is_err());
        drop((input, store));
        assert!(budget.snapshot().unwrap().live > 0);
        gate.release();
        let mut records = Vec::new();
        loop {
            let record = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(
                record["loss_watermark"].is_null(),
                "unexpected loss: {record}"
            );
            let end = record["direction"] == "response_delivered" && record["phase"] == "finish";
            records.push(record);
            if end {
                break;
            }
        }
        observation.inner.channels.content_work.flush(&observation);
        assert!(
            records
                .iter()
                .all(|record| record["occurred_at_unix_nanos"] == 123_456)
        );
        assert_eq!(budget.snapshot().unwrap().live, 0);
        let request_records = records
            .iter()
            .filter(|record| record["direction"] == "request_input")
            .cloned()
            .collect::<Vec<_>>();
        assert_capture_ids(&observation, &request_records);
        let stable = |record: &Value| {
            let mut fields = record.as_object().unwrap().clone();
            for name in ["producer", "sequence", "event_id", "occurred_at_unix_nanos"] {
                fields.remove(name);
            }
            fields
        };
        assert_eq!(
            request_records.iter().map(stable).collect::<Vec<_>>(),
            oracle.records.iter().map(stable).collect::<Vec<_>>()
        );
        let parent = &oracle.records.last().unwrap()["result_transcript_root"];
        for record in records
            .iter()
            .filter(|record| record["direction"] == "response_delivered")
        {
            assert_eq!(&record["parent_transcript_root"], parent);
            assert_eq!(record["attempt_id"], "attempt:test");
        }
    }
}

#[test]
fn full_content_work_queue_declares_loss_without_waiting_and_next_request_recovers() {
    use super::super::content::CanonicalFrameDelivery;
    use crate::content_ref::model_content_refs;
    let fixture = ReplayFixture::new(1024 * 1024);
    let mut input = state_request([json!({"text":"bounded captured input"})]);
    externalize_model_request(&mut input, &fixture.store, 0).unwrap();
    fixture
        .store
        .prevalidate(&model_content_refs(&input))
        .unwrap();
    let (observation, receiver, gate) = asynchronous_observation();
    observation.capture_request_async(&input, &fixture.store);
    let attempt = content_attempt();
    observation.begin_response_content(&attempt);
    for _ in 0..512 {
        observation.emit_canonical_response_delivery(
            &attempt,
            "accepted-frame",
            CanonicalFrameDelivery::default(),
        );
    }
    // The independent producer must report loss while the encoder is still
    // blocked. A successful content terminal is forbidden in this state.
    let gap = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        gap["schema_version"],
        "hiroute.observation.gap-heartbeat/v1"
    );
    assert!(gap["loss_watermark"].is_object());
    gate.release();
    observation.inner.channels.content_work.flush(&observation);
    assert!(
        receiver
            .try_iter()
            .all(|record| record["phase"] != "finish")
    );

    let mut recovered = request(OtelContentPolicy::Disabled);
    let inner = Arc::get_mut(&mut recovered.inner).unwrap();
    inner.channels = observation.inner.channels.clone();
    inner.key = observation.inner.key;
    inner.metadata.request_id = "request:recovered".into();
    recovered.capture_request_async(&input, &fixture.store);
    observation.inner.channels.content_work.flush(&recovered);
    loop {
        let record = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            record["loss_watermark"].is_null(),
            "recovery retained a gap: {record}"
        );
        if record["phase"] == "finish" {
            assert_eq!(record["correlation"]["request_id"], "request:recovered");
            break;
        }
    }
}

#[test]
fn unreadable_deferred_input_aborts_capture_without_a_complete_response_root() {
    use crate::content_ref::{JsonValueExt, model_content_refs};
    let fixture = ReplayFixture::new(31);
    let mut input = state_request([json!({"text":"deferred disk input".repeat(1024)})]);
    externalize_model_request(&mut input, &fixture.store, 0).unwrap();
    fixture
        .store
        .prevalidate(&model_content_refs(&input))
        .unwrap();
    let (observation, receiver, gate) = asynchronous_observation();
    observation.capture_request_async(&input, &fixture.store);
    observation.begin_response_content(&content_attempt());
    observation.finish_response_content("finish", None);
    // The original content was validated at admission. Simulate backing loss
    // only after the optional worker owns its lease and before it reads.
    let ContentPart::ProviderState { state } = &input.messages[0].content[0] else {
        panic!("canonical state fixture")
    };
    let reference = state.value.content_ref().unwrap();
    fixture.store.truncate_stream_last_byte(&reference).unwrap();
    assert!(
        std::io::copy(
            &mut fixture.store.reader(&reference).unwrap(),
            &mut std::io::sink()
        )
        .is_err(),
        "fault must corrupt the exact captured range"
    );
    gate.release();
    observation.inner.channels.content_work.flush(&observation);
    let mut records = Vec::new();
    loop {
        let record = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        let gap = record["schema_version"] == "hiroute.observation.gap-heartbeat/v1";
        records.push(record);
        if (gap
            || records
                .iter()
                .any(|record| record["loss_watermark"].is_object()))
            && records.iter().any(|record| record["phase"] == "abort")
        {
            break;
        }
    }
    records.extend(receiver.try_iter());
    assert!(records.iter().any(|record| record["phase"] == "abort"));
    assert!(!records.iter().any(|record| record["phase"] == "finish"));
}

#[test]
fn optional_retention_pressure_preserves_the_original_synchronous_capture() {
    use crate::server::core_runtime::adapters::decode_ingress_request;
    use hiroute_gateway_core::runtime::body::MemoryRole;
    let fixture = ReplayFixture::new(1024 * 1024);
    let input = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"route","input":"already owned inline content"}),
    )
    .unwrap();
    fixture.store.prevalidate(&[]).unwrap();
    let budget = fixture.store.budget();
    let guard = budget
        .reserve(
            MemoryRole::ModelIrBacking,
            budget.available_bytes().unwrap() - 64,
        )
        .unwrap();
    let (observation, receiver, _gate) = asynchronous_observation();
    observation.capture_request_async(&input, &fixture.store);
    assert!(
        observation.inner.async_content.get().is_none(),
        "no async ownership was admitted"
    );
    loop {
        let record = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(record["loss_watermark"].is_null(), "{record}");
        assert_ne!(record["phase"], "abort");
        if record["phase"] == "finish" {
            break;
        }
    }
    drop(guard);
}

#[test]
fn queued_snapshot_pressure_falls_back_before_async_content_ownership() {
    use crate::server::core_runtime::adapters::decode_ingress_request;
    let fixture = ReplayFixture::new(1024 * 1024);
    let text = "x".repeat(20 * 1024);
    let input = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"route","input":text}),
    )
    .unwrap();
    fixture.store.prevalidate(&[]).unwrap();
    let before = fixture.store.budget().snapshot().unwrap().live;
    let (sender, receiver) = mpsc::channel();
    let observation = request_with_sinks_capacity(
        OtelContentPolicy::Disabled,
        DiagnosticsPort::default(),
        true,
        GatewayObservationSinks {
            conversation_content: Arc::new(RecordSink(sender)),
            ..GatewayObservationSinks::discard()
        },
        512 * 1024,
    );
    // A different request's queued snapshot fits, but leaves too little work
    // credit for this one. The encoded original content still fits the channel.
    let _other = observation
        .inner
        .channels
        .content
        .reserve_work(400 * 1024)
        .unwrap();
    observation.capture_request_async(&input, &fixture.store);
    assert!(observation.inner.async_content.get().is_none());
    assert_eq!(fixture.store.budget().snapshot().unwrap().live, before);
    let mut captured = Vec::new();
    loop {
        let record = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(record["loss_watermark"].is_null(), "{record}");
        assert_ne!(record["phase"], "abort");
        if record["phase"] == "append" {
            captured.extend(decoded_base64(
                record["canonical_bytes_base64"].as_str().unwrap(),
            ));
        }
        if record["phase"] == "finish" {
            break;
        }
    }
    assert_eq!(captured, text.as_bytes());
}

#[test]
fn two_queued_snapshots_use_the_existing_content_ceiling_without_sync_fallback() {
    use crate::server::core_runtime::adapters::decode_ingress_request;
    let fixture = ReplayFixture::new(1024 * 1024);
    fixture.store.prevalidate(&[]).unwrap();
    let text = "x".repeat(190 * 1024);
    let input = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"route","input":text}),
    )
    .unwrap();
    let (sender, receiver) = mpsc::channel();
    let mut first = request_with_sinks_capacity(
        OtelContentPolicy::Disabled,
        DiagnosticsPort::default(),
        true,
        GatewayObservationSinks {
            conversation_content: Arc::new(RecordSink(sender)),
            ..GatewayObservationSinks::discard()
        },
        512 * 1024,
    );
    let (worker, gate) = super::super::content_work::ContentWorkProducer::paused();
    Arc::get_mut(&mut first.inner)
        .unwrap()
        .channels
        .content_work = worker;
    let mut second = request(OtelContentPolicy::Disabled);
    let inner = Arc::get_mut(&mut second.inner).unwrap();
    inner.channels = first.inner.channels.clone();
    inner.key = first.inner.key;
    inner.metadata.request_id = "request:other".into();
    first.capture_request_async(&input, &fixture.store);
    second.capture_request_async(&input, &fixture.store);
    assert!(first.inner.async_content.get().is_some());
    assert!(
        second.inner.async_content.get().is_some(),
        "preallocating 256 inline jobs forced this snapshot into foreground encoding"
    );
    assert!(receiver.try_recv().is_err());
    let (total, _, capacity) = first.inner.channels.content.work_state().unwrap();
    assert_eq!(capacity, 512 * 1024);
    assert!(total <= capacity);
    gate.release();
    first.inner.channels.content_work.flush(&first);
    let mut bodies = BTreeMap::<String, Vec<u8>>::new();
    let mut finished = 0;
    while finished < 2 {
        let record = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(record["loss_watermark"].is_null(), "{record}");
        assert_ne!(record["phase"], "abort");
        if record["phase"] == "append" {
            bodies
                .entry(record["correlation"]["request_id"].as_str().unwrap().into())
                .or_default()
                .extend(decoded_base64(
                    record["canonical_bytes_base64"].as_str().unwrap(),
                ));
        }
        finished += usize::from(record["phase"] == "finish");
    }
    assert_eq!(finished, 2);
    assert_eq!(bodies.len(), 2);
    assert!(bodies.values().all(|body| body == text.as_bytes()));
}

#[test]
fn active_snapshot_does_not_reduce_encoded_capacity_while_the_sink_is_blocked() {
    use crate::server::core_runtime::adapters::decode_ingress_request;
    struct SinkGate(Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);
    impl Drop for SinkGate {
        fn drop(&mut self) {
            *self.0.0.lock().unwrap() = true;
            self.0.1.notify_all();
        }
    }
    struct BlockedSink {
        gate: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
        records: mpsc::Sender<Value>,
    }
    impl ObservationRecordSink for BlockedSink {
        fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
            let mut open = self.gate.0.lock().unwrap();
            while !*open {
                open = self.gate.1.wait(open).unwrap();
            }
            self.records
                .send(serde_json::from_slice(record.payload()).unwrap())
                .unwrap();
            Ok(accounted_acknowledgement(record))
        }
    }
    let fixture = ReplayFixture::new(1024 * 1024);
    let text = "x".repeat(200 * 1024);
    let input = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"route","input":text}),
    )
    .unwrap();
    fixture.store.prevalidate(&[]).unwrap();
    let gate = SinkGate(Arc::new((
        std::sync::Mutex::new(false),
        std::sync::Condvar::new(),
    )));
    let (sender, receiver) = mpsc::channel();
    let observation = request_with_sinks_capacity(
        OtelContentPolicy::Disabled,
        DiagnosticsPort::default(),
        true,
        GatewayObservationSinks {
            conversation_content: Arc::new(BlockedSink {
                gate: Arc::clone(&gate.0),
                records: sender,
            }),
            ..GatewayObservationSinks::discard()
        },
        512 * 1024,
    );
    observation.capture_request_async(&input, &fixture.store);
    assert!(observation.inner.async_content.get().is_some());
    observation.inner.channels.content_work.flush(&observation);
    // All encoding completes while the existing sink is still blocked. The
    // snapshot remains charged by the request budget until its job is done.
    assert!(receiver.try_recv().is_err());
    drop(gate);
    let mut captured = Vec::new();
    loop {
        let record = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(record["loss_watermark"].is_null(), "{record}");
        if record["phase"] == "append" {
            captured.extend(decoded_base64(
                record["canonical_bytes_base64"].as_str().unwrap(),
            ));
        }
        if record["phase"] == "finish" {
            break;
        }
    }
    assert_eq!(captured, text.as_bytes());
}

#[test]
fn dense_async_history_waits_for_record_credit_and_keeps_every_original_byte() {
    struct Gate(Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);
    impl Drop for Gate {
        fn drop(&mut self) {
            *self.0.0.lock().unwrap() = true;
            self.0.1.notify_all();
        }
    }
    struct HeldSink {
        gate: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
        records: mpsc::Sender<Value>,
    }
    impl ObservationRecordSink for HeldSink {
        fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
            let mut open = self.gate.0.lock().unwrap();
            while !*open {
                open = self.gate.1.wait(open).unwrap();
            }
            self.records
                .send(serde_json::from_slice(record.payload()).unwrap())
                .unwrap();
            Ok(accounted_acknowledgement(record))
        }
    }
    let fixture = ReplayFixture::new(1024 * 1024);
    let messages: Vec<_> = (0..256)
        .map(|index| {
            json!({
                "role":"user", "content":format!("part-{index}:{}", "x".repeat(512))
            })
        })
        .collect();
    let expected: Vec<u8> = messages
        .iter()
        .flat_map(|message| message["content"].as_str().unwrap().bytes())
        .collect();
    let input = production_request(
        &json!({"model":"route","input":messages}),
        &fixture.store,
        0,
    );
    fixture
        .store
        .prevalidate(&crate::content_ref::model_content_refs(&input))
        .unwrap();
    let gate = Gate(Arc::new((
        std::sync::Mutex::new(false),
        std::sync::Condvar::new(),
    )));
    let (sender, receiver) = mpsc::channel();
    let observation = request_with_sinks_capacity(
        OtelContentPolicy::Disabled,
        DiagnosticsPort::default(),
        true,
        GatewayObservationSinks {
            conversation_content: Arc::new(HeldSink {
                gate: gate.0.clone(),
                records: sender,
            }),
            ..GatewayObservationSinks::discard()
        },
        512 * 1024,
    );
    observation.capture_request_async(&input, &fixture.store);
    assert!(observation.inner.async_content.get().is_some());
    // The caller already returned. Park the optional encoder behind the exact
    // full-record boundary, rather than depending on a slow-sink sleep.
    observation
        .inner
        .channels
        .content
        .wait_until_credit_blocked();
    assert!(receiver.try_recv().is_err());
    drop(input);
    drop(gate);
    observation.inner.channels.content_work.flush(&observation);
    let mut captured = Vec::new();
    loop {
        let record = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(record["loss_watermark"].is_null(), "{record}");
        assert_ne!(record["phase"], "abort", "{record}");
        if record["phase"] == "append" {
            captured.extend(decoded_base64(
                record["canonical_bytes_base64"].as_str().unwrap(),
            ));
        }
        if record["phase"] == "finish" {
            break;
        }
    }
    assert_eq!(captured, expected);
    assert!(!observation.inner.async_content.get().unwrap().failed());
}
