use super::*;
use hiroute_diagnostics::event::ProcessRole;
use hiroute_diagnostics::level::DiagnosticLevel;
use hiroute_diagnostics::record::Component;
use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};

#[tokio::test]
async fn one_info_summary_survives_rejection_and_async_record_loss() {
    let root = std::env::temp_dir().join(format!(
        "hiroute-request-timing-{}-{}",
        std::process::id(),
        crate::server::core_runtime::observation::unix_nanos()
    ));
    let runtime = DiagnosticRuntime::start(RuntimeConfig {
        root: root.clone(),
        role: ProcessRole::Daemon,
        component: Component::Gateway,
        parent_session_id: None,
        level_override: Some(DiagnosticLevel::Info),
    });
    let latency = RequestLatency::new(
        runtime.port().handle().context(),
        Some(IngressProtocol::OpenAiResponses),
        Instant::now(),
    );
    let record = latency.record();
    let delivered = latency.record();
    latency
        .run(async {
            hiroute_gateway_core::runtime::timing::wait(
                hiroute_gateway_core::runtime::timing::WaitKind::ClientRead,
                tokio::time::sleep(std::time::Duration::from_millis(2)),
            )
            .await;
        })
        .await;
    latency.finish(RequestOutcome::Failed, Some(401));
    let request_us = latency.0.finished.get().unwrap().timing.elapsed_us;
    latency.finish(RequestOutcome::Completed, Some(200));
    drop(latency);
    // Queue and sink work can finish later; they cannot extend the frozen HTTP clock.
    std::thread::sleep(std::time::Duration::from_millis(3));
    let started = delivered.begin_delivery();
    std::thread::sleep(std::time::Duration::from_millis(2));
    delivered.end_delivery(started, true);
    drop(delivered);
    drop(record); // A queue-full, skipped or lost record is terminally accounted.
    runtime.shutdown();
    let log = std::fs::read_to_string(root.join("daemon/current.jsonl")).unwrap();
    let summaries = log
        .lines()
        .filter_map(|line| {
            let record: hiroute_diagnostics::record::DiagnosticRecordV1 =
                serde_json::from_str(line).unwrap();
            match record.event {
                DiagnosticEvent::RequestTimingSummary(s) => Some(s),
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(summaries.len(), 1, "{log}");
    let summary = &summaries[0];
    assert_eq!(summary.elapsed_us, request_us);
    assert_eq!(summary.outcome, RequestOutcome::Failed);
    assert_eq!(summary.http_status, Some(401));
    assert_eq!(summary.asynchronous.records, 2);
    assert_eq!(summary.asynchronous.delivered, 1);
    assert_eq!(summary.asynchronous.dropped, 1);
    assert!(summary.asynchronous.queue_wait_max_us >= 3_000);
    assert!(summary.asynchronous.sink_work_us >= 2_000);
    assert!(summary.asynchronous.after_response_us >= 5_000);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn detached_diagnostics_and_cancelled_request_need_no_async_worker() {
    let diagnostics = hiroute_diagnostics::runtime::DiagnosticsPort::default();
    let latency = RequestLatency::new(diagnostics.handle().context(), None, Instant::now());
    latency.finish(RequestOutcome::Cancelled, None);
    drop(latency);
}
