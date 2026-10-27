use super::*;
use hiroute_domain::RoutingTimelineQueryV1;

#[test]
fn known_native_auxiliary_keeps_receipts_without_manufacturing_a_user_turn() {
    use super::super::support;
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let main = Fixture::new("native-main");
    let mut title = Fixture::new("native-title");
    title.session = main.session.clone();
    title.fact_stream = main.fact_stream.clone();
    let channel = fact_channel(&main, 256 * 1024);
    let writer = writer(&store);
    let mut auxiliary = support::route_decision(PLAN);
    if let ExecutionFactV1::RouteDecision(route) = &mut auxiliary {
        route.native_auxiliary = true;
    }
    for (index, (fixture, fact)) in [(&main, support::route_decision(PLAN)), (&title, auxiliary)]
        .into_iter()
        .enumerate()
    {
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                fixture.fact(index as u64 + 1, fact, 1000 + index as i64)
            ),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let query = RoutingTimelineQueryV1 {
        session_id: main.session.to_string(),
        after_watermark: None,
        limit: 100,
    };
    let page = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(page.turns.len(), 1);
    assert_eq!(page.turns[0].requests[0].request_id, main.request.as_str());
    let retained: i64 = store
        .connection
        .lock()
        .query_row("SELECT COUNT(*) FROM logical_requests", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        retained, 2,
        "auxiliary request accounting and receipts remain retained"
    );
    drop(writer);
    drop(store);
    let reopened = open_store(directory.path());
    assert_eq!(
        reopened
            .observed_routing_timeline(&reader(), &query, 5000)
            .unwrap()
            .turns
            .len(),
        1
    );
}

#[test]
fn native_timeline_identity_uses_the_executed_candidate_not_the_last_evaluated_one() {
    use super::super::support;
    for primary in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let store = open_store(directory.path());
        let fixture = Fixture::new(if primary {
            "executed-primary"
        } else {
            "executed-regular"
        });
        let channel = fact_channel(&fixture, 256 * 1024);
        let writer = writer(&store);
        let mut regular = support::candidate("model/regular");
        let mut other = support::candidate("model/primary");
        if let ExecutionFactV1::CandidateDecision(candidate) = &mut regular {
            candidate.reasoning_profile_id = Some("max".into());
        }
        if let ExecutionFactV1::CandidateDecision(candidate) = &mut other {
            candidate.candidate_id = "candidate-b".into();
            candidate.stable_binding_id = "binding-b".into();
            candidate.reasoning_profile_id = Some("xhigh".into());
        }
        for (index, fact) in [support::route_decision(PLAN), regular, other]
            .into_iter()
            .enumerate()
        {
            assert!(matches!(
                offer_fact(
                    &writer,
                    &channel,
                    fixture.fact(index as u64 + 1, fact, 1000 + index as i64)
                ),
                WriterCycleOutcome::Ack(_)
            ));
        }
        let query = RoutingTimelineQueryV1 {
            session_id: fixture.session.to_string(),
            after_watermark: None,
            limit: 100,
        };
        let planned = store
            .observed_routing_timeline(&reader(), &query, 5000)
            .unwrap();
        assert_eq!(
            planned.turns[0].model_configuration_id, None,
            "candidate evaluation is not execution proof"
        );
        let model = if primary {
            "model/primary"
        } else {
            "model/regular"
        };
        let mut started = support::attempt_started(model);
        let mut ended = support::attempt_finished();
        if primary {
            if let ExecutionFactV1::AttemptStarted {
                candidate_id,
                stable_binding_id,
                ..
            } = &mut started
            {
                *candidate_id = "candidate-b".into();
                *stable_binding_id = "binding-b".into();
            }
            if let ExecutionFactV1::AttemptFinished(fact) = &mut ended {
                fact.stable_binding_id = "binding-b".into();
            }
        }
        for (index, fact) in [
            started,
            support::semantic_commit(),
            support::usage_and_cache(),
            ended,
        ]
        .into_iter()
        .enumerate()
        {
            assert!(matches!(
                offer_fact(
                    &writer,
                    &channel,
                    fixture.attempt_fact(
                        index as u64 + 4,
                        "attempt/executed",
                        fact,
                        1010 + index as i64,
                        PLAN
                    )
                ),
                WriterCycleOutcome::Ack(_)
            ));
        }
        let executed = store
            .observed_routing_timeline(&reader(), &query, 5000)
            .unwrap();
        assert_eq!(
            executed.turns[0].model_configuration_id.as_deref(),
            Some(model)
        );
        assert_eq!(
            executed.turns[0].reasoning_profile_id.as_deref(),
            Some(if primary { "xhigh" } else { "max" })
        );
        assert_eq!(
            executed.turns[0].requests[0].native_model.as_deref(),
            Some("native-a"),
            "both candidates share a native name; it cannot prove configuration identity"
        );
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                fixture.fact(
                    8,
                    finished_for_model(
                        &fixture,
                        "segment/executed",
                        "turn/executed",
                        1,
                        "smart_saving",
                        1000,
                        model
                    ),
                    1030
                )
            ),
            WriterCycleOutcome::Ack(_)
        ));
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                fixture.fact(9, support::request_finished(), 1031)
            ),
            WriterCycleOutcome::Ack(_)
        ));
        let closed = store
            .observed_routing_timeline(&reader(), &query, 5000)
            .unwrap();
        assert_eq!(
            closed.turns[0].model_configuration_id.as_deref(),
            Some(model)
        );
        assert_eq!(
            closed.turns[0].reasoning_profile_id,
            executed.turns[0].reasoning_profile_id
        );
        assert_eq!(
            closed.turns[0].requests[0].outcome.as_deref(),
            Some("accepted")
        );
    }
}

