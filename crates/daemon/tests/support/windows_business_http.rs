//! Bounded independent loopback HTTP oracle/client. No HiRoute output is fabricated.
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const BOUND: usize = 4 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(15);

pub struct Request {
    pub method: String,
    pub path: String,
    authorization: Option<String>,
    headers: Vec<(String, String)>,
    pub body: Value,
}
impl Request {
    pub fn authenticated(&self, secret: &str) -> bool {
        self.authorization.as_deref() == Some(secret)
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}
pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}
impl Reply {
    pub fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: serde_json::to_vec(&body).unwrap(),
        }
    }
    pub fn sse(body: String) -> Self {
        Self {
            status: 200,
            content_type: "text/event-stream",
            body: body.into_bytes(),
        }
    }
}
pub struct Server {
    pub address: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Server {
    pub fn start(handler: impl FnMut(Request) -> Reply + Send + 'static) -> Self {
        Self::start_observed(handler, || {})
    }

    fn start_observed(
        mut handler: impl FnMut(Request) -> Reply + Send + 'static,
        mut accepted: impl FnMut() + Send + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let worker = thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // Winsock can inherit the listener's nonblocking mode. Read/write
                        // timeouts do not clear it: a real delayed request otherwise fails
                        // immediately with WSAEWOULDBLOCK instead of awaiting its bytes.
                        stream.set_nonblocking(false).unwrap();
                        stream.set_read_timeout(Some(TIMEOUT)).unwrap();
                        stream.set_write_timeout(Some(TIMEOUT)).unwrap();
                        accepted();
                        let request = read_request(&mut stream);
                        let reply = handler(request);
                        assert!(reply.body.len() <= BOUND);
                        super::assert_no_secret(&reply.body);
                        let reason = if reply.status == 200 {
                            "OK"
                        } else {
                            "Rejected"
                        };
                        write!(stream, "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", reply.status, reason, reply.content_type, reply.body.len()).unwrap();
                        stream.write_all(&reply.body).unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("loopback oracle accept failed: {error}"),
                }
            }
        });
        Self {
            address,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}
fn head(stream: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0];
        assert_eq!(
            stream.read(&mut byte).unwrap(),
            1,
            "HTTP closed before its header"
        );
        bytes.push(byte[0]);
        assert!(bytes.len() <= 65536, "HTTP header exceeds bound");
        if bytes.ends_with(b"\r\n\r\n") {
            return bytes;
        }
    }
}
fn read_request(stream: &mut TcpStream) -> Request {
    let bytes = head(stream);
    let text = std::str::from_utf8(&bytes).unwrap();
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap().split_whitespace();
    let method = first.next().unwrap().to_owned();
    let path = first.next().unwrap().to_owned();
    assert_eq!(first.next(), Some("HTTP/1.1"));
    let mut length = 0;
    let mut authorization = None;
    let mut headers = Vec::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').unwrap();
        headers.push((name.to_owned(), value.trim().to_owned()));
        if name.eq_ignore_ascii_case("content-length") {
            length = value.trim().parse::<usize>().unwrap();
        }
        if name.eq_ignore_ascii_case("authorization") {
            authorization = Some(value.trim().to_owned());
        }
        assert!(
            !name.eq_ignore_ascii_case("transfer-encoding"),
            "oracle requires a bounded content-length request"
        );
    }
    assert!(length <= BOUND);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).unwrap();
    let body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    Request {
        method,
        path,
        authorization,
        headers,
        body,
    }
}
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}
impl Response {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("actual HTTP JSON response")
    }
    pub fn events(&self) -> Vec<(String, Value)> {
        let text = std::str::from_utf8(&self.body)
            .unwrap()
            .replace("\r\n", "\n");
        text.split("\n\n")
            .filter(|event| !event.trim().is_empty())
            .filter_map(|event| {
                let name = event
                    .lines()
                    .find_map(|line| line.strip_prefix("event:").map(str::trim))
                    .unwrap_or("message");
                let data = event
                    .lines()
                    .filter_map(|line| line.strip_prefix("data:").map(str::trim))
                    .collect::<Vec<_>>()
                    .join("\n");
                if data.is_empty() || data == "[DONE]" {
                    None
                } else {
                    Some((
                        name.to_owned(),
                        serde_json::from_str(&data).expect("actual HTTP SSE data"),
                    ))
                }
            })
            .collect()
    }
}
pub fn post(address: SocketAddr, path: &str, token: Option<&str>, body: &Value) -> Response {
    post_with_headers(address, path, token, &[], body)
}
pub fn post_with_headers(
    address: SocketAddr,
    path: &str,
    token: Option<&str>,
    headers: &[(&str, &str)],
    body: &Value,
) -> Response {
    post_exchange(address, path, token, headers, body, false)
}

/// Keep the original large rejected body without racing an early auth response:
/// Windows may reset a socket closed while the peer is still writing that body.
pub fn post_expect_continue(
    address: SocketAddr,
    path: &str,
    token: Option<&str>,
    body: &Value,
) -> Response {
    post_expect_continue_with_headers(address, path, token, &[], body)
}

pub fn post_expect_continue_with_headers(
    address: SocketAddr,
    path: &str,
    token: Option<&str>,
    headers: &[(&str, &str)],
    body: &Value,
) -> Response {
    post_exchange(address, path, token, headers, body, true)
}

