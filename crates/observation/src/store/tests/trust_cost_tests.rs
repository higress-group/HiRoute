use super::*;

#[path = "trust_cost_auxiliary_tests.rs"]
mod auxiliary;

#[path = "trust_cost_purpose_work_tests.rs"]
mod purpose_work;

fn trial_request(
    store: &LocalObservationStore,
    suffix: &str,
    revision: u64,
    group: ExecutionGroupV1,
) -> Fixture {
    trial_request_with(store, suffix, revision, group, |_| {})
}

fn trial_request_with(
    store: &LocalObservationStore,
    suffix: &str,
    revision: u64,
    group: ExecutionGroupV1,
    edit: impl FnOnce(&mut ExecutionFactV1),
) -> Fixture {
    trial_request_with_facts(store, suffix, revision, group, edit, |_| {})
}

fn trial_request_with_facts(
    store: &LocalObservationStore,
    suffix: &str,
    revision: u64,
    group: ExecutionGroupV1,
    edit: impl FnOnce(&mut ExecutionFactV1),
    edit_fact: impl Fn(&mut ExecutionFactV1),
) -> Fixture {
    let f = Fixture::new(suffix);
    let mut all = facts(&f, UsageFrameKindV1::Cumulative);
    let mut terminal = all.pop().unwrap();
    for mut e in all {
        edit_fact(&mut e.fact);
        store.ingest_fact(&e, &[]).unwrap();
    }
    let mut turn = trial_turn(&f, revision, group);
    edit(&mut turn);
    assert!(matches!(
        store.ingest_fact(&f.fact(10, turn, 110), &[]).unwrap(),
        IngestOutcome::Ack(_)
    ));
    terminal.sequence = 11;
    terminal.event_id = EventId::parse("fact-event-11").unwrap();
    terminal.occurred_at_unix_nanos = 111_000_000;
    assert!(matches!(
        store.ingest_fact(&terminal, &[]).unwrap(),
        IngestOutcome::Ack(_)
    ));
    store.settle_pending_valuations(16).unwrap();
    f
}

#[test]
fn trial_shows_full_routed_estimate_when_only_reference_cache_write_is_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let f = trial_request_with_facts(
        &store,
        "unknown-cache-write",
        17,
        ExecutionGroupV1::Regular,
        |_| {},
        |fact| {
            if let ExecutionFactV1::UsageAndCache {
                cache_write_tokens,
                cache_write_provenance,
                ..
            } = fact
            {
                *cache_write_tokens = None;
                *cache_write_provenance = UsageProvenanceV1::Unknown;
            }
        },
    );
    let frozen = serde_json::to_value(latest(&store, &f)).unwrap();
    let scope = TrustCostScopeV1 {
        plan_id: "plan/codex-daily".into(),
        revision: Some(17),
        branch_id: Some("smart_saving".into()),
        group: ExecutionGroupV1::Regular,
        from_ms: 0,
        to_ms: 1000,
        currency: "USD".into(),
    };
    // The source bills cache creation at the same rate as uncached input.
    // The reference's different creation rate needs the unreported split.
    let actual = current_price("actual", "model-a", 1_000_000);
    let mut reference = current_price("reference", "model-ref", 4_000_000);
    reference.quote.rates.cache_write = TokenRateV1::known(5_000_000);
    reference.quote.quote_digest = reference.quote.computed_digest().unwrap();
    let report = store
        .observed_trust_cost(
            &reader(),
            &scope,
            &reference,
            std::slice::from_ref(&actual),
            1000,
        )
        .unwrap();
    assert_eq!(
        (report.request_count, report.compared_request_count),
        (1, 0)
    );
    assert_eq!(report.coverage, ObservationMetricCoverageV2::Unknown);
    assert_eq!(report.routed_estimate_micros, Some(125));
    assert_eq!(report.reference_estimate_micros, None);
    assert_eq!(report.estimated_savings_micros, None);
    assert_eq!(report.compared_routed_subtotal_micros, None);
    assert_eq!(
        report.same_usage_bounds,
        Some(ApiEquivalentBoundsV1 {
            reference_min_micros: 500,
            reference_max_micros: 560,
            difference_min_micros: -435,
            difference_max_micros: -375,
        })
    );
    assert_eq!(serde_json::to_value(latest(&store, &f)).unwrap(), frozen);

    // Changing the source price so that creation differs makes the source
    // amount unknown too; no unreported usage is rewritten as zero.
    let mut unequal = actual.clone();
    unequal.quote.rates.cache_write = TokenRateV1::known(2_000_000);
    unequal.quote.quote_digest = unequal.quote.computed_digest().unwrap();
    let unknown = store
        .observed_trust_cost(&reader(), &scope, &reference, &[unequal], 1000)
        .unwrap();
    assert_eq!(unknown.routed_estimate_micros, None);
    assert_eq!(unknown.same_usage_bounds, None);

    // If both quotes permit equal-rate aggregation the paired difference
    // recovers without changing the captured execution.
    reference.quote.rates.cache_write = TokenRateV1::known(4_000_000);
    reference.quote.quote_digest = reference.quote.computed_digest().unwrap();
    let recovered = store
        .observed_trust_cost(&reader(), &scope, &reference, &[actual], 1000)
        .unwrap();
    assert_eq!(recovered.coverage, ObservationMetricCoverageV2::Complete);
    assert_eq!(recovered.estimated_savings_micros, Some(375));
    assert_eq!(recovered.same_usage_bounds, None);
    assert_eq!(serde_json::to_value(latest(&store, &f)).unwrap(), frozen);
}

