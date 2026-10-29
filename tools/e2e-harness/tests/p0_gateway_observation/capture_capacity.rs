use super::*;

#[test]
fn undersized_capture_queue_reports_loss_without_waiting_or_poisoning_the_next_request() {
    let provider = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: ACCEPTED_BODY,
        };
        2
    ]);
    let fixture = RuntimeFixture::launch_with_observation(
        &[&provider],
        1,
        ObservationFaults {
            lifecycle_mode: "slow_once:1000",
            queue_bytes: 512,
            ..ObservationFaults::healthy()
        },
    );
    let started = Instant::now();
    let response = fixture.request();
    assert_eq!(response.status, 200);
    assert_eq!(response_tool_logical_id(&response.body), "call-22008");
    assert!(started.elapsed() < Duration::from_millis(750));
    let root = fixture.observation_root.as_deref().unwrap();
    for file in ["lifecycle.jsonl", "conversation-content.jsonl"] {
        let records = wait_for_gap(root, file);
        assert!(
            records.iter().any(|record| {
                record
                    .pointer("/loss_watermark/reason")
                    .and_then(Value::as_str)
                    == Some("queue_bytes_exceeded")
            }),
            "{file}: {records:#?}"
        );
    }
    let content = read_records(&root.join("conversation-content.jsonl"));
    assert!(!content.iter().any(|record| {
        record["phase"] == "finish" && record["completeness_delta"] == "complete"
    }));
    assert_eq!(fixture.request().status, 200);
    assert_eq!(provider.calls(), 2);
}
