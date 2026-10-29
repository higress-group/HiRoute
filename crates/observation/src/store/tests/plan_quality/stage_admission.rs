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
        include_presentation: false,
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
        include_presentation: false,
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
                    include_presentation: false,
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
                    include_presentation: false,
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

#[test]
fn presentation_is_opt_in_and_transitions_follow_recorded_facts() {
    use super::super::support;
    use hiroute_domain::{AgentTurnAttributionV1, RoutingTimelineGroupStateV1 as State};
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let f = Fixture::new("presentation-admission");
    let channel = fact_channel(&f, 256 * 1024);
    let writer = writer(&store);
    assert!(matches!(
        offer_fact(
            &writer,
            &channel,
            f.fact(1, admitted("turn-a", "branch-a", 1), 1000)
        ),
        WriterCycleOutcome::Ack(_)
    ));
    let mut legacy: RoutingTimelineQueryV1 = serde_json::from_value(
        serde_json::json!({"session_id":f.session.to_string(),"after_watermark":null,"limit":50}),
    )
    .unwrap();
    assert!(!legacy.include_presentation);
    assert!(
        serde_json::to_value(&legacy)
            .unwrap()
            .get("include_presentation")
            .is_none()
    );
    let old = store
        .observed_routing_timeline(&reader(), &legacy, 5000)
        .unwrap();
    let old_json = serde_json::to_value(&old.turns[0]).unwrap();
    assert!(old_json.get("presentation").is_none());
    let old_id = old.turns[0].turn_id.clone();
    let mut rich = RoutingTimelineQueryV1 {
        include_presentation: true,
        ..legacy.clone()
    };
    let page = store
        .observed_routing_timeline(&reader(), &rich, 5000)
        .unwrap();
    let presentation = page.turns[0].presentation.as_ref().unwrap();
    assert_eq!(presentation.group_state, State::Pending);
    assert_eq!(
        presentation.execution_attribution,
        AgentTurnAttributionV1::Unknown
    );
    assert_eq!(presentation.request_models[0].model_configuration_id, None);
    assert_eq!(page.turns[0].turn_id, old_id);
    rich.after_watermark = Some(old.watermark.clone());
    assert!(matches!(
        store.observed_routing_timeline(&reader(), &rich, 5000),
        Err(crate::query_v2::ObservationV2Error::Stale)
    ));
    rich.after_watermark = Some(page.watermark.clone());
    legacy.after_watermark = Some(page.watermark);
    assert!(matches!(
        store.observed_routing_timeline(&reader(), &legacy, 5000),
        Err(crate::query_v2::ObservationV2Error::Stale)
    ));
    legacy.after_watermark = Some(old.watermark);
    assert!(
        store
            .observed_routing_timeline(&reader(), &legacy, 5000)
            .unwrap()
            .turns
            .is_empty()
    );
    assert!(matches!(
        offer_fact(
            &writer,
            &channel,
            f.fact(2, support::route_decision(PLAN), 1010)
        ),
        WriterCycleOutcome::Ack(_)
    ));
    let routed = store
        .observed_routing_timeline(&reader(), &rich, 5000)
        .unwrap();
    assert_eq!(routed.turns[0].turn_id, old_id);
    assert_eq!(
        routed.turns[0].presentation.as_ref().unwrap().group_state,
        State::Resolved
    );
    rich.after_watermark = Some(routed.watermark);
    let mut relay_candidate = support::candidate("model/relay");
    if let ExecutionFactV1::CandidateDecision(candidate) = &mut relay_candidate {
        candidate.candidate_id = "candidate-relay".into();
        candidate.stable_binding_id = "binding-relay".into();
    }
    for (sequence, candidate) in [support::candidate("model/actual"), relay_candidate]
        .into_iter()
        .enumerate()
    {
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                f.fact(sequence as u64 + 3, candidate, 1015)
            ),
            WriterCycleOutcome::Ack(_)
        ));
    }
    assert!(matches!(
        offer_fact(
            &writer,
            &channel,
            f.attempt_fact(
                5,
                "attempt/actual",
                support::attempt_started("model/actual"),
                1020,
                PLAN
            )
        ),
        WriterCycleOutcome::Ack(_)
    ));
    let actual = store
        .observed_routing_timeline(&reader(), &rich, 5000)
        .unwrap();
    let display = actual.turns[0].presentation.as_ref().unwrap();
    assert_eq!(
        display.execution_attribution,
        AgentTurnAttributionV1::Single
    );
    assert_eq!(
        display.request_models[0].model_configuration_id.as_deref(),
        Some("model/actual")
    );
    rich.after_watermark = Some(actual.watermark);
    let mut key_relay = support::attempt_started("model/actual");
    if let ExecutionFactV1::AttemptStarted {
        ordinal, key_id, ..
    } = &mut key_relay
    {
        *ordinal = 2;
        *key_id = "another-key".into();
    }
    assert!(matches!(
        offer_fact(
            &writer,
            &channel,
            f.attempt_fact(6, "attempt/key-relay", key_relay, 1030, PLAN)
        ),
        WriterCycleOutcome::Ack(_)
    ));
    let same = store
        .observed_routing_timeline(&reader(), &rich, 5000)
        .unwrap();
    assert_eq!(
        same.turns[0]
            .presentation
            .as_ref()
            .unwrap()
            .execution_attribution,
        AgentTurnAttributionV1::Single
    );
    rich.after_watermark = Some(same.watermark);
    let mut relay = support::attempt_started("model/relay");
    if let ExecutionFactV1::AttemptStarted {
        ordinal,
        candidate_id,
        stable_binding_id,
        ..
    } = &mut relay
    {
        *ordinal = 3;
        *candidate_id = "candidate-relay".into();
        *stable_binding_id = "binding-relay".into();
    }
    assert!(matches!(
        offer_fact(
            &writer,
            &channel,
            f.attempt_fact(7, "attempt/model-relay", relay, 1040, PLAN)
        ),
        WriterCycleOutcome::Ack(_)
    ));
    let mixed = store
        .observed_routing_timeline(&reader(), &rich, 5000)
        .unwrap();
    let display = mixed.turns[0].presentation.as_ref().unwrap();
    assert_eq!(display.group_state, State::Resolved);
    assert_eq!(display.execution_attribution, AgentTurnAttributionV1::Mixed);
    assert_eq!(
        display.request_models[0].model_configuration_id.as_deref(),
        Some("model/relay")
    );
    rich.after_watermark = Some(mixed.watermark);
    let mut accepted_attempt = support::attempt_finished();
    if let ExecutionFactV1::AttemptFinished(fact) = &mut accepted_attempt {
        fact.ordinal = 2;
    }
    assert!(matches!(
        offer_fact(
            &writer,
            &channel,
            f.attempt_fact(8, "attempt/key-relay", accepted_attempt, 1045, PLAN)
        ),
        WriterCycleOutcome::Ack(_)
    ));
    let finished = ExecutionFactV1::RequestFinished {
        outcome: hiroute_domain::ExecutionRequestOutcomeV1::Accepted,
        attempts_started: 3,
        attempts_finished: 1,
        accepted_attempt_ordinal: Some(2),
        facts_completeness: hiroute_domain::FactsCompleteness::Complete,
    };
    assert!(matches!(
        offer_fact(&writer, &channel, f.fact(9, finished, 1050)),
        WriterCycleOutcome::Ack(_)
    ));
    let accepted = store
        .observed_routing_timeline(&reader(), &rich, 5000)
        .unwrap();
    assert_eq!(
        accepted.turns[0]
            .presentation
            .as_ref()
            .unwrap()
            .request_models[0]
            .model_configuration_id
            .as_deref(),
        Some("model/actual")
    );
    legacy.after_watermark = None;
    let legacy_again = store
        .observed_routing_timeline(&reader(), &legacy, 5000)
        .unwrap();
    assert!(
        serde_json::to_value(&legacy_again.turns[0])
            .unwrap()
            .get("presentation")
            .is_none()
    );
    assert_eq!(legacy_again.turns[0].turn_id, old_id);
}