fn trial_turn(f: &Fixture, revision: u64, group: ExecutionGroupV1) -> ExecutionFactV1 {
    let suffix = f.request.as_str();
    ExecutionFactV1::AgentTurnFinished {
        history_membership: None,
        opening_cause: None,
        branch_execution: Some(BranchExecutionV1 {
            policy: BranchExecutionPolicyV1 {
                name: "Trial".into(),
                extension: None,
                floor_millis: Some(500),
                criteria_digest: None,
            },
            group,
            candidate_index: 0,
        }),
        agent_turn_id: format!("turn-{suffix}"),
        segment_id: format!("stage-{suffix}"),
        ordinal: 1,
        plan_id: AgentPlanId::parse("plan/codex-daily").unwrap(),
        plan_revision: revision,
        selected_branch_id: "smart_saving".into(),
        executed_branch_id: Some("smart_saving".into()),
        model_configuration_id: Some("model-a".into()),
        profile_digest: Some(ObservedProfileDigestV1::from(CanonicalDigest::of_bytes(
            b"profile",
        ))),
        attribution: AgentTurnAttributionV1::Single,
        started_at_ms: 101,
        finished_at_ms: 110,
        status: AgentTurnStatusV1::Completed,
        history_partial: false,
        first_request_id: Some(f.request.clone()),
        last_request_id: Some(f.request.clone()),
    }
}
#[test]
fn live_trust_cost_accepts_current_millisecond_window() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    trial_request(&store, "current-window", 17, ExecutionGroupV1::Regular);
    let mut scope = TrustCostScopeV1 {
        plan_id: "plan/codex-daily".into(),
        revision: Some(17),
        branch_id: Some("smart_saving".into()),
        group: ExecutionGroupV1::Regular,
        from_ms: 0,
        to_ms: 1001,
        currency: "USD".into(),
    };
    let prices = [current_price("actual", "model-a", 1_000_000)];
    let reference = current_price("reference", "model-ref", 4_000_000);
    let report = store
        .observed_trust_cost(&reader(), &scope, &reference, &prices, 1000)
        .unwrap();
    assert_eq!(report.request_count, 1);
    assert_eq!(report.coverage, ObservationMetricCoverageV2::Complete);
    scope.to_ms = 1002;
    assert!(matches!(
        store.observed_trust_cost(&reader(), &scope, &reference, &prices, 1000),
        Err(crate::query_v2::ObservationV2Error::Invalid)
    ));
}

