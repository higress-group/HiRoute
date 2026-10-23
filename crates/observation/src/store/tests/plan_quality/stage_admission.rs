use super::*;
use hiroute_domain::{
    AgentTurnAdmissionV1, AgentTurnOpeningCauseV1, HistoryMembershipV1, HistoryRelationV1,
    RoutingTimelineQueryV1,
};

fn admitted(stage: &str, branch: &str, revision: u64) -> ExecutionFactV1 {
    ExecutionFactV1::AgentTurnAdmitted(AgentTurnAdmissionV1 {
        version: 1,
        agent_turn_id: stage.into(),
        branch_id: branch.into(),
        revision,
        ordinal: 1,
        opening_cause: AgentTurnOpeningCauseV1::NoComparableHistory,
    })
}

fn closed(first: &Fixture, last: &Fixture, revision: u64, ids: &[&Fixture]) -> ExecutionFactV1 {
    let mut fact = finished(last, "segment-a", "turn-a", 1, "smart_saving", 1000);
    if let ExecutionFactV1::AgentTurnFinished {
        history_membership,
        opening_cause,
        first_request_id,
        last_request_id,
        ..
    } = &mut fact
    {
        *opening_cause = Some(AgentTurnOpeningCauseV1::NoComparableHistory);
        *first_request_id = Some(first.request.clone());
        *last_request_id = Some(last.request.clone());
        *history_membership = Some(HistoryMembershipV1 {
            version: 1,
            branch_id: "branch-a".into(),
            revision,
            request_ids: ids.iter().map(|f| f.request.to_string()).collect(),
            clipped: false,
            relation: HistoryRelationV1::Contextual,
            source_unverified: true,
            association_unverified: true,
            capture_partial: false,
        });
    }
    fact
}

#[test]
fn pending_admission_keeps_interleaved_stages_separate_without_affinity_and_survives_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let first = Fixture::new("admission-a1");
    let mut other = Fixture::new("admission-b1");
    let mut later = Fixture::new("admission-a2");
    for f in [&mut other, &mut later] {
        f.session = first.session.clone();
        f.fact_stream = first.fact_stream.clone();
    }
    let query = RoutingTimelineQueryV1 {
        session_id: first.session.to_string(),
        after_watermark: None,
        limit: 50,
    };
    let preserved;
    {
        let store = open_store(directory.path());
        let channel = fact_channel(&first, 256 * 1024);
        let writer = writer(&store);
        let mut sequence = 0;
        for (f, stage, branch, revision) in [
            (&first, "turn-a", "branch-a", 1),
            (&other, "turn-b", "branch-b", 1),
            (&later, "turn-a", "branch-a", 2),
        ] {
            for fact in [
                admitted(stage, branch, revision),
                super::super::support::route_decision(PLAN),
            ] {
                sequence += 1;
                assert!(matches!(
                    offer_fact(
                        &writer,
                        &channel,
                        f.fact(sequence, fact, 1000 + sequence as i64)
                    ),
                    WriterCycleOutcome::Ack(_)
                ));
            }
        }
        let page = store
            .observed_routing_timeline(&reader(), &query, 5000)
            .unwrap();
        assert_eq!(page.turns.len(), 2);
        assert_eq!(page.turns[0].agent_turn_id.as_deref(), Some("turn-a"));
        assert_eq!(
            page.turns[0]
                .requests
                .iter()
                .map(|r| r.request_id.as_str())
                .collect::<Vec<_>>(),
            vec![first.request.as_str(), later.request.as_str()]
        );
        assert_eq!(page.turns[1].agent_turn_id.as_deref(), Some("turn-b"));
        assert!(
            page.turns
                .iter()
                .all(|t| t.segment_id.is_none() && t.assessment.is_none())
        );
        let rows: i64 = store.connection.lock().query_row("SELECT COUNT(*) FROM observation_analysis_turns_v1 WHERE segment_id IS NULL AND first_request_id IS NULL AND last_request_id IS NULL AND membership_conflict=0", [], |r| r.get(0)).unwrap();
        assert_eq!(
            rows, 3,
            "analysis shares admission identity without fabricating completion"
        );
        preserved = super::history_membership::fact_digests(&store.connection.lock());
    }
    let store = open_store(directory.path());
    assert_eq!(
        super::history_membership::fact_digests(&store.connection.lock()),
        preserved
    );
    let page = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(page.turns.len(), 2);
    assert_eq!(page.turns[0].requests.len(), 2);
}

