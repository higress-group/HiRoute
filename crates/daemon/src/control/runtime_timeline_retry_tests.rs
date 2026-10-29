use super::*;
use hiroute_application_api::{ErrorCode, ObservationReadIntentV2, ObservationReadRequestV2};
use hiroute_domain::RoutingTimelineQueryV1;

#[test]
fn review_timeline_and_quality_dispatch_exhaust_real_readers_then_recover() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::tests::timeline_retry_tests::review_timeline_and_quality_dispatch_exhaust_real_readers_then_recover",
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
            include_presentation: false,
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
    let quality = ObservationReadRequestV2::new(ObservationReadIntentV2::PlanQuality(Box::new(
        serde_json::from_value(json!({
            "plan_id": "plan/codex-daily", "limit": 50,
            "include_routing_observations": true,
        }))
        .unwrap(),
    )));
    let quality_request = LocalControlRequestV2 {
        request_id: "quality-busy".into(),
        operation_id: quality.operation().into(),
        payload: serde_json::to_value(quality).unwrap(),
        ..request.clone()
    };
    let mut legacy_request = quality_request.clone();
    legacy_request.payload["intent"]["query"]
        .as_object_mut()
        .unwrap()
        .remove("include_routing_observations");
    let mut disabled_request = quality_request.clone();
    disabled_request.payload["intent"]["query"]["include_routing_observations"] = json!(false);
    for (request, detailed) in [
        (request, true),
        (quality_request, true),
        (legacy_request, false),
        (disabled_request, false),
    ] {
        // Exercise real permit exhaustion and recovery through Application.
        // Released strict quality readers retain their original failure shape.
        runtime.observation.with_all_query_permits_for_test(|| {
            let response = application.dispatch(request.clone());
            let wire = serde_json::to_value(&response).unwrap();
            assert_eq!(
                response.request_id.as_deref(),
                Some(request.request_id.as_str())
            );
            let error = response.error.unwrap();
            assert_eq!(error.code, ErrorCode::ObservationUnavailable, "{wire}");
            assert_eq!(error.retryable, detailed, "{wire}");
            if detailed {
                assert_eq!(error.details_schema, "hiroute.observation.read-failure/v1");
                assert_eq!(wire["error"]["observation_read"]["reason"], "reader_busy");
                assert_eq!(wire["error"]["observation_read"]["retry_after_ms"], 100);
            } else {
                assert_eq!(error.details_schema, "hiroute.error-details/none-v1");
                assert!(error.observation_read.is_none());
            }
        });
        let response = application.dispatch(request);
        assert!(response.error.is_none(), "{response:?}");
        assert!(response.data.is_some());
    }
}
