//! Deterministic upstream injections through the production HTTPS provider and listener.
//! Each child owns its environment, diagnostics, Replay and optional private capture.
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hiroute_diagnostics::event::ProcessRole;
use hiroute_diagnostics::level::DiagnosticLevel;
use hiroute_diagnostics::record::Component;
use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};
use serde_json::{Value, json};

use super::*;
use crate::server::core_runtime::observation::{
    GatewayObservationSinks, ObservationAck, ObservationNack, ObservationRecord,
    ObservationRecordSink, accounted_acknowledgement,
};
use crate::server::publication::{
    AliasPlanV1, GatewayPublicationInstaller, GatewayPublicationSnapshotV3, GrantV1, ModelRouteV2,
    token_sha256,
};
use crate::server::test_control::{
    E2E_DIAL_CONFIG_ENV, E2E_DIAL_CONFIG_FILE, TestTlsListener, sealed_native_candidate,
    write_dial_config,
};

mod capture;
use capture::{assert_capture, create_capture_session, wait_for_captured_request_eof};

const CHILD_CASE: &str = "HIROUTE_STREAM_DIAGNOSTIC_CASE";
const CHILD_ROOT: &str = "HIROUTE_STREAM_DIAGNOSTIC_ROOT";
const TOKEN: &str = "SENTINEL_CLIENT_CREDENTIAL";
const TOOL: &str = "SENTINEL_DYNAMIC_TOOL";
const TEST: &str = "server::core_runtime::response_diagnostics_tests::production_listener_stream_diagnostics_and_private_capture";

