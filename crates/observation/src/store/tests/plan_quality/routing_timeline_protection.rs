use super::*;
use crate::store::tests::support::route_decision;
use hiroute_domain::*;

#[test]
fn timeline_preserves_opening_protection_and_keeps_later_scores_separate() {
    for (variant, complex_release) in [(2, false), (3, false), (4, false), (7, false), (3, true)] {
        let directory = tempfile::tempdir().unwrap();
        let store = open_store(directory.path());
        let current = Fixture::new(&format!("timeline-protection-{variant}"));
        let mut previous = current.clone();
        previous.request = LogicalRequestId::parse(format!("previous-{variant}")).unwrap();
        let channel = fact_channel(&current, 1024 * 1024);
        let writer = writer(&store);
        let mut observation = super::routing_observations::observation(variant);
        if complex_release {
            observation.simple_probability = serde_json::Number::from_f64(0.7);
        }
        let mut route = route_decision(PLAN);
        let ExecutionFactV1::RouteDecision(decision) = &mut route else {
            unreachable!()
        };
        let selection = decision.complexity.as_mut().unwrap();
        selection.decision_protocol = Some(DecisionProtocolV1::SystemOneC);
        selection.routing_observation = Some(observation.clone());
        selection.simple_probability = observation.simple_probability.clone();
        selection.simple_threshold_millis = Some(observation.simple_threshold_millis);
        selection.complexity_score = None;
        selection.threshold = None;
        selection.decision_source = ComplexityDecisionSourceV1::ExternalClassifier;
        selection.classification_duration_micros = Some(1);
        let protected = matches!(
            observation.protection_action,
            HistoricalProtectionActionV1::Applied
                | HistoricalProtectionActionV1::RetainedUnavailable
        );
        selection.execution_group = if protected || complex_release {
            ExecutionGroupV1::Primary
        } else {
            ExecutionGroupV1::Regular
        };
        selection.selection_reason = if protected {
            ModelGroupReasonV1::LowCompetence
        } else if complex_release {
            ModelGroupReasonV1::ComplexTask
        } else {
            ModelGroupReasonV1::SimpleTask
        };
        if protected {
            selection.competence_trigger = Some(CompetenceProtectionV1 {
                segment_id: "stage-source".into(),
                score: observation.score.clone().unwrap(),
                floor_millis: observation.competence_floor_millis,
                from_group: ExecutionGroupV1::Regular,
            });
        }
        let original_selection = serde_json::to_value(&selection).unwrap();
        let previous_score = observation.score.as_ref().unwrap().as_f64().unwrap();
        let mut continuation = current.clone();
        continuation.request = LogicalRequestId::parse(format!("continuation-{variant}")).unwrap();
        let mut inherited = route.clone();
        if let ExecutionFactV1::RouteDecision(decision) = &mut inherited {
            let selection = decision.complexity.as_mut().unwrap();
            selection.decision_source = ComplexityDecisionSourceV1::Inherited;
            selection
                .reason_codes
                .push(ComplexityReasonCodeV1::InheritedToolContinuation);
            decision.reason_ledger.push(ReasonLedgerEntryV1 {
                ordinal: 1,
                code: LedgerReasonCodeV1::ContextHoldApplied,
                group_id: None,
            });
        }
        let mut close = finished(
            &current,
            "current-stage",
            "current-turn",
            2,
            SMART_SAVING_SCOPE_ID,
            1200,
        );
        if let ExecutionFactV1::AgentTurnFinished {
            last_request_id, ..
        } = &mut close
        {
            *last_request_id = Some(continuation.request.clone());
        }
        let facts = [
            previous.fact(
                1,
                finished(
                    &previous,
                    "stage-source",
                    "previous-turn",
                    1,
                    SMART_SAVING_SCOPE_ID,
                    1000,
                ),
                1010,
            ),
            previous.fact(
                2,
                assessment(
                    &previous,
                    "stage-source",
                    ("previous-turn", "previous-turn", 1, 1),
                    1100,
                    previous_score,
                ),
                1100,
            ),
            current.fact(3, route, 1200),
            continuation.fact(4, inherited, 1205),
            current.fact(5, close, 1210),
            current.fact(
                6,
                assessment(
                    &current,
                    "current-stage",
                    ("current-turn", "current-turn", 2, 2),
                    1300,
                    0.95,
                ),
                1300,
            ),
        ];
        for fact in facts {
            assert!(matches!(
                offer_fact(&writer, &channel, fact),
                WriterCycleOutcome::Ack(_)
            ));
        }
        let query = RoutingTimelineQueryV1 {
            include_presentation: false,
            session_id: current.session.to_string(),
            after_watermark: None,
            limit: 50,
        };
        let page = store
            .observed_routing_timeline(&reader(), &query, 5000)
            .unwrap();
        let turn = page
            .turns
            .iter()
            .find(|turn| turn.turn_id == current.request.as_str())
            .unwrap();
        assert_eq!(
            page.turns.len(),
            2,
            "a continuation stays inside the opening turn"
        );
        assert_eq!(turn.requests.len(), 2);
        assert_eq!(turn.tool_continuations, 1);
        let selection = turn.selection.as_ref().unwrap();
        let history = selection.historical_protection.as_ref().unwrap();
        if complex_release {
            assert_eq!(selection.execution_group, ExecutionGroupV1::Primary);
            assert_eq!(selection.selection_reason, ModelGroupReasonV1::ComplexTask);
            assert!(selection.competence_trigger.is_none());
        }
        assert_eq!(history.outcome, observation.protection_action);
        assert_eq!(history.previous_score, observation.score);
        assert_eq!(
            history.applicability_probability,
            observation.applicability_probability
        );
        assert_eq!(
            history.competence_floor_millis,
            observation.competence_floor_millis
        );
        assert_eq!(turn.assessment.as_ref().unwrap().score, 0.95);
        let mut serialized = serde_json::to_value(selection).unwrap();
        serialized
            .as_object_mut()
            .unwrap()
            .remove("historical_protection");
        assert_eq!(
            serialized, original_selection,
            "existing selection fields keep their wire meaning"
        );
        let encoded = serde_json::to_value(turn).unwrap();
        let decoded: RoutingTimelineTurnV1 = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.selection.as_ref().unwrap(), selection);
    }
}

