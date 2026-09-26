use serde_json::{Value, json};

use hiroute_diagnostics::runtime::DiagnosticsPort;

use crate::ports::{ProbeLeaseOutcome, RuntimeStateKey};
use hiroute_gateway_core::runtime::attempt::{
    AttemptGeneration, AttemptId, Disposition, PublishedDisposition, RequestId,
};

use super::contracts::{contract_sources, port_set_digest, schema_digest};
use super::otel::{OtelAttempt, OtelGenAiMapper};
use super::*;

fn feedback_identity(channel: &str) -> ObservationFeedbackIdentityV1 {
    ObservationFeedbackIdentityV1 {
        channel: channel.into(),
        producer_id: "producer:test".into(),
        producer_epoch: "epoch:test".into(),
        stream_id: "stream:test".into(),
    }
}

fn content_acknowledgement() -> ObservationContentAcknowledgementV1 {
    ObservationContentAcknowledgementV1 {
        request_id: "request:test".into(),
        direction: ObservationContentDirectionV1::ResponseDelivered,
        fork_id: "fork:test".into(),
        next_chunk_ordinal: 4,
        transcript_root: Some("transcript:result".into()),
        delta_parent_transcript_root: Some("transcript:parent".into()),
        acknowledged_blobs: vec![ObservationBlobAcknowledgementV1 {
            content_id: "content:test".into(),
            digest: "blob:digest".into(),
        }],
    }
}

#[test]
fn native_agent_session_derivation_reuses_the_producer_identity_formula() {
    let key = [41_u8; 32];
    let workspace = WorkspaceId::default();
    let plan_id = hiroute_domain::AgentPlanId::parse("plan/live-check").unwrap();
    let trust = FrozenExecutionTrustV1 {
        authority_id: "authority/live-check".into(),
        authority_epoch: 3,
        served_model_id: "hiroute.live.model".into(),
        selector_source: hiroute_domain::SelectorSourceV1::TrustedModelAlias,
        agent_plan_id: Some(plan_id.clone()),
        route: ModelRequestRouteV2::Plan {
            revision: 7,
            semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(b"live-route"),
        },
        plan_display_name: None,
        gateway_publication_revision: "11".into(),
        gateway_publication_digest: hiroute_domain::CanonicalDigest::of_bytes(b"live-publication"),
        grant_id: "grant/live-check".into(),
        grant_generation: 5,
        ingress_protocol: hiroute_domain::IngressProtocolV1::Responses,
    };
    let identity = NativeAgentObservationIdentityV1::CodexThreadSession {
        thread_id: "thread-live".into(),
        session_id: "session-live".into(),
    };
    let expected = SessionId::parse(reliable_observation_session_id(
        &key,
        workspace.as_str(),
        &trust.authority_id,
        &trust.grant_id,
        plan_id.as_str(),
        &trust.served_model_id,
        "responses",
        "codex_thread_session",
        &["thread-live", "session-live"],
    ))
    .unwrap();
    assert_eq!(
        derive_native_agent_observation_session_id(&workspace, &key, &trust, &identity).unwrap(),
        expected
    );
    assert!(
        derive_native_agent_observation_session_id(&workspace, &[0; 32], &trust, &identity)
            .is_err()
    );
}

fn request(policy: OtelContentPolicy) -> RequestObservation {
    request_with_diagnostics(policy, DiagnosticsPort::default())
}

fn request_with_diagnostics(
    policy: OtelContentPolicy,
    diagnostics: DiagnosticsPort,
) -> RequestObservation {
    let gateway = GatewayObservation::with_sinks_and_policy(
        true,
        64 * 1024,
        GatewayObservationSinks::discard(),
        policy,
    );
    RequestObservation::new(
        super::request::RequestObservationCapture {
            enabled: true,
            content: true,
        },
        gateway.key,
        gateway.channels.clone(),
        policy,
        super::pricing::capture(None, "workspace:test", &[], 0),
        RequestObservationMetadata {
            workspace_id: "workspace:test".into(),
            conversation_id: "conversation:test".into(),
            session_scope: "request_scoped".into(),
            correlation_provenance: "unproven".into(),
            turn_id: "turn:test".into(),
            request_id: "request:test".into(),
            authority_id: "authority:test".into(),
            authority_epoch: 1,
            publication_revision: 7,
            publication_digest: "sha256:publication".into(),
            route: hiroute_domain::ModelRequestRouteV2::Plan {
                revision: 5,
                semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(b"plan"),
            },
            plan_display_name: Some("Test plan".into()),
            served_model_id: "served-test".into(),
            grant_id: "grant:test".into(),
            grant_generation: 3,
            ingress_protocol: "responses".into(),
        },
        diagnostics,
    )
}