#[test]
fn timeline_pages_replay_late_scores_without_losing_or_repeating_changes() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let first = Fixture::new("timeline-first");
    let mut second = Fixture::new("timeline-second");
    second.session = first.session.clone();
    second.fact_stream = first.fact_stream.clone();
    let channel = fact_channel(&first, 256 * 1024);
    let writer = writer(&store);
    for (index, fixture) in [&first, &second].into_iter().enumerate() {
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                fixture.fact(
                    index as u64 + 1,
                    finished(
                        fixture,
                        &format!("segment-{index}"),
                        &format!("agent-turn-{index}"),
                        index as u64 + 1,
                        "smart_saving",
                        1000 + index as u64 * 100
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
    let page = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(page.turns.len(), 1);
    assert!(!page.complete);
    assert_eq!(page.turns[0].assessment_state, "unrated");
    let first_id = page.turns[0].turn_id.clone();
    let old_digest = page.turns[0].content_digest.clone();
    // A score appended while paging is delivered after the frozen initial snapshot.
    assert!(matches!(
        offer_fact(
            &writer,
            &channel,
            second.fact(
                3,
                assessment(
                    &second,
                    "segment-0",
                    ("agent-turn-0", "agent-turn-0", 1, 1),
                    1200,
                    0.4
                ),
                1200
            )
        ),
        WriterCycleOutcome::Ack(_)
    ));
    query.after_watermark = Some(page.watermark);
    let page = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert!(page.complete);
    assert_eq!(page.turns.len(), 1);
    assert_ne!(page.turns[0].turn_id, first_id);
    query.after_watermark = Some(page.watermark);
    let page = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(page.turns[0].turn_id, first_id);
    assert_eq!(page.turns[0].assessment.as_ref().unwrap().score, 0.4);
    assert_ne!(page.turns[0].content_digest, old_digest);
    // Advancing through the remaining changed turn leaves an empty read.
    query.after_watermark = Some(page.watermark);
    let page = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    query.after_watermark = Some(page.watermark);
    assert!(
        store
            .observed_routing_timeline(&reader(), &query, 5000)
            .unwrap()
            .turns
            .is_empty()
    );
    // A cursor is bound to the reader and its session/page size.
    query.limit = 2;
    assert!(matches!(
        store.observed_routing_timeline(&reader(), &query, 5000),
        Err(crate::query_v2::ObservationV2Error::Stale)
    ));
    query.limit = 1;
    crate::query_v2::invalidate_visibility(&store.connection.lock()).unwrap();
    assert!(matches!(
        store.observed_routing_timeline(&reader(), &query, 5000),
        Err(crate::query_v2::ObservationV2Error::Stale)
    ));
}

#[test]
fn timeline_refuses_oversized_stored_fact_before_deserializing_it() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("timeline-bounded");
    let channel = fact_channel(&fixture, 256 * 1024);
    assert!(matches!(
        offer_fact(
            &writer(&store),
            &channel,
            fixture.fact(
                1,
                finished(&fixture, "segment", "turn", 1, "smart_saving", 1000),
                1010
            )
        ),
        WriterCycleOutcome::Ack(_)
    ));
    store
        .connection
        .lock()
        .execute(
            "UPDATE observation_sensitive_payloads_v2 SET body_json=?1",
            ["x".repeat(1024 * 1024 + 1)],
        )
        .unwrap();
    assert!(matches!(
        store.observed_routing_timeline(
            &reader(),
            &RoutingTimelineQueryV1 {
                session_id: fixture.session.to_string(),
                after_watermark: None,
                limit: 100,
            },
            5000
        ),
        Err(crate::query_v2::ObservationV2Error::ScopeTooLarge)
    ));
}

#[test]
fn timeline_incremental_pending_and_late_range_match_cold_projection() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let first = Fixture::new("incremental-first");
    let channel = fact_channel(&first, 256 * 1024);
    let writer = writer(&store);
    let mut query = RoutingTimelineQueryV1 {
        session_id: first.session.to_string(),
        after_watermark: None,
        limit: 200,
    };
    let mut all = std::collections::BTreeMap::new();
    for index in 0..8 {
        let mut fixture = Fixture::new(&format!("incremental-{index}"));
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
                        "shared-segment",
                        &format!("turn-{index}"),
                        index + 1,
                        "smart_saving",
                        1000 + index * 20
                    ),
                    1010 + index as i64 * 20
                )
            ),
            WriterCycleOutcome::Ack(_)
        ));
        let page = store
            .observed_routing_timeline(&reader(), &query, 5000)
            .unwrap();
        assert!(page.complete);
        for t in page.turns {
            all.insert(t.turn_id.clone(), t);
        }
        query.after_watermark = Some(page.watermark);
    }
    assert_eq!(
        store.timeline_work().0,
        8,
        "only newly affected requests are read"
    );
    assert!(
        store.timeline_work().1 <= 16,
        "only the new and previous pending turns are reprojected"
    );
    let before = store.timeline_work();
    let unchanged = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert!(unchanged.turns.is_empty());
    assert_eq!(
        store.timeline_work(),
        before,
        "unchanged poll does no session read or projection"
    );
    let mut trigger = Fixture::new("incremental-7");
    trigger.session = first.session.clone();
    trigger.fact_stream = first.fact_stream.clone();
    assert!(matches!(
        offer_fact(
            &writer,
            &channel,
            trigger.fact(
                9,
                assessment(
                    &trigger,
                    "shared-segment",
                    ("turn-0", "turn-6", 1, 7),
                    3000,
                    0.4
                ),
                3000
            )
        ),
        WriterCycleOutcome::Ack(_)
    ));
    let page = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    for t in page.turns {
        all.insert(t.turn_id.clone(), t);
    }
    query.after_watermark = Some(page.watermark);
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
    assert_eq!(all.len(), cold.turns.len());
    for t in cold.turns {
        assert_eq!(
            serde_json::to_value(&all[&t.turn_id]).unwrap(),
            serde_json::to_value(&t).unwrap()
        );
    }
}

