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
        include_presentation: false,
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
                include_presentation: false,
                session_id: fixture.session.to_string(),
                after_watermark: None,
                limit: 50,
            },
            5000,
        )
        .unwrap();
    assert_eq!(page.turns[0].boundary, RoutingTurnBoundaryV1::Unknown);
}

#[test]
fn recorded_opening_cause_overrides_legacy_ledger_inference_and_survives_reopen() {
    use RoutingTurnBoundaryV1::{ContextRebuilt, NewUserMessage, Unknown};
    use hiroute_domain::AgentTurnOpeningCauseV1 as Cause;
    let directory = tempfile::tempdir().unwrap();
    let first = Fixture::new("recorded-causes");
    let cases = [
        (Cause::NoComparableHistory, NewUserMessage),
        (Cause::HistoryShortened, ContextRebuilt),
        (Cause::HistoryRewrite, Unknown),
        (Cause::UserMessage, NewUserMessage),
        (Cause::NoComparableHistory, Unknown),
        (Cause::Other, Unknown),
    ];
    {
        let store = open_store(directory.path());
        let channel = fact_channel(&first, 256 * 1024);
        for (index, (cause, _)) in cases.iter().enumerate() {
            let mut fixture = Fixture::new(&format!("cause-{index}"));
            fixture.session = first.session.clone();
            fixture.fact_stream = first.fact_stream.clone();
            let start = 1000 + index as u64 * 100;
            let mut fact = finished(
                &fixture,
                "stage",
                &format!("turn-{index}"),
                index as u64 + 1,
                "smart_saving",
                start,
            );
            let ExecutionFactV1::AgentTurnFinished { opening_cause, .. } = &mut fact else {
                unreachable!()
            };
            *opening_cause = Some(*cause);
            assert!(matches!(
                offer_fact(
                    &writer(&store),
                    &channel,
                    fixture.fact(index as u64 + 1, fact, start as i64)
                ),
                WriterCycleOutcome::Ack(_)
            ));
        }
    }
    let store = open_store(directory.path());
    let page = store
        .observed_routing_timeline(
            &reader(),
            &RoutingTimelineQueryV1 {
                include_presentation: false,
                session_id: first.session.to_string(),
                after_watermark: None,
                limit: 50,
            },
            5000,
        )
        .unwrap();
    assert_eq!(
        page.turns
            .iter()
            .map(|turn| turn.boundary)
            .collect::<Vec<_>>(),
        cases
            .iter()
            .map(|(_, boundary)| *boundary)
            .collect::<Vec<_>>()
    );
}

#[test]
fn pre_opening_facts_keep_canonical_bytes_and_cannot_authenticate_new_causes() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("pre-opening");
    let mut event = fixture.fact(
        1,
        finished(&fixture, "stage", "turn", 1, "smart_saving", 1000),
        1010,
    );
    // The retained pre-opening producer contract predates this optional field.
    event.schema_digest = hiroute_domain::CanonicalDigest::parse(
        "sha256:7ff01a649de96b1f62e069b872b23609d64a3734b5ad42c337324d3eb7cfc9f2",
    )
    .unwrap();
    let bytes = serde_json::to_value(&event).unwrap();
    assert!(bytes["fact"].get("opening_cause").is_none());
    let digest = hiroute_domain::CanonicalDigest::of(&event).unwrap();
    let decoded: hiroute_domain::ExecutionFactEnvelopeV1 =
        serde_json::from_value(bytes.clone()).unwrap();
    decoded.validate_persisted_contract().unwrap();
    assert!(decoded.validate().is_err());
    assert_eq!(serde_json::to_value(&decoded).unwrap(), bytes);
    assert_eq!(
        hiroute_domain::CanonicalDigest::of(&decoded).unwrap(),
        digest
    );
    {
        let store = open_store(directory.path());
        let mut connection = store.connection.lock();
        let transaction = connection.transaction().unwrap();
        crate::store::fact_log::insert(&transaction, &decoded, &digest).unwrap();
        crate::receipt::apply_fact(&transaction, &decoded).unwrap();
        transaction.commit().unwrap();
    }
    let store = open_store(directory.path());
    let page = store
        .observed_routing_timeline(
            &reader(),
            &RoutingTimelineQueryV1 {
                include_presentation: false,
                session_id: fixture.session.to_string(),
                after_watermark: None,
                limit: 50,
            },
            5000,
        )
        .unwrap();
    assert_eq!(
        page.turns[0].boundary,
        RoutingTurnBoundaryV1::NewUserMessage
    );
    let ExecutionFactV1::AgentTurnFinished { opening_cause, .. } = &mut event.fact else {
        unreachable!()
    };
    *opening_cause = Some(hiroute_domain::AgentTurnOpeningCauseV1::HistoryRewrite);
    assert!(event.validate_persisted_contract().is_err());
}