/// Drives the real observation branches that production runs: candidate staging, the
/// published accept, the first accepted frame and the terminal request, and asserts that
/// the diagnostic sink receives typed correlated events instead of raw identities.
#[test]
fn request_lifecycle_records_typed_diagnostics_without_raw_identities() {
    use hiroute_diagnostics::event::ProcessRole;
    use hiroute_diagnostics::level::DiagnosticLevel;
    use hiroute_diagnostics::record::Component;
    use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "hiroute-gateway-diagnostics-{}-{}-{nanos}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed),
    ));
    let runtime = DiagnosticRuntime::start(RuntimeConfig {
        root: root.clone(),
        role: ProcessRole::Daemon,
        component: Component::Gateway,
        parent_session_id: None,
        // Pipeline stages and staged attempts are Debug; the test asserts them.
        level_override: Some(DiagnosticLevel::Debug),
    });
    let request = request_with_diagnostics(OtelContentPolicy::Disabled, runtime.port());
    request.record_pipeline_stages(
        std::time::Duration::from_millis(1),
        std::time::Duration::from_millis(2),
        std::time::Duration::from_millis(3),
    );
    request.record_plan_stage(std::time::Duration::from_millis(4));
    request.no_credential_materialized("binding:test", "credential/none/source-local-test");
    request.disposition_published(&PublishedDisposition {
        request_id: RequestId(9),
        attempt_id: AttemptId(4),
        generation: AttemptGeneration(2),
        disposition: Disposition::Accept,
    });
    assert!(request.accept_current("frame:test", 128).is_some());
    request.finish("accepted");
    runtime.shutdown();

    let log = std::fs::read_to_string(root.join("daemon").join("current.jsonl")).unwrap();
    for kind in [
        "request_begin",
        "attempt_begin",
        "semantic_commit",
        "request_end",
        "model_stage",
    ] {
        assert!(
            log.contains(&format!("\"{kind}\":")),
            "{kind} missing: {log}"
        );
    }
    assert!(log.contains("\"attempt_index\":1"), "{log}");
    assert!(log.contains("\"outcome\":\"completed\""), "{log}");
    assert!(log.contains("\"stage\":\"parse\""), "{log}");
    assert!(log.contains("\"state\":\"semantic_committed\""), "{log}");
    assert!(
        !log.contains("request:test"),
        "raw request id leaked: {log}"
    );
    assert!(
        !log.contains("binding:test"),
        "raw binding id leaked: {log}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Rejects every delivery, so the channel's own sink-failure projection is exercised
/// without a real sink outage.
struct RejectingSink;

impl ObservationRecordSink for RejectingSink {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        let identity = &record.stamp().identity;
        Err(Box::new(ObservationNackV1 {
            schema_version: OBSERVATION_NACK_SCHEMA.into(),
            identity: ObservationFeedbackIdentityV1 {
                channel: identity.channel.to_string(),
                producer_id: identity.producer_id.to_string(),
                producer_epoch: identity.producer_epoch.to_string(),
                stream_id: identity.stream_id.to_string(),
            },
            rejected_sequence: record.stamp().sequence,
            expected_sequence: record.stamp().sequence,
            retryable: true,
            detail: ObservationNackDetailV1::ReceiverUnavailable {
                retry_after_millis: None,
            },
        }))
    }
}

