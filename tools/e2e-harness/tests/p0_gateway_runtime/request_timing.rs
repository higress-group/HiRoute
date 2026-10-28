use super::*;
use serde_json::Value;

fn summaries(fixture: &RuntimeFixture, expected: usize) -> Vec<Value> {
    let path = fixture
        .diagnostics_root
        .as_ref()
        .unwrap()
        .join("daemon/current.jsonl");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let log = std::fs::read_to_string(&path).unwrap();
        let summaries: Vec<_> = log
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|record| record["event"].get("request_timing_summary").is_some())
            .collect();
        if summaries.len() >= expected {
            assert_eq!(summaries.len(), expected, "{log}");
            return summaries;
        }
        assert!(Instant::now() < deadline, "missing request summary: {log}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn assert_partition(record: &Value) {
    assert_eq!(record["level"], "info");
    let summary = &record["event"]["request_timing_summary"];
    let accounted = ["local", "wait"]
        .into_iter()
        .flat_map(|key| summary[key].as_object().unwrap().values())
        .map(|value| value.as_u64().unwrap())
        .sum::<u64>();
    assert_eq!(
        accounted,
        summary["elapsed_us"].as_u64().unwrap(),
        "{summary}"
    );
    assert!(serde_json::to_vec(record).unwrap().len() < 4096);
}

#[test]
fn info_request_timing_covers_all_ingress_protocols_large_replay_and_rejections() {
    let provider = NativeProvider::start(
        (0..3)
            .map(|_| ProviderReply::DelayedComplete {
                duration: Duration::from_millis(20),
                status: 200,
                error_kind: None,
                body: RESPONSES_OK,
            })
            .collect(),
    );
    let fixture = RuntimeFixture::launch_with_timing_diagnostics(&[&provider], 1, None);
    let large = serde_json::to_vec(&serde_json::json!({"model":MODEL,"input":"private-history-marker".repeat(60_000),"stream":false})).unwrap();
    for (index, (path, token, body)) in [
        ("/v1/responses", TOKEN, large.as_slice()),
        ("/v1/chat/completions", CHAT_TOKEN, br#"{"model":"runtime-model","messages":[{"role":"user","content":"hello"}],"stream":false}"#.as_slice()),
        ("/v1/messages", MESSAGES_TOKEN, br#"{"model":"runtime-model","messages":[{"role":"user","content":"hello"}],"max_tokens":64,"stream":false}"#.as_slice()),
    ].into_iter().enumerate() {
        let response = request(fixture.address, "POST", path, &[("X-HiRoute-Token", token)], body);
        assert_eq!(response.status, 200, "{}", String::from_utf8_lossy(&response.body));
        let records = summaries(&fixture, index + 1);
        let record = records.last().unwrap();
        assert_partition(record);
        let summary = &record["event"]["request_timing_summary"];
        assert_eq!(summary["outcome"], "completed");
        assert!(summary["request_token"].is_string());
        assert!(summary["wait"]["upstream_read_us"].as_u64().unwrap() >= 20_000);
        assert_eq!(summary["asynchronous"]["records"], 0);
    }
    let rejected = request(
        fixture.address,
        "POST",
        "/v1/responses",
        &[("X-HiRoute-Token", "private-invalid-token")],
        br#"{}"#,
    );
    assert_eq!(rejected.status, 401);
    let malformed = fixture.request_body(br#"{"model":"runtime-model","input":["#);
    assert_eq!(malformed.status, 400);
    let records = summaries(&fixture, 5);
    for record in &records {
        assert_partition(record);
    }
    for record in &records[3..] {
        assert_eq!(
            record["event"]["request_timing_summary"]["outcome"],
            "failed"
        );
    }
    let log = std::fs::read_to_string(
        fixture
            .diagnostics_root
            .as_ref()
            .unwrap()
            .join("daemon/current.jsonl"),
    )
    .unwrap();
    assert!(!log.contains("private-history-marker"));
    assert!(!log.contains("private-invalid-token"));
    assert_eq!(provider.calls(), 3);
}

#[test]
fn info_request_timing_is_single_across_fallback_and_separates_async_sink_work() {
    let primary = NativeProvider::start(vec![ProviderReply::Complete {
        status: 429,
        error_kind: None,
        body: QUOTA_ERROR,
    }]);
    let fallback = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: FALLBACK_OK,
    }]);
    let fixture = RuntimeFixture::launch_with_timing_diagnostics(
        &[&primary, &fallback],
        2,
        Some(ObservationFaults {
            execution_mode: "slow_once:500",
            ..ObservationFaults::healthy()
        }),
    );
    let response = fixture.request();
    assert_eq!(response.status, 200);
    let records = summaries(&fixture, 1);
    assert_partition(&records[0]);
    let summary = &records[0]["event"]["request_timing_summary"];
    assert_eq!(summary["outcome"], "completed");
    assert!(summary["asynchronous"]["records"].as_u64().unwrap() > 0);
    assert_eq!(
        summary["asynchronous"]["records"],
        summary["asynchronous"]["delivered"]
    );
    assert_eq!(summary["asynchronous"]["dropped"], 0);
    assert!(summary["asynchronous"]["sink_work_us"].as_u64().unwrap() >= 500_000);
    let log = std::fs::read_to_string(
        fixture
            .diagnostics_root
            .as_ref()
            .unwrap()
            .join("daemon/current.jsonl"),
    )
    .unwrap();
    let begin = log
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find_map(|record| record["event"].get("request_begin").cloned())
        .unwrap();
    assert_eq!(begin["request_token"], summary["request_token"]);
    assert_eq!((primary.calls(), fallback.calls()), (1, 1));
}

#[test]
fn info_request_timing_closes_when_downstream_cancels() {
    let provider = NativeProvider::start(vec![ProviderReply::Stall {
        duration: Duration::from_millis(300),
    }]);
    let fixture = RuntimeFixture::launch_with_timing_diagnostics(&[&provider], 1, None);
    let downstream = open_request(
        fixture.address,
        "POST",
        "/v1/responses",
        &[("X-HiRoute-Token", TOKEN)],
        br#"{"model":"runtime-model","input":"cancel","stream":false}"#,
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while provider.calls() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(provider.calls(), 1);
    downstream.shutdown(std::net::Shutdown::Both).unwrap();
    drop(downstream);
    let records = summaries(&fixture, 1);
    assert_partition(&records[0]);
    assert_eq!(
        records[0]["event"]["request_timing_summary"]["outcome"],
        "cancelled"
    );
}
