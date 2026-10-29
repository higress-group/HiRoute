use super::*;
use crate::query_v2::ObservationV2Error;
use hiroute_domain::{RoutingTimelineQueryV1, WorkspaceId};

#[test]
fn timeline_late_completion_resolves_only_its_redundant_intermediate_comparison() {
    use super::super::support;
    use hiroute_domain::AnalysisTransitionKindV1;
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let prior = Fixture::new("comparison-prior");
    let mut opening = Fixture::new("comparison-opening");
    let mut closing = Fixture::new("comparison-closing");
    for fixture in [&mut opening, &mut closing] {
        fixture.session = prior.session.clone();
        fixture.fact_stream = prior.fact_stream.clone();
    }
    let channel = fact_channel(&prior, 256 * 1024);
    let writer = writer(&store);
    let assert_ingested = |envelope: hiroute_domain::ExecutionFactEnvelopeV1| {
        let request = envelope.correlation.request_id.clone();
        let sequence = envelope.sequence;
        let fact_kind = serde_json::to_value(&envelope.fact).unwrap()["kind"]
            .as_str()
            .unwrap()
            .to_owned();
        let outcome = offer_fact(&writer, &channel, envelope);
        assert!(
            matches!(outcome, WriterCycleOutcome::Ack(_)),
            "request={request} sequence={sequence} fact={fact_kind:?}: {outcome:?}"
        );
    };
    let mut sequence = 0;
    for (fixture, model, at) in [
        (&prior, "regular", 1000),
        (&opening, "primary", 1100),
        (&closing, "primary", 1200),
    ] {
        let mut candidate = support::candidate(model);
        if let ExecutionFactV1::CandidateDecision(candidate) = &mut candidate {
            candidate.native_model = model.into();
        }
        let mut started = support::attempt_started(model);
        if let ExecutionFactV1::AttemptStarted { request_model, .. } = &mut started {
            *request_model = model.into();
        }
        let mut facts = vec![
            (support::route_decision(PLAN), false),
            (candidate, false),
            (started, true),
            (support::attempt_finished(), true),
        ];
        if fixture.request == prior.request {
            facts.push((
                finished_for_model(&prior, "stage-1", "turn-1", 1, "smart_saving", 1000, model),
                false,
            ));
        }
        if fixture.request != closing.request {
            facts.push((support::request_finished(), false));
        }
        for (offset, (fact, attempt)) in facts.into_iter().enumerate() {
            sequence += 1;
            let envelope = if attempt {
                fixture.attempt_fact(
                    sequence,
                    &format!("attempt/{}", fixture.request.as_str()),
                    fact,
                    at + offset as i64,
                    PLAN,
                )
            } else {
                fixture.fact(sequence, fact, at + offset as i64)
            };
            assert_ingested(envelope);
        }
    }
    let mut query = RoutingTimelineQueryV1 {
        include_presentation: false,
        session_id: prior.session.to_string(),
        after_watermark: None,
        limit: 50,
    };
    let provisional = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(provisional.turns.len(), 3);
    assert!(
        provisional.turns[1]
            .transitions
            .iter()
            .any(|t| t.kind == AnalysisTransitionKindV1::Unknown)
    );
    // Turn completion belongs to the still-open final request. Appending it to
    // the already-frozen opening request would violate receipt immutability.
    let mut finished = finished_for_model(
        &closing,
        "stage-2",
        "turn-2",
        2,
        "smart_saving",
        1100,
        "primary",
    );
    if let ExecutionFactV1::AgentTurnFinished {
        first_request_id,
        finished_at_ms,
        ..
    } = &mut finished
    {
        *first_request_id = Some(opening.request.clone());
        *finished_at_ms = 1210;
    }
    for (fact, at) in [(finished, 1210), (support::request_finished(), 1211)] {
        sequence += 1;
        assert_ingested(closing.fact(sequence, fact, at));
    }
    query.after_watermark = Some(provisional.watermark);
    let delta = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    let completed = delta
        .turns
        .iter()
        .find(|t| t.turn_id == opening.request.as_str())
        .unwrap();
    assert_eq!(completed.requests.len(), 2);
    assert_eq!(completed.transitions.len(), 1);
    let comparison = &completed.transitions[0];
    assert_eq!(comparison.kind, AnalysisTransitionKindV1::BetweenTurnChange);
    assert_eq!(comparison.request_id, closing.request.as_str());
    assert_eq!(
        comparison.previous_request_id.as_deref(),
        Some(prior.request.as_str())
    );
    assert_eq!(comparison.completeness, "complete");
    query.after_watermark = Some(delta.watermark);
    assert!(
        store
            .observed_routing_timeline(&reader(), &query, 5000)
            .unwrap()
            .turns
            .is_empty()
    );
    query.after_watermark = None;
    let cold = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(cold.turns.len(), 2);
    assert_eq!(
        serde_json::to_value(&cold.turns[1]).unwrap(),
        serde_json::to_value(completed).unwrap()
    );
}

#[test]
fn timeline_late_turn_completion_reemits_the_opening_and_absorbed_request_membership() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let first = Fixture::new("provisional-opening");
    let mut continuation = Fixture::new("provisional-continuation");
    continuation.session = first.session.clone();
    continuation.fact_stream = first.fact_stream.clone();
    let channel = fact_channel(&first, 256 * 1024);
    let writer = writer(&store);
    for (sequence, fixture, at) in [(1, &first, 1000), (2, &continuation, 1005)] {
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                fixture.fact(sequence, super::super::support::route_decision(PLAN), at)
            ),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let mut query = RoutingTimelineQueryV1 {
        include_presentation: false,
        session_id: first.session.to_string(),
        after_watermark: None,
        limit: 50,
    };
    let provisional = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(provisional.turns.len(), 2);
    let mut closed = finished(&first, "stage", "closed-turn", 1, "smart_saving", 1000);
    if let ExecutionFactV1::AgentTurnFinished {
        last_request_id, ..
    } = &mut closed
    {
        *last_request_id = Some(continuation.request.clone());
    }
    assert!(matches!(
        offer_fact(&writer, &channel, first.fact(3, closed, 1010)),
        WriterCycleOutcome::Ack(_)
    ));
    query.after_watermark = Some(provisional.watermark);
    let changed = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(changed.turns.len(), 1);
    assert_eq!(changed.turns[0].turn_id, provisional.turns[0].turn_id);
    assert_eq!(changed.turns[0].requests.len(), 2);
    assert!(
        changed.turns[0]
            .requests
            .iter()
            .any(|request| request.request_id == provisional.turns[1].turn_id),
        "clients can remove the absorbed provisional row by exact request membership"
    );
    query.after_watermark = None;
    let completed = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(completed.turns.len(), 1);
    assert_eq!(completed.turns[0].turn_id, provisional.turns[0].turn_id);
    assert_eq!(completed.turns[0].requests.len(), 2);
}

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
        include_presentation: false,
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
                    include_presentation: false,
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
        include_presentation: false,
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
        include_presentation: false,
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
