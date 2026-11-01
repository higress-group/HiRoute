//! Opt-in offline field corpus. Inputs are operator-owned selected semantic
//! fields and original accepted events, never committed private conversations.
use super::super::{CanonicalFrameDelivery, ModelStreamEventV1};
use super::*;
use crate::server::core_runtime::observation;
use crate::server::core_runtime::observation::{
    GatewayObservationSinks, accounted_acknowledgement,
    producer::{ObservationAck, ObservationNack, ObservationRecord, ObservationRecordSink},
    tests::request_with_sinks_capacity,
};
use hiroute_diagnostics::runtime::DiagnosticsPort;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

#[derive(Deserialize)]
struct Part {
    ordinal: u32,
    part: u32,
    role: String,
    kind: String,
    media: String,
    bytes: Vec<u8>,
}
#[derive(Deserialize)]
struct Sample {
    workspace: String,
    session: String,
    request: String,
    turn: String,
    started_ms: u64,
    scope: String,
    provenance: String,
    current: Option<u32>,
    activity: u32,
    attempt: String,
    input: Vec<Part>,
    events: Vec<Value>,
}

struct Export {
    file: Mutex<File>,
    done: mpsc::Sender<()>,
}
impl ObservationRecordSink for Export {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        let value: Value = serde_json::from_slice(record.payload()).unwrap();
        {
            let mut file = self.file.lock().unwrap();
            file.write_all(record.payload()).unwrap();
            file.write_all(b"\n").unwrap();
        }
        if value["direction"] == "response_delivered"
            && matches!(value["phase"].as_str(), Some("finish" | "abort"))
        {
            self.done.send(()).unwrap();
        }
        Ok(accounted_acknowledgement(record))
    }
}

fn event(mut value: Value) -> ModelStreamEventV1 {
    if matches!(
        value["event"]["kind"].as_str(),
        Some("tool_call_started" | "web_search")
    ) {
        // Execution-private ownership was deliberately not archived. Restore
        // inert test ownership only; the production serializer removes it again.
        value["event"]["native_id"] = json!("offline-native");
        value["event"]["owner"] = json!({"provider_id":"offline","endpoint_id":"offline","entitlement_id":"offline","connector_id":"offline","connector_revision":"offline","capability_id":"offline","capability_revision":"offline","model_configuration_id":"offline","native_model":"offline","upstream_protocol":"chat_completions","adapter_revision":"offline","serializer_revision":"offline","decoder_revision":"offline"});
    }
    serde_json::from_value(value).unwrap()
}

#[test]
#[ignore = "requires an explicitly staged private semantic field corpus"]
fn export_actual_semantic_capture_for_private_field_corpus() {
    let directory = std::path::Path::new("artifacts/semantic-observation-field");
    let input =
        BufReader::new(File::open(directory.join("input.jsonl")).expect("explicit private corpus"));
    fs::create_dir_all(directory).unwrap();
    let (sender, receiver) = mpsc::channel();
    let sink = Arc::new(Export {
        file: Mutex::new(File::create(directory.join("capture.jsonl")).unwrap()),
        done: sender,
    });
    let mut channels: Option<observation::ObservationChannels> = None;
    let mut requests = 0;
    for line in input.lines() {
        let sample: Sample = serde_json::from_str(&line.unwrap()).unwrap();
        let mut observation = request_with_sinks_capacity(
            observation::otel::OtelContentPolicy::Disabled,
            DiagnosticsPort::default(),
            true,
            GatewayObservationSinks {
                conversation_content: sink.clone(),
                ..GatewayObservationSinks::discard()
            },
            128 * 1024 * 1024,
        );
        let inner = Arc::get_mut(&mut observation.inner).unwrap();
        if let Some(shared) = &channels {
            inner.channels = shared.clone();
        } else {
            channels = Some(inner.channels.clone());
        }
        inner.key = [7; 32];
        inner.metadata.workspace_id = sample.workspace;
        inner.metadata.conversation_id = sample.session;
        inner.metadata.request_id = sample.request;
        inner.metadata.turn_id = sample.turn;
        inner.metadata.session_scope = sample.scope;
        inner.metadata.correlation_provenance = sample.provenance;
        inner
            .input_capture
            .set(observation::content_work::InputCaptureSelection {
                start: 0,
                ordinal_base: 0,
                current_ordinal: sample.current,
                activity_ordinal: sample.activity,
                boundary_verified: false,
                turn_anchor: None,
            })
            .unwrap();
        observation.content_occurred_at = Some(sample.started_ms * 1_000_000);
        let attempt = observation::request::AttemptObservation {
            attempt_id: sample.attempt,
            reasoning_fields_removed: 0,
            response_cleanup_reported: false,
            ordinal: 1,
            stable_binding_id: "offline".into(),
            credential_ref: "offline".into(),
            key_id: "offline".into(),
            provider_name: "offline".into(),
            request_model: "offline".into(),
            upstream_protocol: "chat_completions".into(),
            model_configuration_id: "offline".into(),
            adapter_revision: "offline".into(),
            candidate_id: "offline".into(),
            profile_digest: "offline".into(),
            start_reason: "initial".into(),
            previous_attempt_id: None,
            started_at: Instant::now(),
            effective_cost_micros: None,
            cost_class: None,
            credential_generation: 1,
        };
        observation.lock_state().accepted_attempt = Some(attempt.clone());
        observation::content_work::inline(async {
            observation
                .begin_content_at_run("request_input", None)
                .await;
            for part in &sample.input {
                observation
                    .append_inline_run(
                        part.ordinal,
                        part.part,
                        &part.role,
                        &part.kind,
                        &part.media,
                        &part.bytes,
                    )
                    .await;
            }
            observation
                .terminal_content_run("request_input", "finish", None)
                .await;
            observation
                .begin_content_at_run("response_delivered", Some(&attempt.attempt_id))
                .await;
            // One delivery at a time exercises state across the original event
            // stream; the output coalescer and encoder are production code.
            for value in sample.events {
                observation
                    .emit_response_content_at_run(
                        &attempt.attempt_id,
                        "offline",
                        CanonicalFrameDelivery {
                            events: vec![event(value)],
                            ..Default::default()
                        },
                    )
                    .await;
            }
            observation
                .terminal_content_run("response_delivered", "finish", None)
                .await;
        });
        receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        requests += 1;
    }
    sink.file.lock().unwrap().sync_all().unwrap();
    assert!(requests > 0);
    println!("field_capture_requests={requests}");
}