#[test]
fn unsupported_execution_digest_is_rejected_without_rewriting_retained_facts() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("unsupported-execution");
    let mut event = fixture.fact(
        1,
        finished(&fixture, "stage", "turn", 2, "smart_saving", 1000),
        1010,
    );
    event.schema_digest = hiroute_domain::CanonicalDigest::parse(
        "sha256:5f7a8a65779fea426fbe0b247e2691ff893de0f638c3a42f4408f5345b9f5bf6",
    )
    .unwrap();
    let ExecutionFactV1::AgentTurnFinished { opening_cause, .. } = &mut event.fact else {
        unreachable!()
    };
    *opening_cause = Some(hiroute_domain::AgentTurnOpeningCauseV1::HistoryRewrite);
    assert!(event.validate_persisted_contract().is_err());
    assert!(event.validate().is_err());
    let digest = hiroute_domain::CanonicalDigest::of(&event).unwrap();
    {
        let store = open_store(directory.path());
        let mut connection = store.connection.lock();
        let transaction = connection.transaction().unwrap();
        crate::store::fact_log::insert(&transaction, &event, &digest).unwrap();
        assert!(matches!(
            crate::receipt::apply_fact(&transaction, &event),
            Err(crate::receipt::FactProjectionError::Corrupt),
        ));
        transaction.commit().unwrap();
    }
    let store = open_store(directory.path());
    let before: (String, String, String) = store
        .connection
        .lock()
        .query_row(
            "SELECT envelope_json,envelope_digest,body_json FROM execution_fact_events
             JOIN observation_sensitive_payloads_v2 ON id='fact:' || envelope_digest",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(before.2, serde_json::to_string(&event).unwrap());
    assert!(
        matches!(
            crate::store::fact_log::load_request(
                &store.connection.lock(),
                &event.correlation.workspace_id,
                &event.correlation.request_id,
            ),
            Err(crate::writer::ObservationStoreError::Corrupt),
        ),
        "the authenticated fact reader must reject an unsupported historical digest"
    );
    let after: (String, String, String) = store
        .connection
        .lock()
        .query_row(
            "SELECT envelope_json,envelope_digest,body_json FROM execution_fact_events
             JOIN observation_sensitive_payloads_v2 ON id='fact:' || envelope_digest",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(before, after);
}

#[test]
fn pre_extension_facts_keep_bytes_without_authenticating_new_decision_fields() {
    use hiroute_domain::{DecisionExtensionIdentityV1, DecisionProtocolV1, ModelGroupReasonV1};
    let fixture = Fixture::new("pre-extension");
    let mut event = fixture.fact(1, super::super::support::route_decision(PLAN), 1010);
    event.schema_digest = hiroute_domain::CanonicalDigest::parse(
        "sha256:806871b6b7a3c1a563e2e214e9af5f0a0b18d1f704e093494c2e1b2316b08069",
    )
    .unwrap();
    let original = serde_json::to_value(&event).unwrap();
    let digest = hiroute_domain::CanonicalDigest::of(&event).unwrap();
    let decoded: hiroute_domain::ExecutionFactEnvelopeV1 =
        serde_json::from_value(original.clone()).unwrap();
    decoded.validate_persisted_contract().unwrap();
    assert!(decoded.validate().is_err());
    assert_eq!(serde_json::to_value(&decoded).unwrap(), original);
    assert_eq!(
        hiroute_domain::CanonicalDigest::of(&decoded).unwrap(),
        digest
    );
    for field in 0..3 {
        let mut changed = decoded.clone();
        let ExecutionFactV1::RouteDecision(route) = &mut changed.fact else {
            unreachable!()
        };
        let selection = route.complexity.as_mut().unwrap();
        selection.decision_protocol = Some(DecisionProtocolV1::CustomExtensionV1);
        match field {
            0 => selection.extension_reason = Some("Extension chose regular".into()),
            1 => selection.selection_reason = ModelGroupReasonV1::ExtensionDecision,
            _ => {
                selection.policy = Some(
                    DecisionExtensionIdentityV1 {
                        id: "extension/test".into(),
                        revision: 1,
                    }
                    .execution_policy("Custom"),
                )
            }
        }
        assert!(changed.validate_persisted_contract().is_err());
        changed.schema_digest =
            hiroute_domain::CanonicalDigest::parse(hiroute_domain::EXECUTION_FACT_PORT_DIGEST_V2)
                .unwrap();
        changed.validate().unwrap();
    }
}
