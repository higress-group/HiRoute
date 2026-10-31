//! Optional observation ownership cannot displace model forwarding.
use super::*;

#[test]
fn queued_capture_keeps_large_attempt_preparation_within_the_existing_memory_budget() {
    use crate::server::core_runtime::adapters::{
        ReplacementEncoding, RequestedReplacement, prepare_replay_json_template,
        sequential_replay_body,
    };
    use hiroute_gateway_core::runtime::body::MemoryRole;

    for threshold in [1024 * 1024, 8 * 1024 * 1024] {
        let fixture = ReplayFixture::with_record_bytes(threshold, 16 * 1024);
        let text = "historical content\n".repeat(120_000);
        let input = production_request(&json!({"model":"route", "input":text}), &fixture.store, 0);
        fixture
            .store
            .prevalidate(&crate::content_ref::model_content_refs(&input))
            .unwrap();
        let ContentPart::Text { text: marker } = &input.messages[0].content[0] else {
            panic!("text request")
        };
        let reference = marker.content_ref().unwrap();
        let document = json!({"input":marker,"metadata":"s".repeat(384 * 1024)});
        let template = prepare_replay_json_template(
            &document,
            [RequestedReplacement {
                content: reference,
                encoding: ReplacementEncoding::JsonString,
            }],
        )
        .unwrap();
        let budget = fixture.store.budget().clone();
        // Represent already admitted IR/control state. Leave real room for
        // the sequential wire template, rather than enlarging the 8 MiB limit.
        let resident = budget.snapshot().unwrap().live;
        let required = budget
            .reserve(MemoryRole::ModelIrBacking, 23 * 1024 * 1024 / 4 - resident)
            .unwrap();
        let before = budget.snapshot().unwrap().live;
        let (observation, receiver, gate) = asynchronous_observation();
        observation.capture_request_async(&input, &fixture.store);
        assert!(observation.inner.async_content.get().is_some());
        assert!(receiver.try_recv().is_err(), "encoding must stay deferred");
        assert!(
            budget.snapshot().unwrap().live - before < 64 * 1024,
            "a shared Replay lease must not recharge backing or disk lengths"
        );
        let mut body = sequential_replay_body(template, fixture.store.clone(), &budget, 16 * 1024)
            .expect("queued optional capture must leave the admitted request forwardable");
        let mut wire = Vec::new();
        while let Some(chunk) = body.next_chunk().unwrap() {
            wire.extend_from_slice(chunk.bytes());
        }
        assert_eq!(
            serde_json::from_slice::<Value>(&wire).unwrap()["input"],
            text
        );
        drop(body);
        gate.release();
        observation.inner.channels.content_work.flush(&observation);
        let mut records = Vec::new();
        loop {
            let record = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_ne!(record["phase"], "abort", "{record}");
            let finished = record["phase"] == "finish";
            records.push(record);
            if finished {
                break;
            }
        }
        assert!(records.iter().all(|r| r["loss_watermark"].is_null()));
        assert_capture_ids(&observation, &records);
        let captured: Vec<u8> = records
            .iter()
            .filter(|r| r["phase"] == "append")
            .flat_map(|r| decoded_base64(r["canonical_bytes_base64"].as_str().unwrap()))
            .collect();
        assert_eq!(captured, text.as_bytes());
        drop((required, input, fixture.store));
        assert_eq!(budget.snapshot().unwrap().live, 0);
    }
}

#[test]
fn optional_capture_allocations_do_not_use_the_forwarding_stream_budget() {
    use crate::server::core_runtime::adapters::{
        decode_ingress_request, prepare_replay_json_template, sequential_replay_body,
    };
    use hiroute_gateway_core::runtime::body::MemoryRole;
    let fixture = ReplayFixture::new(1024 * 1024);
    fixture.store.prevalidate(&[]).unwrap();
    let text = "x".repeat(300 * 1024);
    let document = json!({"model":"route","input":text});
    let input = decode_ingress_request(IngressProtocol::Responses, &document).unwrap();
    let template = prepare_replay_json_template(&document, []).unwrap();
    let budget = fixture.store.budget().clone();
    let available = budget.available_bytes().unwrap();
    let required = budget
        .reserve(MemoryRole::ModelIrBacking, available - 400 * 1024)
        .unwrap();
    let before = budget.snapshot().unwrap().live;
    let (observation, receiver, gate) = asynchronous_observation();
    observation.capture_request_async(&input, &fixture.store);
    assert!(observation.inner.async_content.get().is_some());
    assert!(receiver.try_recv().is_err());
    assert_eq!(
        budget.snapshot().unwrap().live,
        before,
        "optional snapshot and lease allocations belong to the observation owner"
    );
    let mut body = sequential_replay_body(template, fixture.store.clone(), &budget, 16 * 1024)
        .expect("optional encoding must not displace admitted forwarding work");
    let mut wire = Vec::new();
    while let Some(chunk) = body.next_chunk().unwrap() {
        wire.extend_from_slice(chunk.bytes());
    }
    assert_eq!(serde_json::from_slice::<Value>(&wire).unwrap(), document);
    drop(body);
    gate.release();
    observation.inner.channels.content_work.flush(&observation);
    let mut records = Vec::new();
    loop {
        let record = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_ne!(record["phase"], "abort", "{record}");
        let finished = record["phase"] == "finish";
        records.push(record);
        if finished {
            break;
        }
    }
    assert!(records.iter().all(|r| r["loss_watermark"].is_null()));
    drop((required, input, fixture.store));
    assert_eq!(budget.snapshot().unwrap().live, 0);
}

#[test]
fn forwarding_budget_pressure_does_not_prevent_optional_async_capture() {
    use crate::server::core_runtime::adapters::decode_ingress_request;
    use hiroute_gateway_core::runtime::body::MemoryRole;
    let fixture = ReplayFixture::new(1024 * 1024);
    let input = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"route","input":"already owned inline content"}),
    )
    .unwrap();
    fixture.store.prevalidate(&[]).unwrap();
    let budget = fixture.store.budget();
    let guard = budget
        .reserve(
            MemoryRole::ModelIrBacking,
            budget.available_bytes().unwrap() - 64,
        )
        .unwrap();
    let before = budget.snapshot().unwrap().live;
    let (observation, receiver, gate) = asynchronous_observation();
    observation.capture_request_async(&input, &fixture.store);
    assert!(observation.inner.async_content.get().is_some());
    assert_eq!(budget.snapshot().unwrap().live, before);
    assert!(receiver.try_recv().is_err(), "capture must remain deferred");
    gate.release();
    observation.inner.channels.content_work.flush(&observation);
    loop {
        let record = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(record["loss_watermark"].is_null(), "{record}");
        assert_ne!(record["phase"], "abort");
        if record["phase"] == "finish" {
            break;
        }
    }
    drop(guard);
}