#[test]
fn missing_or_partial_terminal_evidence_is_not_pending() {
    use hiroute_domain::{AgentTurnAttributionV1, RoutingTimelineGroupStateV1 as State};
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let f = Fixture::new("presentation-finished");
    let channel = fact_channel(&f, 256 * 1024);
    let writer = writer(&store);
    let mut close = closed(&f, &f, 1, &[&f]);
    if let ExecutionFactV1::AgentTurnFinished {
        attribution,
        history_partial,
        executed_branch_id,
        model_configuration_id,
        profile_digest,
        ..
    } = &mut close
    {
        *attribution = AgentTurnAttributionV1::Mixed;
        *history_partial = true;
        *executed_branch_id = None;
        *model_configuration_id = None;
        *profile_digest = None;
    }
    for (n, fact) in [admitted("turn-a", "branch-a", 1), close]
        .into_iter()
        .enumerate()
    {
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                f.fact(n as u64 + 1, fact, 1000 + n as i64)
            ),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let query = RoutingTimelineQueryV1 {
        include_presentation: true,
        session_id: f.session.to_string(),
        after_watermark: None,
        limit: 50,
    };
    let page = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    let turn = &page.turns[0];
    assert!(turn.evidence_partial);
    assert_eq!(
        turn.presentation.as_ref().unwrap().execution_attribution,
        AgentTurnAttributionV1::Mixed
    );
    assert_eq!(
        turn.presentation.as_ref().unwrap().group_state,
        State::Mixed
    );
}