/// A normal streamed answer can produce more than 256 small content deltas or
/// execution facts while the storage worker is busy. The byte budget, not a
/// second fixed event cap, must determine whether these small records fit.
#[test]
fn observation_channels_keep_small_bursts_contiguous_during_sink_backpressure() {
    use std::sync::Condvar;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    struct GatedSink {
        gate: Arc<(Mutex<bool>, Condvar)>,
        delivered: AtomicUsize,
        gaps: AtomicUsize,
    }

    impl ObservationRecordSink for GatedSink {
        fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
            let (lock, wake) = &*self.gate;
            let released = lock.lock().unwrap_or_else(|error| error.into_inner());
            let _released = wake
                .wait_while(released, |released| !*released)
                .unwrap_or_else(|error| error.into_inner());
            if record.is_gap_heartbeat() {
                self.gaps.fetch_add(1, Ordering::Relaxed);
            } else {
                self.delivered.fetch_add(1, Ordering::Relaxed);
            }
            Ok(accounted_acknowledgement(record))
        }
    }

    const RECORDS: usize = 600;
    for channel in [ObservationChannel::Content, ObservationChannel::Facts] {
        let sink = Arc::new(GatedSink {
            gate: Arc::new((Mutex::new(false), Condvar::new())),
            delivered: AtomicUsize::new(0),
            gaps: AtomicUsize::new(0),
        });
        let discard = GatewayObservationSinks::discard();
        let mut sinks = GatewayObservationSinks {
            lifecycle: discard.lifecycle,
            execution_fact: discard.execution_fact,
            conversation_content: discard.conversation_content,
            run_relation: discard.run_relation,
            otel: discard.otel,
        };
        let selected = match channel {
            ObservationChannel::Content => &mut sinks.conversation_content,
            ObservationChannel::Facts => &mut sinks.execution_fact,
            _ => unreachable!(),
        };
        *selected = sink.clone();
        let gateway = GatewayObservation::with_sinks_and_policy(
            true,
            4 * 1024 * 1024,
            sinks,
            OtelContentPolicy::Disabled,
        );

        for ordinal in 0..RECORDS {
            let publish = |_, sequence, _| json!({"schema_version":"hiroute.observation.test/v1","sequence":sequence,"ordinal":ordinal});
            match channel {
                ObservationChannel::Content => gateway.channels.content.publish(publish),
                ObservationChannel::Facts => gateway.channels.execution.publish(publish),
                _ => unreachable!(),
            }
        }
        let (lock, wake) = &*sink.gate;
        *lock.lock().unwrap_or_else(|error| error.into_inner()) = true;
        wake.notify_all();

        let deadline = Instant::now() + Duration::from_secs(5);
        while sink.delivered.load(Ordering::Relaxed) < RECORDS && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            sink.delivered.load(Ordering::Relaxed),
            RECORDS,
            "{channel:?}"
        );
        assert_eq!(sink.gaps.load(Ordering::Relaxed), 0, "{channel:?}");
    }
}

