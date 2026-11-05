//! Bounded loopback Messages fixture for the explicit Claude compatibility check.
use super::collaboration::CollaborationChallenge;
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

const FRAME_TIMEOUT: Duration = Duration::from_secs(2);

fn read_before(
    stream: &mut TcpStream,
    deadline: Instant,
    chunk: &mut [u8],
) -> std::io::Result<usize> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(std::io::ErrorKind::TimedOut.into());
    }
    stream.set_read_timeout(Some(remaining))?;
    stream.read(chunk)
}

pub(super) fn serve(
    stream: &mut TcpStream,
    expected_auth: &str,
    model: &str,
    challenge: Option<&mut CollaborationChallenge>,
) -> Result<bool, &'static str> {
    stream
        .set_write_timeout(Some(Duration::from_millis(500)))
        .map_err(|_| "timeout")?;
    let deadline = Instant::now() + FRAME_TIMEOUT;
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    let (end, length, counting) = loop {
        if bytes.len() > 128 * 1024 || Instant::now() >= deadline {
            return Err("request bound");
        }
        if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
            let header = std::str::from_utf8(&bytes[..end]).map_err(|_| "encoding")?;
            let mut first = header.lines().next().ok_or("request")?.split_whitespace();
            let method = first.next().ok_or("method")?;
            let path = first
                .next()
                .ok_or("path")?
                .split('?')
                .next()
                .ok_or("path")?;
            // Readiness reads vary between native clients. They are not model evidence.
            if matches!(method, "HEAD" | "GET") {
                stream
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .map_err(|_| "hello response")?;
                return Ok(false);
            }
            if method != "POST" {
                return Err("method");
            }
            let counting = path == "/v1/messages/count_tokens";
            if !counting && path != "/v1/messages" {
                return Err("path");
            }
            let mut authorization = None;
            let mut length = None;
            for line in header.lines().skip(1) {
                let (key, value) = line.split_once(':').ok_or("header")?;
                let value = value.trim();
                if key.eq_ignore_ascii_case("authorization") {
                    let value = value.strip_prefix("Bearer ").ok_or("bearer")?;
                    if authorization.replace(value).is_some() {
                        return Err("duplicate auth");
                    }
                }
                if key.eq_ignore_ascii_case("content-length")
                    && length
                        .replace(value.parse::<usize>().map_err(|_| "length")?)
                        .is_some()
                {
                    return Err("duplicate length");
                }
            }
            if authorization != Some(expected_auth) {
                return Err("wrong authentication");
            }
            break (
                end + 4,
                length.filter(|size| *size <= 128 * 1024).ok_or("length")?,
                counting,
            );
        }
        let mut chunk = [0; 4096];
        let count = read_before(stream, deadline, &mut chunk).map_err(|_| "header read")?;
        if count == 0 {
            return Err("short header");
        }
        bytes.extend_from_slice(&chunk[..count]);
    };
    while bytes.len() < end + length {
        let mut chunk = [0; 4096];
        let count = read_before(stream, deadline, &mut chunk).map_err(|_| "body read")?;
        if count == 0 {
            return Err("short body");
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let request: serde_json::Value =
        serde_json::from_slice(&bytes[end..end + length]).map_err(|_| "json")?;
    if request["model"].as_str() != Some(model) {
        return Err("wrong model");
    }
    if counting {
        let body = "{\"input_tokens\":1}";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .map_err(|_| "response")?;
        return Ok(false);
    }
    let model_probe = request["stream"] == false && request["max_tokens"] == 1;
    let (content, stop_reason) = if model_probe {
        (json!({"type":"text", "text":"OK"}), "max_tokens")
    } else if let Some(challenge) = challenge {
        challenge.reply_claude(&request)?
    } else {
        (json!({"type":"text", "text":"OK"}), "end_turn")
    };
    if request["stream"] == false {
        let body = json!({"id":"msg_probe", "type":"message", "role":"assistant",
            "model":model, "content":[content], "stop_reason":stop_reason,
            "stop_sequence":null, "usage":{"input_tokens":1,"output_tokens":1}})
        .to_string();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
            .map_err(|_| "response")?;
        return Ok(!model_probe);
    }
    let (start_content, delta) = if content["type"] == "tool_use" {
        (
            json!({"type":"tool_use", "id":content["id"], "name":content["name"], "input":{}}),
            json!({"type":"input_json_delta", "partial_json":content["input"].to_string()}),
        )
    } else {
        (
            json!({"type":"text", "text":""}),
            json!({"type":"text_delta", "text":"OK"}),
        )
    };
    let events = [
        json!({"type":"message_start","message":{"id":"msg_probe","type":"message","role":"assistant","content":[],"model":model,"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}}),
        json!({"type":"content_block_start","index":0,"content_block":start_content}),
        json!({"type":"content_block_delta","index":0,"delta":delta}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":stop_reason,"stop_sequence":null},"usage":{"output_tokens":1}}),
        json!({"type":"message_stop"}),
    ];
    let body = events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().expect("fixture event type")
            )
        })
        .collect::<String>();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .map_err(|_| "response")?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange(request: &[u8]) -> (Result<bool, &'static str>, String) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            serve(&mut stream, "local-challenge", "frozen-model", None)
        });
        let mut client = TcpStream::connect(address).unwrap();
        client.write_all(request).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        (server.join().unwrap(), response)
    }

    #[test]
    fn readiness_reads_never_count_as_an_authenticated_model_request() {
        for target in ["HEAD /", "HEAD /api/hello", "GET /api/hello"] {
            let (evidence, response) =
                exchange(format!("{target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes());
            assert_eq!(evidence, Ok(false));
            assert!(response.starts_with("HTTP/1.1 404"));
        }
        let body = "{\"model\":\"frozen-model\"}";
        let (evidence, response) = exchange(format!("POST /v1/messages HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer local-challenge\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes());
        assert_eq!(evidence, Ok(true));
        assert!(response.starts_with("HTTP/1.1 200"));
        let (evidence, _) = exchange(format!("POST /v1/messages HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer other\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes());
        assert_eq!(evidence, Err("wrong authentication"));
    }

    #[test]
    fn nonstreaming_token_probes_receive_json_without_model_capability_evidence() {
        for (max_tokens, expected) in [(1, false), (32, true)] {
            let body = json!({"model":"frozen-model", "stream":false, "max_tokens":max_tokens})
                .to_string();
            let (evidence, response) = exchange(format!("POST /v1/messages HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer local-challenge\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes());
            assert_eq!(evidence, Ok(expected));
            assert!(response.contains("Content-Type: application/json\r\n"));
            let (_, body) = response.split_once("\r\n\r\n").unwrap();
            let body: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(body["type"], "message");
            assert_eq!(body["model"], "frozen-model");
        }
    }

    #[test]
    fn waits_for_a_fragmented_request_within_the_existing_frame_deadline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            serve(&mut stream, "local-challenge", "frozen-model", None)
        });
        let mut client = TcpStream::connect(address).unwrap();
        client.write_all(b"POST /v1/messages HTTP/1.1\r\n").unwrap();
        std::thread::sleep(Duration::from_millis(650));
        let body = r#"{"model":"frozen-model"}"#;
        write!(
            client,
            "Host: localhost\r\nAuthorization: Bearer local-challenge\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert_eq!(server.join().unwrap(), Ok(true));
        assert!(response.starts_with("HTTP/1.1 200"));
    }

    fn nonblocking_fragmented_exchange(delay_body: bool) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let (accepted, ready) = std::sync::mpsc::sync_channel(1);
        let (start, receive_start) = std::sync::mpsc::sync_channel(1);
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + FRAME_TIMEOUT;
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("accept probe connection: {error}"),
                }
            };
            // Explicitly retain the mode inherited on Windows, including when
            // this regression runs on a platform whose accept resets it.
            stream.set_nonblocking(true).unwrap();
            accepted.send(()).unwrap();
            receive_start.recv_timeout(FRAME_TIMEOUT).unwrap();
            serve(&mut stream, "local-challenge", "frozen-model", None)
        });
        let mut client = TcpStream::connect(address).unwrap();
        client.set_read_timeout(Some(FRAME_TIMEOUT)).unwrap();
        ready.recv_timeout(FRAME_TIMEOUT).unwrap();
        let body = br#"{"model":"frozen-model"}"#;
        let header = format!(
            "POST /v1/messages HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer local-challenge\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        if delay_body {
            client.write_all(header.as_bytes()).unwrap();
            client.write_all(&body[..1]).unwrap();
        }
        start.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(650));
        let sent = if delay_body {
            client.write_all(&body[1..])
        } else {
            client
                .write_all(header.as_bytes())
                .and_then(|()| client.write_all(body))
        };
        let mut response = Vec::new();
        let received = client.read_to_end(&mut response);
        assert_eq!(server.join().unwrap(), Ok(true));
        sent.unwrap();
        received.unwrap();
        assert!(response.starts_with(b"HTTP/1.1 200"));
    }

    #[test]
    fn nonblocking_connection_waits_for_delayed_headers() {
        nonblocking_fragmented_exchange(false);
    }

    #[test]
    fn nonblocking_connection_waits_for_delayed_body() {
        nonblocking_fragmented_exchange(true);
    }
}
