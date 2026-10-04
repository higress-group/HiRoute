//! A deterministic Responses peer: native Skill discovery, exact user path/body, then one CLI.
//! Requests stay in memory and are never forwarded or included in diagnostics.
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub(super) struct Challenge {
    pub skill_name: String,
    pub skill_directory: String,
    pub skill_body: String,
    pub command: String,
    pub expected_cli: String,
    pub step: usize,
}

impl Challenge {
    pub fn complete(&self) -> bool {
        self.step == 3
    }

    fn reply(&mut self, body: &Value) -> Result<Value, &'static str> {
        let input = body["input"].as_array().ok_or("native input contract")?;
        let tool_declared = |name| {
            body["tools"]
                .as_array()
                .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == name))
        };
        let tool_result = |call: &str| {
            input
                .iter()
                .find(|item| item["type"] == "function_call_output" && item["call_id"] == call)
                .and_then(|item| item["output"].as_str())
        };
        let item = match self.step {
            0 => {
                if !body.to_string().contains(&self.skill_name)
                    || !tool_declared("Skill")
                    || !tool_declared("Bash")
                {
                    return Err("native Skill discovery");
                }
                function("skill", "Skill", json!({"skill": self.skill_name}))
            }
            1 => {
                if tool_result("call_hiroute_skill")
                    != Some(format!("Launching skill: {}", self.skill_name).as_str())
                {
                    return Err("native Skill call result");
                }
                let prefix = format!(
                    "Base directory for this skill: {}\n\n",
                    self.skill_directory
                );
                let loaded = input
                    .iter()
                    .filter(|item| item["type"] == "message" && item["role"] == "user")
                    .filter_map(|item| item["content"].as_array())
                    .flatten()
                    .filter_map(|item| item["text"].as_str())
                    .any(|text| {
                        text.strip_prefix(&prefix)
                            .is_some_and(|body| body.trim() == self.skill_body.trim())
                    });
                if !loaded {
                    return Err("actual native Skill path and contents");
                }
                function(
                    "cli",
                    "Bash",
                    json!({"command": self.command, "timeout": 10_000}),
                )
            }
            2 => {
                let output = tool_result("call_hiroute_cli").ok_or("trusted CLI call result")?;
                if output.trim() != self.expected_cli.trim() {
                    return Err("trusted CLI result mismatch");
                }
                json!({"id":"msg_hiroute_probe", "type":"message", "status":"completed", "role":"assistant", "content":[{"type":"output_text", "text":"OK", "annotations":[]}]})
            }
            _ => return Err("native request limit"),
        };
        self.step += 1;
        Ok(item)
    }
}

fn function(suffix: &str, name: &str, args: Value) -> Value {
    json!({"id":format!("fc_hiroute_{suffix}"), "type":"function_call", "status":"completed", "call_id":format!("call_hiroute_{suffix}"), "name":name, "arguments":args.to_string()})
}

pub(super) fn serve(
    stream: &mut TcpStream,
    token: &str,
    model: &str,
    challenge: &mut Challenge,
) -> Result<(), &'static str> {
    let body = read_request(stream, token, model)?;
    let item = challenge.reply(&body)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn challenge() -> Challenge {
        Challenge {
            skill_name: "hiroute-collaboration".into(),
            skill_directory: "/user/.qoder/skills/hiroute-collaboration".into(),
            skill_body: "confirmed Skill body".into(),
            command: "trusted cli".into(),
            expected_cli: "trusted result".into(),
            step: 1,
        }
    }

    #[test]
    fn installed_skill_requires_correlated_call_and_actual_user_source() {
        let body = |directory, call| {
            json!({"input":[
            {"type":"function_call_output","call_id":call,"output":"Launching skill: hiroute-collaboration"},
            {"type":"message","role":"user","content":[{"type":"input_text","text":format!("Base directory for this skill: {directory}\n\nconfirmed Skill body")}]}]})
        };
        assert!(
            challenge()
                .reply(&body(
                    "/project/.qoder/skills/hiroute-collaboration",
                    "call_hiroute_skill"
                ))
                .is_err()
        );
        assert!(
            challenge()
                .reply(&body(
                    "/user/.qoder/skills/hiroute-collaboration",
                    "unrelated_call"
                ))
                .is_err()
        );
        assert!(
            challenge()
                .reply(&body(
                    "/user/.qoder/skills/hiroute-collaboration",
                    "call_hiroute_skill"
                ))
                .is_ok()
        );
    }

    #[test]
    fn model_claim_cannot_replace_cli_tool_evidence() {
        let mut check = challenge();
        check.step = 2;
        assert!(check.reply(&json!({"input":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"trusted result"}]}]})).is_err());
    }
}
