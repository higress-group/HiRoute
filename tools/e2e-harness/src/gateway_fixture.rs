//! Feature-gated Gateway fixtures shared by the process E2E targets.

use std::io::{Error, ErrorKind, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub use hiroute_gateway::server::test_control::{
    E2E_ATTEMPT_TIMEOUT_MS_ENV, E2E_DIAL_CONFIG_ENV, E2E_DIAL_CONFIG_FILE, TestTlsListener,
    TestTlsStream, sealed_native_candidate, write_dial_config, write_dial_config_with_dns_failures,
};

/// Decode delivered HTTP chunk bytes without treating a disconnected prefix as
/// a complete entity. Model-protocol terminal evidence remains independent.
pub fn decode_chunked_response_body(wire: &[u8]) -> std::io::Result<(Vec<u8>, bool)> {
    crate::http::decode_chunked_prefix(wire)
        .map_err(|error| Error::new(ErrorKind::InvalidData, error.to_string()))
}

pub fn read_complete_http_request(
    stream: &mut TestTlsStream,
    timeout: Duration,
) -> std::io::Result<Vec<u8>> {
    read_http_request(stream, timeout, None)
}

pub fn read_http_request_with_body_barrier(
    stream: &mut TestTlsStream,
    timeout: Duration,
    body_prefix_bytes: usize,
    mut body_barrier: impl FnMut(),
) -> std::io::Result<Vec<u8>> {
    if body_prefix_bytes == 0 {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "provider body barrier prefix must be non-zero",
        ));
    }
    read_http_request(
        stream,
        timeout,
        Some((body_prefix_bytes, &mut body_barrier)),
    )
}

fn read_http_request(
    stream: &mut TestTlsStream,
    timeout: Duration,
    mut body_barrier: Option<(usize, &mut dyn FnMut())>,
) -> std::io::Result<Vec<u8>> {
    stream.set_read_timeout(Some(timeout))?;
    let mut wire = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut expected = None;
    let mut body_start = None;
    loop {
        let read = stream.read(&mut buffer).map_err(|error| {
            Error::new(
                error.kind(),
                format!(
                    "Provider request read failed after {} bytes (expected {expected:?})",
                    wire.len()
                ),
            )
        })?;
        if read == 0 {
            return Err(Error::new(
                ErrorKind::UnexpectedEof,
                "provider request ended before its declared body",
            ));
        }
        wire.extend_from_slice(&buffer[..read]);
        if expected.is_none()
            && let Some(split) = wire.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let head = String::from_utf8_lossy(&wire[..split]);
            if head.lines().any(|line| {
                line.split_once(':').is_some_and(|(name, value)| {
                    name.eq_ignore_ascii_case("expect")
                        && value.trim().eq_ignore_ascii_case("100-continue")
                })
            }) {
                stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
                stream.flush()?;
            }
            let content_length = head
                .lines()
                .find_map(|line| {
                    line.split_once(':').and_then(|(name, value)| {
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                })
                .ok_or_else(|| Error::new(ErrorKind::InvalidData, "missing Content-Length"))?;
            let start = split + 4;
            if body_barrier
                .as_ref()
                .is_some_and(|(prefix, _)| *prefix > content_length)
            {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "provider body barrier prefix exceeds Content-Length",
                ));
            }
            body_start = Some(start);
            expected = Some(start + content_length);
        }
        let should_wait = body_barrier.as_ref().is_some_and(|(prefix, _)| {
            body_start.is_some_and(|start| wire.len().saturating_sub(start) >= *prefix)
        });
        if should_wait {
            let (_, barrier) = body_barrier.take().expect("checked provider body barrier");
            barrier();
        }
        if expected.is_some_and(|expected| wire.len() >= expected) {
            return Ok(wire);
        }
    }
}

pub fn serve_one_tls_json_response(
    listener: TestTlsListener,
    expected_authorization: &'static str,
    body: Vec<u8>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let served = Arc::new(AtomicBool::new(false));
        let mut handlers = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !served.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let served = Arc::clone(&served);
                    let body = body.clone();
                    handlers.push(std::thread::spawn(move || {
                        stream.set_nonblocking(false).unwrap();
                        match read_complete_http_request(&mut stream, Duration::from_secs(5)) {
                            Ok(request) => {
                                assert_eq!(
                                    http_header(&request, "authorization").as_deref(),
                                    Some(expected_authorization)
                                );
                                assert!(
                                    !served.swap(true, Ordering::AcqRel),
                                    "Provider received an extra request"
                                );
                                write!(
                                    stream,
                                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                    body.len()
                                )
                                .unwrap();
                                stream.write_all(&body).unwrap();
                                stream.finish().unwrap();
                            }
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    ErrorKind::WouldBlock | ErrorKind::TimedOut
                                ) => {}
                            Err(error) => panic!("Provider request failed: {error}"),
                        }
                    }));
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "Provider was never invoked"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("Provider accept failed: {error}"),
            }
        }
        for handler in handlers {
            handler.join().unwrap();
        }
    })
}

fn http_header(wire: &[u8], expected_name: &str) -> Option<String> {
    let split = wire.windows(4).position(|window| window == b"\r\n\r\n")?;
    String::from_utf8_lossy(&wire[..split])
        .lines()
        .skip(1)
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case(expected_name)
                    .then(|| value.trim().to_owned())
            })
        })
}
