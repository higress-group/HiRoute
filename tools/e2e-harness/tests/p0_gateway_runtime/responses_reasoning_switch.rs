//! Verify the actual Responses request body at each source in a routed session.
use super::native_encrypted_continuation::{configure_native_provider_state, message, streamed};
use super::*;
use serde_json::{Value, json};

#[test]
fn responses_reasoning_switch_preserves_portable_history_and_target_source_state() {
    let ciphertext = "synthetic-opaque-state-".repeat(4096);
    let encrypted = json!({
        "type": "reasoning", "id": "rs_openai", "summary": [],
        "encrypted_content": ciphertext, "status": "completed"
    });
    let reference = json!({
        "type": "reasoning", "id": "rs_qoder",
        "summary": [{"type": "summary_text", "text": "Synthetic summary."}],
        "encrypted_content": null, "status": "completed"
    });
    let tool = json!({
        "type": "function_call", "id": "fc_lookup", "call_id": "lookup-call",
        "name": "lookup", "arguments": "{}", "status": "completed"
    });
    let gpt = NativeProvider::start(vec![
        streamed("gpt-tool", vec![encrypted.clone(), tool]),
        streamed(
            "gpt-answer",
            vec![message("msg_a", "final_answer", "Found it.")],
        ),
        streamed(
            "gpt-return",
            vec![message("msg_c", "final_answer", "Done.")],
        ),
        streamed(
            "gpt-compact",
            vec![message("msg_summary", "final_answer", "Compacted.")],
        ),
    ]);
    let kimi = NativeProvider::start(vec![
        streamed(
            "kimi-answer",
            vec![reference.clone(), message("msg_b", "final_answer", "Hi.")],
        ),
        streamed(
            "kimi-cold",
            vec![message("msg_d", "final_answer", "Hi again.")],
        ),
    ]);
    let classifier = NativeProvider::start(
        [
            CLASSIFIER_COMPLEX,
            CLASSIFIER_COMPLEX,
            CLASSIFIER_SIMPLE,
            CLASSIFIER_COMPLEX,
            CLASSIFIER_SIMPLE,
        ]
        .into_iter()
        .map(|body| ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body,
        })
        .collect(),
    );
    let fixture =
        RuntimeFixture::launch_rest_classified_with_native_control(&[&kimi, &gpt, &classifier], 1);
    let mut publication = fixture.publication();
    configure_native_provider_state(&mut publication);
    publication.publication_revision += 1;
    fixture.publish(&mut publication);

    let session = "same-responses-reasoning-session";
    let mut input = vec![json!({"role": "user", "content": "Find the record."})];
    input.extend(send(&fixture, session, "main", "turn-a", &input));
    assert_eq!(input[1], encrypted);
    input
        .push(json!({"type": "function_call_output", "call_id": "lookup-call", "output": "found"}));
    // Select GPT again after the tool result so this assertion exercises native
    // replay to the same source independently of classifier inheritance.
    input.push(json!({"role": "user", "content": "Verify the record before reporting back."}));
    input.extend(send(&fixture, session, "main", "turn-a2", &input));
    assert_eq!((gpt.calls(), kimi.calls(), classifier.calls()), (2, 0, 2));
    assert_eq!(upstream_input(&gpt, 1)[1], encrypted);

    input.push(json!({"role": "user", "content": "Say hello."}));
    let kimi_output = send(&fixture, session, "main", "turn-b", &input);
    assert_eq!((gpt.calls(), kimi.calls(), classifier.calls()), (2, 1, 3));
    assert_eq!(kimi_output[0], reference);
    assert_eq!(
        upstream_input(&kimi, 0),
        excluding(&input, "rs_openai"),
        "GPT ciphertext must stay at its source, with text and tool history preserved"
    );

    input.extend(kimi_output);
    input.push(json!({"role": "user", "content": "Solve the next complex task."}));
    let gpt_output = send(&fixture, session, "main", "turn-c", &input);
    assert_eq!((gpt.calls(), kimi.calls(), classifier.calls()), (3, 1, 4));
    let mut expected = input.clone();
    expected
        .iter_mut()
        .find(|item| item["id"] == "rs_qoder")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("id");
    assert_eq!(
        upstream_input(&gpt, 2),
        json!(&expected),
        "Kimi reference must be removed while preserving its summary and earlier GPT state"
    );

    let summary_request = json!({"role": "user", "content": "Summarize the conversation."});
    let mut compact_input = input.clone();
    compact_input.extend(gpt_output.iter().cloned());
    compact_input.push(summary_request.clone());
    send(&fixture, session, "compaction", "turn-c", &compact_input);
    expected.extend(gpt_output);
    expected.push(summary_request);
    assert_eq!(upstream_input(&gpt, 3), json!(expected));
    assert_eq!((gpt.calls(), kimi.calls(), classifier.calls()), (4, 1, 4));

    send(&fixture, "unrelated-session", "main", "turn-c", &input);
    assert_eq!((gpt.calls(), kimi.calls(), classifier.calls()), (4, 2, 5));
    assert_eq!(upstream_input(&kimi, 1), json!(input));
}