#[test]
#[ignore = "long-session timing evidence; run explicitly in release mode"]
fn timeline_long_session_read_cost() {
    use std::time::Instant;
    fn timing(values: &mut [f64]) -> (f64, f64) {
        values.sort_by(f64::total_cmp);
        (
            values[values.len() / 2],
            values[(values.len() * 95 / 100).min(values.len() - 1)],
        )
    }
    for count in [1000u64, 5000, 10_000] {
        let directory = tempfile::tempdir().unwrap();
        let store = open_store(directory.path());
        let first = Fixture::new("perf-root");
        let channel = fact_channel(&first, 256 * 1024);
        let writer = writer(&store);
        for i in 0..count - 40 {
            let mut f = Fixture::new(&format!("perf-{i:05}"));
            f.session = first.session.clone();
            f.fact_stream = first.fact_stream.clone();
            assert!(matches!(
                offer_fact(
                    &writer,
                    &channel,
                    f.fact(
                        i + 1,
                        finished(
                            &f,
                            &format!("seg-{i}"),
                            &format!("turn-{i}"),
                            i + 1,
                            "smart_saving",
                            1000 + i
                        ),
                        1000 + i as i64
                    )
                ),
                WriterCycleOutcome::Ack(_)
            ));
        }
        let reader = ObservationReaderContext::local_user(
            hiroute_domain::WorkspaceId::default(),
            "perf".into(),
            1,
            100_000,
            false,
            false,
        )
        .unwrap();
        let mut q = RoutingTimelineQueryV1 {
            session_id: first.session.to_string(),
            after_watermark: None,
            limit: 200,
        };
        let mut initial = store
            .observed_routing_timeline(&reader, &q, 50_000)
            .unwrap();
        while !initial.complete {
            q.after_watermark = Some(initial.watermark);
            initial = store
                .observed_routing_timeline(&reader, &q, 50_000)
                .unwrap();
        }
        q.after_watermark = Some(initial.watermark);
        let mut appended = Vec::new();
        for i in count - 40..count {
            let mut f = Fixture::new(&format!("perf-{i:05}"));
            f.session = first.session.clone();
            f.fact_stream = first.fact_stream.clone();
            assert!(matches!(
                offer_fact(
                    &writer,
                    &channel,
                    f.fact(
                        i + 1,
                        finished(
                            &f,
                            &format!("seg-{i}"),
                            &format!("turn-{i}"),
                            i + 1,
                            "smart_saving",
                            1000 + i
                        ),
                        1000 + i as i64
                    )
                ),
                WriterCycleOutcome::Ack(_)
            ));
            let start = Instant::now();
            let page = store
                .observed_routing_timeline(&reader, &q, 50_000)
                .unwrap();
            appended.push(start.elapsed().as_secs_f64() * 1000.);
            assert_eq!(page.turns.len(), 2);
            assert!(page.complete);
            q.after_watermark = Some(page.watermark);
        }
        let (ap50, ap95) = timing(&mut appended);
        eprintln!(
            "TIMELINE_APPEND requests_from={} requests_through={count} samples=40 p50_ms={ap50:.3} p95_ms={ap95:.3}",
            count - 39
        );
        assert!(ap95 < 50., "new-request p95 target");
        q.after_watermark = None;
        let start = Instant::now();
        let mut page = store
            .observed_routing_timeline(&reader, &q, 50_000)
            .unwrap();
        let cold = start.elapsed().as_secs_f64() * 1000.;
        while !page.complete {
            q.after_watermark = Some(page.watermark);
            page = store
                .observed_routing_timeline(&reader, &q, 50_000)
                .unwrap();
        }
        q.after_watermark = Some(page.watermark);
        let mut unchanged = Vec::new();
        let mut changed = Vec::new();
        for i in 0..40 {
            let start = Instant::now();
            let page = store
                .observed_routing_timeline(&reader, &q, 50_000)
                .unwrap();
            unchanged.push(start.elapsed().as_secs_f64() * 1000.);
            assert!(page.turns.is_empty());
            q.after_watermark = Some(page.watermark);
            let mut f = Fixture::new(&format!("perf-{:05}", count - 1));
            f.session = first.session.clone();
            f.fact_stream = first.fact_stream.clone();
            assert!(matches!(
                offer_fact(
                    &writer,
                    &channel,
                    f.fact(
                        count + i + 1,
                        assessment(
                            &f,
                            &format!("seg-{}", count - 1),
                            (
                                &format!("turn-{}", count - 1),
                                &format!("turn-{}", count - 1),
                                count,
                                count
                            ),
                            20_000 + i,
                            0.9
                        ),
                        20_000 + i as i64
                    )
                ),
                WriterCycleOutcome::Ack(_)
            ));
            let start = Instant::now();
            let page = store
                .observed_routing_timeline(&reader, &q, 50_000)
                .unwrap();
            changed.push(start.elapsed().as_secs_f64() * 1000.);
            assert!(!page.turns.is_empty());
            assert!(page.complete);
            q.after_watermark = Some(page.watermark);
        }
        let (up50, up95) = timing(&mut unchanged);
        let (cp50, cp95) = timing(&mut changed);
        eprintln!(
            "TIMELINE requests={count} facts={} samples=40 cold_ms={cold:.3} unchanged_p50_ms={up50:.3} unchanged_p95_ms={up95:.3} changed_p50_ms={cp50:.3} changed_p95_ms={cp95:.3}",
            count + 40
        );
        assert!(up95 < 20.0, "unchanged p95 target");
        assert!(cp95 < 50.0, "changed p95 target");
        if count == 10_000 {
            let mut f = Fixture::new("perf-over-limit");
            f.session = first.session.clone();
            f.fact_stream = first.fact_stream.clone();
            assert!(matches!(
                offer_fact(
                    &writer,
                    &channel,
                    f.fact(
                        count + 41,
                        finished(&f, "extra", "extra", count + 1, "smart_saving", 30_000),
                        30_000
                    )
                ),
                WriterCycleOutcome::Ack(_)
            ));
            assert!(matches!(
                store.observed_routing_timeline(&reader, &q, 50_000),
                Err(crate::query_v2::ObservationV2Error::ScopeTooLarge)
            ));
        }
    }
}