#[test]
fn channel_loss_and_sink_nack_are_projected_without_record_payloads() {
    use hiroute_diagnostics::event::ProcessRole;
    use hiroute_diagnostics::record::Component;
    use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "hiroute-gateway-channel-diagnostics-{}-{}-{nanos}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed),
    ));
    let runtime = DiagnosticRuntime::start(RuntimeConfig {
        root: root.clone(),
        role: ProcessRole::Daemon,
        component: Component::Gateway,
        parent_session_id: None,
        // Channel loss and NACK are warnings; the default info threshold admits them.
        level_override: None,
    });
    let discard: Arc<dyn ObservationRecordSink> = GatewayObservationSinks::discard().lifecycle;
    let gateway = GatewayObservation::with_sinks_and_policy(
        true,
        1024,
        GatewayObservationSinks {
            lifecycle: discard,
            execution_fact: Arc::new(RejectingSink),
            conversation_content: GatewayObservationSinks::discard().conversation_content,
            run_relation: GatewayObservationSinks::discard().run_relation,
            otel: GatewayObservationSinks::discard().otel,
        },
        OtelContentPolicy::Disabled,
    )
    .with_diagnostics(runtime.port());

    // Over the byte capacity: the channel declares a loss range instead of queueing it.
    gateway.channels.lifecycle.publish(|_, _, _| {
        let mut payload = b"channel-payload-sentinel".to_vec();
        payload.resize(64 * 1024, b'x');
        payload
    });
    // A sink that rejects the record: the failure is reported once, with its reason.
    gateway
        .channels
        .execution
        .publish(|_, _, _| br#"{"schema_version":"hiroute.observation.test/v1"}"#.to_vec());

    let path = root.join("daemon").join("current.jsonl");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let log = loop {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        if (text.contains("\"observation_gap\":") && text.contains("\"sink_nack\":"))
            || std::time::Instant::now() >= deadline
        {
            break text;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    runtime.shutdown();

    assert!(log.contains("\"observation_gap\":"), "{log}");
    assert!(log.contains("\"reason\":\"queue_overflow\""), "{log}");
    assert!(log.contains("\"missing\":1"), "{log}");
    assert!(log.contains("\"sink_nack\":"), "{log}");
    assert!(log.contains("\"reason\":\"unavailable\""), "{log}");
    assert!(
        !log.contains("channel-payload-sentinel"),
        "channel payload leaked: {log}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The capture summary reports the real byte count, 64KiB chunks and short reads of a
/// request whose canonical content lives in the replay backing, and never the content.
#[test]
fn request_content_capture_reports_bytes_chunks_and_short_reads() {
    use hiroute_diagnostics::event::ProcessRole;
    use hiroute_diagnostics::level::DiagnosticLevel;
    use hiroute_diagnostics::record::Component;
    use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};
    use hiroute_gateway_core::runtime::body::BudgetTree;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let unique = format!("{}-{nanos}", SEQUENCE.fetch_add(1, Ordering::Relaxed));
    let diagnostics_root = std::env::temp_dir().join(format!(
        "hiroute-gateway-content-diagnostics-{}-{unique}",
        std::process::id(),
    ));
    let replay_root = std::env::temp_dir().join(format!(
        "hiroute-gateway-content-replay-{}-{unique}",
        std::process::id(),
    ));
    let sentinel = "content-summary-sentinel ".repeat(4096);
    let mut request_ir = crate::server::core_runtime::adapters::decode_ingress_request(
        crate::server::request_plan::IngressProtocol::Responses,
        &json!({
            "model": "alias",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": sentinel}],
            }],
        }),
    )
    .expect("decode ingress request");
    let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
        root: replay_root.clone(),
        memory_threshold_bytes: 4 * 1024 * 1024,
        // Small records make the capture read the backing in many short reads.
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .expect("replay manager");
    let tree = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024).expect("budget tree");
    let budget = tree.stream(8 * 1024 * 1024).expect("stream budget");
    let store = manager.begin_request(budget.clone()).expect("replay store");
    crate::content_ref::externalize_model_request(&mut request_ir, &store, 8)
        .expect("externalize canonical content");
    let runtime = DiagnosticRuntime::start(RuntimeConfig {
        root: diagnostics_root.clone(),
        role: ProcessRole::Daemon,
        component: Component::Gateway,
        parent_session_id: None,
        // The capture summary is Info; the default threshold would admit it too.
        level_override: Some(DiagnosticLevel::Debug),
    });
    let observation = request_with_diagnostics(OtelContentPolicy::Disabled, runtime.port());
    observation.capture_request(&request_ir, &store);
    runtime.shutdown();

    let log =
        std::fs::read_to_string(diagnostics_root.join("daemon").join("current.jsonl")).unwrap();
    assert!(log.contains("\"content_capture_end\":"), "{log}");
    assert!(log.contains("\"role\":\"request\""), "{log}");
    assert!(log.contains("\"outcome\":\"success\""), "{log}");
    assert!(log.contains("\"max_chunk_bytes\":65536"), "{log}");
    assert!(log.contains("\"chunks\":2"), "{log}");
    assert!(!log.contains("\"short_reads\":0"), "{log}");
    assert!(log.contains("\"spill_begin\":"), "{log}");
    assert!(log.contains("\"mode\":\"memory\""), "{log}");
    assert!(
        !log.contains("content-summary-sentinel"),
        "content leaked: {log}"
    );

    let line = log
        .lines()
        .find(|line| line.contains("\"content_capture_end\":"))
        .expect("one capture summary line");
    let summary: Value = serde_json::from_str(line).expect("summary is valid JSON");
    let bytes = summary["event"]["content_capture_end"]["bytes"]
        .as_u64()
        .expect("byte count");
    assert!(bytes >= sentinel.len() as u64, "{log}");
    let read_calls = summary["event"]["content_capture_end"]["read_calls"]
        .as_u64()
        .expect("read calls");
    assert!(read_calls > 2, "{log}");
    let _ = std::fs::remove_dir_all(&diagnostics_root);
    let _ = std::fs::remove_dir_all(&replay_root);
}

#[test]
fn predictable_workspace_hmac_key_disables_observation() {
    let gateway = GatewayObservation::with_sinks_and_policy_and_workspace_key(
        true,
        64 * 1024,
        GatewayObservationSinks::discard(),
        OtelContentPolicy::Disabled,
        [0_u8; 32],
    );
    assert!(!gateway.is_enabled());
}

