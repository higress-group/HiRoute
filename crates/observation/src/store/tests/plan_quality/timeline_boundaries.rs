use super::*;
use hiroute_domain::{
    ComplexityDecisionSourceV1, LedgerReasonCodeV1, ReasonLedgerEntryV1, RoutingTimelineQueryV1,
    RoutingTurnBoundaryV1,
};

#[test]
fn timeline_boundaries_follow_fresh_gateway_continuity_not_group_changes() {
    use RoutingTurnBoundaryV1::{NewUserMessage, Unknown};
    let cases = [
        (None, false, NewUserMessage),
        (None, false, Unknown),
        (
            Some(LedgerReasonCodeV1::ContextHoldApplied),
            false,
            NewUserMessage,
        ),
        (
            Some(LedgerReasonCodeV1::ContextHoldInvalidated),
            false,
            NewUserMessage,
        ),
        (Some(LedgerReasonCodeV1::ContextHoldApplied), true, Unknown),
        (
            Some(LedgerReasonCodeV1::ContextHoldInvalidated),
            true,
            Unknown,
        ),
    ];
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let first = Fixture::new("boundary-first");
    let channel = fact_channel(&first, 256 * 1024);
    let writer = writer(&store);
    for (index, (hold, inherited, _)) in cases.iter().enumerate() {
        let mut fixture = Fixture::new(&format!("boundary-{index}"));
        fixture.session = first.session.clone();
        fixture.fact_stream = first.fact_stream.clone();
        let mut decision = super::super::support::route_decision(PLAN);
        let ExecutionFactV1::RouteDecision(route) = &mut decision else {
            unreachable!()
        };
        if *inherited {
            route.complexity.as_mut().unwrap().decision_source =
                ComplexityDecisionSourceV1::Inherited;
        }
        if let Some(code) = hold {
            route.reason_ledger.push(ReasonLedgerEntryV1 {
                ordinal: 1,
                code: *code,
                group_id: None,
            });
        }
        let start = 1000 + index as u64 * 100;
        for (offset, fact) in [
            decision,
            finished(
                &fixture,
                "stage",
                &format!("turn-{index}"),
                index as u64 + 1,
                "smart_saving",
                start,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            assert!(matches!(
                offer_fact(
                    &writer,
                    &channel,
                    fixture.fact(
                        index as u64 * 2 + offset as u64 + 1,
                        fact,
                        start as i64 + offset as i64
                    )
                ),
                WriterCycleOutcome::Ack(_)
            ));
        }
    }
    let query = RoutingTimelineQueryV1 {
        session_id: first.session.to_string(),
        after_watermark: None,
        limit: 50,
    };
    let page = store
        .observed_routing_timeline(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(page.turns.len(), cases.len());
    for (turn, (_, _, expected)) in page.turns.iter().zip(cases) {
        assert_eq!(turn.boundary, expected, "{}", turn.turn_id);
    }
    // This rule deliberately does not pretend to distinguish a client's automatically
    // appended background message from a human-authored message with identical evidence.
    assert_eq!(page.turns[2].boundary, NewUserMessage);
}

#[test]
fn timeline_retained_tail_is_not_the_first_session_turn() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("boundary-retained-tail");
    let channel = fact_channel(&fixture, 256 * 1024);
    assert!(matches!(
        offer_fact(
            &writer(&store),
            &channel,
            fixture.fact(
                1,
                finished(&fixture, "stage", "later-turn", 7, "smart_saving", 1000),
                1010
            )
        ),
        WriterCycleOutcome::Ack(_)
    ));
    let page = store
        .observed_routing_timeline(
            &reader(),
            &RoutingTimelineQueryV1 {
                session_id: fixture.session.to_string(),
                after_watermark: None,
                limit: 50,
            },
            5000,
        )
        .unwrap();
    assert_eq!(page.turns[0].boundary, RoutingTurnBoundaryV1::Unknown);
}
