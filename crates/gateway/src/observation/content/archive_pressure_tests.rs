//! Optional archive pressure must not move body capture onto the foreground.
use super::*;

#[test]
fn queued_archive_pressure_marks_optional_loss_without_synchronous_history_fallback() {
    let (foreground, background) = capture_under_pressure(false);
    assert_only_gap_before_encoder(&foreground);
    assert_eq!(
        background.len(),
        2,
        "failed capture must terminate explicitly"
    );
    assert_eq!(background[0]["phase"], "begin");
    assert_eq!(background[1]["phase"], "abort");
    assert_eq!(
        background[1]["abort_reason"],
        "content_capture_queue_unavailable"
    );
    assert!(background[0]["result_transcript_root"].is_null());
    assert_eq!(background[1]["completeness_delta"], "partial");
    assert!(
        background
            .iter()
            .all(|record| record["canonical_bytes_base64"].is_null())
    );
}

#[test]
#[should_panic(expected = "only the asynchronous gap heartbeat may precede the encoder")]
fn foreground_archive_oracle_rejects_synchronous_body_capture() {
    let (foreground, _) = capture_under_pressure(true);
    assert_only_gap_before_encoder(&foreground);
}

fn assert_only_gap_before_encoder(records: &[Value]) {
    assert_eq!(
        records.len(),
        1,
        "only the asynchronous gap heartbeat may precede the encoder"
    );
    let gap = &records[0];
    assert_eq!(
        gap["schema_version"],
        "hiroute.observation.gap-heartbeat/v1"
    );
    assert_eq!(gap["channel"], "conversation_content");
    assert_eq!(gap["sequence"], 1);
    assert_eq!(
        gap["loss_watermark"],
        json!({
            "first_sequence": 1,
            "last_sequence": 1,
            "reason": "queue_bytes_exceeded"
        })
    );
    assert_eq!(gap["completeness_delta"], "partial");
}

fn capture_under_pressure(synchronous_capture: bool) -> (Vec<Value>, Vec<Value>) {
    use crate::server::core_runtime::adapters::decode_ingress_request;
    let fixture = ReplayFixture::new(1024 * 1024);
    let text = "x".repeat(20 * 1024);
    let input = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"route","input":text}),
    )
    .unwrap();
    fixture.store.prevalidate(&[]).unwrap();
    let before = fixture.store.budget().snapshot().unwrap().live;
    let (sender, receiver) = mpsc::channel();
    let mut observation = request_with_sinks_capacity(
        OtelContentPolicy::Disabled,
        DiagnosticsPort::default(),
        true,
        GatewayObservationSinks {
            conversation_content: Arc::new(RecordSink(sender)),
            ..GatewayObservationSinks::discard()
        },
        512 * 1024,
    );
    let (worker, gate) = super::super::super::content_work::ContentWorkProducer::paused();
    Arc::get_mut(&mut observation.inner)
        .unwrap()
        .channels
        .content_work = worker;
    // Fill the current data allowance, leaving one descriptor quantum but
    // less room than this 20KiB snapshot. Controls may retain their independent
    // metadata; no queued/active payload may be invented for the refused input.
    let other_bytes = 512 * 1024
        - super::super::super::producer::ContentQueueLimits::for_capacity(512 * 1024).control
        - 4096;
    let _other = observation
        .inner
        .channels
        .content
        .reserve_work(other_bytes)
        .unwrap();
    observation.capture_request_async(&input, &fixture.store);
    assert_eq!(
        observation.inner.channels.content.work_state().unwrap().1,
        other_bytes
    );
    assert!(observation.inner.async_content.get().unwrap().failed());
    assert_eq!(fixture.store.budget().snapshot().unwrap().live, before);
    if synchronous_capture {
        // Negative control: execute the real synchronous body-capture path
        // while the asynchronous encoder is still paused.
        observation.capture_request(&input, &fixture.store);
    }
    // Drain only the independent record producer. Its loss heartbeat is legal
    // before the encoder runs; body events at this point are a regression.
    observation
        .inner
        .channels
        .content
        .flush(Duration::from_secs(5));
    let foreground = receiver.try_iter().collect();
    drop(_other);
    gate.release();
    observation.inner.channels.content_work.flush(&observation);
    observation
        .inner
        .channels
        .content
        .flush(Duration::from_secs(5));
    assert_eq!(
        observation.inner.channels.content.work_state().unwrap().1,
        0
    );
    (foreground, receiver.try_iter().collect())
}