#[test]
fn trial_comparison_scopes_revision_and_group_and_keeps_missing_prices_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    trial_request(&store, "regular-current", 17, ExecutionGroupV1::Regular);
    trial_request(&store, "primary-current", 17, ExecutionGroupV1::Primary);
    trial_request(&store, "regular-old", 16, ExecutionGroupV1::Regular);
    let scope = TrustCostScopeV1 {
        plan_id: "plan/codex-daily".into(),
        revision: Some(17),
        branch_id: Some("smart_saving".into()),
        group: ExecutionGroupV1::Regular,
        from_ms: 0,
        to_ms: 1000,
        currency: "USD".into(),
    };
    let actual = current_price("actual", "model-a", 1_000_000);
    let reference = current_price("reference", "model-ref", 4_000_000);
    let report = store
        .observed_trust_cost(
            &reader(),
            &scope,
            &reference,
            std::slice::from_ref(&actual),
            1000,
        )
        .unwrap();
    assert_eq!(report.request_count, 1);
    assert_eq!(report.coverage, ObservationMetricCoverageV2::Complete);
    assert_eq!(
        report.reference_estimate_micros,
        report.routed_estimate_micros.map(|n| n * 4)
    );
    let mut all = scope.clone();
    all.revision = None;
    assert_eq!(
        store
            .observed_trust_cost(&reader(), &all, &reference, &[actual], 1000)
            .unwrap()
            .request_count,
        2
    );
    let expensive = current_price("actual", "model-a", 8_000_000);
    let extra = store
        .observed_trust_cost(&reader(), &scope, &reference, &[expensive], 1000)
        .unwrap();
    assert!(extra.estimated_savings_micros.unwrap() < 0);
    let unknown = store
        .observed_trust_cost(&reader(), &scope, &reference, &[], 1000)
        .unwrap();
    assert_eq!(unknown.missing_request_count, 1);
    assert_eq!(unknown.routed_estimate_micros, None);
    assert_eq!(unknown.estimated_savings_micros, None);
    let f = ingest(&store, "unassigned");
    assert!(
        store
            .observed_trust_cost(&reader(), &scope, &reference, &[], 1000)
            .is_err(),
        "unassigned requests cannot silently vanish from the denominator"
    );
    assert!(
        store
            .connection
            .lock()
            .execute(
                "DELETE FROM logical_requests WHERE workspace_id=?1 AND request_id=?2",
                rusqlite::params![f.workspace.as_str(), f.request.as_str()]
            )
            .is_ok()
    );
}

fn emit_late_turn(
    store: &LocalObservationStore,
    target: &Fixture,
    suffix: &str,
    turn: ExecutionFactV1,
    at: i64,
) {
    // A subsequent request carries delayed turn evidence. Finalized request
    // streams admit late usage only; their receipts must stay immutable.
    let mut carrier = Fixture::new(suffix);
    carrier.session = target.session.clone();
    let mut route = facts(&carrier, UsageFrameKindV1::Cumulative).remove(0);
    route.occurred_at_unix_nanos = at as u64 * 1_000_000;
    let outcome = store.ingest_fact(&route, &[]).unwrap();
    assert!(matches!(outcome, IngestOutcome::Ack(_)), "{outcome:?}");
    let outcome = store
        .ingest_fact(&carrier.fact(2, turn, at + 1), &[])
        .unwrap();
    assert!(matches!(outcome, IngestOutcome::Ack(_)), "{outcome:?}");
    store.settle_pending_valuations(16).unwrap();
}

fn trial_report(
    store: &LocalObservationStore,
    now: i64,
) -> Result<ApiEquivalentValueReportV1, crate::query_v2::ObservationV2Error> {
    let reader = ObservationReaderContext::local_user(
        WorkspaceId::default(),
        "trial-cost".into(),
        1,
        now + 1000,
        false,
        false,
    )
    .unwrap();
    store.observed_trust_cost(
        &reader,
        &TrustCostScopeV1 {
            plan_id: "plan/codex-daily".into(),
            revision: Some(17),
            branch_id: Some("smart_saving".into()),
            group: ExecutionGroupV1::Regular,
            from_ms: 0,
            to_ms: 1000,
            currency: "USD".into(),
        },
        &current_price("reference", "model-ref", 4_000_000),
        &[current_price("actual", "model-a", 1_000_000)],
        now,
    )
}

