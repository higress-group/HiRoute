use super::*;
use std::io::Read;

#[test]
fn real_listener_cancel_after_delivered_tool_closes_interrupted_stage_and_recovers() {
    let prefix = b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"cancel-tool\",\"model\":\"runtime-native\",\"status\":\"in_progress\",\"output\":[]}}\n\nevent: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc-cancel\",\"call_id\":\"cancel-call\",\"name\":\"lookup\",\"arguments\":\"\",\"status\":\"in_progress\"}}\n\nevent: response.function_call_arguments.done\ndata: {\"type\":\"response.function_call_arguments.done\",\"output_index\":0,\"item_id\":\"fc-cancel\",\"arguments\":\"{}\"}\n\nevent: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc-cancel\",\"call_id\":\"cancel-call\",\"name\":\"lookup\",\"arguments\":\"{}\",\"status\":\"completed\"}}\n\n";
    let regular = NativeProvider::start(vec![ProviderReply::StreamDrip {
        status: 200,
        chunks: vec![
            prefix.to_vec(),
            b": delayed transport progress\n\n".to_vec(),
        ],
        interval: Duration::from_secs(2),
    }]);
    let primary = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let classifier = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CLASSIFIER_SIMPLE,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CLASSIFIER_COMPLEX,
        },
    ]);
    let fixture = RuntimeFixture::launch_rest_classified_with_observation(
        &[&regular, &primary, &classifier],
        1,
        ObservationFaults::healthy(),
    );
    let headers = [
        ("X-HiRoute-Token", TOKEN),
        ("session-id", "cancel-tool-recovery"),
    ];
    let mut client = open_request(
        fixture.address,
        "POST",
        "/v1/responses",
        &headers,
        br#"{"model":"runtime-model","input":"actual A: look up the record","stream":true}"#,
    );
    client
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut wire = Vec::new();
    let mut buffer = [0u8; 4096];
    while !String::from_utf8_lossy(&wire).contains("response.output_item.done") {
        let count = client.read(&mut buffer).unwrap();
        assert!(count > 0, "stream ended before accepted tool");
        wire.extend_from_slice(&buffer[..count]);
    }
    client.shutdown(std::net::Shutdown::Both).unwrap();
    drop(client);
    let facts = wait_execution_facts(&fixture, 1);
    let stage = facts
        .iter()
        .find(|e| e["fact"]["kind"] == "agent_turn_finished")
        .expect("cancellation must freeze the actual stage even after delivering a tool");
    assert_eq!(stage["fact"]["status"], "interrupted");
    assert_eq!(stage["fact"]["history_partial"], true);
    let page = super::observed_history::timeline(&facts);
    assert_eq!(page.turns.len(), 1);
    assert_eq!(page.turns[0].status, "interrupted");
    assert!(page.turns[0].assessment.is_none());
    let response=request(fixture.address,"POST","/v1/responses",&headers,
        br#"{"model":"runtime-model","input":"actual B: design an independent task","stream":false}"#);
    assert_eq!(response.status, 200);
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (1, 1, 2)
    );
    let facts = wait_execution_facts(&fixture, 2);
    let page = super::observed_history::timeline(&facts);
    assert_eq!(page.turns.len(), 2);
    assert_eq!(page.turns[1].status, "completed");
    assert_ne!(page.turns[0].agent_turn_id, page.turns[1].agent_turn_id);
}
