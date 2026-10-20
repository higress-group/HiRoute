use super::*;
use hiroute_application_api::{ErrorCode, ObservationReadIntentV2, ObservationReadRequestV2};
use hiroute_domain::RoutingTimelineQueryV1;

#[test]
fn review_timeline_dispatch_exhausts_real_readers_then_recovers() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::tests::timeline_retry_tests::review_timeline_dispatch_exhausts_real_readers_then_recovers",
    ) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let runtime = ProductionControlRuntime::open_with_release_catalog(
        temp.path(),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    ingest_value(
        &runtime.observation,
        "timeline-readers",
        "plan/codex-daily",
        now - 1000,
    );
    let application = ApplicationService::new(runtime.application_ports());
    let query = ObservationReadRequestV2::new(ObservationReadIntentV2::RoutingTimeline(
        RoutingTimelineQueryV1 {
            session_id: "session-timeline-readers".into(),
            after_watermark: None,
            limit: 50,
        },
    ));
    let request = LocalControlRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: "timeline-busy".into(),
        principal: PrincipalV1::ambient_local_peer(),
        operation_id: query.operation().into(),
        payload: serde_json::to_value(query).unwrap(),
        protected_grant: None,
    };
    // Keep all production reader permits held across two real dispatches. No
    // port or failure envelope is mocked; releasing the permits permits recovery.
    runtime.observation.with_all_query_permits_for_test(|| {
        for _ in 0..2 {
            let response = application.dispatch(request.clone());
            let wire = serde_json::to_value(&response).unwrap();
            let error = response.error.unwrap();
            assert_eq!(error.code, ErrorCode::ObservationUnavailable, "{wire}");
            assert!(error.retryable, "{wire}");
            assert_eq!(error.details_schema, "hiroute.observation.read-failure/v1");
            assert_eq!(wire["error"]["observation_read"]["reason"], "reader_busy");
            assert_eq!(wire["error"]["observation_read"]["retry_after_ms"], 100);
        }
    });
    let response = application.dispatch(request);
    assert!(response.error.is_none(), "{response:?}");
    assert!(response.data.is_some());
}
