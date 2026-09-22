//! Bounded loopback Responses challenge; no forwarding, tools, or upstream credentials.
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub(super) fn serve(stream: &mut TcpStream, token: &[u8], model: &str) -> Result<(), &'static str> {
    serve_challenge(stream, token, model, None)
}

pub(super) use super::super::collaboration_probe::CollaborationChallenge;

pub(super) fn serve_challenge(
    stream: &mut TcpStream,
    token: &[u8],
    model: &str,
    challenge: Option<&mut CollaborationChallenge>,
) -> Result<(), &'static str> {
    // The probe listener is nonblocking so the parent can keep its process deadline. Accepted
    // sockets inherit that mode on Unix, while this bounded request parser deliberately performs
    // synchronous reads with per-stream timeouts.
    stream.set_nonblocking(false).map_err(|_| "mode")?;
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|_| "timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_millis(500)))
        .map_err(|_| "timeout")?;
    let started = Instant::now();
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    let (header_end, length) = loop {
        if bytes.len() > 128 * 1024 || started.elapsed() > Duration::from_secs(2) {
            return Err("request bound");
        }
        if let Some(end) = bytes.windows(4).position(|slice| slice == b"\r\n\r\n") {
            let header = std::str::from_utf8(&bytes[..end]).map_err(|_| "header encoding")?;
            if header.lines().next() != Some("POST /v1/responses HTTP/1.1") {
                return Err("request path");
            }
            let mut auth = None;
            let mut length = None;
            for line in header.lines().skip(1) {
                let (name, value) = line.split_once(':').ok_or("header")?;
                if name.eq_ignore_ascii_case("authorization")
                    && auth.replace(value.trim()).is_some()
                {
                    return Err("duplicate auth");
                }
                if name.eq_ignore_ascii_case("content-length")
                    && length
                        .replace(value.trim().parse::<usize>().map_err(|_| "length")?)
                        .is_some()
                {
                    return Err("duplicate length");
                }
            }
            let auth = auth
                .and_then(|value| value.strip_prefix("Bearer "))
                .ok_or("missing bearer")?;
            if auth.as_bytes() != token {
                return Err("wrong bearer");
            }
            break (
                end + 4,
                length
                    .filter(|value| *value <= 128 * 1024)
                    .ok_or("length")?,
            );
        }
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).map_err(|_| "read header")?;
        if count == 0 {
            return Err("short header");
        }
        bytes.extend_from_slice(&chunk[..count]);
    };
    while bytes.len() < header_end + length {
        if started.elapsed() > Duration::from_secs(2) {
            return Err("body deadline");
        }
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).map_err(|_| "read body")?;
        if count == 0 {
            return Err("short body");
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let body: serde_json::Value = serde_json::from_slice(&bytes[header_end..header_end + length])
        .map_err(|_| "request json")?;
    if body.get("model").and_then(|value| value.as_str()) != Some(model) {
        return Err("wrong model");
    }
    let output = if let Some(challenge) = challenge {
        challenge.reply(&body)?
    } else {
        json!([{"id":"msg_probe", "type":"message", "status":"completed",
            "role":"assistant", "content":[{"type":"output_text", "text":"OK", "annotations":[]}]}])
    };
    let response = json!({"type":"response.completed", "response":{
        "id":"resp_native_probe", "object":"response", "created_at":1, "status":"completed",
        "model":model, "output":output,
        "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}
    }});
    let mut body = String::new();
    for (index, item) in output
        .as_array()
        .ok_or("response output")?
        .iter()
        .enumerate()
    {
        let event = json!({"type":"response.output_item.done", "output_index":index, "item":item});
        body.push_str(&format!(
            "event: response.output_item.done\ndata: {event}\n\n"
        ));
    }
    body.push_str(&format!("event: response.completed\ndata: {response}\n\n"));
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
        .map_err(|_| "response write")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loopback_challenge_rejects_wrong_auth_model_and_duplicate_auth() {
        for (headers, body) in [
            ("Authorization: Bearer wrong\r\n", r#"{"model":"probe"}"#),
            (
                "Authorization: Bearer challenge\r\n",
                r#"{"model":"wrong"}"#,
            ),
            (
                "Authorization: Bearer challenge\r\nAuthorization: Bearer challenge\r\n",
                r#"{"model":"probe"}"#,
            ),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let client = std::thread::spawn(move || {
                let mut client = TcpStream::connect(address).unwrap();
                write!(
                    client,
                    "POST /v1/responses HTTP/1.1\r\n{headers}Content-Length: {}\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            });
            let (mut stream, _) = listener.accept().unwrap();
            assert!(serve(&mut stream, b"challenge", "probe").is_err());
            client.join().unwrap();
        }
    }

    #[test]
    fn loopback_challenge_handles_connection_from_nonblocking_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let client = std::thread::spawn(move || {
            let mut client = TcpStream::connect(address).unwrap();
            let body = r#"{"model":"probe"}"#;
            write!(
                client,
                "POST /v1/responses HTTP/1.1\r\nAuthorization: Bearer challenge\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut response = Vec::new();
            client.read_to_end(&mut response).unwrap();
            assert!(response.starts_with(b"HTTP/1.1 200 OK"));
        });
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("accept connection: {error}"),
            }
        };
        let result = serve(&mut stream, b"challenge", "probe");
        drop(stream);
        client.join().unwrap();
        assert!(result.is_ok(), "server outcome: {result:?}");
    }
}
