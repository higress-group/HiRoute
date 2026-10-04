//! The isolated loopback native-protocol endpoint used by installation revalidation.
//!
//! It accepts only the exact bearer, model, and minimal request shape expected from the selected
//! Harness.  It is not a general mock or an upstream proxy, and is dropped as soon as the probe
//! finishes or fails.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use hiroute_domain::delegation::{DelegationErrorV1, WorkerHarnessV1};
use serde_json::{Value, json};
use tokio::time::Instant;

pub(super) const PROBE_TEXT: &str = "OK";
pub(super) const PROBE_DEADLINE: Duration = Duration::from_secs(45);
// The revalidator performs New, an optional Load, and Cancel serially.  The loopback listener
// must outlive all three bounded child runs, rather than making a slow but valid first start
// consume the later cancellation window.
const SERVER_LIFETIME: Duration = Duration::from_secs(180);
const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_MODEL_REQUESTS: usize = 8;
const MAX_HEAD_REQUESTS: usize = 8;
const MAX_HEAD_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProbeRoute {
    Head,
    CountTokens,
    Model,
}

struct ProbeEndpoint {
    token: Vec<u8>,
    model: &'static str,
    harness: WorkerHarnessV1,
    requests: Arc<AtomicUsize>,
    head_requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    release_stall: Arc<AtomicBool>,
    stall_next: Arc<AtomicBool>,
}

pub(super) struct ProbeServer {
    address: std::net::SocketAddr,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    release_stall: Arc<AtomicBool>,
    stall_next: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<Result<(), DelegationErrorV1>>>,
}

