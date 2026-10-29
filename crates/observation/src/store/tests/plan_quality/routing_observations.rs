use super::*;
use crate::store::tests::support::route_decision;
use hiroute_domain::*;

pub(super) fn observation(variant: u8) -> RoutingObservationV1 {
    let protocol = NativeProtocolV1::default();
    let mut j = JudgmentSettingsV1::default();
    protocol.stock_assessment(&mut j);
    let frozen = FrozenNativeProtocolV1::freeze(
        &protocol,
        j.degree.definition().into(),
        [(SMART_SAVING_SCOPE_ID.into(), j)],
    )
    .unwrap();
    let mut o = RoutingObservationV1::not_dispatched(&frozen, SMART_SAVING_SCOPE_ID);
    o.provider = "fixture".into();
    o.model = "fixture-model".into();
    o.simple_probability = Some(serde_json::json!(0.9).as_number().unwrap().clone());
    if variant == 0 {
        o.eligibility = ProtectionEligibilityV1::NoTarget;
        o.applicability_reason = ApplicabilityReasonV1::NoTarget;
        return o;
    }
    o.target_segment = Some("stage-source".into());
    if variant == 1 {
        o.eligibility = ProtectionEligibilityV1::PartialTarget;
        o.applicability_reason = ApplicabilityReasonV1::PartialTarget;
        return o;
    }
    o.target_complete = true;
    o.eligibility = ProtectionEligibilityV1::Eligible;
    o.score = Some(serde_json::json!(0.1).as_number().unwrap().clone());
    o.applicability_requested = variant != 5;
    if variant == 4 {
        o.applicability_state = ApplicabilityStateV1::Unavailable;
        o.applicability_reason = ApplicabilityReasonV1::Timeout;
        o.protection_action = HistoricalProtectionActionV1::RetainedUnavailable;
    } else if variant == 5 {
        o.applicability_reason = ApplicabilityReasonV1::RequiredQuestionMissing;
        o.protection_action = HistoricalProtectionActionV1::RetainedUnavailable;
    } else {
        o.applicability_state = ApplicabilityStateV1::Valid;
        o.applicability_reason = ApplicabilityReasonV1::Answered;
        o.applicability_probability =
            serde_json::Number::from_f64(if variant == 3 { 0.1 } else { 0.9 });
        o.protection_action = if variant == 3 {
            HistoricalProtectionActionV1::Suppressed
        } else {
            HistoricalProtectionActionV1::Applied
        };
        if variant == 7 {
            o.score = serde_json::Number::from_f64(0.5);
            o.protection_action = HistoricalProtectionActionV1::NotNeeded;
        }
    }
    assert!(o.validate());
    o
}

