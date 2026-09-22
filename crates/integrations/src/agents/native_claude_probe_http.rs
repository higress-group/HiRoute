//! Bounded loopback Messages fixture for the explicit Claude compatibility check.
use super::collaboration::CollaborationChallenge;
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub(super) fn serve(
    stream: &mut TcpStream,
    expected_auth: &str,
    model: &str,
    challenge: Option<&mut CollaborationChallenge>,
) -> Result<bool, &'static str> {
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|_| "timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_millis(500)))
        .map_err(|_| "timeout")?;
    let started = Instant::now();
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    let (end, length, counting) = loop {
        if bytes.len() > 128 * 1024 || started.elapsed() > Duration::from_secs(2) {
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
            if method == "HEAD" && path == "/api/hello" {
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
        let count = stream.read(&mut chunk).map_err(|_| "header read")?;
        if count == 0 {
            return Err("short header");
        }
        bytes.extend_from_slice(&chunk[..count]);
    };
    while bytes.len() < end + length {
        if started.elapsed() > Duration::from_secs(2) {
            return Err("body deadline");
        }
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).map_err(|_| "body read")?;
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
    let (content, stop_reason) = if let Some(challenge) = challenge {
        challenge.reply_claude(&request)?
    } else {
        (json!({"type":"text", "text":"OK"}), "end_turn")
    };
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