impl ProbeServer {
    pub(super) fn bind(
        token: Vec<u8>,
        model: &'static str,
        harness: WorkerHarnessV1,
    ) -> Result<Self, DelegationErrorV1> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
        let address = listener
            .local_addr()
            .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
        listener
            .set_nonblocking(true)
            .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
        let requests = Arc::new(AtomicUsize::new(0));
        let head_requests = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let release_stall = Arc::new(AtomicBool::new(false));
        let stall_next = Arc::new(AtomicBool::new(false));
        let endpoint = ProbeEndpoint {
            token,
            model,
            harness,
            requests: Arc::clone(&requests),
            head_requests,
            stop: Arc::clone(&stop),
            release_stall: Arc::clone(&release_stall),
            stall_next: Arc::clone(&stall_next),
        };
        let thread = thread::Builder::new()
            .name("hiroute-worker-probe".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                while !endpoint.stop.load(Ordering::Acquire) && started.elapsed() < SERVER_LIFETIME
                {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            serve_probe_request(&mut stream, &endpoint)?;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => return Err(DelegationErrorV1::CapabilityUnavailable),
                    }
                }
                if endpoint.stop.load(Ordering::Acquire)
                    || endpoint.requests.load(Ordering::Acquire) > 0
                {
                    Ok(())
                } else {
                    Err(DelegationErrorV1::DeadlineExceeded)
                }
            })
            .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
        Ok(Self {
            address,
            requests,
            stop,
            release_stall,
            stall_next,
            thread: Some(thread),
        })
    }

    pub(super) fn address(&self) -> std::net::SocketAddr {
        self.address
    }

    /// A safe, token-free probe observation for an opt-in compatibility test.  It distinguishes
    /// a child that never reaches the loopback Gateway from one rejected by its strict endpoint.
    #[cfg(test)]
    pub(super) fn request_count(&self) -> usize {
        self.requests.load(Ordering::Acquire)
    }

    pub(super) async fn wait_for_request(&self, expected: usize) -> Result<(), DelegationErrorV1> {
        // Includes validation of the actual native binary before process launch. A six-second
        // timer could expire during artifact hashing, before there was a prompt to cancel.
        let deadline = Instant::now() + PROBE_DEADLINE;
        while self.requests.load(Ordering::Acquire) < expected {
            if Instant::now() >= deadline {
                return Err(DelegationErrorV1::DeadlineExceeded);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(())
    }

    pub(super) fn release_stall(&self) {
        self.release_stall.store(true, Ordering::Release);
    }

    pub(super) fn begin_stall(&self) -> usize {
        self.release_stall.store(false, Ordering::Release);
        self.stall_next.store(true, Ordering::Release);
        self.requests.load(Ordering::Acquire).saturating_add(1)
    }

    pub(super) fn finish(mut self) -> Result<(), DelegationErrorV1> {
        self.release_stall();
        self.stop.store(true, Ordering::Release);
        self.thread
            .take()
            .ok_or(DelegationErrorV1::Conflict)?
            .join()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?
    }
}

impl Drop for ProbeServer {
    fn drop(&mut self) {
        // A failed preliminary phase must not leave a token-bearing loopback server alive until
        // its deadline.  Joining also makes the private temporary root's teardown unambiguous.
        self.release_stall();
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve_probe_request(
    stream: &mut TcpStream,
    endpoint: &ProbeEndpoint,
) -> Result<(), DelegationErrorV1> {
    stream
        .set_nonblocking(false)
        .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    stream
        .set_write_timeout(Some(Duration::from_millis(500)))
        .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    let started = std::time::Instant::now();
    let mut request = Vec::new();
    let (header_end, length, route) = loop {
        if request.len() > MAX_REQUEST_BYTES || started.elapsed() > Duration::from_secs(3) {
            return Err(DelegationErrorV1::ProtocolFailed);
        }
        if let Some(end) = request.windows(4).position(|value| value == b"\r\n\r\n") {
            let header = std::str::from_utf8(&request[..end])
                .map_err(|_| DelegationErrorV1::ProtocolFailed)?;
            let mut first = header
                .lines()
                .next()
                .ok_or(DelegationErrorV1::ProtocolFailed)?
                .split_whitespace();
            let method = first.next().ok_or(DelegationErrorV1::ProtocolFailed)?;
            let request_target = first.next().ok_or(DelegationErrorV1::ProtocolFailed)?;
            if first.next() != Some("HTTP/1.1") || first.next().is_some() {
                return Err(DelegationErrorV1::ProtocolFailed);
            }
            let route = probe_route(endpoint.harness, method, request_target)
                .ok_or(DelegationErrorV1::ProtocolFailed)?;
            if route == ProbeRoute::Head {
                let index = endpoint.head_requests.fetch_add(1, Ordering::AcqRel) + 1;
                if index > MAX_HEAD_REQUESTS || end.saturating_add(4) > MAX_HEAD_BYTES {
                    return Err(DelegationErrorV1::ProtocolFailed);
                }
                stream
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .map_err(|_| DelegationErrorV1::ProtocolFailed)?;
                return Ok(());
            }
            let mut authorization = None;
            let mut api_key = None;
            let mut length = None;
            for line in header.lines().skip(1) {
                let (name, value) = line
                    .split_once(':')
                    .ok_or(DelegationErrorV1::ProtocolFailed)?;
                if name.eq_ignore_ascii_case("authorization")
                    && authorization.replace(value.trim()).is_some()
                {
                    return Err(DelegationErrorV1::ProtocolFailed);
                }
                if name.eq_ignore_ascii_case("x-api-key") && api_key.replace(value.trim()).is_some()
                {
                    return Err(DelegationErrorV1::ProtocolFailed);
                }
                if name.eq_ignore_ascii_case("content-length")
                    && length
                        .replace(
                            value
                                .trim()
                                .parse::<usize>()
                                .map_err(|_| DelegationErrorV1::ProtocolFailed)?,
                        )
                        .is_some()
                {
                    return Err(DelegationErrorV1::ProtocolFailed);
                }
            }
            let bearer_matches = authorization
                .and_then(|value| value.strip_prefix("Bearer "))
                .is_some_and(|value| value.as_bytes() == endpoint.token.as_slice());
            let api_key_matches =
                api_key.is_some_and(|value| value.as_bytes() == endpoint.token.as_slice());
            if authorization.is_some_and(|_| !bearer_matches)
                || api_key.is_some_and(|_| !api_key_matches)
                || (!bearer_matches && !api_key_matches)
            {
                return Err(DelegationErrorV1::ProtocolFailed);
            }
            break (
                end + 4,
                length
                    .filter(|value| *value <= MAX_REQUEST_BYTES)
                    .ok_or(DelegationErrorV1::ProtocolFailed)?,
                route,
            );
        }
        let mut chunk = [0_u8; 4096];
        let read = stream
            .read(&mut chunk)
            .map_err(|_| DelegationErrorV1::ProtocolFailed)?;
        if read == 0 {
            return Err(DelegationErrorV1::ProtocolFailed);
        }
        request.extend_from_slice(&chunk[..read]);
    };
    while request.len() < header_end.saturating_add(length) {
        if request.len() > MAX_REQUEST_BYTES || started.elapsed() > Duration::from_secs(3) {
            return Err(DelegationErrorV1::ProtocolFailed);
        }
        let mut chunk = [0_u8; 4096];
        let read = stream
            .read(&mut chunk)
            .map_err(|_| DelegationErrorV1::ProtocolFailed)?;
        if read == 0 {
            return Err(DelegationErrorV1::ProtocolFailed);
        }
        request.extend_from_slice(&chunk[..read]);
    }
    if serde_json::from_slice::<Value>(&request[header_end..header_end + length])
        .ok()
        .and_then(|body| body.get("model").and_then(Value::as_str).map(str::to_owned))
        .as_deref()
        != Some(endpoint.model)
    {
        return Err(DelegationErrorV1::ProtocolFailed);
    }
    if route == ProbeRoute::CountTokens {
        let response = r#"{"input_tokens":1}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
            response.len()
        )
        .map_err(|_| DelegationErrorV1::ProtocolFailed)?;
        return Ok(());
    }
    let index = endpoint.requests.fetch_add(1, Ordering::AcqRel) + 1;
    if index > MAX_MODEL_REQUESTS {
        return Err(DelegationErrorV1::ProtocolFailed);
    }
    let stall = endpoint.stall_next.swap(false, Ordering::AcqRel);
    if stall {
        let deadline = std::time::Instant::now() + Duration::from_secs(7);
        while !endpoint.release_stall.load(Ordering::Acquire)
            && std::time::Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        if !endpoint.release_stall.load(Ordering::Acquire) {
            return Err(DelegationErrorV1::DeadlineExceeded);
        }
    }
    let response = match endpoint.harness {
        WorkerHarnessV1::CodexCli => responses_stream(endpoint.model),
        WorkerHarnessV1::ClaudeCode => messages_stream(endpoint.model),
        WorkerHarnessV1::QoderCli => return Err(DelegationErrorV1::CapabilityUnavailable),
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
        response.len()
    )
    .map_err(|_| DelegationErrorV1::ProtocolFailed)
}

fn probe_route(harness: WorkerHarnessV1, method: &str, target: &str) -> Option<ProbeRoute> {
    if !target.starts_with('/') {
        return None;
    }
    let target = target.split('?').next()?;
    match (harness, method, target) {
        (WorkerHarnessV1::CodexCli, "POST", "/v1/responses")
        | (WorkerHarnessV1::ClaudeCode, "POST", "/v1/messages") => Some(ProbeRoute::Model),
        (WorkerHarnessV1::ClaudeCode, "POST", "/v1/messages/count_tokens") => {
            Some(ProbeRoute::CountTokens)
        }
        (WorkerHarnessV1::ClaudeCode, "HEAD", _) => Some(ProbeRoute::Head),
        _ => None,
    }
}

fn messages_stream(model: &str) -> String {
    [
        (
            "message_start",
            json!({
                "type":"message_start",
                "message":{
                    "id":"msg_worker_probe","type":"message","role":"assistant",
                    "content":[],"model":model,"stop_reason":null,"stop_sequence":null,
                    "usage":{"input_tokens":1,"output_tokens":1}
                }
            }),
        ),
        (
            "content_block_start",
            json!({
                "type":"content_block_start","index":0,
                "content_block":{"type":"text","text":""}
            }),
        ),
        (
            "content_block_delta",
            json!({
                "type":"content_block_delta","index":0,
                "delta":{"type":"text_delta","text":PROBE_TEXT}
            }),
        ),
        (
            "content_block_stop",
            json!({"type":"content_block_stop","index":0}),
        ),
        (
            "message_delta",
            json!({
                "type":"message_delta",
                "delta":{"stop_reason":"end_turn","stop_sequence":null},
                "usage":{"output_tokens":1}
            }),
        ),
        ("message_stop", json!({"type":"message_stop"})),
    ]
    .into_iter()
    .map(|(event, data)| format!("event: {event}\ndata: {data}\n\n"))
    .collect()
}

fn responses_stream(model: &str) -> String {
    let message = json!({
        "id":"msg_worker_probe", "type":"message", "status":"completed", "role":"assistant",
        "content":[{"type":"output_text", "text":PROBE_TEXT, "annotations":[]}]
    });
    [
        ("response.created", json!({"type":"response.created","sequence_number":0,"response":{"id":"resp_worker_probe","object":"response","created_at":1,"status":"in_progress","model":model,"output":[]}})),
        ("response.output_item.added", json!({"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{"id":"msg_worker_probe","type":"message","status":"in_progress","role":"assistant","content":[]}})),
        ("response.content_part.added", json!({"type":"response.content_part.added","sequence_number":2,"item_id":"msg_worker_probe","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}})),
        ("response.output_text.delta", json!({"type":"response.output_text.delta","sequence_number":3,"item_id":"msg_worker_probe","output_index":0,"content_index":0,"delta":PROBE_TEXT})),
        ("response.output_text.done", json!({"type":"response.output_text.done","sequence_number":4,"item_id":"msg_worker_probe","output_index":0,"content_index":0,"text":PROBE_TEXT})),
        ("response.content_part.done", json!({"type":"response.content_part.done","sequence_number":5,"item_id":"msg_worker_probe","output_index":0,"content_index":0,"part":{"type":"output_text","text":PROBE_TEXT,"annotations":[]}})),
        ("response.output_item.done", json!({"type":"response.output_item.done","sequence_number":6,"output_index":0,"item":message.clone()})),
        ("response.completed", json!({"type":"response.completed","sequence_number":7,"response":{"id":"resp_worker_probe","object":"response","created_at":1,"status":"completed","model":model,"output":[message],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}})),
    ]
    .into_iter()
    .map(|(event, data)| format!("event: {event}\ndata: {data}\n\n"))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_protocols_have_distinct_bounded_native_routes_and_valid_events() {
        assert_eq!(
            probe_route(WorkerHarnessV1::CodexCli, "POST", "/v1/responses"),
            Some(ProbeRoute::Model)
        );
        assert_eq!(
            probe_route(WorkerHarnessV1::CodexCli, "POST", "/v1/messages"),
            None
        );
        assert_eq!(
            probe_route(
                WorkerHarnessV1::ClaudeCode,
                "POST",
                "/v1/messages?beta=true"
            ),
            Some(ProbeRoute::Model)
        );
        assert_eq!(
            probe_route(
                WorkerHarnessV1::ClaudeCode,
                "POST",
                "/v1/messages/count_tokens?beta=true"
            ),
            Some(ProbeRoute::CountTokens)
        );
        assert_eq!(
            probe_route(WorkerHarnessV1::ClaudeCode, "HEAD", "/api/hello"),
            Some(ProbeRoute::Head)
        );
        assert_eq!(
            probe_route(WorkerHarnessV1::ClaudeCode, "HEAD", "relative"),
            None
        );
        assert_eq!(
            probe_route(
                WorkerHarnessV1::ClaudeCode,
                "HEAD",
                "/future/internal/probe?version=2"
            ),
            Some(ProbeRoute::Head)
        );
        assert_eq!(
            probe_route(WorkerHarnessV1::ClaudeCode, "POST", "/v1/responses"),
            None
        );
        for stream in [responses_stream(PROBE_TEXT), messages_stream(PROBE_TEXT)] {
            let data = stream
                .lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .collect::<Vec<_>>();
            assert!(!data.is_empty());
            assert!(
                data.iter()
                    .all(|value| serde_json::from_str::<Value>(value).is_ok())
            );
            assert!(stream.len() < MAX_REQUEST_BYTES);
        }
    }
}