struct PrivateRoot(PathBuf);
impl Drop for PrivateRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn production_listener_stream_diagnostics_and_private_capture() {
    if let Ok(case) = std::env::var(CHILD_CASE) {
        run_case(&case, &PathBuf::from(std::env::var_os(CHILD_ROOT).unwrap()));
        return;
    }
    let mut failures = Vec::new();
    for case in [
        "unsupported_known",
        "unknown_metadata",
        "unsupported_value",
        "invalid_type",
        "invalid_json",
        "prebody_invalid_json",
        "prebody_capture_invalid_json",
        "prebody_capture_relay",
        "prebody_capture_no_credential",
        "prebody_capture_no_credential_relay",
        "invalid_sse",
        "missing_tool",
        "invalid_arguments",
        "invalid_lifecycle",
        "missing_terminal",
        "duplicate_terminal",
        "resource_limit",
        "drain_failure",
        "capture_known",
        "capture_unknown",
        "capture_disabled",
    ] {
        if let Err(failure) = run_child_case(case) {
            failures.push(failure);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn production_completed_prefix_capture_keeps_formal_attempt_correlation() {
    run_child_case("capture_known").unwrap();
}

#[test]
fn production_completed_prefix_capture_accepts_usage_before_provider_completion() {
    run_child_case("capture_known_usage_first").unwrap();
}

#[test]
fn production_prebody_failed_capture_keeps_promoted_attempt_correlation() {
    let failures: Vec<_> = ["prebody_capture_invalid_json", "prebody_capture_relay"]
        .into_iter()
        .filter_map(|case| run_child_case(case).err())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn production_no_credential_prebody_capture_keeps_promoted_attempt_correlation() {
    let failures: Vec<_> = [
        "prebody_capture_no_credential",
        "prebody_capture_no_credential_relay",
    ]
    .into_iter()
    .filter_map(|case| run_child_case(case).err())
    .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn run_child_case(case: &str) -> Result<(), String> {
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).unwrap();
    let root = PrivateRoot(
        fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "hiroute-stream-acceptance-{:x}",
                u128::from_le_bytes(random)
            )),
    );
    fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", TEST, "--nocapture"])
        .env(CHILD_CASE, case)
        .env(CHILD_ROOT, &root.0)
        .env(E2E_DIAL_CONFIG_ENV, root.0.join(E2E_DIAL_CONFIG_FILE))
        .env("HIROUTE_REPLAY_ROOT", root.0.join("replay"))
        .env_remove("HIROUTE_PRIVATE_STREAM_CAPTURE");
    if matches!(
        case,
        "capture_known"
            | "capture_known_usage_first"
            | "capture_unknown"
            | "prebody_capture_invalid_json"
            | "prebody_capture_relay"
            | "prebody_capture_no_credential"
            | "prebody_capture_no_credential_relay"
    ) {
        child.env("HIROUTE_PRIVATE_STREAM_CAPTURE", root.0.join("capture"));
    }
    let output = child.output().unwrap();
    if output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed") {
        println!("stream injection {case}: green");
        Ok(())
    } else {
        Err(format!(
            "{case}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

#[derive(Clone, Default)]
struct Facts(Arc<Mutex<Vec<Value>>>);
impl ObservationRecordSink for Facts {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        self.0
            .lock()
            .unwrap()
            .push(serde_json::from_slice(record.payload()).unwrap());
        Ok(accounted_acknowledgement(record))
    }
}

fn chat(delta: Value, finish: Value) -> Value {
    json!({"id":"SENTINEL_RESPONSE_BODY","model":"physical",
        "choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
}
fn frame(value: Value) -> Vec<u8> {
    format!("data: {value}\n\n").into_bytes()
}
fn tool(name: Option<&str>, arguments: &str) -> Value {
    let mut call = json!({"index":0,"type":"function","function":{"arguments":arguments}});
    if let Some(name) = name {
        call["id"] = "SENTINEL_NATIVE_CALL".into();
        call["function"]["name"] = name.into();
    }
    json!({"tool_calls":[call]})
}

fn sample(case: &str) -> (Vec<u8>, Option<&'static str>, Option<&'static str>, u64) {
    let (mut wire, reason, field, index) = match case {
        "unsupported_known" => {
            let mut value = chat(json!({}), Value::Null);
            value["system_fingerprint"] = "SENTINEL_PROVIDER_TEXT".into();
            (
                frame(value),
                Some("unsupported_field"),
                Some("system_fingerprint"),
                1,
            )
        }
        "unknown_metadata" => {
            // Unknown sibling metadata is already tolerated by the production
            // decoder. Keep that semantic policy and verify it never enters logs.
            let mut value = chat(json!({"content":"SENTINEL_PROVIDER_TEXT"}), json!("stop"));
            value["SENTINEL_UNKNOWN_FIELD"] = "SENTINEL_PROVIDER_TEXT".into();
            value["usage"] = json!({"prompt_tokens":11,"completion_tokens":7,"total_tokens":18});
            (frame(value), None, None, 2)
        }
        "invalid_type" => (
            frame(chat(
                json!({"content":{"SENTINEL_PROVIDER_TEXT":true}}),
                Value::Null,
            )),
            Some("invalid_field"),
            Some("content"),
            1,
        ),
        "unsupported_value" => (
            frame(chat(json!({"role":"SENTINEL_PROVIDER_ROLE"}), Value::Null)),
            Some("unsupported_value"),
            None,
            1,
        ),
        "invalid_json"
        | "prebody_invalid_json"
        | "prebody_capture_invalid_json"
        | "prebody_capture_relay"
        | "prebody_capture_no_credential"
        | "prebody_capture_no_credential_relay" => (
            b"data: {\"SENTINEL_PROVIDER_TEXT\":\n\n".to_vec(),
            Some("invalid_json"),
            None,
            1,
        ),
        "invalid_sse" => (
            b"data-invalid: SENTINEL_PROVIDER_TEXT\n\n".to_vec(),
            Some("invalid_sse"),
            None,
            1,
        ),
        "missing_tool" => (
            frame(chat(tool(None, "{}"), Value::Null)),
            Some("missing_tool_identity"),
            None,
            1,
        ),
        "invalid_arguments" => (
            frame(chat(
                tool(Some(TOOL), "{\"SENTINEL_ARGUMENT\":"),
                json!("tool_calls"),
            )),
            Some("invalid_tool_arguments"),
            None,
            1,
        ),
        "invalid_lifecycle" => {
            let mut wire = frame(chat(tool(Some(TOOL), "{"), Value::Null));
            let mut changed = tool(Some(TOOL), "}");
            changed["tool_calls"][0]["id"] = "SENTINEL_CHANGED_CALL".into();
            wire.extend(frame(chat(changed, Value::Null)));
            (wire, Some("invalid_lifecycle"), None, 2)
        }
        "missing_terminal" => (
            frame(chat(
                json!({"content":"SENTINEL_PROVIDER_TEXT"}),
                Value::Null,
            )),
            Some("missing_terminal"),
            None,
            1,
        ),
        "duplicate_terminal" => {
            let mut wire = frame(chat(
                json!({"content":"SENTINEL_PROVIDER_TEXT"}),
                json!("stop"),
            ));
            wire.extend_from_slice(b"data: [DONE]\n\ndata: [DONE]\n\n");
            (wire, Some("duplicate_terminal"), None, 3)
        }
        "resource_limit" => (
            frame(chat(
                json!({"content":"x".repeat(6 * 1024 * 1024)}),
                Value::Null,
            )),
            Some("resource_limit"),
            None,
            0,
        ),
        "drain_failure" => {
            let mut wire = Vec::new();
            for _ in 0..40 {
                wire.extend(frame(chat(
                    json!({"content":"SENTINEL_PROVIDER_TEXT"}),
                    Value::Null,
                )));
            }
            let mut value = chat(json!({}), Value::Null);
            value["system_fingerprint"] = "SENTINEL_PROVIDER_TEXT".into();
            wire.extend(frame(value));
            (
                wire,
                Some("unsupported_field"),
                Some("system_fingerprint"),
                41,
            )
        }
        "capture_unknown" => (
            frame(chat(
                tool(Some("SENTINEL_UNKNOWN_TOOL"), "{}"),
                json!("tool_calls"),
            )),
            Some("missing_tool_identity"),
            None,
            1,
        ),
        "capture_known" | "capture_known_usage_first" => {
            let mut value = chat(tool(Some(TOOL), "{}"), json!("tool_calls"));
            value["usage"] = json!({"prompt_tokens":11,"completion_tokens":7,"total_tokens":18});
            (frame(value), None, None, 2)
        }
        "capture_disabled" => {
            let mut value = chat(json!({"content":"SENTINEL_PROVIDER_TEXT"}), json!("stop"));
            value["usage"] = json!({"prompt_tokens":11,"completion_tokens":7,"total_tokens":18});
            (frame(value), None, None, 2)
        }
        _ => panic!("unknown test case"),
    };
    if matches!(
        case,
        "invalid_arguments"
            | "unknown_metadata"
            | "capture_known"
            | "capture_known_usage_first"
            | "capture_unknown"
            | "capture_disabled"
    ) {
        wire.extend_from_slice(b"data: [DONE]\n\n");
    }
    (wire, reason, field, index)
}

fn run_case(case: &str, root: &Path) {
    let facts = Facts::default();
    let capture_enabled = std::env::var_os("HIROUTE_PRIVATE_STREAM_CAPTURE").is_some();
    if capture_enabled {
        create_capture_session(&root.join("capture"));
    }
    let (tail, reason, field, frame_index) = sample(case);
    let prebody = case.starts_with("prebody_");
    let relay = case.ends_with("_relay");
    let no_credential = case.contains("no_credential");
    let after_body = reason.is_some() && !prebody;
    let mut wire = if after_body {
        frame(chat(
            json!({"content":"SENTINEL_INITIAL_BODY"}),
            Value::Null,
        ))
    } else {
        Vec::new()
    };
    let prefix_len = wire.len();
    wire.extend(tail);
    let frame_index = if after_body && frame_index != 0 {
        frame_index + 1
    } else {
        frame_index
    };
    let (body_delivered, wait_for_body) = std::sync::mpsc::sync_channel(0);
    let listener = TestTlsListener::bind("stream-diagnostic.invalid").unwrap();
    write_dial_config(root, &[&listener]).unwrap();
    let peer = listener.try_clone().unwrap();
    let sent = wire.clone();
    let capture_root = capture_enabled.then(|| root.join("capture"));
    let usage_first = case == "capture_known_usage_first";
    let upstream_facts = facts.clone();
    let upstream = std::thread::spawn(move || {
        let (mut stream, _) = peer.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let request = read_request(&mut stream);
        if let Some(root) = &capture_root {
            // Receiving Content-Length bytes does not prove the request reader
            // has verified EOF. This fixture promises a replayable capture.
            wait_for_captured_request_eof(root);
        }
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", sent.len()).unwrap();
        if after_body {
            stream.write_all(&sent[..prefix_len]).unwrap();
            stream.flush().unwrap();
            // No error bytes exist on the wire until the client has received
            // the first real body unit. This proves the afterbody boundary.
            wait_for_body.recv_timeout(Duration::from_secs(3)).unwrap();
            let _ = stream.write_all(&sent[prefix_len..]);
        } else if usage_first {
            let terminal = b"data: [DONE]\n\n";
            assert!(sent.ends_with(terminal));
            let split = sent.len() - terminal.len();
            stream.write_all(&sent[..split]).unwrap();
            stream.flush().unwrap();
            // Deliver the accepted canonical usage before the provider can
            // complete. This deterministically exercises production deduplication.
            wait_canonical_usage(&upstream_facts);
            stream.write_all(&sent[split..]).unwrap();
        } else {
            let _ = stream.write_all(&sent);
        }
        let _ = stream.finish();
        if relay {
            let (mut stream, _) = peer.accept().unwrap();
            let _second_request = read_request(&mut stream);
            let mut value = chat(json!({"content":"SENTINEL_RECOVERY_BODY"}), json!("stop"));
            value["usage"] = json!({"prompt_tokens":11,"completion_tokens":7,"total_tokens":18});
            let mut recovery = frame(value);
            recovery.extend_from_slice(b"data: [DONE]\n\n");
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", recovery.len()).unwrap();
            stream.write_all(&recovery).unwrap();
            let _ = stream.finish();
        }
        request
    });
    let publications = Arc::new(GatewayPublicationInstaller::open(root.join("lkg.json")).unwrap());
    let publication = publication(&listener, prebody && !relay, no_credential);
    if let crate::server::publication::GatewayPrepareOutcome::Prepared(prepared) =
        publications.prepare(publication).unwrap()
    {
        publications.publish(prepared).unwrap();
    }
    let credential = json!({"schema_version":"hiroute.gateway.credential-leases/v1",
        "credential_ref":"stream-credential","keys":[{"key_id":"stream-key","generation":1,
        "authorization":"Bearer SENTINEL_UPSTREAM_CREDENTIAL"}]});
    fs::write(
        root.join("lease.json"),
        serde_json::to_vec(&credential).unwrap(),
    )
    .unwrap();
    fs::write(root.join("credentials.json"), serde_json::to_vec(&json!({
        "schema_version":"hiroute.gateway.credentials/v1","credentials":{"stream-credential":"lease.json"}
    })).unwrap()).unwrap();
    let mut ports = ProductionPorts::fail_closed(publications);
    ports.credentials = Arc::new(
        crate::server::composition::FileCredentialResolver::open(&root.join("credentials.json"))
            .unwrap(),
    );
    ports.runtime_state = Arc::new(crate::ports::InMemoryRuntimeStateStore::default());
    let diagnostics = DiagnosticRuntime::start(RuntimeConfig {
        root: root.join("diagnostics"),
        role: ProcessRole::Daemon,
        component: Component::Gateway,
        parent_session_id: None,
        level_override: Some(DiagnosticLevel::Debug),
    });
    let mut sinks = GatewayObservationSinks::discard();
    sinks.execution_fact = Arc::new(facts.clone());
    let observation =
        GatewayObservation::with_sinks(512 * 1024, sinks).with_diagnostics(diagnostics.port());
    let runtime = ProductionGatewayRuntime::compose_with_planner_and_observation(
        ports,
        Arc::new(PublicationPlannerInputAuthority),
        Arc::new(observation),
    );
    let address = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let mut gateway = crate::server::GatewayLauncher::from_runtime(address, Arc::new(runtime))
        .unwrap()
        .start_managed()
        .unwrap();
    let body = serde_json::to_vec(&json!({"model":"stream-alias","max_tokens":64,
        "messages":[{"role":"user","content":"SENTINEL_REQUEST_BODY"}],"stream":true,
        "tools":[{"name":TOOL,"description":"SENTINEL_TOOL_DESCRIPTION","input_schema":{"type":"object"}}]})).unwrap();
    let mut client = TcpStream::connect(address).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    write!(client, "POST /v1/messages HTTP/1.1\r\nHost: {address}\r\nX-HiRoute-Token: {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
    client.write_all(&body).unwrap();
    let mut response = Vec::new();
    if after_body {
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        while !response
            .windows(b"SENTINEL_INITIAL_BODY".len())
            .any(|w| w == b"SENTINEL_INITIAL_BODY")
        {
            let mut chunk = [0u8; 4096];
            let n = client
                .read(&mut chunk)
                .expect("initial body was not delivered before the error");
            assert!(n != 0, "response ended before initial body delivery");
            response.extend_from_slice(&chunk[..n]);
        }
        body_delivered.send(()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
    }
    // A failed committed stream may end with a reset. The formal execution fact is the oracle.
    let _ = client.read_to_end(&mut response);
    assert!(
        response.starts_with(if prebody && !relay {
            b"HTTP/1.1 502"
        } else {
            b"HTTP/1.1 200"
        }),
        "{}",
        String::from_utf8_lossy(&response)
    );
    if matches!(case, "capture_known" | "capture_known_usage_first") {
        let response = String::from_utf8_lossy(&response);
        assert!(response.contains("\"type\":\"tool_use\""));
        assert!(response.contains(&format!("\"name\":\"{TOOL}\"")));
        assert!(response.contains("\"partial_json\":\"{}\""));
        assert!(response.contains("event: message_stop"));
    }
    if relay {
        assert!(
            response
                .windows(b"SENTINEL_RECOVERY_BODY".len())
                .any(|w| w == b"SENTINEL_RECOVERY_BODY")
        );
    }
    let upstream_request = upstream.join().unwrap();
    assert!(!upstream_request.is_empty());
    if no_credential {
        assert!(
            !String::from_utf8_lossy(&upstream_request)
                .to_ascii_lowercase()
                .contains("authorization:")
        );
    }
    wait_finished(&facts, reason.is_some() && !relay);
    gateway.shutdown();
    gateway.join(Duration::from_secs(5)).unwrap();
    let status = diagnostics.status();
    assert!(status.armed);
    assert!(status.unavailable.is_none());
    assert!(status.settings_error.is_none());
    diagnostics.shutdown();
    // Keep the collector live through Gateway shutdown; request_finished is
    // a terminal business fact, not a barrier for every observation producer.
    let facts = facts.0.lock().unwrap().clone();
    if usage_first {
        assert!(
            facts.iter().all(|r| {
                r.pointer("/fact/source").and_then(Value::as_str) != Some("provider_completion")
            }),
            "canonical usage must suppress the later duplicate provider completion"
        );
    }
    if no_credential {
        assert!(
            !facts.iter().any(
                |r| r.pointer("/fact/kind").and_then(Value::as_str) == Some("credential_lease")
            )
        );
    }
    let log = fs::read_to_string(root.join("diagnostics/daemon/current.jsonl")).unwrap();
    assert!(
        !log.contains("SENTINEL"),
        "untrusted body/tool/header leaked into product diagnostics"
    );
    let records: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        records
            .iter()
            .any(|r| payload(r, "level_applied").is_some_and(|v| v["level"] == "debug")),
        "actual Debug level missing: {log}"
    );
    if relay {
        assert_prebody_relay_business(&facts);
    } else {
        assert_business(&facts, reason.is_some(), after_body);
    }
    if let Some(reason) = reason {
        let failures: Vec<_> = records
            .iter()
            .filter_map(|r| payload(r, "response_failure"))
            .collect();
        assert_eq!(failures.len(), 1, "{case}: {log}");
        let failure = failures[0];
        assert_eq!(failure["reason"], reason, "{case}");
        assert_eq!(
            failure.get("field").and_then(Value::as_str),
            field,
            "{case}"
        );
        assert_eq!(failure["stage"], "decode", "{case}");
        if frame_index != 0 {
            assert_eq!(failure["frame_index"], frame_index, "{case}");
        }
        assert!(
            failure["received_bytes"]
                .as_u64()
                .is_some_and(|n| n > 0 && n <= wire.len() as u64)
        );
        assert_eq!(failure["attempt_index"], 1);
        let begin = records
            .iter()
            .find_map(|r| payload(r, "attempt_begin"))
            .unwrap();
        let end = records
            .iter()
            .find_map(|r| payload(r, "request_end"))
            .unwrap();
        if after_body {
            assert!(!failure["attempt_token"].is_null());
            assert_eq!(failure["attempt_token"], begin["attempt_token"]);
        } else {
            // Prebody classification precedes formal attempt promotion. The
            // diagnostic must retain the request and ordinal, without inventing
            // an unavailable attempt token.
            assert!(failure["attempt_token"].is_null());
        }
        assert!(!failure["request_token"].is_null());
        assert_eq!(failure["request_token"], end["request_token"]);
    } else {
        assert!(
            records
                .iter()
                .all(|r| payload(r, "response_failure").is_none())
        );
    }
    wait_replay_clean(&root.join("replay"));
    if capture_enabled {
        assert_capture(
            &root.join("capture"),
            &upstream_request,
            &wire,
            reason,
            &records,
            &facts,
        );
    } else {
        assert!(
            !root.join("capture").exists(),
            "default-disabled path created capture state"
        );
    }
}

fn publication(
    provider: &TestTlsListener,
    limit_prebody_attempts: bool,
    no_credential: bool,
) -> GatewayPublicationSnapshotV3 {
    let protocol = IngressProtocol::Messages;
    GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "stream-authority",
        1,
        1,
        "stream-renderer/v1",
        vec![AliasPlanV1 {
            served_model_id: "stream-alias".into(),
            purpose: "stream diagnostics acceptance".into(),
            agent_plan_revision: 1,
            protocols: vec![protocol],
            overall_timeout_ms: 30_000,
            // The initial malformed entity is tested with an explicit one-attempt
            // budget. Other cases retain both candidates and prove no replay once
            // the client has actually received the first body unit.
            max_attempts: if limit_prebody_attempts { 1 } else { 2 },
            routing: None,
            candidates: (1..=2)
                .map(|i| {
                    let mut candidate = sealed_native_candidate(
                        i,
                        &format!("stream-target-{i}"),
                        &["stream-credential".into()],
                        provider.authority(),
                        "physical",
                        &[(protocol, IngressProtocol::ChatCompletions)],
                    );
                    if no_credential {
                        candidate.credential_refs = vec![format!("credential/none/stream-{i}")];
                        for profile in &mut candidate.protocol_profiles {
                            profile.connector.authentication =
                                hiroute_domain::GatewayCriticalFactV1::Exact(
                                    hiroute_domain::GatewayAuthenticationSemanticsV1::None,
                                );
                        }
                        candidate.protocol_profile_digest =
                            hiroute_domain::CanonicalDigest::of(&candidate.protocol_profiles)
                                .unwrap();
                    }
                    candidate
                })
                .collect(),
        }],
        vec![GrantV1 {
            grant_id: "stream-grant".into(),
            generation: 1,
            bearer_token_sha256: token_sha256(TOKEN),
            protocol,
            route_protocols: Default::default(),
            routes: [(
                "stream-alias".into(),
                ModelRouteV2::Plan {
                    plan_id: "legacy/stream-alias".into(),
                    alias: "stream-alias".into(),
                    revision: 1,
                    semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(b"stream-alias"),
                },
            )]
            .into_iter()
            .collect(),
        }],
    )
    .unwrap()
}

fn read_request(stream: &mut impl Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut expected = None;
    loop {
        let mut chunk = [0u8; 4096];
        let len = stream.read(&mut chunk).unwrap();
        assert!(len > 0, "upstream request ended before its body");
        bytes.extend_from_slice(&chunk[..len]);
        if expected.is_none()
            && let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n")
        {
            let head = String::from_utf8_lossy(&bytes[..end]);
            let length = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_owned)
                })
                .unwrap()
                .parse::<usize>()
                .unwrap();
            expected = Some(end + 4 + length);
        }
        if expected.is_some_and(|expected| bytes.len() == expected) {
            return bytes;
        }
        assert!(bytes.len() < 1024 * 1024);
    }
}

fn payload<'a>(record: &'a Value, name: &str) -> Option<&'a Value> {
    record
        .get(name)
        .or_else(|| record.as_object()?.values().find_map(|v| payload(v, name)))
}
fn wait_finished(facts: &Facts, failed: bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let records = facts.0.lock().unwrap().clone();
        let finished = records
            .iter()
            .any(|r| r.pointer("/fact/kind").and_then(Value::as_str) == Some("request_finished"));
        // Canonical usage can arrive after the terminal fact. Provider usage
        // is omitted when canonical usage was already recorded, so it is not
        // a completion barrier. The business oracle checks each source below.
        let usage_complete = failed
            || records.iter().any(|r| {
                r.pointer("/fact/kind").and_then(Value::as_str) == Some("usage_and_cache")
                    && r["fact"]["source"] == "accepted_canonical_model_event"
            });
        if finished && usage_complete {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "incomplete terminal/usage facts: {records:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_canonical_usage(facts: &Facts) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let records = facts.0.lock().unwrap().clone();
        if records.iter().any(|r| {
            r.pointer("/fact/kind").and_then(Value::as_str) == Some("usage_and_cache")
                && r.pointer("/fact/source").and_then(Value::as_str)
                    == Some("accepted_canonical_model_event")
        }) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "canonical usage was not delivered"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn assert_prebody_relay_business(facts: &[Value]) {
    let named = |name| {
        facts
            .iter()
            .filter(|r| r.pointer("/fact/kind").and_then(Value::as_str) == Some(name))
            .collect::<Vec<_>>()
    };
    let starts = named("attempt_started");
    assert_eq!(starts.len(), 2);
    let finished = named("attempt_finished");
    assert_eq!(finished.len(), 2);
    let first = finished.iter().find(|r| r["fact"]["ordinal"] == 1).unwrap();
    let second = finished.iter().find(|r| r["fact"]["ordinal"] == 2).unwrap();
    assert_eq!(first["fact"]["outcome"], "rejected");
    assert_eq!(first["fact"]["disposition"], "continue");
    assert_eq!(first["fact"]["retryable"], true);
    assert_eq!(first["fact"]["commits"]["downstream_headers"], "clear");
    assert_eq!(first["fact"]["commits"]["downstream_semantic"], "clear");
    assert_eq!(second["fact"]["outcome"], "accepted");
    assert_eq!(
        second["fact"]["commits"]["downstream_headers"],
        "write_confirmed"
    );
    let requests = named("request_finished");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["fact"]["outcome"], "accepted");
    assert_eq!(requests[0]["fact"]["attempts_started"], 2);
    assert_eq!(requests[0]["fact"]["attempts_finished"], 2);
    assert_eq!(named("semantic_commit").len(), 1);
    assert_success_usage(facts, 2);
}

fn assert_business(facts: &[Value], failed: bool, after_body: bool) {
    let named = |name| {
        facts
            .iter()
            .filter(|r| r.pointer("/fact/kind").and_then(Value::as_str) == Some(name))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        named("attempt_started").len(),
        1,
        "only one actual attempt may run: bounded prebody or committed afterbody"
    );
    let finished = named("attempt_finished");
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0]["fact"]["ordinal"], 1);
    assert_eq!(
        finished[0]["fact"]["outcome"],
        if after_body {
            "postcommit_transport_failed"
        } else if failed {
            "rejected"
        } else {
            "accepted"
        }
    );
    if failed && !after_body {
        // Relay eligibility remains Continue; the separately frozen one-attempt
        // budget ends the request without promoting another candidate.
        assert_eq!(finished[0]["fact"]["disposition"], "continue");
        assert_eq!(finished[0]["fact"]["retryable"], true);
        assert_eq!(
            finished[0]["fact"]["commits"]["downstream_semantic"],
            "clear"
        );
    }
    if after_body {
        let semantic_started = finished[0]["fact"]["commits"]["downstream_semantic"] != "clear";
        assert_eq!(
            finished[0]["fact"]["termination_reason"],
            if semantic_started {
                "stream_started_no_retry"
            } else {
                "provider_encoder_failure"
            }
        );
        assert_eq!(
            finished[0]["fact"]["stream_outcome"],
            if semantic_started {
                "stream_started_no_retry"
            } else {
                "aborted_before_semantic_commit"
            }
        );
    }
    assert_eq!(
        finished[0]["fact"]["commits"]["upstream_request"],
        "write_confirmed"
    );
    assert_eq!(
        finished[0]["fact"]["commits"]["downstream_headers"],
        if failed && !after_body {
            "clear"
        } else {
            "write_confirmed"
        }
    );
    let requests = named("request_finished");
    assert_eq!(finished[0]["fact"]["cleanup_outcome"], "completed");
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]["fact"]["outcome"],
        if failed {
            if finished[0]["fact"]["commits"]["downstream_semantic"] != "clear" {
                "postcommit_transport_failed"
            } else {
                "failed"
            }
        } else {
            "accepted"
        }
    );
    assert_eq!(requests[0]["fact"]["attempts_started"], 1);
    assert_eq!(requests[0]["fact"]["attempts_finished"], 1);
    let semantic_commits = named("semantic_commit").len();
    assert_eq!(semantic_commits, if failed && !after_body { 0 } else { 1 });
    if failed {
        // Failed entities report no usage; never synthesize zero tokens.
        assert!(named("usage_and_cache").is_empty());
    } else {
        assert_success_usage(facts, 1);
    }
}