#[test]
fn external_port_schema_and_aggregate_digests_are_exact() {
    for (source, schema, expected) in contract_sources() {
        let parsed: Value = serde_json::from_str(source).expect("contract source is valid JSON");
        assert_eq!(parsed.get("schema").and_then(Value::as_str), Some(schema));
        assert!(!source.contains('\n'));
        assert_eq!(schema_digest(source), expected);
    }
    let contracts = gateway_port_contracts();
    assert_eq!(contracts.schema_version, GATEWAY_PORT_SET_SCHEMA);
    assert_eq!(contracts.gate, "G_port_process_22020");
    assert_eq!(contracts.ports.len(), 6);
    assert_eq!(port_set_digest(&contracts.ports), GATEWAY_PORT_SET_DIGEST);
    assert_eq!(
        contracts
            .ports
            .iter()
            .map(|port| port.schema_version.as_str())
            .collect::<Vec<_>>(),
        [
            RUNTIME_PUBLICATION_PORT_SCHEMA,
            CREDENTIAL_PORT_SCHEMA,
            RUNTIME_STATE_PORT_SCHEMA,
            LIFECYCLE_FACT_PORT_SCHEMA,
            EXECUTION_FACT_PORT_SCHEMA,
            CONVERSATION_CONTENT_PORT_SCHEMA,
        ]
    );
    for schema in [
        LIFECYCLE_FACT_PORT_SCHEMA,
        EXECUTION_FACT_PORT_SCHEMA,
        CONVERSATION_CONTENT_PORT_SCHEMA,
    ] {
        let source = gateway_port_contract_source(schema).unwrap();
        assert!(source.contains("hiroute.observation.ack/v2"));
        assert!(source.contains("hiroute.observation.nack/v1"));
        assert!(source.contains("\"free_form_reason\":\"forbidden\""));
    }
}

#[test]
fn observation_ack_v2_has_exact_golden_shape_and_denies_unknown_fields() {
    let acknowledgement = ObservationAckV2 {
        schema_version: OBSERVATION_ACK_SCHEMA.into(),
        identity: feedback_identity(CONVERSATION_CONTENT_CHANNEL),
        highest_contiguous_sequence: 6,
        highest_accounted_sequence: 9,
        content_acknowledgement: Some(content_acknowledgement()),
    };
    assert_eq!(
        serde_json::to_value(&acknowledgement).unwrap(),
        json!({
            "schema_version": "hiroute.observation.ack/v2",
            "identity": {
                "channel": "conversation_content",
                "producer_id": "producer:test",
                "producer_epoch": "epoch:test",
                "stream_id": "stream:test"
            },
            "highest_contiguous_sequence": 6,
            "highest_accounted_sequence": 9,
            "content_acknowledgement": {
                "request_id": "request:test",
                "direction": "response_delivered",
                "fork_id": "fork:test",
                "next_chunk_ordinal": 4,
                "transcript_root": "transcript:result",
                "delta_parent_transcript_root": "transcript:parent",
                "acknowledged_blobs": [{
                    "content_id": "content:test",
                    "digest": "blob:digest"
                }]
            }
        })
    );
    acknowledgement.validate().unwrap();

    let mut unknown_top_level = serde_json::to_value(&acknowledgement).unwrap();
    unknown_top_level
        .as_object_mut()
        .unwrap()
        .insert("reason".into(), json!("not permitted"));
    assert!(serde_json::from_value::<ObservationAckV2>(unknown_top_level).is_err());

    let mut unknown_nested = serde_json::to_value(&acknowledgement).unwrap();
    unknown_nested["content_acknowledgement"]
        .as_object_mut()
        .unwrap()
        .insert("opaque".into(), json!({}));
    assert!(serde_json::from_value::<ObservationAckV2>(unknown_nested).is_err());
}

