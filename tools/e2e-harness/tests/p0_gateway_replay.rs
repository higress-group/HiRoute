mod runtime_support;

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
#[cfg(all(unix, not(target_os = "linux")))]
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use runtime_support::*;

#[path = "p0_gateway_replay/literal_locator.rs"]
mod literal_locator;

const OK: &[u8] = br#"{"id":"replay-ok","model":"runtime-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#;
const RETRY: &[u8] = br#"{"error":{"type":"server_error","message":"retry"}}"#;

#[test]
fn real_hirouted_replays_threshold_below_and_above_for_two_fallbacks() {
    hiroute_e2e::p0_execution_receipt!(
        "replay.threshold",
        [
            "replay.threshold",
            "replay.two_attempts",
            "replay.bounded_memory"
        ]
    );
    let small_provider = NativeProvider::start(vec![success()]);
    let fixture = RuntimeFixture::launch_with_replay(&[&small_provider], 1, 4_096, 1_024);
    let below = request_document("below-threshold-".repeat(32));
    assert_eq!(fixture.request_body(&below).status, 200);
    wait_replay_empty(&fixture.replay_root);
    assert_eq!(small_provider.calls(), 1);
    drop(fixture);
    drop(small_provider);

    let first = NativeProvider::start(vec![retry()]);
    let second = NativeProvider::start(vec![retry()]);
    let third = NativeProvider::start(vec![success()]);
    let fixture = RuntimeFixture::launch_with_replay(&[&first, &second, &third], 3, 4_096, 1_024);
    let warmed_rss = resident_kib(fixture.process.id());

    let large_text = "large replay \"quoted\" \\ unicode 路由\n".repeat(16_384);
    let above = request_document(large_text.clone());
    assert!(
        above.len() < 1024 * 1024,
        "stay inside the logical body plan"
    );
    let response = fixture.request_body(&above);
    assert_eq!(
        response.status,
        200,
        "calls={:?} body={}",
        (first.calls(), second.calls(), third.calls()),
        String::from_utf8_lossy(&response.body)
    );
    wait_replay_empty(&fixture.replay_root);

    assert_eq!((first.calls(), second.calls(), third.calls()), (1, 1, 1));
    let requests = [first.requests(), second.requests(), third.requests()];
    let mut projected = requests
        .iter()
        .map(|requests| {
            serde_json::from_slice::<serde_json::Value>(http_body(&requests[0]))
                .expect("provider JSON")
        })
        .collect::<Vec<_>>();
    for document in &mut projected {
        document["model"] = serde_json::Value::Null;
    }
    assert!(
        projected[1] == projected[0] && projected[2] == projected[0],
        "fallback candidates must receive identical logical content"
    );
    assert_eq!(projected[2]["input"][0]["content"][0]["text"], large_text);

    if let (Some(before), Some(after)) = (warmed_rss, resident_kib(fixture.process.id())) {
        assert!(
            after.saturating_sub(before) < 64 * 1024,
            "one sub-MiB request retained an unexpected {} KiB RSS",
            after.saturating_sub(before)
        );
    }
}