#[test]
fn timeline_tool_continuations_require_recorded_tool_inheritance() {
    use ComplexityDecisionSourceV1::{ExternalClassifier, Inherited};
    use ComplexityReasonCodeV1::{InheritedTaskRoot, InheritedToolContinuation};
    let cases = [
        (ExternalClassifier, None, true, 0),
        (Inherited, Some(InheritedTaskRoot), true, 0),
        (Inherited, None, true, 0),
        (Inherited, Some(InheritedToolContinuation), false, 1),
        (Inherited, Some(InheritedToolContinuation), true, 1),
    ];
    for (index, (source, reason, held, expected)) in cases.into_iter().enumerate() {
        let directory = tempfile::tempdir().unwrap();
        let fixture = Fixture::new(&format!("tool-continuation-{index}"));
        let query = RoutingTimelineQueryV1 {
            include_presentation: false,
            session_id: fixture.session.to_string(),
            after_watermark: None,
            limit: 50,
        };
        {
            let store = open_store(directory.path());
            let channel = fact_channel(&fixture, 256 * 1024);
            let writer = writer(&store);
            let mut route = route_decision(PLAN);
            let ExecutionFactV1::RouteDecision(decision) = &mut route else {
                unreachable!()
            };
            let selection = decision.complexity.as_mut().unwrap();
            selection.decision_source = source;
            selection.decision_protocol = Some(DecisionProtocolV1::SystemOneC);
            selection.complexity_score = None;
            selection.threshold = None;
            selection.classification_duration_micros = Some(1);
            selection.reason_codes = reason.into_iter().collect();
            if held {
                decision.reason_ledger.push(ReasonLedgerEntryV1 {
                    ordinal: 1,
                    code: LedgerReasonCodeV1::ContextHoldApplied,
                    group_id: None,
                });
            }
            for (sequence, fact) in [
                route,
                finished(&fixture, "stage", "turn", 1, SMART_SAVING_SCOPE_ID, 1000),
            ]
            .into_iter()
            .enumerate()
            {
                let verdict = offer_fact(
                    &writer,
                    &channel,
                    fixture.fact(sequence as u64 + 1, fact, 1000 + sequence as i64),
                );
                assert!(matches!(verdict, WriterCycleOutcome::Ack(_)), "{verdict:?}");
                let page = store
                    .observed_routing_timeline(&reader(), &query, 5000)
                    .unwrap();
                assert_eq!(page.turns.len(), 1);
                assert_eq!(page.turns[0].requests.len(), 1);
                assert_eq!(
                    page.turns[0].tool_continuations,
                    expected,
                    "case {index}, closing fact recorded: {}",
                    sequence == 1
                );
            }
        }
        let reopened = open_store(directory.path());
        let page = reopened
            .observed_routing_timeline(&reader(), &query, 5000)
            .unwrap();
        assert_eq!(page.turns[0].tool_continuations, expected);
        assert_eq!(page.turns[0].status, "completed");
    }
}

#[test]
fn selections_without_historical_observations_remain_unrecorded() {
    let ExecutionFactV1::RouteDecision(decision) = route_decision(PLAN) else {
        unreachable!()
    };
    let original = decision.complexity.unwrap();
    let view = RoutingTimelineSelectionV1::from(original.clone());
    assert!(view.historical_protection.is_none());
    let bytes = serde_json::to_value(&view).unwrap();
    assert!(bytes.get("historical_protection").is_none());
    assert_eq!(bytes, serde_json::to_value(original).unwrap());
    let decoded: RoutingTimelineSelectionV1 = serde_json::from_value(bytes).unwrap();
    assert!(decoded.historical_protection.is_none());
}
