//! Authorized cleanup and contextual-history non-authority are separate contracts.
use super::*;

// Free-first routing retains its existing serial ContextHold recovery contract.
// Classified histories below require an actual causal edge between executions.
#[test]
fn free_first_context_break_cleans_once_and_reuses_only_the_old_prefix() {
    let simple = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 400,
            error_kind: None,
            body: PROTOCOL_ERROR,
        },
        ProviderReply::Complete {
            status: 400,
            error_kind: None,
            body: REASONING_ERROR,
        },
        complete(),
        complete(),
        complete(),
    ]);
    let complex = NativeProvider::start(vec![complete()]);
    let fixture = RuntimeFixture::launch(&[&simple, &complex], 3);
    assert_eq!(
        send(
            &fixture,
            "cleanup",
            vec![message("user", "complex-route initial work")]
        )
        .status,
        200
    );
    let old = json!({"type":"reasoning","summary":[],"encrypted_content":"old-state"});
    let mut history = vec![old, message("user", "rename after compaction")];
    assert_eq!(send(&fixture, "cleanup", history.clone()).status, 200);
    assert_eq!((simple.calls(), complex.calls()), (3, 1));
    let requests = simple.requests();
    assert_eq!(
        request_json_body(&requests[1])["input"][0]["encrypted_content"],
        "old-state"
    );
    assert_eq!(
        request_json_body(&requests[2])["input"],
        json!([message("user", "rename after compaction")])
    );
    history.push(json!({"type":"reasoning","summary":[],"encrypted_content":"new-model-state"}));
    history.push(message("assistant", "ok"));
    history.push(message("user", "rename the label"));
    assert_eq!(send(&fixture, "cleanup", history.clone()).status, 200);
    let projected = request_json_body(&simple.requests()[3]);
    let wire = serde_json::to_string(&projected).unwrap();
    assert!(!wire.contains("old-state"));
    assert!(wire.contains("new-model-state"));
    // An instruction change invalidates the strong candidate hint, not an
    // already successful cleanup prefix in otherwise continuous history.
    let response = send_document(
        &fixture,
        "cleanup",
        json!({
            "model":MODEL, "stream":false, "instructions":"Keep answers concise",
            "input":history,
        }),
    );
    assert_eq!(response.status, 200);
    let projected = request_json_body(&simple.requests()[4]);
    assert_eq!(
        projected["input"],
        request_json_body(&simple.requests()[3])["input"]
    );
    assert_eq!((simple.calls(), complex.calls()), (5, 1));
}

#[test]
fn free_first_failed_cleaned_retry_returns_to_original_without_mutating_its_history() {
    let simple = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 400,
            error_kind: None,
            body: PROTOCOL_ERROR,
        },
        ProviderReply::Complete {
            status: 400,
            error_kind: None,
            body: REASONING_ERROR,
        },
        ProviderReply::Complete {
            status: 400,
            error_kind: None,
            body: REASONING_ERROR,
        },
        ProviderReply::Complete {
            status: 400,
            error_kind: None,
            body: REASONING_ERROR,
        },
    ]);
    let complex = NativeProvider::start(vec![complete(), complete(), complete(), complete()]);
    let fixture = RuntimeFixture::launch(&[&simple, &complex], 3);
    assert_eq!(
        send(
            &fixture,
            "cleanup-fallback",
            vec![message("user", "complex-route initial")]
        )
        .status,
        200
    );
    let history = vec![
        json!({"type":"reasoning","summary":[],"encrypted_content":"keep-for-original"}),
        message("user", "rename after compaction"),
    ];
    assert_eq!(
        send(&fixture, "cleanup-fallback", history.clone()).status,
        200
    );
    assert_eq!((simple.calls(), complex.calls()), (3, 2));
    assert_eq!(
        request_json_body(&complex.requests()[1])["input"],
        json!(history)
    );
    assert_eq!(
        send(&fixture, "cleanup-fallback", history.clone()).status,
        200
    );
    // Free-first replay holds the actually delivered fallback. A failed
    // cleanup never commits, so that fallback still gets the original history.
    assert_eq!((simple.calls(), complex.calls()), (3, 3));
    assert_eq!(
        request_json_body(&complex.requests()[2])["input"],
        json!(history)
    );
    // Changing instructions invalidates the free-first hold. The continuous
    // message history does not grant another cleanup retry; neither candidate
    // may lose the original prefix.
    let mut next_turn = history;
    next_turn.push(message("assistant", "ok"));
    next_turn.push(message("user", "rename the label again"));
    assert_eq!(
        send_document(
            &fixture,
            "cleanup-fallback",
            json!({
                "model": MODEL, "stream": false,
                "instructions": "Use the new instruction", "input": next_turn,
            }),
        )
        .status,
        200
    );
    assert_eq!((simple.calls(), complex.calls()), (4, 4));
    for request in [&simple.requests()[3], &complex.requests()[3]] {
        assert_eq!(request_json_body(request)["input"], json!(next_turn));
    }
}

#[test]
fn classified_rewrite_cannot_borrow_a_contextual_heads_cleanup() {
    let simple = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 400,
            error_kind: None,
            body: REASONING_ERROR,
        },
        complete(),
    ]);
    let complex = NativeProvider::start(vec![complete(), complete()]);
    let fixture = RuntimeFixture::launch_classified(&[&simple, &complex], 3);
    assert_eq!(
        send(
            &fixture,
            "unproven-cleanup",
            vec![message("user", "complex-route original task")],
        )
        .status,
        200,
    );
    let opaque =
        json!({"type":"reasoning", "summary":[], "encrypted_content":"unowned-rewritten-state"});
    let history = vec![opaque, message("user", "rename in another history")];
    let response = send(&fixture, "unproven-cleanup", history.clone());
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body),
    );
    assert_eq!(
        (simple.calls(), complex.calls()),
        (1, 2),
        "a same-session contextual stage cannot authorize a cleaned retry",
    );
    for request in [&simple.requests()[0], &complex.requests()[1]] {
        assert_eq!(request_json_body(request)["input"], json!(history));
    }
    // The actual accepted output establishes this branch's edge. A later new
    // request may choose regular without dropping its opaque provider history.
    let mut next = history;
    next.push(message("assistant", "ok"));
    next.push(message("user", "rename the next label"));
    assert_eq!(send(&fixture, "unproven-cleanup", next.clone()).status, 200);
    assert_eq!((simple.calls(), complex.calls()), (2, 2));
    assert_eq!(
        request_json_body(&simple.requests()[1])["input"],
        json!(next),
    );
}