fn post_exchange(
    address: SocketAddr,
    path: &str,
    token: Option<&str>,
    headers: &[(&str, &str)],
    body: &Value,
    expect_continue: bool,
) -> Response {
    let mut stream = TcpStream::connect_timeout(&address, TIMEOUT).unwrap();
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    stream.set_write_timeout(Some(TIMEOUT)).unwrap();
    let raw = serde_json::to_vec(body).unwrap();
    super::assert_no_secret(&raw);
    assert!(raw.len() <= BOUND);
    write!(stream, "POST {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n", raw.len()).unwrap();
    if let Some(token) = token {
        assert!(!token.contains(['\r', '\n']));
        write!(stream, "Authorization: Bearer {token}\r\n").unwrap();
    }
    for (name, value) in headers {
        assert!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && !value.contains(['\r', '\n'])
                && ![
                    "authorization",
                    "content-length",
                    "transfer-encoding",
                    "host",
                    "connection"
                ]
                .iter()
                .any(|reserved| name.eq_ignore_ascii_case(reserved)),
            "test request must not override HTTP framing/authentication"
        );
        write!(stream, "{name}: {value}\r\n").unwrap();
    }
    if expect_continue {
        stream.write_all(b"Expect: 100-continue\r\n").unwrap();
    }
    stream.write_all(b"\r\n").unwrap();
    let early = expect_continue.then(|| head(&mut stream));
    let header = match early {
        Some(header) if !header.starts_with(b"HTTP/1.1 100 ") => header,
        _ => {
            stream.write_all(&raw).unwrap();
            head(&mut stream)
        }
    };
    let header = std::str::from_utf8(&header).unwrap();
    let status = header
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let mut body = Vec::new();
    stream
        .take((BOUND + 1) as u64)
        .read_to_end(&mut body)
        .unwrap();
    assert!(body.len() <= BOUND, "actual Gateway HTTP exceeds its bound");
    if header.lines().any(|line| {
        line.to_ascii_lowercase()
            .starts_with("transfer-encoding: chunked")
    }) {
        body = dechunk(&body);
    }
    super::assert_no_secret(&body);
    Response { status, body }
}
fn dechunk(raw: &[u8]) -> Vec<u8> {
    let mut position = 0;
    let mut result = Vec::new();
    loop {
        let end = raw[position..]
            .windows(2)
            .position(|bytes| bytes == b"\r\n")
            .unwrap()
            + position;
        let length = std::str::from_utf8(&raw[position..end])
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        let length = usize::from_str_radix(length, 16).unwrap();
        position = end + 2;
        if length == 0 {
            assert!(raw[position..].ends_with(b"\r\n"));
            return result;
        }
        assert!(length <= BOUND && position + length + 2 <= raw.len());
        result.extend_from_slice(&raw[position..position + length]);
        position += length;
        assert_eq!(&raw[position..position + 2], b"\r\n");
        position += 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::mpsc;
    use std::time::Instant;

    #[test]
    fn windows_loopback_oracle_reads_delayed_header_and_body_from_nonblocking_listener() {
        let (accepted, observed) = mpsc::channel();
        let (handled, received) = mpsc::channel();
        let server = Server::start_observed(
            move |request| {
                assert!(request.method == "POST" && request.path == "/bounded-native-oracle");
                assert!(request.authorization.is_none());
                assert!(request.body == json!({"receipt":"delayed-native-request"}));
                handled.send(()).unwrap();
                Reply::json(200, json!({"receipt":"delayed-native-response"}))
            },
            move || accepted.send(()).unwrap(),
        );
        let started = Instant::now();
        let mut client = TcpStream::connect_timeout(&server.address, TIMEOUT).unwrap();
        client.set_read_timeout(Some(TIMEOUT)).unwrap();
        client.set_write_timeout(Some(TIMEOUT)).unwrap();
        // The real server has accepted before any HTTP byte exists. A nonblocking
        // accepted stream cannot pass this phase by consuming an already queued header.
        observed.recv_timeout(TIMEOUT).unwrap();
        thread::sleep(Duration::from_millis(75));
        let body = br#"{"receipt":"delayed-native-request"}"#;
        write!(
            client,
            "POST /bounded-native-oracle HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\n",
            server.address,
            body.len()
        )
        .unwrap();
        thread::sleep(Duration::from_millis(75));
        client.write_all(b"Connection: close\r\n\r\n").unwrap();
        thread::sleep(Duration::from_millis(75));
        client.write_all(&body[..7]).unwrap();
        thread::sleep(Duration::from_millis(75));
        client.write_all(&body[7..]).unwrap();
        let header = head(&mut client);
        assert!(header.starts_with(b"HTTP/1.1 200 OK\r\n"));
        let mut response = Vec::new();
        client
            .take((BOUND + 1) as u64)
            .read_to_end(&mut response)
            .unwrap();
        assert!(response.len() <= BOUND);
        assert!(
            serde_json::from_slice::<Value>(&response).unwrap()
                == json!({"receipt":"delayed-native-response"})
        );
        received.recv_timeout(TIMEOUT).unwrap();
        assert!(
            started.elapsed() < TIMEOUT,
            "native HTTP fixture exceeded its bound"
        );
        drop(server);
    }
}
