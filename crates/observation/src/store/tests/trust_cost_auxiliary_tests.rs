use super::*;

fn title_request(store: &LocalObservationStore) -> Fixture {
    let f = Fixture::new("trial-native-title");
    for mut envelope in facts(&f, UsageFrameKindV1::Cumulative) {
        if let ExecutionFactV1::RouteDecision(route) = &mut envelope.fact {
            route.native_auxiliary = true;
        }
        store.ingest_fact(&envelope, &[]).unwrap();
    }
    store.settle_pending_valuations(16).unwrap();
    f
}

#[test]
fn trial_cost_auxiliary_is_outside_stages_but_keeps_generic_cost_and_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    trial_request(&store, "trial-main", 17, ExecutionGroupV1::Regular);
    let title = title_request(&store);
    let frozen = serde_json::to_value(latest(&store, &title)).unwrap();
    let report = trial_report(&store, 1000).unwrap();
    assert_eq!(
        (report.request_count, report.compared_request_count),
        (1, 1)
    );
    assert_eq!(
        (report.missing_request_count, report.excluded_request_count),
        (0, 0)
    );
    assert_eq!(report.coverage, ObservationMetricCoverageV2::Complete);
    let generic = store
        .observed_api_equivalent_value(
            &reader(),
            &ObservationValueQueryV2 {
                from_ms: 0,
                to_ms: 1000,
                session_id: None,
                plan_id: None,
                currency: Some("USD".into()),
            },
            &current_price("reference", "model-ref", 4_000_000),
            &[current_price("actual", "model-a", 1_000_000)],
            1000,
        )
        .unwrap();
    assert_eq!(
        (generic.request_count, generic.compared_request_count),
        (2, 2)
    );
    drop(store);
    let store = open_store(dir.path());
    assert_eq!(trial_report(&store, 1000).unwrap().request_count, 1);
    assert_eq!(
        serde_json::to_value(latest(&store, &title)).unwrap(),
        frozen
    );
}

#[test]
fn trial_cost_auxiliary_requires_retained_authenticated_purpose() {
    for delete in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(dir.path());
        trial_request(&store, "trial-main", 17, ExecutionGroupV1::Regular);
        let title = title_request(&store);
        assert_eq!(trial_report(&store, 1000).unwrap().request_count, 1);
        let db = store.connection.lock();
        if delete {
            db.execute(
                "DELETE FROM observation_sensitive_payloads_v2 WHERE request_id=?1",
                [title.request.as_str()],
            )
            .unwrap();
        } else {
            db.execute("UPDATE observation_sensitive_payloads_v2 SET body_json=json_set(body_json,'$.fact.planner_version','tampered') WHERE request_id=?1", [title.request.as_str()]).unwrap();
        }
        let updated: i64 = db
            .query_row(
                "SELECT updated_at_ms FROM sessions WHERE workspace_id=?1 AND session_id=?2",
                rusqlite::params![title.workspace.as_str(), title.session.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        drop(db);
        assert!(matches!(
            trial_report(&store, 1000),
            Err(crate::query_v2::ObservationV2Error::Unavailable)
        ));
        let report = trial_report(&store, updated + 300_000).unwrap();
        assert_eq!(
            (report.request_count, report.compared_request_count),
            (2, 1)
        );
        assert_eq!(
            (report.missing_request_count, report.excluded_request_count),
            (1, 1)
        );
        assert_eq!(report.coverage, ObservationMetricCoverageV2::Partial);
        assert_eq!(report.estimated_savings_micros, None);
    }
}