#[test]
fn review_trial_cost_partial_capture_preserves_proven_membership() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    trial_request(&store, "normal", 17, ExecutionGroupV1::Regular);
    trial_request_with(
        &store,
        "after-interruption",
        17,
        ExecutionGroupV1::Regular,
        |turn| {
            let ExecutionFactV1::AgentTurnFinished {
                history_partial, ..
            } = turn
            else {
                unreachable!()
            };
            *history_partial = true;
        },
    );
    let report = trial_report(&store, 1000).unwrap();
    assert_eq!(report.request_count, 2);
    assert_eq!(report.compared_request_count, 2);
    assert_eq!(report.coverage, ObservationMetricCoverageV2::Complete);
    assert_eq!(report.excluded_request_count, 0);
}

#[test]
fn review_trial_cost_mixed_attribution_is_a_counted_gap() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    trial_request(&store, "normal", 17, ExecutionGroupV1::Regular);
    trial_request_with(&store, "mixed", 17, ExecutionGroupV1::Regular, |turn| {
        let ExecutionFactV1::AgentTurnFinished {
            attribution,
            branch_execution,
            executed_branch_id,
            model_configuration_id,
            profile_digest,
            history_partial,
            ..
        } = turn
        else {
            unreachable!()
        };
        *attribution = AgentTurnAttributionV1::Mixed;
        *branch_execution = None;
        *executed_branch_id = None;
        *model_configuration_id = None;
        *profile_digest = None;
        *history_partial = true;
    });
    let report = trial_report(&store, 1000).unwrap();
    assert_eq!(
        (report.request_count, report.compared_request_count),
        (2, 1)
    );
    assert_eq!(
        (report.missing_request_count, report.excluded_request_count),
        (1, 1)
    );
    assert_eq!(report.coverage, ObservationMetricCoverageV2::Partial);
    assert_eq!(report.estimated_savings_micros, None);
    assert!(report.compared_routed_subtotal_micros.is_some());
}

#[test]
fn review_trial_cost_unknown_only_is_unknown_and_never_zero() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    trial_request_with(&store, "unknown", 17, ExecutionGroupV1::Regular, |turn| {
        let ExecutionFactV1::AgentTurnFinished {
            attribution,
            branch_execution,
            executed_branch_id,
            model_configuration_id,
            profile_digest,
            history_partial,
            ..
        } = turn
        else {
            unreachable!()
        };
        *attribution = AgentTurnAttributionV1::Unknown;
        *branch_execution = None;
        *executed_branch_id = None;
        *model_configuration_id = None;
        *profile_digest = None;
        *history_partial = true;
    });
    let report = trial_report(&store, 1000).unwrap();
    assert_eq!(
        (report.request_count, report.compared_request_count),
        (1, 0)
    );
    assert_eq!(
        (report.missing_request_count, report.excluded_request_count),
        (1, 1)
    );
    assert_eq!(report.coverage, ObservationMetricCoverageV2::Unknown);
    assert_eq!(report.estimated_savings_micros, None);
    assert_eq!(report.compared_routed_subtotal_micros, None);
    assert_eq!(report.compared_reference_subtotal_micros, None);
}

#[test]
fn review_trial_cost_conflicting_membership_is_not_partial_history() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    trial_request(&store, "normal", 17, ExecutionGroupV1::Regular);
    let f = trial_request(&store, "conflict", 16, ExecutionGroupV1::Regular);
    let mut second = trial_turn(&f, 17, ExecutionGroupV1::Regular);
    let ExecutionFactV1::AgentTurnFinished {
        agent_turn_id,
        segment_id,
        finished_at_ms,
        ..
    } = &mut second
    else {
        unreachable!()
    };
    *agent_turn_id = "conflicting-turn".into();
    *segment_id = "conflicting-stage".into();
    *finished_at_ms = 2001;
    emit_late_turn(&store, &f, "conflict-carrier", second, 2000);
    let report = trial_report(&store, 3000).unwrap();
    assert_eq!(
        (report.request_count, report.compared_request_count),
        (2, 1)
    );
    assert_eq!(
        report.excluded_request_count, 1,
        "an apparent old revision cannot hide a membership conflict"
    );
    assert_eq!(report.coverage, ObservationMetricCoverageV2::Partial);
    assert_eq!(report.estimated_savings_micros, None);
}