#[test]
fn later_admission_reopens_display_without_expanding_frozen_assessment() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let first = Fixture::new("reopened-a1");
    let mut scoring = Fixture::new("reopened-b");
    let mut later = Fixture::new("reopened-a2");
    for f in [&mut scoring, &mut later] {
        f.session = first.session.clone();
        f.fact_stream = first.fact_stream.clone();
    }
    let channel = fact_channel(&first, 256 * 1024);
    let writer = writer(&store);
    let mut sequence = 0;
    let mut write = |f: &Fixture, fact, at| {
        sequence += 1;
        assert!(matches!(
            offer_fact(&writer, &channel, f.fact(sequence, fact, at)),
            WriterCycleOutcome::Ack(_)
        ));
    };
    write(&first, admitted("turn-a", "branch-a", 1), 1000);
    let closure = closed(&first, &first, 1, &[&first]);
    let mut score = assessment(&scoring, "segment-a", ("turn-a", "turn-a", 1, 1), 1020, 0.4);
    if let (
        ExecutionFactV1::BranchAssessmentRecorded {
            history_membership, ..
        },
        ExecutionFactV1::AgentTurnFinished {
            history_membership: frozen,
            ..
        },
    ) = (&mut score, &closure)
    {
        *history_membership = frozen.clone();
    }
    write(&first, closure, 1010);
    write(&scoring, score, 1020);
    let query = RoutingTimelineQueryV1 {
        session_id: first.session.to_string(),
        after_watermark: None,
        limit: 50,
    };
    let before = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert!(
        before
            .turns
            .iter()
            .any(|t| t.agent_turn_id.as_deref() == Some("turn-a") && t.assessment.is_some())
    );
    write(&later, admitted("turn-a", "branch-a", 2), 1030);
    let pending = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    let a = pending
        .turns
        .iter()
        .find(|t| t.agent_turn_id.as_deref() == Some("turn-a"))
        .unwrap();
    assert_eq!(a.turn_id, first.request.as_str());
    assert_eq!(a.requests.len(), 2);
    assert_eq!(a.status, "running");
    assert!(a.segment_id.is_none() && a.assessment.is_none());
    write(&later, closed(&first, &later, 2, &[&first, &later]), 1040);
    let final_page = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    let a = final_page
        .turns
        .iter()
        .find(|t| t.agent_turn_id.as_deref() == Some("turn-a"))
        .unwrap();
    assert_eq!(a.status, "completed");
    assert!(
        a.assessment.is_none(),
        "old score covers only the frozen first request"
    );
    // A delayed duplicate closure may be carried by a later request. It cannot
    // roll the current timeline or retained endpoint projection back to A1.
    write(&scoring, closed(&first, &first, 1, &[&first]), 1050);
    let late = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    let a = late
        .turns
        .iter()
        .find(|t| t.agent_turn_id.as_deref() == Some("turn-a"))
        .unwrap();
    assert_eq!(a.status, "completed");
    assert_eq!(a.requests.len(), 2);
    let endpoints = store.connection.lock().prepare("SELECT last_request_id,membership_revision,membership_conflict FROM observation_analysis_turns_v1 WHERE agent_turn_id='turn-a'").unwrap()
        .query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u64>(1)?,r.get::<_,bool>(2)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    assert_eq!(endpoints, vec![(later.request.to_string(), 2, false); 2]);
}

#[test]
fn conflicting_admission_is_not_resolved_by_routing_or_native_id_guessing() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let f = Fixture::new("admission-conflict");
    let channel = fact_channel(&f, 256 * 1024);
    let writer = writer(&store);
    for (index, stage) in ["turn-a", "turn-b"].into_iter().enumerate() {
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                f.fact(
                    index as u64 + 1,
                    admitted(stage, "branch-a", 1),
                    1000 + index as i64
                )
            ),
            WriterCycleOutcome::Ack(_)
        ));
    }
    assert!(
        store
            .observed_routing_timeline(
                &reader(),
                &RoutingTimelineQueryV1 {
                    session_id: f.session.to_string(),
                    after_watermark: None,
                    limit: 50
                },
                5000
            )
            .is_err()
    );
    let conflict: bool = store
        .connection
        .lock()
        .query_row(
            "SELECT membership_conflict FROM observation_analysis_turns_v1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(conflict);
}

#[test]
fn finished_request_without_stage_closure_cannot_remain_running_or_claim_task_completion() {
    use hiroute_domain::{ExecutionRequestOutcomeV1 as Outcome, FactsCompleteness};
    for (outcome, expected) in [
        (Outcome::Cancelled, "interrupted"),
        (Outcome::Failed, "failed"),
        (Outcome::Accepted, "unknown"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let store = open_store(directory.path());
        let f = Fixture::new("admission-terminal");
        let channel = fact_channel(&f, 256 * 1024);
        let writer = writer(&store);
        for (index, fact) in [
            admitted("turn-a", "branch-a", 1),
            super::super::support::route_decision(PLAN),
            ExecutionFactV1::RequestFinished {
                outcome,
                attempts_started: 0,
                attempts_finished: 0,
                accepted_attempt_ordinal: None,
                facts_completeness: FactsCompleteness::Complete,
            },
        ]
        .into_iter()
        .enumerate()
        {
            assert!(matches!(
                offer_fact(
                    &writer,
                    &channel,
                    f.fact(index as u64 + 1, fact, 1000 + index as i64)
                ),
                WriterCycleOutcome::Ack(_)
            ));
        }
        let page = store
            .observed_routing_timeline(
                &reader(),
                &RoutingTimelineQueryV1 {
                    session_id: f.session.to_string(),
                    after_watermark: None,
                    limit: 50,
                },
                5000,
            )
            .unwrap();
        assert_eq!(page.turns[0].status, expected);
        assert!(page.turns[0].segment_id.is_none() && page.turns[0].finished_at_ms.is_none());
    }
}