#[test]
fn routing_stats_use_all_decisions_and_preserve_safe_evidence_after_content_deletion() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("c-routing-statistics");
    let channel = fact_channel(&fixture, 1024 * 1024);
    let writer = writer(&store);
    for variant in 0..8 {
        let mut fact = route_decision(PLAN);
        let ExecutionFactV1::RouteDecision(decision) = &mut fact else {
            unreachable!()
        };
        let c = decision.complexity.as_mut().unwrap();
        c.decision_protocol = Some(DecisionProtocolV1::SystemOneC);
        c.routing_observation = Some(observation(variant));
        c.decision_source = if variant == 6 {
            ComplexityDecisionSourceV1::Inherited
        } else {
            ComplexityDecisionSourceV1::ExternalClassifier
        };
        c.complexity_score = None;
        c.threshold = None;
        c.classification_duration_micros = Some(1);
        let mut request = fixture.clone();
        if variant > 0 {
            request.request =
                LogicalRequestId::parse(format!("request-c-routing-{variant}")).unwrap();
        }
        let envelope = request.fact(u64::from(variant) + 1, fact, 1000 + i64::from(variant));
        let verdict = offer_fact(&writer, &channel, envelope.clone());
        assert!(
            matches!(verdict, WriterCycleOutcome::Ack(_)),
            "variant {variant}: {verdict:?}"
        );
        assert!(matches!(
            offer_fact(&writer, &channel, envelope),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let final_stage = fixture.fact(
        9,
        finished(
            &fixture,
            "stage-only-one",
            "turn-stage",
            1,
            SMART_SAVING_SCOPE_ID,
            1500,
        ),
        1500,
    );
    assert!(matches!(
        offer_fact(&writer, &channel, final_stage),
        WriterCycleOutcome::Ack(_)
    ));
    let mut q = query(Some(17));
    q.include_routing_observations = true;
    q.model_configuration_id = Some("does-not-match".into());
    q.limit = 1;
    let page = store
        .observed_plan_quality_samples(&reader(), &q, 5000)
        .unwrap();
    assert!(page.samples.is_empty());
    let report = page.routing_observations.unwrap();
    assert_eq!(report.coverage, "complete");
    assert_eq!(report.retained_decisions, 8);
    assert_eq!(report.cohorts.len(), 1);
    let c = &report.cohorts[0].counts;
    assert_eq!(
        (
            c.c_decisions,
            c.requested,
            c.unavailable,
            c.gate_opportunities,
            c.suppressed,
            c.inherited,
            c.required_question_missing
        ),
        (7, 4, 1, 4, 1, 1, 1)
    );
    assert_eq!((c.no_target_omissions, c.partial_target_omissions), (1, 1));
    assert_eq!(c.unavailable_per_requested, Some(0.25));
    assert_eq!(c.suppressed_per_opportunity, Some(0.25));
    assert_eq!(c.suppressed_per_c_decision, Some(1.0 / 7.0));
    let mut legacy_query = q.clone();
    legacy_query.include_routing_observations = false;
    let legacy = store
        .observed_plan_quality_samples(&reader(), &legacy_query, 5000)
        .unwrap();
    assert!(legacy.routing_observations.is_none());
    assert!(
        serde_json::to_value(&legacy)
            .unwrap()
            .get("routing_observations")
            .is_none()
    );
    let mut fq = ObservationFactsQueryV2 {
        include_routing_observations: true,
        request_id: fixture.request.clone(),
        limit: 1,
        cursor: None,
    };
    let facts = store.observed_facts(&reader(), &fq, 5000).unwrap();
    assert!(facts.facts[0].routing_observation.is_some());
    fq.cursor = facts.next_cursor;
    assert!(matches!(
        store.observed_facts(
            &reader(),
            &{
                let mut legacy = fq.clone();
                legacy.include_routing_observations = false;
                legacy
            },
            5000
        ),
        Err(crate::query_v2::ObservationV2Error::Stale)
    ));
    fq.cursor = None;
    let old = store
        .observed_facts(
            &reader(),
            &{
                let mut legacy = fq.clone();
                legacy.include_routing_observations = false;
                legacy
            },
            5000,
        )
        .unwrap();
    assert!(
        serde_json::to_value(&old.facts[0])
            .unwrap()
            .get("routing_observation")
            .is_none()
    );
    let principal = ObservationPrincipalV1::local_user(fixture.workspace.clone());
    let mut spec = SessionDeletionSpecV1 {
        workspace_id: fixture.workspace.clone(),
        session_id: fixture.session.clone(),
        data_class: DeletionDataClass::ContentOnly,
        delete_rollups: false,
    };
    let preview = store.preview_session_deletion(&principal, &spec).unwrap();
    store
        .apply_session_deletion(
            &principal,
            &spec,
            preview.store_revision,
            &preview.change_digest,
            2000,
        )
        .unwrap();
    assert_eq!(
        store
            .observed_plan_quality_samples(&reader(), &q, 5000)
            .unwrap()
            .routing_observations
            .unwrap(),
        report
    );
    let encoded: Vec<String> = {
        let connection = store.connection.lock();
        let mut stmt = connection
            .prepare("SELECT body_json FROM observation_safe_facts_v2")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert!(
        encoded
            .iter()
            .all(|s| !s.contains("Distinguish those meanings")
                && !s.contains("assessment_stage")
                && !s.contains("credentials"))
    );
    drop(writer);
    drop(store);
    let reopened = open_store(directory.path());
    assert_eq!(
        reopened
            .observed_plan_quality_samples(&reader(), &q, 5000)
            .unwrap()
            .routing_observations
            .unwrap(),
        report
    );
    spec.data_class = DeletionDataClass::FactsAndContent;
    let preview = reopened
        .preview_session_deletion(&principal, &spec)
        .unwrap();
    reopened
        .apply_session_deletion(
            &principal,
            &spec,
            preview.store_revision,
            &preview.change_digest,
            3000,
        )
        .unwrap();
    let report = reopened
        .observed_plan_quality_samples(&reader(), &q, 5000)
        .unwrap()
        .routing_observations
        .unwrap();
    assert_eq!(report.retained_decisions, 0);
    assert!(report.cohorts.is_empty());
}

#[test]
fn impossible_applicability_combinations_are_rejected() {
    let mut o = observation(4);
    o.applicability_probability = serde_json::Number::from_f64(0.3);
    assert!(!o.validate());
    let mut o = observation(3);
    o.eligibility = ProtectionEligibilityV1::DifferentPlan;
    assert!(!o.validate());
    let mut o = observation(0);
    o.protection_action = HistoricalProtectionActionV1::Suppressed;
    assert!(!o.validate());
}

#[test]
fn recorded_opportunities_include_complex_actions_but_not_old_or_inherited_decisions() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("mixed-protection-rules");
    let channel = fact_channel(&fixture, 1024 * 1024);
    let writer = writer(&store);
    for (index, (variant, probability, inherited)) in [
        (2, 0.9, false), // simple, applied
        (2, 0.7, false), // complex, applied
        (3, 0.7, false), // complex, suppressed
        (4, 0.7, false), // complex, retained after unavailable applicability
        (7, 0.7, false), // historical complex decision that did not enter the gate
        (2, 0.7, true),  // inherited duplicate
    ]
    .into_iter()
    .enumerate()
    {
        let mut o = observation(variant);
        o.simple_probability = serde_json::Number::from_f64(probability);
        if variant == 7 {
            o.score = serde_json::Number::from_f64(0.1);
        }
        assert!(o.validate(), "{o:?}");
        let mut fact = route_decision(PLAN);
        let ExecutionFactV1::RouteDecision(decision) = &mut fact else {
            unreachable!()
        };
        let selection = decision.complexity.as_mut().unwrap();
        selection.decision_protocol = Some(DecisionProtocolV1::SystemOneC);
        selection.routing_observation = Some(o);
        selection.simple_probability = serde_json::Number::from_f64(probability);
        selection.decision_source = if inherited {
            ComplexityDecisionSourceV1::Inherited
        } else {
            ComplexityDecisionSourceV1::ExternalClassifier
        };
        selection.complexity_score = None;
        selection.threshold = None;
        selection.classification_duration_micros = Some(1);
        let mut request = fixture.clone();
        request.request = LogicalRequestId::parse(format!("mixed-protection-{index}")).unwrap();
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                request.fact(index as u64 + 1, fact, 1000 + index as i64)
            ),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let mut q = query(Some(17));
    q.include_routing_observations = true;
    let summary = store
        .observed_plan_quality_samples(&reader(), &q, 5000)
        .unwrap()
        .routing_observations
        .unwrap();
    assert_eq!(summary.coverage, "complete");
    assert_eq!(summary.cohorts.len(), 1);
    let counts = &summary.cohorts[0].counts;
    assert_eq!(
        (
            counts.c_decisions,
            counts.gate_opportunities,
            counts.suppressed,
            counts.inherited
        ),
        (5, 4, 1, 1)
    );
    assert_eq!(counts.suppressed_per_opportunity, Some(0.25));
    assert_eq!(counts.suppressed_per_c_decision, Some(0.2));
}

#[test]
fn supported_v1_and_v2_observations_remain_readable_in_distinct_cohorts() {
    let baseline: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../domain/src/routing/fixtures/baseline-c-default-publication.json"
    ))
    .unwrap();
    let legacy: FrozenNativeProtocolV1 =
        serde_json::from_value(baseline["frozen"].clone()).unwrap();
    legacy.validate().unwrap();
    let mut old = observation(2);
    old.protocol = legacy.protocol.schema.clone();
    old.content_digest = legacy.digest.clone();
    old.bundle_digest = CanonicalDigest::of(&legacy.protocol.fixed).unwrap();
    let new = observation(3);
    assert_eq!(old.protocol, SYSTEM_ONE_C_V1);
    assert_eq!(new.protocol, SYSTEM_ONE_C_V2);
    assert!(old.validate() && new.validate());
    let old_bytes = serde_json::to_vec(&old).unwrap();

    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("versioned-c-observation");
    let channel = fact_channel(&fixture, 1024 * 1024);
    let writer = writer(&store);
    for (index, observation) in [old.clone(), new.clone()].into_iter().enumerate() {
        let mut fact = route_decision(PLAN);
        let ExecutionFactV1::RouteDecision(decision) = &mut fact else {
            unreachable!()
        };
        let selection = decision.complexity.as_mut().unwrap();
        selection.decision_protocol = Some(DecisionProtocolV1::SystemOneC);
        selection.routing_observation = Some(observation);
        selection.decision_source = ComplexityDecisionSourceV1::ExternalClassifier;
        selection.complexity_score = None;
        selection.threshold = None;
        selection.classification_duration_micros = Some(1);
        let mut request = fixture.clone();
        request.request = LogicalRequestId::parse(format!("versioned-c-{index}")).unwrap();
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                request.fact(index as u64 + 1, fact, 1000 + index as i64)
            ),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let mut q = query(Some(17));
    q.include_routing_observations = true;
    let summary = store
        .observed_plan_quality_samples(&reader(), &q, 5000)
        .unwrap()
        .routing_observations
        .unwrap();
    assert_eq!(summary.coverage, "complete");
    assert_eq!(summary.cohorts.len(), 2);
    assert!(
        summary
            .cohorts
            .iter()
            .any(|c| c.cohort.protocol == SYSTEM_ONE_C_V1
                && c.cohort.content_digest == old.content_digest)
    );
    assert!(
        summary
            .cohorts
            .iter()
            .any(|c| c.cohort.protocol == SYSTEM_ONE_C_V2
                && c.cohort.content_digest == new.content_digest)
    );
    let facts = store
        .observed_facts(
            &reader(),
            &ObservationFactsQueryV2 {
                include_routing_observations: true,
                request_id: LogicalRequestId::parse("versioned-c-0").unwrap(),
                limit: 10,
                cursor: None,
            },
            5000,
        )
        .unwrap();
    assert_eq!(
        serde_json::to_vec(
            &facts.facts[0]
                .routing_observation
                .as_ref()
                .unwrap()
                .observation
        )
        .unwrap(),
        old_bytes
    );
    let mut unsupported = new;
    unsupported.protocol = "hiroute.system-one-c/v99".into();
    assert!(!unsupported.validate());
}