#[test]
fn timeline_other_session_changes_advance_scan_without_reading_this_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let f = Fixture::new("gate-own");
    let c = fact_channel(&f, 256 * 1024);
    let w = writer(&store);
    assert!(matches!(
        offer_fact(
            &w,
            &c,
            f.fact(
                1,
                finished(&f, "stage", "turn", 1, "smart_saving", 1000),
                1000
            )
        ),
        WriterCycleOutcome::Ack(_)
    ));
    let mut q = RoutingTimelineQueryV1 {
        session_id: f.session.to_string(),
        after_watermark: None,
        limit: 50,
    };
    let page = store
        .observed_routing_timeline(&reader(), &q, 5000)
        .unwrap();
    q.after_watermark = Some(page.watermark.clone());
    let work = store.timeline_work();
    let other = Fixture::new("gate-other");
    let oc = fact_channel(&other, 256 * 1024);
    assert!(matches!(
        offer_fact(
            &w,
            &oc,
            other.fact(
                1,
                finished(&other, "other-stage", "other-turn", 1, "smart_saving", 1100),
                1100
            )
        ),
        WriterCycleOutcome::Ack(_)
    ));
    let next = store
        .observed_routing_timeline(&reader(), &q, 5000)
        .unwrap();
    assert!(next.turns.is_empty());
    assert!(next.complete);
    assert_ne!(next.watermark, page.watermark);
    assert_eq!(work, store.timeline_work());
    q.after_watermark = Some(next.watermark.clone());
    let again = store
        .observed_routing_timeline(&reader(), &q, 5000)
        .unwrap();
    assert_eq!(again.watermark, next.watermark);
    assert_eq!(work, store.timeline_work());
}
