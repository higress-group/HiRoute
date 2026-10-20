//! Shared bounded Responses peer for explicit native collaboration checks.
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub(super) fn serve(
    stream: &mut TcpStream,
    token: &str,
    model: &str,
    reply: impl FnOnce(&Value) -> Result<Value, &'static str>,
) -> Result<(), &'static str> {
    let body = read_request(stream, token, model)?;
    let item = reply(&body)?;
    let response = json!({"id":"resp_hiroute_probe", "object":"response", "created_at":1, "status":"completed", "model":model, "output":[item], "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}});
    let mut started = response.clone();
    started["status"] = "in_progress".into();
    started["output"] = json!([]);
    let mut added = item.clone();
    added["status"] = "in_progress".into();
    let mut events = vec![("response.created", json!({"response":started}))];
    if item["type"] == "function_call" {
        // Qoder validates streaming argument consistency; adding the final arguments and then
        // appending a delta would duplicate them and fail before any tool executes.
        added["arguments"] = "".into();
        events.push((
            "response.output_item.added",
            json!({"output_index":0,"item":added}),
        ));
        events.push((
            "response.function_call_arguments.delta",
            json!({"output_index":0,"item_id":item["id"],"delta":item["arguments"]}),
        ));
        events.push((
            "response.function_call_arguments.done",
            json!({"output_index":0,"item_id":item["id"],"arguments":item["arguments"]}),
        ));
    } else {
        added["content"] = json!([]);
        events.push((
            "response.output_item.added",
            json!({"output_index":0,"item":added}),
        ));
        events.push((
            "response.output_text.delta",
            json!({"output_index":0,"content_index":0,"item_id":item["id"],"delta":"OK"}),
        ));
    }
    events.push((
        "response.output_item.done",
        json!({"output_index":0,"item":item}),
    ));
    events.push(("response.completed", json!({"response":response})));
    let mut payload = String::new();
    for (index, (kind, mut event)) in events.into_iter().enumerate() {
        event["type"] = kind.into();
        event["sequence_number"] = index.into();
        payload.push_str(&format!("event: {kind}\ndata: {event}\n\n"));
    }
    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",payload.len()).map_err(|_| "loopback response")
}

fn read_request(stream: &mut TcpStream, token: &str, model: &str) -> Result<Value, &'static str> {
    stream.set_nonblocking(false).map_err(|_| "socket mode")?;
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|_| "socket timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_millis(500)))
        .map_err(|_| "socket timeout")?;
    let started = Instant::now();
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    let (header_end, length) = loop {
        if bytes.len() > 128 * 1024 || started.elapsed() > Duration::from_secs(2) {
            return Err("request bound");
        }
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let header = std::str::from_utf8(&bytes[..end]).map_err(|_| "request header")?;
            if header.lines().next() != Some("POST /v1/responses HTTP/1.1") {
                return Err("request path");
            }
            let mut auth = None;
            let mut length = None;
            for line in header.lines().skip(1) {
                let (name, value) = line.split_once(':').ok_or("request header")?;
                if name.eq_ignore_ascii_case("authorization")
                    && auth.replace(value.trim()).is_some()
                {
                    return Err("duplicate auth");
                }
                if name.eq_ignore_ascii_case("content-length")
                    && length
                        .replace(
                            value
                                .trim()
                                .parse::<usize>()
                                .map_err(|_| "request length")?,
                        )
                        .is_some()
                {
                    return Err("duplicate length");
                }
                if name.eq_ignore_ascii_case("transfer-encoding") {
                    return Err("request framing");
                }
            }
            if auth.and_then(|value| value.strip_prefix("Bearer ")) != Some(token) {
                return Err("request credential");
            }
            break (
                end + 4,
                length
                    .filter(|value| *value <= 128 * 1024)
                    .ok_or("request length")?,
            );
        }
        let mut chunk = [0; 4096];
        let size = stream.read(&mut chunk).map_err(|_| "request header read")?;
        if size == 0 {
            return Err("short request");
        }
        bytes.extend_from_slice(&chunk[..size]);
    };
    while bytes.len() < header_end + length {
        if started.elapsed() > Duration::from_secs(2) {
            return Err("request deadline");
        }
        let mut chunk = [0; 4096];
        let size = stream.read(&mut chunk).map_err(|_| "request body read")?;
        if size == 0 {
            return Err("short request");
        }
        bytes.extend_from_slice(&chunk[..size]);
    }
    let body: Value = serde_json::from_slice(&bytes[header_end..header_end + length])
        .map_err(|_| "request json")?;
    if body["model"] != model {
        return Err("request model");
    }
    Ok(body)
}