#[test]
fn observation_nack_v1_all_closed_details_round_trip() {
    let blob = ObservationBlobAcknowledgementV1 {
        content_id: "content:test".into(),
        digest: "blob:digest".into(),
    };
    let details = vec![
        ObservationNackDetailV1::ReceiverUnavailable {
            retry_after_millis: Some(25),
        },
        ObservationNackDetailV1::UnsupportedSchema {
            rejected_schema_version: "hiroute.observation.future/v9".into(),
            supported_schema_versions: vec![EXECUTION_FACT_SCHEMA.into()],
        },
        ObservationNackDetailV1::InvalidEnvelope {
            violation: ObservationEnvelopeViolationV1::MissingField,
            field: Some("event_id".into()),
        },
        ObservationNackDetailV1::MissingSequenceRanges {
            ranges: vec![ObservationSequenceRangeV1 {
                first_sequence: 3,
                last_sequence: 5,
            }],
        },
        ObservationNackDetailV1::SequenceEventConflict {
            sequence: 7,
            expected_event_id: "event:expected".into(),
            rejected_event_id: "event:rejected".into(),
        },
        ObservationNackDetailV1::MissingPrerequisite {
            prerequisite_sequence: 6,
            prerequisite_event_id: Some("event:prerequisite".into()),
        },
        ObservationNackDetailV1::UnknownTranscriptRoot {
            request_id: "request:test".into(),
            direction: ObservationContentDirectionV1::RequestInput,
            fork_id: "fork:test".into(),
            transcript_root: "transcript:unknown".into(),
        },
        ObservationNackDetailV1::MissingBlob {
            request_id: "request:test".into(),
            direction: ObservationContentDirectionV1::ResponseDelivered,
            fork_id: "fork:test".into(),
            blobs: vec![blob],
        },
        ObservationNackDetailV1::ChunkOrdinalConflict {
            request_id: "request:test".into(),
            direction: ObservationContentDirectionV1::ResponseDelivered,
            fork_id: "fork:test".into(),
            expected_chunk_ordinal: 4,
            rejected_chunk_ordinal: 6,
        },
        ObservationNackDetailV1::ContentStateConflict {
            request_id: "request:test".into(),
            direction: ObservationContentDirectionV1::ResponseDelivered,
            fork_id: "fork:test".into(),
            expected_phase: ObservationContentPhaseV1::Append,
            rejected_phase: ObservationContentPhaseV1::Finish,
        },
        ObservationNackDetailV1::DigestMismatch {
            subject: ObservationDigestSubjectV1::ContentBlob,
            subject_id: "content:test".into(),
            expected_digest: "blob:expected".into(),
            rejected_digest: "blob:rejected".into(),
        },
        ObservationNackDetailV1::ImmutableProjectionConflict {
            projection_key: "receipt:test".into(),
            existing_digest: "sha256:existing".into(),
            rejected_digest: "sha256:rejected".into(),
        },
    ];
    let expected_kinds = [
        "receiver_unavailable",
        "unsupported_schema",
        "invalid_envelope",
        "missing_sequence_ranges",
        "sequence_event_conflict",
        "missing_prerequisite",
        "unknown_transcript_root",
        "missing_blob",
        "chunk_ordinal_conflict",
        "content_state_conflict",
        "digest_mismatch",
        "immutable_projection_conflict",
    ];
    for (detail, expected_kind) in details.into_iter().zip(expected_kinds) {
        let nack = ObservationNackV1 {
            schema_version: OBSERVATION_NACK_SCHEMA.into(),
            identity: feedback_identity(CONVERSATION_CONTENT_CHANNEL),
            rejected_sequence: 7,
            expected_sequence: 3,
            retryable: true,
            detail,
        };
        nack.validate().unwrap();
        let value = serde_json::to_value(&nack).unwrap();
        assert_eq!(value["detail"]["kind"], expected_kind);
        assert!(value.get("reason").is_none());
        assert_eq!(
            serde_json::from_value::<ObservationNackV1>(value).unwrap(),
            nack
        );
    }
}

#[test]
fn observation_nack_v1_denies_unknown_reason_and_detail() {
    let valid = json!({
        "schema_version": "hiroute.observation.nack/v1",
        "identity": {
            "channel": "execution_fact",
            "producer_id": "producer:test",
            "producer_epoch": "epoch:test",
            "stream_id": "stream:test"
        },
        "rejected_sequence": 7,
        "expected_sequence": 3,
        "retryable": true,
        "detail": {"kind": "receiver_unavailable", "retry_after_millis": 25}
    });
    assert!(serde_json::from_value::<ObservationNackV1>(valid.clone()).is_ok());

    let mut free_form_reason = valid.clone();
    free_form_reason
        .as_object_mut()
        .unwrap()
        .insert("reason".into(), json!("receiver failed"));
    assert!(serde_json::from_value::<ObservationNackV1>(free_form_reason).is_err());

    let mut unknown_detail_field = valid.clone();
    unknown_detail_field["detail"]
        .as_object_mut()
        .unwrap()
        .insert("opaque".into(), json!({}));
    assert!(serde_json::from_value::<ObservationNackV1>(unknown_detail_field).is_err());

    let mut unknown_detail_kind = valid;
    unknown_detail_kind["detail"]["kind"] = json!("free_form");
    assert!(serde_json::from_value::<ObservationNackV1>(unknown_detail_kind).is_err());
}