#[test]
fn real_hirouted_bounds_peak_rss_for_concurrent_near_limit_replays_during_provider_stall() {
    let body_barrier = ProviderBodyBarrier::default();
    let first = NativeProvider::start(vec![
        success(),
        body_barrier_success(&body_barrier),
        body_barrier_success(&body_barrier),
        body_barrier_success(&body_barrier),
    ]);
    let second = NativeProvider::start(vec![
        delayed_success(),
        delayed_success(),
        delayed_success(),
    ]);
    let third = NativeProvider::start(vec![
        delayed_success(),
        delayed_success(),
        delayed_success(),
    ]);
    let fixture =
        RuntimeFixture::launch_with_replay(&[&first, &second, &third], 3, 64 * 1024, 16 * 1024);
    let _barrier_release = ProviderBodyBarrierReleaseGuard(body_barrier.clone());
    assert_eq!(fixture.request().status, 200, "warm real runtime");
    wait_replay_empty(&fixture.replay_root);
    let warmed_rss = resident_kib(fixture.process.id()).expect("Unix process RSS is required");
    let body = request_document("concurrent retained replay ".repeat(26_000));
    assert!(body.len() < 1024 * 1024, "stay inside the body plan");
    let rss_sampler =
        RssSampler::start(fixture.process.id(), Some(warmed_rss)).expect("required RSS sampler");

    let responses = std::thread::scope(|scope| {
        let requests = (0..3)
            .map(|_| {
                let body = &body;
                scope.spawn(|| fixture.request_body_with_timeout(body, Duration::from_secs(10)))
            })
            .collect::<Vec<_>>();
        body_barrier.wait_for_arrivals(3, Duration::from_secs(5));
        let samples_before_barrier = rss_sampler.sample_count();
        wait_until(
            Duration::from_secs(1),
            "RSS sample during body barrier",
            || rss_sampler.sample_count() > samples_before_barrier,
        );
        let readiness_started = Instant::now();
        let readiness = request_with_timeout(
            fixture.address,
            "GET",
            "/_hiroute/ready",
            &[],
            b"",
            Duration::from_secs(1),
        );
        assert_eq!(readiness.status, 200, "readiness during replay stall");
        assert!(
            readiness_started.elapsed() < Duration::from_secs(1),
            "readiness exceeded the one-second budget during concurrent replay"
        );
        body_barrier.release();
        let deadline = Instant::now() + Duration::from_secs(10);
        while requests.iter().any(|request| !request.is_finished()) {
            assert!(
                Instant::now() < deadline,
                "concurrent requests timed out; calls={:?}, complete requests={:?}",
                (first.calls(), second.calls(), third.calls()),
                (
                    first.requests().len(),
                    second.requests().len(),
                    third.requests().len()
                )
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        requests
            .into_iter()
            .map(|request| request.join().expect("concurrent client"))
            .collect::<Vec<_>>()
    });
    let peak_rss = rss_sampler.finish();
    // A Provider body barrier does not prove every client-side reader remains
    // alive: socket buffering can finish an upload before the Provider resumes.
    // Linux VmHWM also captures transient allocations between sampler ticks.
    #[cfg(target_os = "linux")]
    let peak_rss = peak_rss.max(
        linux_memory_kib(fixture.process.id(), "VmHWM:")
            .expect("Linux peak RSS is required; missing samples cannot pass"),
    );
    assert!(
        peak_rss.saturating_sub(warmed_rss) < 48 * 1024,
        "three concurrent near-limit requests grew peak RSS by {} KiB",
        peak_rss.saturating_sub(warmed_rss)
    );

    assert!(
        responses.iter().all(|response| response.status == 200),
        "statuses/bodies: {:?}",
        responses
            .iter()
            .map(|response| (
                response.status,
                String::from_utf8_lossy(&response.body).into_owned()
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        first.calls() + second.calls() + third.calls(),
        4,
        "one warm request plus three concurrently held logical requests"
    );
    wait_replay_empty(&fixture.replay_root);
    let after = resident_kib(fixture.process.id()).expect("Unix RSS after cleanup is required");
    assert!(
        after.saturating_sub(warmed_rss) < 32 * 1024,
        "cleaned concurrent requests retained {} KiB RSS",
        after.saturating_sub(warmed_rss)
    );
}

#[test]
fn real_hirouted_releases_large_namespace_replay_before_long_sse_completion() {
    hiroute_e2e::p0_execution_receipt!(
        "replay.long_sse",
        [
            "replay.long_prompt",
            "replay.long_sse",
            "replay.rss_bound",
            "replay.release"
        ]
    );
    let provider = NativeProvider::start(vec![
        success(),
        ProviderReply::StreamDrip {
            status: 200,
            chunks: responses_stream_chunks(48),
            interval: Duration::from_millis(30),
        },
    ]);
    let fixture = RuntimeFixture::launch_with_replay(&[&provider], 1, 4 * 1024, 1024);
    assert_eq!(fixture.request().status, 200, "warm real runtime");
    wait_replay_empty(&fixture.replay_root);
    let warmed_rss = resident_kib(fixture.process.id());
    let input = "long namespace replay input ".repeat(10_000);
    let namespace_description = "namespace-description-".repeat(4_000);
    let schema_description = "schema-description-".repeat(4_000);
    let body =
        namespace_streaming_request_document(&input, &namespace_description, &schema_description);
    assert!(body.len() < 1024 * 1024, "stay inside the body plan");
    let mut client = begin_partial_request(fixture.address, &body, 16 * 1024);
    wait_replay_nonempty(&fixture.replay_root);
    client.write_all(&body[16 * 1024..]).unwrap();
    client.flush().unwrap();

    let response = std::thread::scope(|scope| {
        let request = scope.spawn(move || {
            client
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut wire = Vec::new();
            client.read_to_end(&mut wire).unwrap();
            wire
        });
        wait_until(Duration::from_secs(5), "second provider call", || {
            provider.calls() == 2
        });
        wait_replay_empty(&fixture.replay_root);
        assert!(
            !request.is_finished(),
            "the request Replay must be released while the controlled upstream is still streaming"
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut peak_rss = warmed_rss;
        while !request.is_finished() {
            if let Some(sample) = resident_kib(fixture.process.id()) {
                peak_rss = Some(peak_rss.unwrap_or(sample).max(sample));
            }
            assert!(Instant::now() < deadline, "long SSE request timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        if let (Some(warmed), Some(peak)) = (warmed_rss, peak_rss) {
            assert!(
                peak.saturating_sub(warmed) < 32 * 1024,
                "one near-limit namespace request grew peak RSS by {} KiB",
                peak.saturating_sub(warmed)
            );
        }
        request.join().expect("streaming client")
    });

    assert_eq!(response_status(&response), 200);
    wait_replay_empty(&fixture.replay_root);
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let projected: serde_json::Value =
        serde_json::from_slice(http_body(&requests[1])).expect("provider request JSON");
    assert_eq!(projected["input"][0]["role"], "user");
    assert_eq!(projected["input"][0]["content"][0]["type"], "input_text");
    assert_eq!(projected["input"][0]["content"][0]["text"], input);
    assert_eq!(projected["tools"][0]["type"], "namespace");
    assert_eq!(projected["tools"][0]["name"], "bulk-services");
    assert_eq!(projected["tools"][0]["description"], namespace_description);
    assert_eq!(
        projected["tools"][0]["tools"][0]["parameters"]["properties"]["payload"]["description"],
        schema_description
    );
    assert_eq!(projected["tools"][1]["name"], "flat_probe");
    if let (Some(warmed), Some(after)) = (warmed_rss, resident_kib(fixture.process.id())) {
        assert!(
            after.saturating_sub(warmed) < 24 * 1024,
            "completed SSE retained {} KiB RSS",
            after.saturating_sub(warmed)
        );
    }
}

#[cfg(unix)]
#[test]
fn real_hirouted_uses_one_owner_only_plaintext_replay_backing() {
    hiroute_e2e::p0_execution_receipt!(
        "replay.local_plaintext",
        ["replay.owner_only_plaintext", "replay.single_backing"]
    );
    let provider = NativeProvider::start(vec![success()]);
    let fixture = RuntimeFixture::launch_with_replay(&[&provider], 1, 128, 31);
    let body = request_document("plaintext-replay-marker-".repeat(30_000));
    let prefix = body.len() / 2;
    let mut client = begin_partial_request(fixture.address, &body, prefix);
    let backing = wait_for_live_replay_file(&fixture.replay_root, prefix as u64);
    let files = replay_files(&fixture.replay_root);
    assert_eq!(
        files.as_slice(),
        std::slice::from_ref(&backing),
        "request has one live backing"
    );
    assert_eq!(
        fs::read(&backing).expect("read live backing"),
        body[..prefix],
        "the owner-only spill stores the received request bytes directly"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            backing
                .metadata()
                .expect("backing metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "plaintext backing must remain owner-only"
        );
    }

    client
        .write_all(&body[prefix..])
        .expect("finish request body");
    client.flush().expect("flush request body");
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("response timeout");
    let mut response = Vec::new();
    client.read_to_end(&mut response).expect("read response");
    assert_eq!(response_status(&response), 200);
    assert_eq!(provider.calls(), 1);
    wait_replay_empty(&fixture.replay_root);
}

#[test]
fn real_hirouted_rejects_large_non_http_image_source_before_provider_call() {
    let forbidden = NativeProvider::start(vec![success()]);
    let fixture = RuntimeFixture::launch_with_replay(&[&forbidden], 1, 128, 31);
    let invalid_url = format!("ftp://invalid.example/{}", "x".repeat(12_000));
    let body = serde_json::to_vec(&serde_json::json!({
        "model":"runtime-model",
        "input":[{
            "type":"message",
            "role":"user",
            "content":[{"type":"input_image","image_url":invalid_url}]
        }],
        "stream":false
    }))
    .expect("invalid image request JSON");

    let response = fixture.request_body(&body);
    assert_eq!(
        response.status, 400,
        "invalid image source must fail as a client semantic error"
    );
    assert_eq!(forbidden.calls(), 0, "Provider must not be connected");
    wait_replay_empty(&fixture.replay_root);
}

#[test]
fn real_hirouted_cleans_exhaustion_deadline_disconnect_and_restart_orphan() {
    hiroute_e2e::p0_execution_receipt!(
        "replay.cancel_orphan",
        ["replay.cancel_cleanup", "replay.orphan_cleanup"]
    );
    let exhausted = NativeProvider::start(vec![retry()]);
    let fixture = RuntimeFixture::launch_with_replay(&[&exhausted], 1, 128, 31);
    assert_ne!(
        fixture
            .request_body(&request_document("exhaust ".repeat(4_096)))
            .status,
        200
    );
    wait_replay_empty(&fixture.replay_root);
    drop(fixture);

    let deadline = NativeProvider::start(vec![ProviderReply::Stall {
        duration: Duration::from_millis(700),
    }]);
    let fixture = RuntimeFixture::launch_with_replay_timeout(&[&deadline], 1, 128, 100);
    let mut deadline_client = begin_request(
        fixture.address,
        &request_document("deadline ".repeat(4_096)),
    );
    deadline_client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("deadline read timeout");
    let mut deadline_response = Vec::new();
    let _ = deadline_client.read_to_end(&mut deadline_response);
    assert!(deadline.calls() <= 1);
    wait_replay_empty(&fixture.replay_root);
    drop(fixture);

    let disconnected = NativeProvider::start(vec![ProviderReply::Stall {
        duration: Duration::from_millis(500),
    }]);
    let fixture = RuntimeFixture::launch_with_replay_timeout(&[&disconnected], 1, 128, 5_000);
    let client = begin_request(
        fixture.address,
        &request_document("disconnect ".repeat(4_096)),
    );
    wait_until(
        Duration::from_secs(5),
        "disconnected provider request",
        || !disconnected.requests().is_empty(),
    );
    wait_replay_nonempty(&fixture.replay_root);
    let _ = client.shutdown(std::net::Shutdown::Both);
    drop(client);
    wait_replay_empty(&fixture.replay_root);
    drop(fixture);

    let accepted = NativeProvider::start(vec![success()]);
    let fixture = RuntimeFixture::launch_with_seed_orphan(&[&accepted], 1);
    assert!(
        !fixture
            .replay_root
            .join("req-seeded-restart-orphan")
            .exists()
    );
    assert_eq!(fixture.request().status, 200);
    wait_replay_empty(&fixture.replay_root);
}

#[cfg(unix)]
#[test]
fn real_hirouted_rejects_non_owner_only_replay_root() {
    hiroute_e2e::p0_execution_receipt!(
        "replay.owner_only_permission",
        [
            "replay.owner_only_permission",
            "replay.zero_provider_on_permission"
        ]
    );
    let forbidden = NativeProvider::start(vec![success()]);
    let fixture = RuntimeFixture::launch_with_unsafe_replay_root(&[&forbidden], 1);
    let response = fixture.request();
    assert_eq!(response.status, 503);
    assert_eq!(forbidden.calls(), 0);
}

#[test]
fn real_hirouted_rejects_unavailable_request_workspace_before_provider_call() {
    hiroute_e2e::p0_execution_receipt!(
        "replay.request_capacity",
        [
            "replay.request_capacity_limit",
            "replay.zero_provider_on_capacity"
        ]
    );
    let forbidden = NativeProvider::start(vec![success()]);
    let fixture = RuntimeFixture::launch_with_replay(&[&forbidden], 1, 4_096, 1_024);
    let body = request_document("capacity boundary ".repeat(500_000));
    assert!(
        body.len() > 8 * 1024 * 1024,
        "parsed JSON workspace must exceed the actual stream memory budget"
    );

    let response = fixture.request_body(&body);
    assert_eq!(response.status, 503);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&response.body).unwrap(),
        serde_json::json!({
            "schema_version": "hiroute.gateway.error/v1",
            "code": "REPLAY_BUDGET_UNAVAILABLE",
            "phase": "canonical_request"
        })
    );
    assert_eq!(forbidden.calls(), 0);
    wait_replay_empty(&fixture.replay_root);
}

fn retry() -> ProviderReply {
    ProviderReply::Complete {
        status: 503,
        error_kind: None,
        body: RETRY,
    }
}

fn success() -> ProviderReply {
    ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: OK,
    }
}

fn delayed_success() -> ProviderReply {
    ProviderReply::DelayedComplete {
        duration: Duration::from_secs(2),
        status: 200,
        error_kind: None,
        body: OK,
    }
}

fn body_barrier_success(barrier: &ProviderBodyBarrier) -> ProviderReply {
    ProviderReply::BodyReadBarrier {
        barrier: barrier.clone(),
        status: 200,
        error_kind: None,
        body: OK,
    }
}

struct ProviderBodyBarrierReleaseGuard(ProviderBodyBarrier);

impl Drop for ProviderBodyBarrierReleaseGuard {
    fn drop(&mut self) {
        self.0.release();
    }
}

fn request_document(input: String) -> Vec<u8> {
    let input = serde_json::to_string(&input).expect("request input");
    format!(r#"{{"model":"runtime-model","input":{input},"stream":false}}"#).into_bytes()
}

fn namespace_streaming_request_document(
    input: &str,
    namespace_description: &str,
    schema_description: &str,
) -> Vec<u8> {
    let mut remainder = serde_json::json!({
        "model": "runtime-model",
        "input": input,
        "stream": true,
        "parallel_tool_calls": false,
        "tool_choice": "auto",
        "tools": [
            {
                "type": "namespace",
                "name": "bulk-services",
                "description": namespace_description,
                "tools": [{
                    "type": "function",
                    "name": "ingest",
                    "description": "Ingest a bounded payload",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "payload": {"type":"string","description":schema_description}
                        },
                        "required": ["payload"]
                    },
                    "strict": true
                }]
            },
            {
                "type": "function",
                "name": "flat_probe",
                "parameters": {"type":"object"}
            }
        ]
    });
    let model = remainder
        .as_object_mut()
        .expect("namespace request object")
        .remove("model")
        .expect("top-level model");
    let remainder = serde_json::to_vec(&remainder).expect("namespace replay request");
    let mut body = format!("{{\"model\":{},", serde_json::to_string(&model).unwrap()).into_bytes();
    body.extend_from_slice(&remainder[1..]);
    body
}

fn responses_stream_chunks(delta_count: usize) -> Vec<Vec<u8>> {
    let mut chunks = vec![b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"replay-stream\",\"model\":\"runtime-native\"}}\n\n".to_vec()];
    chunks.extend((0..delta_count).map(|_| {
        b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"x\"}\n\n".to_vec()
    }));
    chunks.push(b"event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":48,\"total_tokens\":49}}}\n\n".to_vec());
    chunks
}

fn http_body(wire: &[u8]) -> &[u8] {
    let split = wire
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP head terminator");
    &wire[split + 4..]
}

fn wait_replay_empty(root: &Path) {
    wait_until(
        Duration::from_secs(5),
        "Replay root to become empty",
        || {
            root.is_dir()
                && fs::read_dir(root)
                    .expect("read replay root")
                    .next()
                    .is_none()
        },
    );
}

fn wait_replay_nonempty(root: &Path) {
    wait_until(
        Duration::from_secs(5),
        "Replay root to become nonempty",
        || {
            root.is_dir()
                && fs::read_dir(root)
                    .expect("read replay root")
                    .next()
                    .is_some()
        },
    );
}

fn wait_for_live_replay_file(root: &Path, minimum_len: u64) -> PathBuf {
    let mut found = None;
    wait_until(Duration::from_secs(5), "live plaintext Replay file", || {
        found = replay_files(root).into_iter().find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "replay")
                && path
                    .metadata()
                    .is_ok_and(|metadata| metadata.len() >= minimum_len)
        });
        found.is_some()
    });
    found.expect("plaintext replay file")
}

