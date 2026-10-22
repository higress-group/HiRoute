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

fn fact_digests(db: &rusqlite::Connection) -> Vec<String> {
    db.prepare("SELECT envelope_digest FROM execution_fact_events ORDER BY envelope_digest")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
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