#[test]
fn observation_feedback_validation_fails_closed() {
    let mut acknowledgement = ObservationAckV2 {
        schema_version: OBSERVATION_ACK_SCHEMA.into(),
        identity: feedback_identity("execution_fact"),
        highest_contiguous_sequence: 8,
        highest_accounted_sequence: 7,
        content_acknowledgement: None,
    };
    assert_eq!(
        acknowledgement.validate(),
        Err(ObservationFeedbackValidationError::InvalidFrontier)
    );
    acknowledgement.highest_contiguous_sequence = 7;
    acknowledgement.content_acknowledgement = Some(content_acknowledgement());
    assert_eq!(
        acknowledgement.validate(),
        Err(ObservationFeedbackValidationError::InvalidContentChannel)
    );
    acknowledgement.identity = feedback_identity(CONVERSATION_CONTENT_CHANNEL);
    acknowledgement
        .content_acknowledgement
        .as_mut()
        .unwrap()
        .request_id = " ".into();
    assert!(matches!(
        acknowledgement.validate(),
        Err(ObservationFeedbackValidationError::EmptyField(_))
    ));
    acknowledgement.schema_version = "hiroute.observation.ack/v1".into();
    assert_eq!(
        acknowledgement.validate(),
        Err(ObservationFeedbackValidationError::UnsupportedSchema)
    );

    let mut nack = ObservationNackV1 {
        schema_version: OBSERVATION_NACK_SCHEMA.into(),
        identity: feedback_identity("execution_fact"),
        rejected_sequence: 7,
        expected_sequence: 3,
        retryable: true,
        detail: ObservationNackDetailV1::UnknownTranscriptRoot {
            request_id: "request:test".into(),
            direction: ObservationContentDirectionV1::RequestInput,
            fork_id: "fork:test".into(),
            transcript_root: "transcript:unknown".into(),
        },
    };
    assert_eq!(
        nack.validate(),
        Err(ObservationFeedbackValidationError::InvalidContentChannel)
    );
    nack.identity = feedback_identity(CONVERSATION_CONTENT_CHANNEL);
    nack.expected_sequence = 0;
    assert_eq!(
        nack.validate(),
        Err(ObservationFeedbackValidationError::InvalidSequence)
    );
    nack.expected_sequence = 3;
    nack.detail = ObservationNackDetailV1::MissingSequenceRanges {
        ranges: vec![
            ObservationSequenceRangeV1 {
                first_sequence: 5,
                last_sequence: 6,
            },
            ObservationSequenceRangeV1 {
                first_sequence: 4,
                last_sequence: 4,
            },
        ],
    };
    assert_eq!(
        nack.validate(),
        Err(ObservationFeedbackValidationError::InvalidSequenceRange)
    );
}

#[test]
fn default_gen_ai_mapper_has_standard_small_fields_and_zero_content() {
    let request = request(OtelContentPolicy::Disabled);
    let signal = OtelGenAiMapper::new(OtelContentPolicy::Disabled).attempt_span(
        &request,
        &OtelAttempt {
            ordinal: 1,
            attempt_id: "attempt:test".into(),
            stable_binding_id: "binding:test".into(),
            provider_name: "provider:test".into(),
            request_model: "native:test".into(),
        },
        "accepted",
        None,
    );
    let rendered = serde_json::to_string(&signal).unwrap();
    assert!(rendered.contains("gen_ai.operation.name"));
    assert!(rendered.contains("gen_ai.client.inference.operation.details"));
    for forbidden in [
        "prompt",
        "response content",
        "canonical_bytes_base64",
        "authorization",
        "gen_ai.input.messages",
        "gen_ai.output.messages",
    ] {
        assert!(!rendered.contains(forbidden));
    }
    assert!(
        OtelGenAiMapper::new(OtelContentPolicy::Disabled)
            .content_ref(
                "request_input",
                ContentRefV1 {
                    content_id: "content:test".into(),
                    digest: "digest:test".into(),
                    byte_count: 9,
                    media_type: "text/plain".into(),
                },
            )
            .is_none()
    );
}

#[test]
fn content_ref_opt_in_is_hiroute_extension_without_inline_bytes() {
    let signal = OtelGenAiMapper::new(OtelContentPolicy::ContentRefOnly)
        .content_ref(
            "response_delivered",
            ContentRefV1 {
                content_id: "content:test".into(),
                digest: "digest:test".into(),
                byte_count: 9,
                media_type: "text/plain".into(),
            },
        )
        .unwrap();
    let value = serde_json::to_value(signal).unwrap();
    assert_eq!(
        value.get("name"),
        Some(&Value::String("hiroute.gen_ai.response.content".into()))
    );
    assert!(value.get("content_ref").is_some());
    assert!(value.get("canonical_bytes_base64").is_none());
}