fn replay_files(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .flat_map(|entry| {
            fs::read_dir(entry.path())
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn wait_until(timeout: Duration, label: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn begin_request(address: std::net::SocketAddr, body: &[u8]) -> TcpStream {
    let mut stream = TcpStream::connect(address).expect("connect hirouted");
    write!(
        stream,
        "POST /v1/responses HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Type: application/json\r\nX-HiRoute-Token: runtime-token\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .expect("write request head");
    stream.write_all(body).expect("write request body");
    stream.flush().expect("flush request");
    stream
}

fn begin_partial_request(address: std::net::SocketAddr, body: &[u8], prefix: usize) -> TcpStream {
    assert!(prefix > 0 && prefix < body.len());
    let mut stream = TcpStream::connect(address).expect("connect hirouted");
    write!(
        stream,
        "POST /v1/responses HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Type: application/json\r\nX-HiRoute-Token: runtime-token\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .expect("write request head");
    stream
        .write_all(&body[..prefix])
        .expect("write request prefix");
    stream.flush().expect("flush request prefix");
    stream
}

fn response_status(wire: &[u8]) -> u16 {
    let split = wire
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("response head terminator");
    std::str::from_utf8(&wire[..split])
        .expect("response head UTF-8")
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse().ok())
        .expect("response status")
}

struct RssSampler {
    stop: Arc<AtomicBool>,
    peak_kib: Arc<AtomicU64>,
    sample_count: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
}

impl RssSampler {
    fn start(pid: u32, initial_kib: Option<u64>) -> Option<Self> {
        let initial_kib = initial_kib?;
        let stop = Arc::new(AtomicBool::new(false));
        let peak_kib = Arc::new(AtomicU64::new(initial_kib));
        let sample_count = Arc::new(AtomicUsize::new(0));
        let thread_stop = Arc::clone(&stop);
        let thread_peak = Arc::clone(&peak_kib);
        let thread_sample_count = Arc::clone(&sample_count);
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                if let Some(sample) = resident_kib(pid) {
                    thread_peak.fetch_max(sample, Ordering::Relaxed);
                    thread_sample_count.fetch_add(1, Ordering::Release);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        Some(Self {
            stop,
            peak_kib,
            sample_count,
            thread: Some(thread),
        })
    }

    fn sample_count(&self) -> usize {
        self.sample_count.load(Ordering::Acquire)
    }

    fn finish(mut self) -> u64 {
        self.stop_and_join();
        self.peak_kib.load(Ordering::Relaxed)
    }

    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("RSS sampler");
        }
    }
}

impl Drop for RssSampler {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

#[cfg(target_os = "linux")]
fn resident_kib(pid: u32) -> Option<u64> {
    linux_memory_kib(pid, "VmRSS:")
}

#[cfg(target_os = "linux")]
fn linux_memory_kib(pid: u32, field: &str) -> Option<u64> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix(field)?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

#[cfg(all(unix, not(target_os = "linux")))]
fn resident_kib(pid: u32) -> Option<u64> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    output.status.success().then_some(())?;
    std::str::from_utf8(&output.stdout)
        .ok()?
        .trim()
        .parse()
        .ok()
}

#[cfg(not(unix))]
fn resident_kib(_pid: u32) -> Option<u64> {
    None
}
