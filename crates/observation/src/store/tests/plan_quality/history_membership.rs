use super::*;
use hiroute_domain::{HistoryMembershipV1, HistoryRelationV1, RoutingTimelineQueryV1};

fn membership(branch: &str, revision: u64, requests: &[&Fixture]) -> HistoryMembershipV1 {
    HistoryMembershipV1 {
        version: 1,
        branch_id: branch.into(),
        revision,
        request_ids: requests.iter().map(|f| f.request.to_string()).collect(),
        clipped: false,
        relation: HistoryRelationV1::ExactPrefix,
        source_unverified: true,
        association_unverified: false,
        capture_partial: false,
    }
}

#[test]
fn interleaved_branches_use_exact_membership_and_late_execution_cannot_extend_a_score() {
    let directory = tempfile::tempdir().unwrap();
    let first = Fixture::new("membership-a1");
    let mut other = Fixture::new("membership-b1");
    let mut later = Fixture::new("membership-a2");
    for f in [&mut other, &mut later] {
        f.session = first.session.clone();
        f.fact_stream = first.fact_stream.clone();
    }
    let query = RoutingTimelineQueryV1 {
        session_id: first.session.to_string(),
        after_watermark: None,
        limit: 50,
    };
    let preserved_digests;
    {
        let store = open_store(directory.path());
        let channel = fact_channel(&first, 256 * 1024);
        let writer = writer(&store);
        let mut sequence = 0;
        let mut write = |fixture: &Fixture, fact: ExecutionFactV1, at: i64| {
            sequence += 1;
            assert!(matches!(
                offer_fact(&writer, &channel, fixture.fact(sequence, fact, at)),
                WriterCycleOutcome::Ack(_)
            ));
        };
        for (f, at) in [(&first, 1000), (&other, 1100), (&later, 1200)] {
            write(f, super::super::support::route_decision(PLAN), at);
        }
        let mut a = finished(&first, "stage-a", "turn-a", 1, "smart_saving", 1000);
        if let ExecutionFactV1::AgentTurnFinished {
            history_membership, ..
        } = &mut a
        {
            *history_membership = Some(membership("branch-a", 1, &[&first]));
        }
        write(&first, a, 1300);
        let mut score = assessment(&other, "stage-a", ("turn-a", "turn-a", 1, 1), 1400, 0.4);
        if let ExecutionFactV1::BranchAssessmentRecorded {
            history_membership, ..
        } = &mut score
        {
            *history_membership = Some(membership("branch-a", 1, &[&first]));
        }
        write(&other, score, 1400);
        let before = store
            .observed_routing_timeline(&reader(), &query, 5000)
            .unwrap();
        assert!(
            before
                .turns
                .iter()
                .any(|t| t.agent_turn_id.as_deref() == Some("turn-a")
                    && t.assessment.as_ref().is_some_and(|s| s.score == 0.4))
        );
        // The second A delivery finished later than B but belongs to A only.
        let mut a = finished(&later, "stage-a", "turn-a", 1, "smart_saving", 1000);
        if let ExecutionFactV1::AgentTurnFinished {
            history_membership,
            first_request_id,
            last_request_id,
            finished_at_ms,
            ..
        } = &mut a
        {
            *history_membership = Some(membership("branch-a", 2, &[&first, &later]));
            *first_request_id = Some(first.request.clone());
            *last_request_id = Some(later.request.clone());
            *finished_at_ms = 1500;
        }
        write(&later, a, 1500);
        let mut b = finished(&other, "stage-b", "turn-b", 1, "smart_saving", 1100);
        if let ExecutionFactV1::AgentTurnFinished {
            history_membership, ..
        } = &mut b
        {
            *history_membership = Some(membership("branch-b", 1, &[&other]));
        }
        write(&other, b, 1600);
        let rows = store.connection.lock().prepare("SELECT request_id,agent_turn_id,membership_conflict FROM observation_analysis_turns_v1 ORDER BY request_id").unwrap()
            .query_map([], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?, r.get::<_,i64>(2)?))).unwrap()
            .collect::<Result<Vec<_>,_>>().unwrap();
        assert_eq!(rows.len(), 3);
        for (id, turn, conflict) in rows {
            assert_eq!(
                turn,
                if id == other.request.as_str() {
                    "turn-b"
                } else {
                    "turn-a"
                }
            );
            assert_eq!(
                conflict, 0,
                "adding a same-branch member is not a range conflict"
            );
        }
        let db = store.connection.lock();
        preserved_digests = fact_digests(&db);
        // Simulate the previous disposable projection. Current facts must
        // rebuild exact memberships instead of trusting old ordinal ranges.
        db.execute("UPDATE observation_analysis_turns_v1 SET agent_turn_id='old-range-join',membership_conflict=1", []).unwrap();
        db.execute(
            "DELETE FROM observation_analysis_meta_v1 WHERE key='turn_membership_version'",
            [],
        )
        .unwrap();
    }
    // Reopening must preserve source facts and the frozen target boundary.
    let store = open_store(directory.path());
    assert_eq!(fact_digests(&store.connection.lock()), preserved_digests);
    let page = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(page.turns.len(), 2);
    let a = page
        .turns
        .iter()
        .find(|t| t.agent_turn_id.as_deref() == Some("turn-a"))
        .unwrap();
    assert_eq!(a.requests.len(), 2);
    assert!(
        a.requests
            .iter()
            .all(|r| r.request_id != other.request.as_str())
    );
    assert!(
        a.assessment.is_none(),
        "the old score never assessed the later A delivery"
    );
    let b = page
        .turns
        .iter()
        .find(|t| t.agent_turn_id.as_deref() == Some("turn-b"))
        .unwrap();
    assert_eq!(b.requests.len(), 1);
    assert_eq!(b.requests[0].request_id, other.request.as_str());
    let samples = store
        .observed_plan_quality_samples(&reader(), &super::query(Some(17)), 5000)
        .unwrap();
    assert_eq!(
        samples.summary.scored_stage_count, 0,
        "the old score does not cover the enlarged stage"
    );
    let scored = samples
        .samples
        .iter()
        .find(|s| s.segment_id == "stage-a")
        .unwrap();
    assert!(scored.assessment.as_ref().unwrap().partial);
    let db = store.connection.lock();
    let earlier = crate::query_v2::routing_reason(
        &db,
        first.workspace.as_str(),
        first.session.as_str(),
        first.request.as_str(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        earlier.assessment.unwrap().score,
        0.4,
        "the earlier score remains available"
    );
    let late = crate::query_v2::routing_reason(
        &db,
        first.workspace.as_str(),
        first.session.as_str(),
        later.request.as_str(),
    )
    .unwrap()
    .unwrap();
    assert!(
        late.assessment.is_none(),
        "a later same-turn request never borrows an earlier score"
    );
    let unchanged: (String, i64) = db.query_row("SELECT assessment_membership_json,assessment_partial FROM plan_quality_segments WHERE segment_id='stage-a'", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(
        unchanged.1, 0,
        "computed coverage must not rewrite the original assessment"
    );
    assert!(!unchanged.0.contains(later.request.as_str()));
    assert_eq!(fact_digests(&db), preserved_digests);
}

pub(super) fn fact_digests(db: &rusqlite::Connection) -> Vec<String> {
    db.prepare("SELECT envelope_digest FROM execution_fact_events ORDER BY envelope_digest")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn partial_or_clipped_current_membership_cannot_reuse_a_frozen_full_score() {
    for (clipped, capture_partial) in [(true, false), (false, true), (true, true)] {
        let directory = tempfile::tempdir().unwrap();
        let mut members: Vec<_> = (0..128)
            .map(|i| Fixture::new(&format!("bounded-member-{i}")))
            .collect();
        let first_session = members[0].session.clone();
        let fact_stream = members[0].fact_stream.clone();
        let mut later = Fixture::new("bounded-member-129");
        for member in members.iter_mut().chain(std::iter::once(&mut later)) {
            member.session = first_session.clone();
            member.fact_stream = fact_stream.clone();
        }
        let refs = members.iter().collect::<Vec<_>>();
        let first = refs[0];
        let preserved;
        {
            let store = open_store(directory.path());
            let channel = fact_channel(first, 256 * 1024);
            let writer = writer(&store);
            let mut sequence = 0;
            let mut write = |fixture: &Fixture, fact: ExecutionFactV1, at| {
                sequence += 1;
                assert!(matches!(
                    offer_fact(&writer, &channel, fixture.fact(sequence, fact, at)),
                    WriterCycleOutcome::Ack(_)
                ));
            };
            for member in refs.iter().copied().chain(std::iter::once(&later)) {
                write(member, super::super::support::route_decision(PLAN), 1000);
            }
            let frozen = membership("bounded-branch", 1, &refs);
            let mut closed = finished(
                first,
                "bounded-stage",
                "bounded-turn",
                1,
                "smart_saving",
                1000,
            );
            if let ExecutionFactV1::AgentTurnFinished {
                history_membership,
                last_request_id,
                ..
            } = &mut closed
            {
                *history_membership = Some(frozen.clone());
                *last_request_id = Some(refs[127].request.clone());
            }
            write(first, closed, 1300);
            let mut scored = assessment(
                &later,
                "bounded-stage",
                ("bounded-turn", "bounded-turn", 1, 1),
                1400,
                0.75,
            );
            if let ExecutionFactV1::BranchAssessmentRecorded {
                history_membership, ..
            } = &mut scored
            {
                *history_membership = Some(frozen.clone());
            }
            write(&later, scored, 1400);
            let before = store
                .observed_plan_quality_samples(&reader(), &super::query(Some(17)), 5000)
                .unwrap();
            assert_eq!(
                before.summary.scored_stage_count, 1,
                "complete observed-source membership remains eligible"
            );
            assert!(!before.samples[0].assessment.as_ref().unwrap().partial);

            let mut current = frozen;
            current.revision = 2;
            current.clipped = clipped;
            current.capture_partial = capture_partial;
            let mut closed = finished(
                &later,
                "bounded-stage",
                "bounded-turn",
                1,
                "smart_saving",
                1000,
            );
            if let ExecutionFactV1::AgentTurnFinished {
                history_membership,
                history_partial,
                first_request_id,
                finished_at_ms,
                status,
                ..
            } = &mut closed
            {
                *history_membership = Some(current);
                *history_partial = true;
                *first_request_id = Some(first.request.clone());
                *finished_at_ms = 1500;
                *status = AgentTurnStatusV1::Unknown;
            }
            // No later C is available. Its absence must not let the old score
            // certify unseen or incomplete execution as fully assessed.
            write(&later, closed, 1500);
            preserved = fact_digests(&store.connection.lock());
        }
        let store = open_store(directory.path());
        let after = store
            .observed_plan_quality_samples(&reader(), &super::query(Some(17)), 5000)
            .unwrap();
        assert_eq!(
            after.summary.scored_stage_count, 0,
            "{clipped}/{capture_partial}"
        );
        assert!(after.samples[0].assessment.as_ref().unwrap().partial);
        let db = store.connection.lock();
        let original: (f64, i64, String) = db.query_row(
            "SELECT score,assessment_partial,assessment_membership_json FROM plan_quality_segments WHERE segment_id='bounded-stage'", [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!((original.0, original.1), (0.75, 0));
        let original: HistoryMembershipV1 = serde_json::from_str(&original.2).unwrap();
        assert_eq!(original.request_ids.len(), 128);
        assert!(!original.request_ids.contains(&later.request.to_string()));
        assert!(!original.clipped && !original.capture_partial);
        assert_eq!(fact_digests(&db), preserved);
    }
}

#[test]
fn pre_membership_format_roundtrips_unchanged_and_cannot_claim_new_membership() {
    let fixture = Fixture::new("pre-membership-format");
    let mut envelope = fixture.fact(
        1,
        finished(&fixture, "stage", "turn", 1, "smart_saving", 1000),
        1010,
    );
    envelope.schema_digest = CanonicalDigest::parse(
        "sha256:806871b6b7a3c1a563e2e214e9af5f0a0b18d1f704e093494c2e1b2316b08069",
    )
    .unwrap();
    // Synthetic old-format regression; released producer evidence has its own fixture.
    let before = serde_json::to_vec(&envelope).unwrap();
    let digest = CanonicalDigest::of(&envelope).unwrap();
    let mut decoded: hiroute_domain::ExecutionFactEnvelopeV1 =
        serde_json::from_slice(&before).unwrap();
    decoded.validate_persisted_contract().unwrap();
    assert!(decoded.validate().is_err());
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), before);
    assert_eq!(CanonicalDigest::of(&decoded).unwrap(), digest);
    if let ExecutionFactV1::AgentTurnFinished {
        history_membership, ..
    } = &mut decoded.fact
    {
        *history_membership = Some(membership("branch", 1, &[&fixture]));
    }
    assert!(decoded.validate_persisted_contract().is_err());
}

#[test]
fn captured_pre_membership_producer_bytes_keep_their_authenticated_storage_identity() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/captured-pre-membership-facts.json"
    ))
    .unwrap();
    assert_eq!(
        fixture["source_revision"],
        "8d1863e0db1219e1a983d394a052504f744711fa"
    );
    assert_eq!(
        fixture["source_artifact_sha256"],
        "8b4bfec3f77814afca208740a7b049c710f4b2ccf2a532c6735cddcec1da0951"
    );
    let facts = fixture["facts"].as_array().unwrap();
    assert_eq!(facts.len(), 2);
    for captured in facts {
        let bytes = captured["envelope_json"].as_str().unwrap();
        let envelope: hiroute_domain::ExecutionFactEnvelopeV1 =
            serde_json::from_str(bytes).unwrap();
        envelope.validate_persisted_contract().unwrap();
        assert!(
            envelope.validate().is_err(),
            "an older authenticated format is storage-only"
        );
        assert_eq!(serde_json::to_string(&envelope).unwrap(), bytes);
        assert_eq!(
            CanonicalDigest::of(&envelope).unwrap().as_str(),
            captured["envelope_digest"].as_str().unwrap()
        );
        match envelope.fact {
            ExecutionFactV1::AgentTurnFinished {
                history_membership, ..
            }
            | ExecutionFactV1::BranchAssessmentRecorded {
                history_membership, ..
            } => assert!(history_membership.is_none()),
            _ => panic!("both changed storage fact types require old-producer evidence"),
        }
    }
}