#[test]
fn known_current_non_c_protocol_keeps_mixed_revision_rates_complete() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("c-mixed-retained-revisions");
    let channel = fact_channel(&fixture, 1024 * 1024);
    let writer = writer(&store);
    for (sequence, is_c) in [(1, true), (2, false)] {
        let mut request = fixture.clone();
        request.request = LogicalRequestId::parse(if is_c {
            "c-current"
        } else {
            "legacy-or-missing-c"
        })
        .unwrap();
        let mut fact = route_decision(PLAN);
        let ExecutionFactV1::RouteDecision(decision) = &mut fact else {
            unreachable!()
        };
        let complexity = decision.complexity.as_mut().unwrap();
        complexity.decision_protocol = Some(if is_c {
            DecisionProtocolV1::SystemOneC
        } else {
            DecisionProtocolV1::SystemOneLegacy
        });
        complexity.routing_observation = is_c.then(|| observation(3));
        let verdict = offer_fact(&writer, &channel, request.fact(sequence, fact, 1000));
        assert!(
            matches!(verdict, WriterCycleOutcome::Ack(_)),
            "sequence {sequence}: {verdict:?}"
        );
    }
    let mut q = query(None);
    q.include_routing_observations = true;
    let report = store
        .observed_plan_quality_samples(&reader(), &q, 5000)
        .unwrap()
        .routing_observations
        .unwrap();
    assert_eq!(report.retained_decisions, 2);
    assert_eq!(
        report.unknown_scope_decisions, 0,
        "both decisions have known Plan identity"
    );
    assert_eq!(report.known_non_c_decisions, 1);
    assert_eq!(report.legacy_or_unknown_decisions, 0);
    assert_eq!(report.coverage, "complete");
    assert_eq!(report.cohorts.len(), 1);
    let c = &report.cohorts[0].counts;
    assert_eq!(
        (
            c.c_decisions,
            c.requested,
            c.gate_opportunities,
            c.suppressed
        ),
        (1, 1, 1, 1)
    );
    assert_eq!(c.suppressed_per_opportunity, Some(1.0));
    assert_eq!(c.unavailable_per_requested, Some(0.0));
    assert_eq!(c.suppressed_per_c_decision, Some(1.0));
}