pub(super) fn assert_success_usage(facts: &[Value], ordinal: u64) {
    let usage: Vec<_> = facts
        .iter()
        .filter(|r| r["fact"]["kind"] == "usage_and_cache")
        .collect();
    let count = |source: &str| {
        usage
            .iter()
            .filter(|r| r["fact"]["source"] == source)
            .count()
    };
    assert_eq!(count("accepted_canonical_model_event"), 1);
    // completed_attempt suppresses provider usage if canonical usage arrived
    // first. Both callback orders must retain exactly one canonical record.
    assert!(count("provider_completion") <= 1);
    let attempt = facts
        .iter()
        .find(|r| r["fact"]["kind"] == "attempt_started" && r["fact"]["ordinal"] == ordinal)
        .unwrap();
    assert!(attempt["attempt_id"].is_string());
    for record in usage {
        assert_eq!(record["fact"]["ordinal"], ordinal);
        assert_eq!(record["attempt_id"], attempt["attempt_id"]);
        assert_eq!(record["correlation"], attempt["correlation"]);
        assert!(matches!(
            record["fact"]["source"].as_str(),
            Some("provider_completion" | "accepted_canonical_model_event")
        ));
        assert_eq!(record["fact"]["input_tokens"], 11);
        assert_eq!(record["fact"]["output_tokens"], 7);
    }
}

fn wait_replay_clean(root: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if !root.exists() || fs::read_dir(root).unwrap().next().is_none() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Replay resources survived request cleanup"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