#[test]
fn review_trial_cost_scopes_readiness_and_waits_for_real_projection_catchup() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let normal = trial_request(&store, "normal", 17, ExecutionGroupV1::Regular);
    let old = trial_request_with(
        &store,
        "old-partial",
        16,
        ExecutionGroupV1::Regular,
        |turn| {
            let ExecutionFactV1::AgentTurnFinished {
                history_partial, ..
            } = turn
            else {
                unreachable!()
            };
            *history_partial = true;
        },
    );
    let primary = trial_request(&store, "primary", 17, ExecutionGroupV1::Primary);
    // Exercise the notifications an unaware, supported core writer leaves.
    for f in [&old, &primary] {
        store
            .connection
            .lock()
            .execute(
                "UPDATE logical_requests SET outcome=outcome WHERE request_id=?1",
                [f.request.as_str()],
            )
            .unwrap();
    }
    assert_eq!(
        trial_report(&store, 1000).unwrap().compared_request_count,
        1
    );
    store
        .connection
        .lock()
        .execute(
            "UPDATE logical_requests SET outcome=outcome WHERE request_id=?1",
            [normal.request.as_str()],
        )
        .unwrap();
    assert!(matches!(
        trial_report(&store, 1000),
        Err(crate::query_v2::ObservationV2Error::Unavailable)
    ));
    drop(store);
    let store = open_store(dir.path());
    store.settle_pending_valuations(16).unwrap();
    assert_eq!(
        trial_report(&store, 1000).unwrap().compared_request_count,
        1
    );
    store
        .connection
        .lock()
        .execute(
            "DELETE FROM observation_analysis_requests_v1 WHERE request_id=?1",
            [normal.request.as_str()],
        )
        .unwrap();
    assert!(matches!(
        trial_report(&store, 1000),
        Err(crate::query_v2::ObservationV2Error::Unavailable)
    ));
}

#[test]
fn review_trial_cost_rebuilds_ambiguous_old_turn_projection() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    trial_request_with(
        &store,
        "history-partial",
        17,
        ExecutionGroupV1::Regular,
        |turn| {
            let ExecutionFactV1::AgentTurnFinished {
                history_partial, ..
            } = turn
            else {
                unreachable!()
            };
            *history_partial = true;
        },
    );
    let f = trial_request(&store, "old-conflict", 16, ExecutionGroupV1::Regular);
    let mut conflicting = trial_turn(&f, 17, ExecutionGroupV1::Regular);
    let ExecutionFactV1::AgentTurnFinished {
        agent_turn_id,
        segment_id,
        finished_at_ms,
        ..
    } = &mut conflicting
    else {
        unreachable!()
    };
    *agent_turn_id = "migration-conflicting-turn".into();
    *segment_id = "migration-conflicting-stage".into();
    *finished_at_ms = 2001;
    emit_late_turn(&store, &f, "migration-carrier", conflicting, 2000);
    store
        .connection
        .lock()
        .execute_batch("ALTER TABLE observation_analysis_turns_v1 DROP COLUMN membership_conflict")
        .unwrap();
    drop(store);
    let store = open_store(dir.path());
    store.settle_pending_valuations(16).unwrap();
    let report = trial_report(&store, 3000).unwrap();
    assert_eq!(
        (
            report.request_count,
            report.compared_request_count,
            report.excluded_request_count
        ),
        (2, 1, 1)
    );
    assert_eq!(report.coverage, ObservationMetricCoverageV2::Partial);
    drop(store);
    let store = open_store(dir.path());
    assert_eq!(
        trial_report(&store, 3000).unwrap().compared_request_count,
        1
    );
}

