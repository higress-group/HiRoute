use super::support::*;
use crate::WriterCycleOutcome;
use hiroute_domain::{CompletenessDeltaV1, ExecutionFactV1, FactsCompleteness};

#[test]
fn long_session_appends_match_full_projection_without_reloading_history() {
    for delta in [None, Some(CompletenessDeltaV1::Unknown)] {
        let temp = tempfile::tempdir().unwrap();
        let store = open_store(temp.path());
        let writer = writer(&store);
        let session = Fixture::new("long-session").session;
        for index in 0..40 {
            let mut fixture = Fixture::new(&format!("long-{index}"));
            fixture.session = session.clone();
            let channel = fact_channel(&fixture, 512 * 1024);
            let attempt = format!("attempt-long-{index}");
            let mut facts = request_facts(&fixture, &attempt, finished(&attempt), 100 + index);
            // Exercise both envelope Unknown and terminal Partial contributions.
            if index == 3 {
                facts[0].completeness_delta = delta;
            }
            if index == 25
                && let ExecutionFactV1::RequestFinished {
                    facts_completeness, ..
                } = &mut facts.last_mut().unwrap().fact
            {
                *facts_completeness = FactsCompleteness::Partial;
            }
            let before = crate::store::fact_log::SESSION_LOADS.with(|n| n.get());
            for fact in facts {
                let outcome = offer_fact(&writer, &channel, fact);
                assert!(matches!(outcome, WriterCycleOutcome::Ack(_)), "{outcome:?}");
            }
            assert_eq!(
                crate::store::fact_log::SESSION_LOADS.with(|n| n.get()),
                before,
                "ordinary ingestion must not hydrate older requests"
            );
            let connection = store.connection.lock();
            let rebuilt = crate::receipt::session_facts_completeness(
                &connection,
                &fixture.workspace,
                &session,
            )
            .unwrap();
            let projected: String = connection.query_row(
                "SELECT facts_completeness FROM sessions WHERE workspace_id=?1 AND session_id=?2",
                rusqlite::params![fixture.workspace.as_str(), session.as_str()], |row| row.get(0),
            ).unwrap();
            assert_eq!(
                serde_json::to_value(rebuilt).unwrap().as_str().unwrap(),
                projected
            );
        }
    }
}

#[test]
fn retention_removes_old_partial_contribution_before_next_append() {
    let temp = tempfile::tempdir().unwrap();
    let store = open_store(temp.path());
    let writer = writer(&store);
    let session = Fixture::new("retained-session").session;
    for (name, timestamp) in [
        ("old-partial", 100),
        ("fresh-complete", hiroute_domain::SEVEN_DAYS_MILLIS),
    ] {
        let mut fixture = Fixture::new(name);
        fixture.session = session.clone();
        let channel = fact_channel(&fixture, 512 * 1024);
        let mut facts = request_facts(&fixture, name, finished(name), timestamp);
        if name == "old-partial"
            && let ExecutionFactV1::RequestFinished {
                facts_completeness, ..
            } = &mut facts.last_mut().unwrap().fact
        {
            *facts_completeness = FactsCompleteness::Partial;
        }
        for fact in facts {
            assert!(matches!(
                offer_fact(&writer, &channel, fact),
                WriterCycleOutcome::Ack(_)
            ));
        }
    }
    assert_eq!(
        store
            .expire_request_details(hiroute_domain::SEVEN_DAYS_MILLIS + 1000, 100)
            .unwrap(),
        1
    );
    let actual: String = store
        .connection
        .lock()
        .query_row(
            "SELECT facts_completeness FROM sessions WHERE session_id=?1",
            [session.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(actual, "complete");
}