#[test]
fn presentation_modes_isolate_frozen_pages_and_incremental_updates() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let first = Fixture::new("presentation-page-a");
    let mut second = Fixture::new("presentation-page-b");
    second.session = first.session.clone();
    second.fact_stream = first.fact_stream.clone();
    let channel = fact_channel(&first, 256 * 1024);
    let writer = writer(&store);
    for (n, (f, stage)) in [(&first, "turn-a"), (&second, "turn-b")]
        .into_iter()
        .enumerate()
    {
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                f.fact(
                    n as u64 + 1,
                    admitted(stage, "branch-a", 1),
                    1000 + n as i64
                )
            ),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let mut legacy = RoutingTimelineQueryV1 {
        include_presentation: false,
        session_id: first.session.to_string(),
        after_watermark: None,
        limit: 1,
    };
    let mut rich = RoutingTimelineQueryV1 {
        include_presentation: true,
        ..legacy.clone()
    };
    let old_first = store
        .observed_routing_timeline(&reader(), &legacy, 5000)
        .unwrap();
    let rich_first = store
        .observed_routing_timeline(&reader(), &rich, 5000)
        .unwrap();
    assert!(!old_first.complete && !rich_first.complete);
    assert_eq!(old_first.turns[0].turn_id, rich_first.turns[0].turn_id);
    legacy.after_watermark = Some(rich_first.watermark.clone());
    assert!(matches!(
        store.observed_routing_timeline(&reader(), &legacy, 5000),
        Err(crate::query_v2::ObservationV2Error::Stale)
    ));
    legacy.after_watermark = Some(old_first.watermark);
    rich.after_watermark = Some(rich_first.watermark);
    let old_second = store
        .observed_routing_timeline(&reader(), &legacy, 5000)
        .unwrap();
    let rich_second = store
        .observed_routing_timeline(&reader(), &rich, 5000)
        .unwrap();
    assert!(old_second.complete && rich_second.complete);
    assert_eq!(old_second.turns[0].turn_id, rich_second.turns[0].turn_id);
    assert!(old_second.turns[0].presentation.is_none());
    assert!(rich_second.turns[0].presentation.is_some());
    assert!(matches!(
        offer_fact(
            &writer,
            &channel,
            first.fact(3, super::super::support::route_decision(PLAN), 1020)
        ),
        WriterCycleOutcome::Ack(_)
    ));
    legacy.after_watermark = Some(old_second.watermark);
    rich.after_watermark = Some(rich_second.watermark);
    let old_update = store
        .observed_routing_timeline(&reader(), &legacy, 5000)
        .unwrap();
    let rich_update = store
        .observed_routing_timeline(&reader(), &rich, 5000)
        .unwrap();
    assert_eq!(old_update.turns[0].turn_id, first.request.as_str());
    assert_eq!(rich_update.turns[0].turn_id, first.request.as_str());
    assert!(old_update.turns[0].presentation.is_none());
    assert!(rich_update.turns[0].presentation.is_some());
}

#[test]
fn frozen_baseline_timeline_output() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let f = Fixture::new("presentation-baseline");
    let channel = fact_channel(&f, 256 * 1024);
    let writer = writer(&store);
    assert!(matches!(
        offer_fact(
            &writer,
            &channel,
            f.fact(1, admitted("turn-a", "branch-a", 1), 1000)
        ),
        WriterCycleOutcome::Ack(_)
    ));
    let query = RoutingTimelineQueryV1 {
        include_presentation: false,
        session_id: f.session.to_string(),
        after_watermark: None,
        limit: 50,
    };
    let page = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(
        serde_json::to_string(&page.turns[0]).unwrap(),
        include_str!("../fixtures/routing-timeline-base-952917.json").trim_end()
    );
}
