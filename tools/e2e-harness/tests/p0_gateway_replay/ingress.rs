use super::*;

#[test]
fn real_listener_spools_model_last_large_bodies_for_all_ingress_protocols() {
    let simple = NativeProvider::start(Vec::new());
    let primary = NativeProvider::start((0..9).map(|_| success()).collect());
    let classifier = NativeProvider::start(
        (0..9)
            .map(|_| ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body: br#"{"group":"primary"}"#,
            })
            .collect(),
    );
    let fixture = RuntimeFixture::launch_rest_classified_with_replay(
        &[&simple, &primary, &classifier],
        1,
        1024 * 1024,
        16 * 1024,
    );
    // Match a long Agent conversation: many small escaped messages and a
    // small current question, rather than one multi-MiB current user string.
    let segment = "history 路由 \\ \" [ ] { }\n".repeat(90);
    let history = (0..1_500)
        .map(|index| {
            serde_json::json!({
                "role": if index % 2 == 0 { "user" } else { "assistant" },
                "content": format!("{index}:{segment}"),
            })
        })
        .collect::<Vec<_>>();
    let expected_prefix = history
        .iter()
        .map(|message| message["content"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    for (path, token, field) in [
        ("/v1/responses", TOKEN, "input"),
        ("/v1/chat/completions", CHAT_TOKEN, "messages"),
        ("/v1/messages", MESSAGES_TOKEN, "messages"),
    ] {
        for position in 0..3 {
            let current = format!("current {path} {position}");
            let mut messages = history.clone();
            messages.push(serde_json::json!({"role":"user","content":current}));
            let encoded = serde_json::to_string(&messages).unwrap();
            assert!(encoded.len() > 3 * 1024 * 1024);
            let fields = if path == "/v1/messages" {
                format!(r#""max_tokens":8,"{field}":{encoded}"#)
            } else {
                format!(r#""{field}":{encoded}"#)
            };
            let body = match position {
                0 => format!(r#"{{"model":"runtime-model",{fields},"stream":false}}"#),
                1 => format!(r#"{{"stream":false,"model":"runtime-model",{fields}}}"#),
                _ => format!(r#"{{{fields},"stream":false,"\u006dodel":"runtime-model"}}"#),
            };
            let response = fragmented_request(&fixture, path, token, body.as_bytes())
                .unwrap_or_else(|error| panic!("{path}, position {position}: {error}"));
            assert!(
                response.starts_with(b"HTTP/1.1 200"),
                "{path}, position {position}: {}",
                String::from_utf8_lossy(&response)
            );
            let wire = primary
                .requests()
                .pop()
                .expect("Provider received the request");
            let document: serde_json::Value = serde_json::from_slice(http_body(&wire)).unwrap();
            let mut received = Vec::new();
            collect_text(&document["input"], &mut received);
            assert_eq!(received.len(), expected_prefix.len() + 1);
            assert!(
                received[..expected_prefix.len()] == expected_prefix,
                "every historical message must retain its text and order"
            );
            assert_eq!(received.last().unwrap(), &current);
            wait_replay_empty(&fixture.replay_root);
        }
    }
    assert_eq!(simple.calls(), 0);
    assert_eq!(primary.calls(), 9);
    assert_eq!(classifier.calls(), 9);
}

#[test]
fn real_listener_releases_spooled_prefix_after_rejection_timeout_and_disconnect() {
    let provider = NativeProvider::start(Vec::new());
    let fixture = RuntimeFixture::launch_with_replay(&[&provider], 1, 1024 * 1024, 16 * 1024);
    let prefix = format!(r#"{{"input":"{}""#, "x".repeat(3 * 1024 * 1024));
    let missing = format!("{prefix}}}");
    let response = fixture.request_body(missing.as_bytes());
    assert_eq!(response.status, 400);
    wait_replay_empty(&fixture.replay_root);

    let invalid = format!(r#"{prefix},"model":"runtime-model","broken":]}}"#);
    assert_eq!(fixture.request_body(invalid.as_bytes()).status, 413);
    wait_replay_empty(&fixture.replay_root);
    let duplicate = format!(r#"{prefix},"model":"runtime-model","model":"other"}}"#);
    assert_eq!(fixture.request_body(duplicate.as_bytes()).status, 400);
    wait_replay_empty(&fixture.replay_root);

    for disconnect in [true, false] {
        let mut stream = TcpStream::connect(fixture.address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(stream,
            "POST /v1/responses HTTP/1.1\r\nHost: {}\r\nX-HiRoute-Token: {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            fixture.address, prefix.len() + 100).unwrap();
        stream.write_all(prefix.as_bytes()).unwrap();
        stream.flush().unwrap();
        wait_replay_nonempty(&fixture.replay_root);
        if disconnect {
            drop(stream);
        } else {
            let mut response = Vec::new();
            stream.read_to_end(&mut response).unwrap();
            assert!(
                response.starts_with(b"HTTP/1.1 408"),
                "{}",
                String::from_utf8_lossy(&response)
            );
        }
        wait_replay_empty(&fixture.replay_root);
    }
    assert_eq!(
        provider.calls(),
        0,
        "invalid ingress must not start an upstream attempt"
    );
}

fn fragmented_request(
    fixture: &RuntimeFixture,
    path: &str,
    token: &str,
    body: &[u8],
) -> Result<Vec<u8>, String> {
    let mut stream = TcpStream::connect(fixture.address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(stream,
        "POST {path} HTTP/1.1\r\nHost: {}\r\nX-HiRoute-Token: {token}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        fixture.address).unwrap();
    let mut writer = std::io::BufWriter::with_capacity(32 * 1024, &mut stream);
    // Fragment the first/last 128 bytes one at a time, keeping bulk writes bounded.
    let leading = body.len().min(128);
    let trailing = body.len().saturating_sub(128).max(leading);
    for chunk in body[..leading]
        .chunks(1)
        .chain(body[leading..trailing].chunks(16 * 1024))
        .chain(body[trailing..].chunks(1))
    {
        write!(writer, "{:x}\r\n", chunk.len()).unwrap();
        writer.write_all(chunk).unwrap();
        writer.write_all(b"\r\n").unwrap();
    }
    writer.write_all(b"0\r\n\r\n").unwrap();
    writer.flush().unwrap();
    drop(writer);
    let mut response = Vec::new();
    if let Err(error) = stream.read_to_end(&mut response) {
        let stderr = fs::read_to_string(
            fixture
                .replay_root
                .parent()
                .unwrap()
                .join("hirouted-runtime.stderr"),
        )
        .unwrap_or_default();
        return Err(format!(
            "{path}: {error}; received {} bytes: {}; daemon: {}; replay={:?}",
            response.len(),
            String::from_utf8_lossy(&response[..response.len().min(1024)]),
            &stderr[stderr.len().saturating_sub(2000)..],
            replay_inventory(&fixture.replay_root),
        ));
    }
    Ok(response)
}

fn replay_inventory(root: &Path) -> Vec<(String, u64)> {
    let mut files = Vec::new();
    for owner in fs::read_dir(root).unwrap().flatten() {
        for file in fs::read_dir(owner.path()).unwrap().flatten() {
            if let Ok(metadata) = file.metadata() {
                files.push((
                    file.file_name().to_string_lossy().into_owned(),
                    metadata.len(),
                ));
            }
        }
    }
    files
}

fn collect_text(value: &serde_json::Value, output: &mut Vec<String>) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                collect_text(value, output);
            }
        }
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                if matches!(key.as_str(), "text" | "content") && value.is_string() {
                    output.push(value.as_str().unwrap().to_owned());
                } else {
                    collect_text(value, output);
                }
            }
        }
        _ => {}
    }
}