#[test]
fn unmarked_historical_projection_with_known_plan_still_withholds_rates() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("unmarked-protocol");
    let channel = fact_channel(&fixture, 1024 * 1024);
    let writer = writer(&store);
    for (sequence, is_c) in [(1, true), (2, false)] {
        let mut request = fixture.clone();
        request.request = LogicalRequestId::parse(format!("unmarked-request-{sequence}")).unwrap();
        let mut fact = route_decision(PLAN);
        let ExecutionFactV1::RouteDecision(route) = &mut fact else {
            unreachable!()
        };
        let decision = route.complexity.as_mut().unwrap();
        if is_c {
            decision.decision_protocol = Some(DecisionProtocolV1::SystemOneC);
            decision.routing_observation = Some(observation(3));
        }
        assert!(matches!(
            offer_fact(&writer, &channel, request.fact(sequence, fact, 1000)),
            WriterCycleOutcome::Ack(_)
        ));
    }
    // Emulate only an older derived safe projection; immutable producer events stay intact.
    store.connection.lock().execute("UPDATE observation_safe_facts_v2 SET body_json=json_remove(body_json,'$.decision_protocol') WHERE json_extract(body_json,'$.decision_protocol')='local_rules'", []).unwrap();
    let mut q = query(None);
    q.include_routing_observations = true;
    let summary = store
        .observed_plan_quality_samples(&reader(), &q, 5000)
        .unwrap()
        .routing_observations
        .unwrap();
    assert_eq!(
        (
            summary.retained_decisions,
            summary.known_non_c_decisions,
            summary.legacy_or_unknown_decisions,
            summary.unknown_scope_decisions
        ),
        (2, 0, 1, 0)
    );
    assert_eq!(summary.coverage, "partial");
    let counts = &summary.cohorts[0].counts;
    assert_eq!(
        (
            counts.requested,
            counts.gate_opportunities,
            counts.suppressed
        ),
        (1, 1, 1)
    );
    assert_eq!(counts.suppressed_per_opportunity, None);
}
