use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct WireResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub http_entity_complete: bool,
}

pub fn request(
    address: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> WireResponse {
    request_with_timeout(
        address,
        method,
        path,
        headers,
        body,
        DEFAULT_RESPONSE_TIMEOUT,
    )
}

pub fn open_request(
    address: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> TcpStream {
    let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).unwrap();
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
    stream
}

pub fn request_with_timeout(
    address: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    response_timeout: Duration,
) -> WireResponse {
    try_request_with_timeout(address, method, path, headers, body, response_timeout)
        .unwrap_or_else(|error| panic!("{error}"))
}

pub(super) fn try_request_with_timeout(
    address: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    response_timeout: Duration,
) -> Result<WireResponse, String> {
    if response_timeout.is_zero() {
        return Err(format!("{method} {path} has a zero response timeout"));
    }
    let started = Instant::now();
    let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT.min(response_timeout))
        .map_err(|error| format!("{method} {path} connect to {address} failed: {error}"))?;
    stream
        .set_read_timeout(Some(response_timeout))
        .map_err(|error| format!("{method} {path} could not set read timeout: {error}"))?;
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream
        .write_all(head.as_bytes())
        .map_err(|error| format!("{method} {path} request head write failed: {error}"))?;
    write_body_or_expected_rejection(&mut stream, body)?;
    read_response(stream, method, path, address, response_timeout, started)
}

pub fn wire_header(wire: &[u8], expected: &str) -> Option<String> {
    let split = wire.windows(4).position(|window| window == b"\r\n\r\n")?;
    std::str::from_utf8(&wire[..split])
        .ok()?
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case(expected))
        .map(|(_, value)| value.trim().to_owned())
}

fn write_body_or_expected_rejection(stream: &mut TcpStream, body: &[u8]) -> Result<(), String> {
    let allowed = |result: std::io::Result<()>| match result {
        Ok(()) => true,
        Err(error) => matches!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
        ),
    };
    if !allowed(stream.write_all(body)) {
        return Err("request body write failed before an expected early rejection".into());
    }
    if !allowed(stream.flush()) {
        return Err("request body flush failed before an expected early rejection".into());
    }
    Ok(())
}

fn read_response(
    mut stream: TcpStream,
    method: &str,
    path: &str,
    address: SocketAddr,
    response_timeout: Duration,
    started: Instant,
) -> Result<WireResponse, String> {
    let mut wire = Vec::new();
    let read_error = stream.read_to_end(&mut wire).err();
    let context = || {
        format!(
            "{method} {path} via {address} after {:?} (timeout {:?}, {} bytes)",
            started.elapsed(),
            response_timeout,
            wire.len()
        )
    };
    let split = wire
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            let detail = read_error.as_ref().map_or_else(
                || "connection closed".to_owned(),
                |error| format!("read failed with {:?}: {error}", error.kind()),
            );
            format!(
                "response has no HTTP head for {}: {detail}; partial={:?}",
                context(),
                String::from_utf8_lossy(&wire)
            )
        })?;
    let head = std::str::from_utf8(&wire[..split])
        .map_err(|error| format!("response head is not UTF-8 for {}: {error}", context()))?;
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .ok_or_else(|| format!("response status is missing for {}", context()))?
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| format!("response status is malformed for {}", context()))?
        .parse()
        .map_err(|error| format!("response status is invalid for {}: {error}", context()))?;
    let headers = lines
        .map(|line| {
            line.split_once(':')
                .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
                .ok_or_else(|| format!("malformed response header for {}: {line:?}", context()))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let body = &wire[split + 4..];
    if let Some(declared) = headers
        .get("content-length")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|error| format!("invalid Content-Length for {}: {error}", context()))?
        && body.len() != declared
    {
        let read_detail = read_error.as_ref().map_or_else(
            || "connection closed".to_owned(),
            |error| format!("read failed with {:?}: {error}", error.kind()),
        );
        return Err(format!(
            "incomplete response body for {}: declared {declared}, received {}; {read_detail}",
            context(),
            body.len()
        ));
    }
    if let Some(error) = &read_error
        && !matches!(error.kind(), ErrorKind::ConnectionReset)
    {
        return Err(format!(
            "response read failed for {} with {:?}: {error}",
            context(),
            error.kind()
        ));
    }
    let (body, http_entity_complete) = if headers
        .get("transfer-encoding")
        .is_some_and(|value| value.eq_ignore_ascii_case("chunked"))
    {
        hiroute_e2e::gateway_fixture::decode_chunked_response_body(body)
            .map_err(|error| format!("invalid chunked response for {}: {error}", context()))?
    } else {
        (body.to_vec(), read_error.is_none())
    };
    Ok(WireResponse {
        status,
        headers,
        body,
        http_entity_complete,
    })
}