fn send(
    fixture: &RuntimeFixture,
    session: &str,
    purpose: &str,
    turn: &str,
    input: &[Value],
) -> Vec<Value> {
    let response = send_wire(fixture, session, purpose, turn, input);
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert!(response.http_entity_complete);
    completed_output(&response.body)
}

fn send_wire(
    fixture: &RuntimeFixture,
    session: &str,
    purpose: &str,
    turn: &str,
    input: &[Value],
) -> WireResponse {
    let (headers, payload) = request_parts(session, purpose, turn, input);
    let headers = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    request(fixture.address, "POST", "/v1/responses", &headers, &payload)
}

fn request_parts(
    session: &str,
    purpose: &str,
    turn: &str,
    input: &[Value],
) -> (Vec<(String, String)>, Vec<u8>) {
    let headers = super::native_history::headers("codex", session, purpose, turn, "before");
    let payload = serde_json::to_vec(&json!({
        "model": MODEL, "stream": true, "store": false, "input": input,
        "include": ["reasoning.encrypted_content"],
        "tools": [{"type": "function", "name": "lookup", "parameters": {"type": "object"}}]
    }))
    .unwrap();
    (headers, payload)
}

fn completed_output(body: &[u8]) -> Vec<Value> {
    std::str::from_utf8(body)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|event| event["type"] == "response.completed")
        .expect("complete Responses SSE")["response"]["output"]
        .as_array()
        .unwrap()
        .clone()
}

fn upstream_input(provider: &NativeProvider, request_index: usize) -> Value {
    let requests = provider.requests();
    let request = &requests[request_index];
    let body_start = request
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .unwrap()
        + 4;
    let body: Value = serde_json::from_slice(&request[body_start..]).unwrap();
    body["input"].clone()
}

fn excluding(input: &[Value], id: &str) -> Value {
    Value::Array(
        input
            .iter()
            .filter(|item| item["id"] != id)
            .cloned()
            .collect(),
    )
}

#[cfg(all(unix, debug_assertions))]
#[test]
fn reasoning_replay_waits_for_exact_pending_delivery_and_ignores_failed_write() {
    let encrypted = json!({"type":"reasoning","id":"rs_pending","summary":[],
        "encrypted_content":"synthetic-state","status":"completed"});
    let final_answer = message("answer", "final_answer", "Done.");
    let gpt = NativeProvider::start(vec![
        streamed("pending", vec![encrypted.clone(), final_answer.clone()]),
        streamed("failed", vec![encrypted.clone(), final_answer.clone()]),
    ]);
    let kimi = NativeProvider::start(vec![
        streamed("continued", vec![final_answer.clone()]),
        streamed("separate", vec![final_answer.clone()]),
    ]);
    let classifier = NativeProvider::start(
        [
            CLASSIFIER_COMPLEX,
            CLASSIFIER_SIMPLE,
            CLASSIFIER_COMPLEX,
            CLASSIFIER_SIMPLE,
        ]
        .into_iter()
        .map(|body| ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body,
        })
        .collect(),
    );
    let fixture =
        RuntimeFixture::launch_rest_classified_with_native_control(&[&kimi, &gpt, &classifier], 1);
    let mut publication = fixture.publication();
    configure_native_provider_state(&mut publication);
    publication.publication_revision += 1;
    fixture.publish(&mut publication);
    let observation = fixture.observation_root.as_ref().unwrap();
    let pause = observation.join("reasoning-delivery.pause");
    let waiting = observation.join("reasoning-delivery.waiting");
    std::fs::write(&pause, b"").unwrap();

    let original = json!({"role":"user","content":"First task."});
    let (headers, payload) = request_parts(
        "immediate-replay",
        "main",
        "first",
        std::slice::from_ref(&original),
    );
    let headers = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let body = read_chunked_eos(open_request(
        fixture.address,
        "POST",
        "/v1/responses",
        &headers,
        &payload,
    ));
    let output = completed_output(&body);
    assert_eq!(output[0], encrypted);
    let mut input = vec![original.clone()];
    input.extend(output);
    input.push(json!({"role":"user","content":"Continue with a different model."}));
    std::thread::scope(|threads| {
        let successor =
            threads.spawn(|| send(&fixture, "immediate-replay", "main", "second", &input));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !waiting.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let did_wait = waiting.exists();
        std::fs::remove_file(&pause).unwrap();
        assert!(
            did_wait,
            "the replay must see the exact in-flight terminal write"
        );
        assert_eq!(
            successor.join().unwrap().as_slice(),
            std::slice::from_ref(&final_answer)
        );
    });
    assert_eq!(upstream_input(&kimi, 0), excluding(&input, "rs_pending"));

    let fail = observation.join("response-final-write.fail");
    std::fs::write(&fail, b"").unwrap();
    let response = send_wire(
        &fixture,
        "failed-replay",
        "main",
        "first",
        std::slice::from_ref(&original),
    );
    assert!(!fail.exists());
    assert!(!response.http_entity_complete);
    send(&fixture, "failed-replay", "main", "second", &input);
    assert_eq!(upstream_input(&kimi, 1), json!(input));
    assert_eq!((gpt.calls(), kimi.calls(), classifier.calls()), (2, 2, 4));
}
