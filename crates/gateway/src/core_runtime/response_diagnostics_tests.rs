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
        if matches!(case, "capture_known" | "capture_unknown") {
            child.env("HIROUTE_PRIVATE_STREAM_CAPTURE", root.0.join("capture"));
        }
        let output = child.output().unwrap();
        if output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed") {
            println!("stream injection {case}: green");
        } else {
            failures.push(format!(
                "{case}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
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
        "invalid_json" => (
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
        "capture_known" => {
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
            | "capture_unknown"
            | "capture_disabled"
    ) {
        wire.extend_from_slice(b"data: [DONE]\n\n");
    }
    (wire, reason, field, index)
}

fn run_case(case: &str, root: &Path) {
    let capture_enabled = std::env::var_os("HIROUTE_PRIVATE_STREAM_CAPTURE").is_some();
    if capture_enabled {
        create_capture_session(&root.join("capture"));
    }
    let (wire, reason, field, frame_index) = sample(case);
    let listener = TestTlsListener::bind("stream-diagnostic.invalid").unwrap();
    write_dial_config(root, &[&listener]).unwrap();
    let peer = listener.try_clone().unwrap();
    let sent = wire.clone();
    let upstream = std::thread::spawn(move || {
        let (mut stream, _) = peer.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let request = read_request(&mut stream);
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", sent.len()).unwrap();
        let _ = stream.write_all(&sent);
        let _ = stream.finish();
        request
    });
    let publications = Arc::new(GatewayPublicationInstaller::open(root.join("lkg.json")).unwrap());
    let publication = publication(&listener);
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
    let facts = Facts::default();
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
    // A failed committed stream may end with a reset. The formal execution fact is the oracle.
    let _ = client.read_to_end(&mut response);
    assert!(
        response.starts_with(b"HTTP/1.1 200"),
        "{}",
        String::from_utf8_lossy(&response)
    );
    if case == "capture_known" {
        let response = String::from_utf8_lossy(&response);
        assert!(response.contains("\"type\":\"tool_use\""));
        assert!(response.contains(&format!("\"name\":\"{TOOL}\"")));
        assert!(response.contains("\"partial_json\":\"{}\""));
        assert!(response.contains("event: message_stop"));
    }
    let upstream_request = upstream.join().unwrap();
    assert!(!upstream_request.is_empty());
    wait_finished(&facts, reason.is_some());
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
    assert_business(&facts, reason.is_some());
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
        assert!(!failure["attempt_token"].is_null());
        assert_eq!(failure["attempt_token"], begin["attempt_token"]);
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

fn publication(provider: &TestTlsListener) -> GatewayPublicationSnapshotV3 {
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
            max_attempts: 2,
            routing: None,
            candidates: (1..=2)
                .map(|i| {
                    sealed_native_candidate(
                        i,
                        &format!("stream-target-{i}"),
                        &["stream-credential".into()],
                        provider.authority(),
                        "physical",
                        &[(protocol, IngressProtocol::ChatCompletions)],
                    )
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
        // The native completion producer can deliver its usage after the
        // request's terminal fact. Successful fixtures require both known
        // usage sources; assert_business still checks the exact count/content.
        let usage_complete = failed
            || records
                .iter()
                .filter(|r| {
                    r.pointer("/fact/kind").and_then(Value::as_str) == Some("usage_and_cache")
                })
                .count()
                >= 2;
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
fn assert_business(facts: &[Value], failed: bool) {
    let named = |name| {
        facts
            .iter()
            .filter(|r| r.pointer("/fact/kind").and_then(Value::as_str) == Some(name))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        named("attempt_started").len(),
        1,
        "no transparent retry after HTTP 200"
    );
    let finished = named("attempt_finished");
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0]["fact"]["ordinal"], 1);
    assert_eq!(
        finished[0]["fact"]["outcome"],
        if failed {
            "postcommit_transport_failed"
        } else {
            "accepted"
        }
    );
    if failed {
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
        "write_confirmed"
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
    if failed {
        assert!(semantic_commits <= 1);
    } else {
        assert_eq!(semantic_commits, 1);
    }
    let usage = named("usage_and_cache");
    // These failed entities report no usage: preserve absence rather than
    // synthesizing zero tokens from an incomplete response.
    assert_eq!(usage.len(), if failed { 0 } else { 2 });
    if !failed {
        assert_eq!(
            usage
                .iter()
                .filter(|r| r["fact"]["source"] == "provider_completion")
                .count(),
            1
        );
    }
    for record in usage {
        assert_eq!(record["fact"]["ordinal"], 1);
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
fn create_capture_session(root: &Path) {
    fs::DirBuilder::new().mode(0o700).create(root).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(root.join("session.json"))
        .unwrap();
    file.write_all(serde_json::to_string(&json!({"source_sha":"a".repeat(40),"binary_sha256":"b".repeat(64),
        "client":"deterministic-production-listener-fixture","expires_at":now+60,"delete_after":now+120})).unwrap().as_bytes()).unwrap();
}

fn assert_capture(
    root: &Path,
    upstream: &[u8],
    entity: &[u8],
    reason: Option<&str>,
    diagnostics: &[Value],
    facts: &[Value],
) {
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join("active.lock"))
        .unwrap();
    // The stable inode survives both clean exit and crashes; kernel ownership
    // must be released before cleanup can use this same lock.
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .expect("writer ownership was not released");
    let files: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "capture"))
        .collect();
    assert_eq!(files.len(), 1);
    let path = &files[0];
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let bytes = fs::read(path).unwrap();
    assert!(
        !bytes
            .windows(b"SENTINEL_UPSTREAM_CREDENTIAL".len())
            .any(|w| w == b"SENTINEL_UPSTREAM_CREDENTIAL")
    );
    let mut remaining = bytes.as_slice();
    let mut records = Vec::new();
    while !remaining.is_empty() {
        assert!(remaining.len() >= 9);
        let size = u64::from_le_bytes(remaining[1..9].try_into().unwrap()) as usize;
        assert!(size <= remaining.len() - 9);
        records.push((remaining[0], &remaining[9..9 + size]));
        remaining = &remaining[9 + size..];
    }
    assert_eq!(records.last().unwrap().0, 8);
    assert!(records.iter().any(|r| r.0 == 2));
    let captured = |kind| {
        records
            .iter()
            .filter(|r| r.0 == kind)
            .flat_map(|r| r.1.iter().copied())
            .collect::<Vec<_>>()
    };
    let body_offset = upstream.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    assert_eq!(
        captured(1),
        upstream[body_offset..],
        "capture must use actual prepared request reader"
    );
    assert_eq!(
        captured(4),
        entity,
        "ordered read blocks must reproduce the exact entity"
    );
    let context: Value = serde_json::from_slice(records[0].1).unwrap();
    assert_eq!(
        context["request_bytes"],
        (upstream.len() - body_offset) as u64
    );
    assert_eq!(context["profile"]["ingress_protocol"], "messages");
    assert_eq!(
        context["profile"]["capability"]["upstream_protocol"],
        "chat_completions"
    );
    assert!(!context["chat_tools"].is_null());
    assert_eq!(records.iter().filter(|r| r.0 == 7).count(), 1);
    let correlation: Value =
        serde_json::from_slice(records.iter().find(|r| r.0 == 7).unwrap().1).unwrap();
    let attempt = diagnostics
        .iter()
        .find_map(|r| payload(r, "attempt_begin"))
        .unwrap();
    assert_eq!(correlation["request_token"], attempt["request_token"]);
    assert_eq!(correlation["attempt_token"], attempt["attempt_token"]);
    assert_eq!(
        correlation["request_id"],
        facts
            .iter()
            .find_map(|r| r.pointer("/correlation/request_id"))
            .unwrap()
            .clone()
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    for chunks in [0, 1, 4096] {
        let replay = rt
            .block_on(crate::runtime::stream_capture::replay_capture(path, chunks))
            .unwrap();
        assert_eq!(
            replay["decoder"]["result"],
            if reason.is_some() {
                "rejected"
            } else {
                "accepted"
            }
        );
        if let Some(reason) = reason {
            assert_eq!(replay["decoder"]["reason"], reason);
        } else {
            assert_eq!(replay["decoder"]["tool_calls"], 1);
        }
        assert!(!replay.to_string().contains("SENTINEL"));
    }
}