#[test]
fn mapper_and_envelope_versions_are_explicit() {
    assert!(EXECUTION_FACT_SCHEMA.ends_with("/v3"));
    assert_eq!(OTEL_MAPPER_VERSION, "hiroute.otel-gen-ai-mapper/1");
    assert_eq!(OTEL_SEMANTIC_CONVENTIONS_VERSION, "1.37.0");
    assert!(OTEL_MAPPING_DIGEST.starts_with("sha256:"));
    assert_eq!(schema_digest(OTEL_MAPPING_CONTRACT), OTEL_MAPPING_DIGEST);
    for schema in [
        LIFECYCLE_FACT_SCHEMA,
        CONVERSATION_CONTENT_SCHEMA,
        OBSERVATION_ACK_SCHEMA,
    ] {
        assert!(schema.ends_with("/v2"));
    }
    assert!(EXECUTION_FACT_SCHEMA.ends_with("/v3"));
    for schema in [
        OTEL_GEN_AI_SCHEMA,
        OBSERVATION_NACK_SCHEMA,
        OBSERVATION_GAP_HEARTBEAT_SCHEMA,
    ] {
        assert!(schema.ends_with("/v1"));
    }
}

#[test]
fn acquired_probe_stages_and_authoritative_publication_starts_the_attempt() {
    let request = request(OtelContentPolicy::Disabled);
    let key = RuntimeStateKey::credential("binding:test", "credential:test", "key:test", 4);
    request.runtime_probe(
        &key,
        7,
        std::time::Duration::from_secs(5),
        Some(ProbeLeaseOutcome::Busy),
        "busy",
    );
    assert!(request.lock_state().pending_attempt.is_none());

    request.runtime_probe(
        &key,
        7,
        std::time::Duration::from_secs(5),
        Some(ProbeLeaseOutcome::Acquired { generation: 8 }),
        "acquired",
    );
    assert!(request.lock_state().current_attempt.is_none());
    assert!(request.lock_state().pending_attempt.is_some());
    request.disposition_published(&PublishedDisposition {
        request_id: RequestId(9),
        attempt_id: AttemptId(4),
        generation: AttemptGeneration(2),
        disposition: Disposition::Accept,
    });
    let state = request.lock_state();
    let attempt = state
        .current_attempt
        .as_ref()
        .expect("core publication promotes the staged candidate to an Attempt");
    assert_eq!(attempt.ordinal, 1);
    assert_eq!(attempt.start_reason, "initial_candidate");
    assert!(attempt.attempt_id.starts_with("attempt-"));
}

#[test]
fn explicit_no_credential_materialization_stages_the_authorized_attempt() {
    let request = request(OtelContentPolicy::Disabled);
    request.no_credential_materialized("binding:test", "credential/none/source-local-test");
    {
        let state = request.lock_state();
        let pending = state
            .pending_attempt
            .as_ref()
            .expect("verified unauthenticated materialization stages an observation attempt");
        assert_eq!(pending.credential_ref, "credential/none/source-local-test");
        assert_eq!(pending.key_id, pending.credential_ref);
        assert_eq!(pending.credential_generation, 0);
    }

    request.disposition_published(&PublishedDisposition {
        request_id: RequestId(9),
        attempt_id: AttemptId(4),
        generation: AttemptGeneration(2),
        disposition: Disposition::Accept,
    });

    let state = request.lock_state();
    assert_eq!(state.next_attempt_ordinal, 2);
    assert_eq!(
        state
            .current_attempt
            .as_ref()
            .map(|attempt| attempt.ordinal),
        Some(1)
    );
}

#[test]
fn pending_candidate_without_core_attempt_authority_finishes_with_zero_attempts() {
    let request = request(OtelContentPolicy::Disabled);
    let key = RuntimeStateKey::credential("binding:test", "credential:test", "key:test", 4);
    request.runtime_probe(
        &key,
        7,
        std::time::Duration::from_secs(5),
        Some(ProbeLeaseOutcome::Acquired { generation: 8 }),
        "acquired",
    );
    assert!(request.lock_state().pending_attempt.is_some());

    request.finish("gateway_error");

    let state = request.lock_state();
    assert!(state.request_finished);
    assert!(state.pending_attempt.is_none());
    assert_eq!(state.next_attempt_ordinal, 1);
    assert_eq!(state.attempts_finished, 0);
}
