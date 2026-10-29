use super::*;
use serde_json::Value;

pub(super) fn summaries(fixture: &RuntimeFixture, expected: usize) -> Vec<Value> {
    let path = fixture
        .diagnostics_root
        .as_ref()
        .unwrap()
        .join("daemon/current.jsonl");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let log = match std::fs::read(&path) {
            Ok(log) => log,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => panic!("read request timing log: {error}"),
        };
        let summaries: Vec<_> = complete_records(&log)
            .expect("closed request timing records must be valid JSON")
            .into_iter()
            .filter(|record| record["event"].get("request_timing_summary").is_some())
            .collect();
        if summaries.len() >= expected {
            assert_eq!(summaries.len(), expected, "{summaries:?}");
            return summaries;
        }
        assert!(
            Instant::now() < deadline,
            "missing request summary: {}",
            String::from_utf8_lossy(&log)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn complete_records(log: &[u8]) -> Result<Vec<Value>, serde_json::Error> {
    log.split_inclusive(|byte| *byte == b'\n')
        .take_while(|line| line.ends_with(b"\n"))
        .map(serde_json::from_slice)
        .collect()
}

#[test]
fn timing_reader_waits_for_open_utf8_tails_and_rejects_closed_corruption() {
    let mut log = b"{\"ready\":1}\n{\"text\":\"".to_vec();
    let unicode = "中".as_bytes();
    log.extend_from_slice(&unicode[..1]);
    assert_eq!(
        complete_records(&log).unwrap(),
        vec![serde_json::json!({"ready":1})]
    );
    log.extend_from_slice(&unicode[1..]);
    log.extend_from_slice(b"\"}\n");
    assert_eq!(
        complete_records(&log).unwrap(),
        vec![
            serde_json::json!({"ready":1}),
            serde_json::json!({"text":"中"})
        ]
    );
    assert!(complete_records(b"{}\n{closed invalid JSON}\n").is_err());
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
    let wrong_method = request(
        fixture.address,
        "GET",
        "/v1/responses",
        &[("X-HiRoute-Token", TOKEN)],
        b"",
    );
    assert_eq!(wrong_method.status, 405);
    let records = summaries(&fixture, 6);
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
fn info_request_timing_partitions_a_real_dripped_stream() {
    let provider = NativeProvider::start(vec![ProviderReply::StreamDrip {
        status: 200,
        chunks: RESPONSES_STREAM_OK
            .chunks(64)
            .map(|chunk| chunk.to_vec())
            .collect(),
        interval: Duration::from_millis(5),
    }]);
    let fixture = RuntimeFixture::launch_with_timing_diagnostics(&[&provider], 1, None);
    let response =
        fixture.request_body(br#"{"model":"runtime-model","input":"stream","stream":true}"#);
    assert_eq!(response.status, 200);
    assert!(String::from_utf8_lossy(&response.body).contains("response.completed"));
    let records = summaries(&fixture, 1);
    assert_partition(&records[0]);
    let summary = &records[0]["event"]["request_timing_summary"];
    assert_eq!(summary["outcome"], "completed");
    assert!(summary["wait"]["upstream_read_us"].as_u64().unwrap() >= 25_000);
    assert!(summary["local"]["stream_us"].as_u64().unwrap() > 0);
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
