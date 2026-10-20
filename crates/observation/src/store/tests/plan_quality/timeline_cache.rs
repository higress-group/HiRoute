use super::*;
use crate::query_v2::ObservationV2Error;
use hiroute_domain::{RoutingTimelineQueryV1, WorkspaceId};

#[test]
fn timeline_evicted_frozen_page_restarts_without_skipping_retained_turns() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let first = Fixture::new("evicted-first");
    let channel = fact_channel(&first, 256 * 1024);
    let writer = writer(&store);
    for index in 0..2 {
        let mut fixture = Fixture::new(&format!("evicted-{index}"));
        fixture.session = first.session.clone();
        fixture.fact_stream = first.fact_stream.clone();
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                fixture.fact(
                    index + 1,
                    finished(
                        &fixture,
                        "stage",
                        &format!("turn-{index}"),
                        index + 1,
                        "smart_saving",
                        1000 + index * 100
                    ),
                    1010 + index as i64 * 100
                )
            ),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let mut query = RoutingTimelineQueryV1 {
        session_id: first.session.to_string(),
        after_watermark: None,
        limit: 1,
    };
    let first_page = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert!(!first_page.complete);
    for session in ["session/other-a", "session/other-b"] {
        store
            .observed_routing_timeline(
                &reader(),
                &RoutingTimelineQueryV1 {
                    session_id: session.into(),
                    after_watermark: None,
                    limit: 1,
                },
                5000,
            )
            .unwrap();
    }
    query.after_watermark = Some(first_page.watermark);
    assert_eq!(
        store
            .observed_routing_timeline(&reader(), &query, 5000)
            .unwrap_err(),
        ObservationV2Error::Stale
    );
    query.after_watermark = None;
    let fresh = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(fresh.turns[0].turn_id, first_page.turns[0].turn_id);
    query.after_watermark = Some(fresh.watermark);
    let last = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert!(last.complete);
    assert_ne!(last.turns[0].turn_id, first_page.turns[0].turn_id);
}

#[test]
fn timeline_cached_cursor_expires_without_a_retention_maintenance_write() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("expired-cached");
    let channel = fact_channel(&fixture, 256 * 1024);
    assert!(matches!(
        offer_fact(
            &writer(&store),
            &channel,
            fixture.fact(
                1,
                finished(&fixture, "stage", "turn", 1, "smart_saving", 1000),
                1010
            )
        ),
        WriterCycleOutcome::Ack(_)
    ));
    let reader = ObservationReaderContext::local_user(
        WorkspaceId::default(),
        "user".into(),
        1,
        SEVEN_DAYS_MILLIS * 2,
        false,
        false,
    )
    .unwrap();
    let mut query = RoutingTimelineQueryV1 {
        session_id: fixture.session.to_string(),
        after_watermark: None,
        limit: 50,
    };
    let page = store
        .observed_routing_timeline(&reader, &query, 5000)
        .unwrap();
    assert_eq!(page.turns.len(), 1);
    query.after_watermark = Some(page.watermark);
    let expired = SEVEN_DAYS_MILLIS + 2000;
    assert_eq!(
        store
            .observed_routing_timeline(&reader, &query, expired)
            .unwrap_err(),
        ObservationV2Error::Stale
    );
    query.after_watermark = None;
    assert!(
        store
            .observed_routing_timeline(&reader, &query, expired)
            .unwrap()
            .turns
            .is_empty()
    );
}

#[test]
fn timeline_incremental_protection_revisits_a_nonadjacent_segment() {
    use hiroute_domain::{
        CompetenceProtectionV1, ExecutionGroupV1, LedgerReasonCodeV1, ModelGroupReasonV1,
        ReasonLedgerEntryV1, RoutingTurnBoundaryV1,
    };
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let first = Fixture::new("protected-first");
    let channel = fact_channel(&first, 256 * 1024);
    let writer = writer(&store);
    for index in 0..3 {
        let mut fixture = Fixture::new(&format!("protected-{index}"));
        fixture.session = first.session.clone();
        fixture.fact_stream = first.fact_stream.clone();
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                fixture.fact(
                    index + 1,
                    finished(
                        &fixture,
                        &format!("stage-{index}"),
                        &format!("turn-{index}"),
                        index + 1,
                        "smart_saving",
                        1000 + index * 100
                    ),
                    1010 + index as i64 * 100
                )
            ),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let mut query = RoutingTimelineQueryV1 {
        session_id: first.session.to_string(),
        after_watermark: None,
        limit: 50,
    };
    let initial = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    let mut merged: std::collections::BTreeMap<_, _> = initial
        .turns
        .into_iter()
        .map(|t| (t.turn_id.clone(), t))
        .collect();
    query.after_watermark = Some(initial.watermark);
    let mut primary = Fixture::new("protected-primary");
    primary.session = first.session.clone();
    primary.fact_stream = first.fact_stream.clone();
    let mut decision = super::super::support::route_decision(PLAN);
    let ExecutionFactV1::RouteDecision(route) = &mut decision else {
        unreachable!()
    };
    let selection = route.complexity.as_mut().unwrap();
    selection.execution_group = ExecutionGroupV1::Primary;
    selection.selection_reason = ModelGroupReasonV1::LowCompetence;
    selection.competence_trigger = Some(CompetenceProtectionV1 {
        segment_id: "stage-0".into(),
        score: serde_json::Number::from_f64(0.2).unwrap(),
        floor_millis: 500,
        from_group: ExecutionGroupV1::Regular,
    });
    route.reason_ledger.push(ReasonLedgerEntryV1 {
        ordinal: 1,
        code: LedgerReasonCodeV1::ContextHoldInvalidated,
        group_id: None,
    });
    for (sequence, fact) in [
        (4, decision),
        (
            5,
            finished(
                &primary,
                "primary-stage",
                "primary-turn",
                4,
                "smart_saving",
                1300,
            ),
        ),
    ] {
        assert!(matches!(
            offer_fact(&writer, &channel, primary.fact(sequence, fact, 1310)),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let delta = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    let protected = delta
        .turns
        .iter()
        .find(|t| t.segment_id.as_deref() == Some("stage-0"))
        .unwrap();
    assert_eq!(protected.protected_turn_ids, [primary.request.to_string()]);
    let new = delta
        .turns
        .iter()
        .find(|t| t.turn_id == primary.request.as_str())
        .unwrap();
    assert_eq!(new.boundary, RoutingTurnBoundaryV1::NewUserMessage);
    for turn in delta.turns {
        merged.insert(turn.turn_id.clone(), turn);
    }
    query.after_watermark = None;
    let cold = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    for turn in cold.turns {
        assert_eq!(
            serde_json::to_value(&merged[&turn.turn_id]).unwrap(),
            serde_json::to_value(&turn).unwrap()
        );
    }
}