#[test]
fn review_trial_cost_interrupted_request_remains_missing_with_known_stage() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    trial_request(
        &store,
        "normal-after-interruption",
        17,
        ExecutionGroupV1::Regular,
    );
    let f = Fixture::new("interrupted-prefix");
    let mut prefix = facts(&f, UsageFrameKindV1::Cumulative);
    prefix.pop(); // Client drop has no drained RequestFinished fact.
    for event in prefix {
        store.ingest_fact(&event, &[]).unwrap();
    }
    let mut turn = trial_turn(&f, 17, ExecutionGroupV1::Regular);
    let ExecutionFactV1::AgentTurnFinished {
        history_partial,
        status,
        ..
    } = &mut turn
    else {
        unreachable!()
    };
    *history_partial = true;
    *status = AgentTurnStatusV1::Interrupted;
    let outcome = store.ingest_fact(&f.fact(10, turn, 110), &[]).unwrap();
    assert!(matches!(outcome, IngestOutcome::Ack(_)), "{outcome:?}");
    store.settle_pending_valuations(16).unwrap();
    let report = trial_report(&store, 1000).unwrap();
    assert_eq!(
        (
            report.request_count,
            report.compared_request_count,
            report.missing_request_count
        ),
        (2, 1, 1)
    );
    assert_eq!(
        report.excluded_request_count, 0,
        "stage identity is known; only request completeness is missing"
    );
    assert_eq!(report.coverage, ObservationMetricCoverageV2::Partial);
    assert_eq!(report.estimated_savings_micros, None);
    assert!(report.compared_routed_subtotal_micros.is_some());
}

#[test]
fn review_trial_cost_unclosed_turn_ages_into_gap_and_late_evidence_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    trial_request(&store, "normal", 17, ExecutionGroupV1::Regular);
    let f = ingest(&store, "unclosed");
    let updated: i64 = store
        .connection
        .lock()
        .query_row(
            "SELECT updated_at_ms FROM sessions WHERE workspace_id=?1 AND session_id=?2",
            rusqlite::params![f.workspace.as_str(), f.session.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    let boundary = updated + 300_000;
    assert!(matches!(
        trial_report(&store, boundary - 1),
        Err(crate::query_v2::ObservationV2Error::Unavailable)
    ));
    let report = trial_report(&store, boundary).unwrap();
    assert_eq!(
        (report.request_count, report.compared_request_count),
        (2, 1)
    );
    assert_eq!(
        (report.missing_request_count, report.excluded_request_count),
        (1, 1)
    );
    assert_eq!(report.coverage, ObservationMetricCoverageV2::Partial);
    assert_eq!(report.routed_estimate_micros, None);
    assert_eq!(report.reference_estimate_micros, None);
    assert_eq!(report.estimated_savings_micros, None);
    drop(store);
    let store = open_store(dir.path());
    assert_eq!(
        trial_report(&store, boundary)
            .unwrap()
            .excluded_request_count,
        1
    );
    let frozen = serde_json::to_value(latest(&store, &f)).unwrap();
    let mut late_turn = trial_turn(&f, 17, ExecutionGroupV1::Regular);
    let ExecutionFactV1::AgentTurnFinished { finished_at_ms, .. } = &mut late_turn else {
        unreachable!()
    };
    *finished_at_ms = (boundary + 2) as u64;
    emit_late_turn(&store, &f, "late-close-carrier", late_turn, boundary + 1);
    let recovered = trial_report(&store, boundary + 2).unwrap();
    assert_eq!(
        serde_json::to_value(latest(&store, &f)).unwrap(),
        frozen,
        "late turn evidence must not reprice the finalized request"
    );
    assert_eq!(
        (recovered.request_count, recovered.compared_request_count),
        (2, 2)
    );
    assert_eq!(
        (
            recovered.missing_request_count,
            recovered.excluded_request_count
        ),
        (0, 0)
    );
    assert_eq!(recovered.coverage, ObservationMetricCoverageV2::Complete);
    drop(store);
    let store = open_store(dir.path());
    assert_eq!(
        trial_report(&store, boundary + 3)
            .unwrap()
            .compared_request_count,
        2
    );
}
