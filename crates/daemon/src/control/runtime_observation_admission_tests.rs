use super::*;
use hiroute_application_api::{
    ObservationReadIntentV2, ObservationReadRequestV2, PrincipalKind, ProtectedClientGrantV2,
};

#[test]
fn protected_grant_admission_is_independent_of_the_global_control_mutex() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::tests::observation_admission_tests::protected_grant_admission_is_independent_of_the_global_control_mutex",
    ) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let runtime = Arc::new(
        ProductionControlRuntime::open_with_release_catalog(
            temp.path(),
            crate::release_catalog::fixture_catalog(),
        )
        .unwrap(),
    );
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let registration = |token: &str| {
        ApplyCapabilityRegistrationV1::from_protected_launcher(
            token.into(),
            "interactive-user",
            WorkspaceId::default(),
            "ListSessionsV2",
            CanonicalDigest::of_bytes(b"exact query"),
            RevisionSetV1 {
                target: 0,
                dependencies: Default::default(),
            },
            now + 60,
        )
        .unwrap()
    };
    let guard = runtime.adapter.stores.lock().unwrap();
    let worker = runtime.clone();
    let value = registration(&"a".repeat(64));
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        tx.send(worker.try_register_apply_capability(value))
            .unwrap()
    });
    let result = rx.recv_timeout(Duration::from_millis(500));
    drop(guard);
    thread.join().unwrap();
    assert_eq!(
        result
            .expect("grant admission waited for the Control mutex")
            .unwrap(),
        hiroute_local_storage::ApplyCapabilityRegistrationOutcome::Registered
    );
    assert_eq!(
        runtime
            .try_register_apply_capability(registration(&"b".repeat(64)))
            .unwrap(),
        hiroute_local_storage::ApplyCapabilityRegistrationOutcome::Registered
    );
}

#[test]
fn ordinary_v2_reads_do_not_wait_for_a_busy_control_writer() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::tests::observation_admission_tests::ordinary_v2_reads_do_not_wait_for_a_busy_control_writer",
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
    let application = Arc::new(ApplicationService::new(runtime.application_ports()));
    let revisions = runtime
        .adapter
        .stores
        .lock()
        .unwrap()
        .control()
        .current_revisions(&WorkspaceId::default())
        .unwrap();
    let now_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let intents = vec![
        ObservationReadIntentV2::PlanQuality(Box::new(
            serde_json::from_value(json!({
                "plan_id":"plan/bounded-read", "limit":50,
                "include_routing_observations":true,
            }))
            .unwrap(),
        )),
        ObservationReadIntentV2::RoutingTimeline(hiroute_domain::RoutingTimelineQueryV1 {
            session_id: "session-bounded-read".into(),
            after_watermark: None,
            limit: 50,
            include_presentation: false,
        }),
        ObservationReadIntentV2::Status(hiroute_application_api::ObservationStatusQueryV2 {}),
        ObservationReadIntentV2::Sessions(
            serde_json::from_value(json!({
                "from_ms":0, "to_ms":1000, "limit":50, "only_model_switch":false,
            }))
            .unwrap(),
        ),
        ObservationReadIntentV2::HomeValue(hiroute_application_api::ObservationHomeValueQueryV2 {
            period: Some(hiroute_application_api::ValuePeriodV1::SevenDays),
            session_id: None,
            currency: Some("USD".into()),
        }),
    ];
    let queries = intents.into_iter().flat_map(|intent| {
        let original = ObservationReadRequestV2::new(intent);
        let mut detailed = original.clone();
        detailed.read_failure_details = true;
        [original, detailed]
    });
    for (index, query) in queries.enumerate() {
        let detailed_error = query.read_failure_details
            || matches!(
                &query.intent,
                ObservationReadIntentV2::PlanQuality(_)
                    | ObservationReadIntentV2::RoutingTimeline(_)
            );
        let capability = format!("bounded-observation-test-capability-{index}");
        runtime
            .register_apply_capability(
                ApplyCapabilityRegistrationV1::from_protected_launcher(
                    capability.clone(),
                    "interactive-user",
                    WorkspaceId::default(),
                    query.protected_operation(),
                    CanonicalDigest::of(&query).unwrap(),
                    revisions.clone(),
                    now_seconds + 120,
                )
                .unwrap(),
            )
            .unwrap();
        let grant = ProtectedClientGrantV2 {
            principal_kind: PrincipalKind::InteractiveUser,
            capability,
        };
        for supplied_grant in [None, Some(grant)] {
            let request = LocalControlRequestV2 {
                schema_version: LOCAL_CONTROL_SCHEMA_V2,
                request_id: format!("busy-observation-{index}"),
                principal: PrincipalV1::ambient_local_peer(),
                operation_id: query.operation().into(),
                payload: serde_json::to_value(&query).unwrap(),
                protected_grant: supplied_grant,
            };
            if query.read_failure_details && request.protected_grant.is_some() {
                let mut changed = request.clone();
                changed
                    .payload
                    .as_object_mut()
                    .unwrap()
                    .remove("read_failure_details");
                assert_eq!(
                    application.dispatch(changed).error.unwrap().code,
                    hiroute_application_api::ErrorCode::CapabilityDenied,
                    "the response option remains part of the exact granted request"
                );
            }
            let guard = runtime.adapter.stores.lock().unwrap();
            let app = application.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let thread = std::thread::spawn(move || tx.send(app.dispatch(request)).unwrap());
            // Release before asserting, so the old blocking path cannot hang the test.
            let result = rx.recv_timeout(Duration::from_millis(200));
            drop(guard);
            thread.join().unwrap();
            let wire =
                serde_json::to_value(result.expect("ordinary V2 admission blocked")).unwrap();
            assert_eq!(wire["error"]["code"], "OBSERVATION_UNAVAILABLE", "{wire}");
            if detailed_error {
                assert_eq!(
                    wire["error"]["observation_read"]["reason"], "reader_busy",
                    "{wire}"
                );
            } else {
                // Status/session callers retain their released legacy error shape.
                assert_eq!(
                    wire["error"]["details_schema"], "hiroute.error-details/none-v1",
                    "{wire}"
                );
                assert!(wire["error"].get("observation_read").is_none(), "{wire}");
            }
        }
    }
}
