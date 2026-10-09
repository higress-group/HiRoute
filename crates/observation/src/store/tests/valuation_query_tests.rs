use super::*;

fn set_usage_pair(
    facts: &mut [ExecutionFactEnvelopeV1],
    input_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
) {
    for envelope in facts {
        if let ExecutionFactV1::UsageAndCache {
            input_tokens: input,
            billable_tokens,
            cache_read_tokens: read,
            cache_write_tokens,
            input_provenance,
            billable_provenance,
            cache_read_provenance,
            cache_write_provenance,
            ..
        } = &mut envelope.fact
        {
            *input = input_tokens;
            *read = cache_read_tokens;
            *cache_write_tokens = input_tokens.map(|_| 0);
            *billable_tokens = input_tokens.and_then(|value| value.checked_add(25));
            *input_provenance = if input_tokens.is_some() {
                UsageProvenanceV1::Reported
            } else {
                UsageProvenanceV1::Unknown
            };
            *cache_read_provenance = if cache_read_tokens.is_some() {
                UsageProvenanceV1::Reported
            } else {
                UsageProvenanceV1::Unknown
            };
            *cache_write_provenance = if cache_write_tokens.is_some() {
                UsageProvenanceV1::Reported
            } else {
                UsageProvenanceV1::Unknown
            };
            *billable_provenance = if billable_tokens.is_some() {
                UsageProvenanceV1::Reported
            } else {
                UsageProvenanceV1::Unknown
            };
        }
    }
}

fn local_reader(fixture: &Fixture, now_ms: i64) -> ObservationReaderContext {
    ObservationReaderContext::local_user(
        fixture.workspace.clone(),
        "local".into(),
        1,
        now_ms + 10_000,
        false,
        false,
    )
    .unwrap()
}

fn value_query(session_id: Option<String>, to_ms: i64) -> ObservationValueQueryV2 {
    ObservationValueQueryV2 {
        from_ms: 0,
        to_ms,
        session_id,
        plan_id: None,
        currency: None,
    }
}

#[test]
fn current_value_report_keeps_unpriced_usage_scoped_and_groups_utc_days() {
    const DAY: i64 = 86_400_000;
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let first = Fixture::new("report-first");
    let mut second = Fixture::new("report-second");
    second.session = first.session.clone();
    let other = Fixture::new("report-other-session");
    for (fixture, offset, input) in [(&first, 0, 100), (&second, DAY, 200), (&other, 0, 900)] {
        let mut events = facts(fixture, UsageFrameKindV1::Cumulative);
        set_usage_pair(&mut events, Some(input), None);
        for mut event in events {
            event.occurred_at_unix_nanos += offset as u64 * 1_000_000;
            event.pricing = None;
            store.ingest_fact(&event, &[]).unwrap();
        }
    }
    store.settle_pending_valuations(16).unwrap();
    let now = DAY + 1_000;
    let reader = local_reader(&first, now);
    let mut query = ObservationValueQueryV2 {
        from_ms: 50,
        to_ms: now,
        session_id: Some(first.session.to_string()),
        plan_id: Some("plan/codex-daily".into()),
        currency: None,
    };
    let report = store
        .observed_value_report(&reader, &query, ValueGroupByV1::Day, now)
        .unwrap();
    assert_eq!(
        report.summary,
        store.observed_value_totals(&reader, &query, now).unwrap()
    );
    assert_eq!(report.summary.usage[0].known_sum, Some(300));
    assert_eq!(report.summary.usage[1].known_sum, Some(50));
    assert_eq!(report.summary.usage[2].known_sum, None);
    assert_eq!(report.day_timezone.as_deref(), Some("UTC"));
    assert_eq!(report.days.len(), 2);
    assert_eq!((report.days[0].from_ms, report.days[0].to_ms), (50, DAY));
    assert_eq!((report.days[1].from_ms, report.days[1].to_ms), (DAY, now));
    assert_eq!(report.days[0].usage[0].known_sum, Some(100));
    assert_eq!(report.days[1].usage[0].known_sum, Some(200));
    query.plan_id = Some("plan/other".into());
    let absent = store
        .observed_value_report(&reader, &query, ValueGroupByV1::None, now)
        .unwrap();
    assert!(absent.summary.usage.iter().all(|u| u.known_sum.is_none()));
    assert!(absent.days.is_empty());
    query.to_ms = 201 * DAY;
    assert!(
        store
            .observed_value_report(&reader, &query, ValueGroupByV1::Day, now)
            .is_err()
    );
}

#[test]
fn aggregate_reads_one_scope_and_does_not_double_count_legacy_and_v2() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("totals");
    // Run admission is delivered independently and may arrive before the first execution fact.
    store
        .link_observed_request(&RunObservationLink {
            workspace_id: fixture.workspace.clone(),
            request_id: fixture.request.clone(),
            task_id: "task-totals".into(),
            run_id: "run-totals".into(),
            producer_epoch: "trusted-epoch".into(),
            source_event_id: "admission-before-facts".into(),
            plan_id: "plan/codex-daily".into(),
            plan_revision: "revision".into(),
            publication_ref: "publication".into(),
            harness_id: "codex".into(),
            protocol_kind: "responses".into(),
            native_session_id: None,
            native_turn_id: None,
            parent_context_ref: None,
            continued_from_run_id: None,
        })
        .unwrap();
    for fact in facts(&fixture, UsageFrameKindV1::Cumulative) {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    store.settle_pending_valuations(16).unwrap();
    let reader = ObservationReaderContext::local_user(
        fixture.workspace.clone(),
        "local".into(),
        1,
        10000,
        false,
        false,
    )
    .unwrap();
    let query = ObservationValueQueryV2 {
        from_ms: 0,
        to_ms: 1000,
        session_id: None,
        plan_id: None,
        currency: Some("USD".into()),
    };
    store.connection.lock().execute(
        "INSERT INTO value_ledger_entries(workspace_id,request_id,session_id,receipt_id,body_json,body_digest,agent_plan_id,currency,billing_unit,price_version,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,reasoning_tokens,chosen_api_equivalent_cost_micros,facts_completeness,frozen_at_ms) VALUES(?1,?2,?3,'legacy','{}','legacy','plan','USD','tokens','old',100,0,0,0,0,999,'complete',100)",
        rusqlite::params![fixture.workspace.as_str(),fixture.request.as_str(),fixture.session.as_str()],
    ).unwrap();
    let result = store.observed_value_totals(&reader, &query, 500).unwrap();
    assert!(!result.archive_boundary_partial);
    assert_eq!(result.amounts.len(), 1);
    assert_eq!(result.amounts[0].valuation_kind, "usage_estimate");
    assert_eq!(result.amounts[0].known_sum_micros, Some(117));
    assert_eq!(result.pending_requests, 0);
    assert_eq!(result.unknown_traffic_requests, 0);
    assert_eq!(result.usage[0].known_sum, Some(100));
}

#[test]
fn weighted_cache_hit_ratio_uses_paired_attempt_sums_without_requiring_prices() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let first = Fixture::new("cache-ratio-small");
    let mut second = Fixture::new("cache-ratio-large");
    second.session = first.session.clone();

    let mut first_facts = facts(&first, UsageFrameKindV1::Cumulative);
    set_usage_pair(&mut first_facts, Some(100), Some(90));
    for envelope in &mut first_facts {
        if matches!(envelope.fact, ExecutionFactV1::AttemptStarted { .. }) {
            envelope.pricing = None;
        }
    }
    let mut second_facts = facts(&second, UsageFrameKindV1::Cumulative);
    set_usage_pair(&mut second_facts, Some(1_000), Some(100));
    for fact in first_facts.into_iter().chain(second_facts) {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    store.settle_pending_valuations(16).unwrap();

    let result = store
        .observed_value_totals(
            &local_reader(&first, 500),
            &value_query(Some(first.session.to_string()), 1_000),
            500,
        )
        .unwrap();
    assert_eq!(
        result
            .usage
            .iter()
            .find(|metric| metric.metric == "input")
            .and_then(|metric| metric.known_sum),
        Some(1_100)
    );
    assert_eq!(
        result.input_cache_hit.state,
        ObservationCacheHitStateV2::Available
    );
    assert_eq!(result.input_cache_hit.ratio_basis_points, Some(1_727));
    assert_eq!(result.input_cache_hit.cache_read_tokens, Some(190));
    assert_eq!(result.input_cache_hit.total_input_tokens, Some(1_100));
    assert_eq!(result.input_cache_hit.eligible_attempt_count, 2);
    assert_eq!(result.input_cache_hit.total_attempt_count, 2);
    assert_eq!(
        result.input_cache_hit.coverage,
        ObservationMetricCoverageV2::Complete
    );
}

#[test]
fn full_u64_usage_is_readable_and_aggregate_overflow_is_unknown() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let first = Fixture::new("huge-usage-first");
    let mut second = Fixture::new("huge-usage-second");
    second.session = first.session.clone();
    let mut first_facts = facts(&first, UsageFrameKindV1::Cumulative);
    set_usage_pair(&mut first_facts, Some(u64::MAX), Some(0));
    let mut second_facts = facts(&second, UsageFrameKindV1::Cumulative);
    set_usage_pair(&mut second_facts, Some(1), Some(0));
    for envelope in first_facts.iter_mut().chain(second_facts.iter_mut()) {
        if matches!(envelope.fact, ExecutionFactV1::AttemptStarted { .. }) {
            envelope.pricing = None;
        }
    }
    for fact in first_facts {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    store.settle_pending_valuations(16).unwrap();
    let reader = local_reader(&first, 500);
    let query = value_query(Some(first.session.to_string()), 1_000);
    let single = store.observed_value_totals(&reader, &query, 500).unwrap();
    let input = single
        .usage
        .iter()
        .find(|item| item.metric == "input")
        .unwrap();
    assert_eq!(input.known_sum, Some(u64::MAX));
    assert_eq!(input.coverage, ObservationMetricCoverageV2::Complete);
    assert_eq!(single.input_cache_hit.ratio_basis_points, Some(0));

    for fact in second_facts {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    store.settle_pending_valuations(16).unwrap();
    let combined = store.observed_value_totals(&reader, &query, 500).unwrap();
    let input = combined
        .usage
        .iter()
        .find(|item| item.metric == "input")
        .unwrap();
    assert_eq!(input.known_sum, None);
    assert_eq!(input.coverage, ObservationMetricCoverageV2::Unknown);
    assert!(combined.input_cache_hit.arithmetic_overflow);
    assert_eq!(combined.input_cache_hit.ratio_basis_points, None);
}

#[test]
fn archived_u64_totals_above_sqlite_integer_range_remain_readable() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("archive-large-totals");
    {
        let db = store.connection.lock();
        for day in 0..2 {
            db.execute(
                "INSERT INTO observation_usage_archives_v2(workspace_id,day,plan_id,metric,known_sum,missing_count) VALUES(?1,?2,NULL,'input',?3,0)",
                rusqlite::params![fixture.workspace.as_str(), day, i64::MAX],
            )
            .unwrap();
        }
    }
    let result = store
        .observed_value_totals(
            &local_reader(&fixture, 500),
            &value_query(None, 2 * 86_400_000),
            500,
        )
        .unwrap();
    let input = result
        .usage
        .iter()
        .find(|item| item.metric == "input")
        .unwrap();
    assert_eq!(input.known_sum, Some(u64::MAX - 1));
    assert_eq!(input.coverage, ObservationMetricCoverageV2::Complete);
}

#[test]
fn full_u64_usage_and_valid_cache_pair_survive_real_detail_expiry() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("archive-full-u64");
    let mut records = facts(&fixture, UsageFrameKindV1::Cumulative);
    set_usage_pair(&mut records, Some(u64::MAX), Some(u64::MAX));
    for record in &mut records {
        if matches!(record.fact, ExecutionFactV1::AttemptStarted { .. }) {
            record.pricing = None;
        }
    }
    for record in records {
        store.ingest_fact(&record, &[]).unwrap();
    }
    store.settle_pending_valuations(16).unwrap();
    let now = SEVEN_DAYS_MILLIS + 1_000;
    let query = value_query(None, now);
    let reader = local_reader(&fixture, now);
    let before = store.observed_value_totals(&reader, &query, now).unwrap();
    assert_eq!(before.usage[0].known_sum, Some(u64::MAX));
    assert_eq!(before.input_cache_hit.ratio_basis_points, Some(10_000));
    assert_eq!(before.input_cache_hit.invalid_attempt_count, 0);

    assert_eq!(store.expire_request_details(now, 16).unwrap(), 1);
    let after = store.observed_value_totals(&reader, &query, now).unwrap();
    assert_eq!(after.usage, before.usage);
    assert_eq!(after.input_cache_hit, before.input_cache_hit);
    assert_eq!(
        store
            .connection
            .lock()
            .query_row(
                "SELECT known_sum FROM observation_usage_archives_v2 WHERE metric='input'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        -1
    );
}

#[test]
fn archived_overflow_stays_unknown_after_later_request_expires() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let first = Fixture::new("archive-overflow-a");
    let mut second = Fixture::new("archive-overflow-b");
    let mut third = Fixture::new("archive-overflow-c");
    second.session = first.session.clone();
    third.session = first.session.clone();
    for (fixture, input) in [(&first, u64::MAX), (&second, 1), (&third, 1)] {
        let mut records = facts(fixture, UsageFrameKindV1::Cumulative);
        set_usage_pair(&mut records, Some(input), Some(0));
        for record in &mut records {
            if matches!(record.fact, ExecutionFactV1::AttemptStarted { .. }) {
                record.pricing = None;
            }
        }
        for record in records {
            store.ingest_fact(&record, &[]).unwrap();
        }
    }
    store.settle_pending_valuations(16).unwrap();
    let now = SEVEN_DAYS_MILLIS + 1_000;
    let query = value_query(None, now);
    let reader = local_reader(&first, now);
    let before = store.observed_value_totals(&reader, &query, now).unwrap();
    assert_eq!(before.usage[0].known_sum, None);
    assert!(before.input_cache_hit.arithmetic_overflow);

    assert_eq!(store.expire_request_details(now, 2).unwrap(), 2);
    assert_eq!(store.expire_request_details(now, 2).unwrap(), 1);
    let after = store.observed_value_totals(&reader, &query, now).unwrap();
    let input = after
        .usage
        .iter()
        .find(|item| item.metric == "input")
        .unwrap();
    assert_eq!(input.known_sum, None);
    assert_eq!(input.coverage, ObservationMetricCoverageV2::Unknown);
    assert_eq!(input.missing_attempt_count, 0);
    assert!(after.input_cache_hit.arithmetic_overflow);
    assert_eq!(after.input_cache_hit.ratio_basis_points, None);
}

#[test]
fn archived_missing_metric_can_still_accumulate_later_known_usage() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let first = Fixture::new("archive-missing-a");
    let mut second = Fixture::new("archive-missing-b");
    second.session = first.session.clone();
    for (fixture, input, read) in [(&first, 0, None), (&second, 100, Some(10))] {
        let mut records = facts(fixture, UsageFrameKindV1::Cumulative);
        set_usage_pair(&mut records, Some(input), read);
        for record in records {
            store.ingest_fact(&record, &[]).unwrap();
        }
    }
    store.settle_pending_valuations(16).unwrap();
    let now = SEVEN_DAYS_MILLIS + 1_000;
    let query = value_query(None, now);
    let reader = local_reader(&first, now);
    let before = store.observed_value_totals(&reader, &query, now).unwrap();
    assert_eq!(store.expire_request_details(now, 16).unwrap(), 2);
    let after = store.observed_value_totals(&reader, &query, now).unwrap();
    assert_eq!(after.usage, before.usage);
    let read = after
        .usage
        .iter()
        .find(|item| item.metric == "cache_read")
        .unwrap();
    assert_eq!(read.known_sum, Some(10));
    assert_eq!(read.missing_attempt_count, 1);
    assert_eq!(read.coverage, ObservationMetricCoverageV2::Partial);
}

#[test]
fn zero_input_is_not_missing_and_incomplete_pairs_remain_unknown() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let zero = Fixture::new("cache-zero");
    let mut zero_facts = facts(&zero, UsageFrameKindV1::Cumulative);
    set_usage_pair(&mut zero_facts, Some(0), Some(0));
    for fact in zero_facts {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    store.settle_pending_valuations(16).unwrap();
    let zero_result = store
        .observed_value_totals(
            &local_reader(&zero, 500),
            &value_query(Some(zero.session.to_string()), 1_000),
            500,
        )
        .unwrap();
    assert_eq!(
        zero_result.input_cache_hit.state,
        ObservationCacheHitStateV2::NotApplicable
    );
    assert_eq!(zero_result.input_cache_hit.zero_input_attempt_count, 1);
    assert_eq!(zero_result.input_cache_hit.missing_attempt_count, 0);
    assert_eq!(
        zero_result.input_cache_hit.coverage,
        ObservationMetricCoverageV2::Complete
    );

    let missing = Fixture::new("cache-missing");
    let mut invalid = Fixture::new("cache-invalid");
    invalid.session = missing.session.clone();
    let mut missing_facts = facts(&missing, UsageFrameKindV1::Cumulative);
    set_usage_pair(&mut missing_facts, Some(100), None);
    let mut invalid_facts = facts(&invalid, UsageFrameKindV1::Cumulative);
    set_usage_pair(&mut invalid_facts, Some(100), Some(101));
    for fact in missing_facts.into_iter().chain(invalid_facts) {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    store.settle_pending_valuations(16).unwrap();
    let incomplete = store
        .observed_value_totals(
            &local_reader(&missing, 500),
            &value_query(Some(missing.session.to_string()), 1_000),
            500,
        )
        .unwrap();
    assert_eq!(
        incomplete.input_cache_hit.state,
        ObservationCacheHitStateV2::Unknown
    );
    assert_eq!(incomplete.input_cache_hit.total_attempt_count, 2);
    assert_eq!(incomplete.input_cache_hit.missing_attempt_count, 1);
    assert_eq!(incomplete.input_cache_hit.invalid_attempt_count, 1);
    assert_eq!(
        incomplete.input_cache_hit.coverage,
        ObservationMetricCoverageV2::Unknown
    );
}

#[test]
fn verified_probe_is_excluded_and_later_run_link_cannot_reclassify_it() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("probe-exclusion");
    for fact in facts(&fixture, UsageFrameKindV1::Cumulative) {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    store.settle_pending_valuations(16).unwrap();
    let reader = local_reader(&fixture, 500);
    let query = value_query(Some(fixture.session.to_string()), 1_000);
    let before = store.observed_value_totals(&reader, &query, 500).unwrap();
    assert_eq!(before.excluded_requests, 0);
    assert_eq!(before.unknown_traffic_requests, 0);
    assert_eq!(before.input_cache_hit.ratio_basis_points, Some(4_000));

    store
        .mark_observed_connectivity_probe(&fixture.workspace, &fixture.request)
        .unwrap();
    let revision: u64 = store
        .connection
        .lock()
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='store_revision'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    store
        .mark_observed_connectivity_probe(&fixture.workspace, &fixture.request)
        .unwrap();
    assert_eq!(
        store
            .connection
            .lock()
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='store_revision'",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap(),
        revision
    );
    store
        .link_observed_request(&RunObservationLink {
            workspace_id: fixture.workspace.clone(),
            request_id: fixture.request.clone(),
            task_id: "task-probe".into(),
            run_id: "run-probe".into(),
            producer_epoch: "epoch-probe".into(),
            source_event_id: "event-probe".into(),
            plan_id: "plan/codex-daily".into(),
            plan_revision: "revision".into(),
            publication_ref: "publication".into(),
            harness_id: "codex".into(),
            protocol_kind: "responses".into(),
            native_session_id: None,
            native_turn_id: None,
            parent_context_ref: None,
            continued_from_run_id: None,
        })
        .unwrap();

    let after = store.observed_value_totals(&reader, &query, 500).unwrap();
    assert_eq!(after.excluded_requests, 1);
    assert_eq!(after.unknown_traffic_requests, 0);
    assert!(after.usage.iter().all(|metric| metric.known_sum.is_none()));
    assert_eq!(
        after.input_cache_hit.state,
        ObservationCacheHitStateV2::Unknown
    );
    assert_eq!(after.input_cache_hit.total_attempt_count, 0);
}

#[test]
fn supported_v4_inline_backfill_preserves_bytes_and_digest() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("inline-v4");
    for fact in facts(&fixture, UsageFrameKindV1::Cumulative) {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    let original:String=store.connection.lock().query_row("SELECT body_json FROM observation_sensitive_payloads_v2 WHERE id LIKE 'fact:%' ORDER BY id LIMIT 1",[],|r|r.get(0)).unwrap();
    store.connection.lock().execute_batch("UPDATE execution_fact_events SET envelope_json=(SELECT body_json FROM observation_sensitive_payloads_v2 WHERE id='fact:'||envelope_digest); UPDATE routing_receipts SET body_json=(SELECT body_json FROM observation_sensitive_payloads_v2 WHERE id='receipt:'||body_digest); DELETE FROM observation_sensitive_payloads_v2; DELETE FROM observation_safe_facts_v2; DELETE FROM observation_attempt_models_v2;").unwrap();
    assert!(store.backfill_inline_payloads().unwrap() > 0);
    assert!(store.backfill_inline_payloads().unwrap() > 0);
    assert_eq!(store.backfill_inline_payloads().unwrap(), 0);
    let exists: bool = store
        .connection
        .lock()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM observation_sensitive_payloads_v2 WHERE body_json=?1)",
            [original],
            |r| r.get(0),
        )
        .unwrap();
    assert!(exists);
    assert_eq!(
        crate::store::fact_log::load_request(
            &store.connection.lock(),
            &fixture.workspace,
            &fixture.request
        )
        .unwrap()
        .len(),
        10
    );
}

#[test]
fn scoped_value_expires_before_sweeper_and_empty_midnight_range_is_valid() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("scope-expiry");
    for fact in facts(&fixture, UsageFrameKindV1::Cumulative) {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    store
        .connection
        .lock()
        .execute("UPDATE logical_requests SET traffic_kind='normal'", [])
        .unwrap();
    store.settle_pending_valuations(16).unwrap();
    let now = SEVEN_DAYS_MILLIS + 1000;
    let reader = ObservationReaderContext::local_user(
        fixture.workspace.clone(),
        "local".into(),
        1,
        now + 10000,
        false,
        false,
    )
    .unwrap();
    let mut query = ObservationValueQueryV2 {
        from_ms: 0,
        to_ms: now,
        session_id: Some(fixture.session.to_string()),
        plan_id: None,
        currency: None,
    };
    let result = store.observed_value_totals(&reader, &query, now).unwrap();
    assert!(result.amounts.is_empty());
    assert!(result.usage.iter().all(|m| m.known_sum.is_none()));
    assert!(result.retention_boundary_partial);
    query.from_ms = now;
    let midnight = store.observed_value_totals(&reader, &query, now).unwrap();
    assert!(midnight.amounts.is_empty());
    assert!(!midnight.archive_boundary_partial);
    assert!(!midnight.retention_boundary_partial);
}

#[test]
fn scoped_value_discloses_expired_request_gap_without_reading_unscoped_archive() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let old = Fixture::new("scoped-old");
    let mut recent = Fixture::new("scoped-recent");
    recent.session = old.session.clone();
    let now = SEVEN_DAYS_MILLIS + 10_000;
    for fact in facts(&old, UsageFrameKindV1::Cumulative) {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    for mut fact in facts(&recent, UsageFrameKindV1::Cumulative) {
        fact.occurred_at_unix_nanos = (now as u64 - 1_000) * 1_000_000;
        store.ingest_fact(&fact, &[]).unwrap();
    }
    store.settle_pending_valuations(16).unwrap();
    assert_eq!(store.expire_request_details(now, 16).unwrap(), 1);

    let reader = local_reader(&old, now);
    let mut query = value_query(Some(old.session.to_string()), now + 1);
    let mixed = store.observed_value_totals(&reader, &query, now).unwrap();
    let input = mixed
        .usage
        .iter()
        .find(|item| item.metric == "input")
        .unwrap();
    assert_eq!(input.known_sum, Some(100));
    assert_eq!(input.missing_attempt_count, 0);
    assert_eq!(input.coverage, ObservationMetricCoverageV2::Partial);
    assert!(mixed.retention_boundary_partial);
    assert!(!mixed.input_cache_hit.archive_coverage_partial);
    assert_eq!(mixed.input_cache_hit.ratio_basis_points, Some(4_000));
    assert_eq!(
        mixed.input_cache_hit.coverage,
        ObservationMetricCoverageV2::Partial
    );

    query.from_ms = now - 2_000;
    let retained = store.observed_value_totals(&reader, &query, now).unwrap();
    assert!(!retained.retention_boundary_partial);
    assert_eq!(
        retained.usage[0].coverage,
        ObservationMetricCoverageV2::Complete
    );
    assert_eq!(
        retained.input_cache_hit.coverage,
        ObservationMetricCoverageV2::Complete
    );
}

#[test]
fn basic_archives_keep_usage_and_known_subtotals_once_after_expiry() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("basic-archive");
    for fact in facts(&fixture, UsageFrameKindV1::Cumulative) {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    store
        .connection
        .lock()
        .execute("UPDATE logical_requests SET traffic_kind='normal'", [])
        .unwrap();
    store.settle_pending_valuations(16).unwrap();
    let now = SEVEN_DAYS_MILLIS + 1000;
    let reader = ObservationReaderContext::local_user(
        fixture.workspace.clone(),
        "local".into(),
        1,
        now + 10000,
        false,
        false,
    )
    .unwrap();
    let query = ObservationValueQueryV2 {
        from_ms: 0,
        to_ms: now,
        session_id: None,
        plan_id: None,
        currency: None,
    };
    let before = store.observed_value_totals(&reader, &query, now).unwrap();
    assert_eq!(before.usage[0].known_sum, Some(100));
    assert_eq!(before.input_cache_hit.ratio_basis_points, Some(4_000));
    assert_eq!(store.expire_request_details(now, 16).unwrap(), 1);
    assert_eq!(store.expire_request_details(now, 16).unwrap(), 0);
    let after = store.observed_value_totals(&reader, &query, now).unwrap();
    assert_eq!(after.usage, before.usage);
    assert_eq!(after.amounts, before.amounts);
    assert_eq!(after.input_cache_hit, before.input_cache_hit);
    let db = store.connection.lock();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM observation_usage_archives_v2",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        10
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM valuation_attempt_inputs_v2",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn legacy_usage_archive_is_visible_but_does_not_invent_a_cache_hit_ratio() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("legacy-cache-archive");
    {
        let connection = store.connection.lock();
        for (metric, value) in [("input", 100), ("cache_read", 40)] {
            connection
                .execute(
                    "INSERT INTO observation_usage_archives_v2
                     (workspace_id,day,plan_id,metric,known_sum,missing_count)
                     VALUES(?1,0,NULL,?2,?3,0)",
                    rusqlite::params![fixture.workspace.as_str(), metric, value],
                )
                .unwrap();
        }
    }
    let result = store
        .observed_value_totals(
            &local_reader(&fixture, 500),
            &value_query(None, 86_400_000),
            500,
        )
        .unwrap();
    assert_eq!(
        result
            .usage
            .iter()
            .find(|metric| metric.metric == "input")
            .and_then(|metric| metric.known_sum),
        Some(100)
    );
    assert_eq!(
        result.input_cache_hit.state,
        ObservationCacheHitStateV2::Unknown
    );
    assert!(result.input_cache_hit.archive_coverage_partial);
    assert!(!result.archive_boundary_partial);
    assert_eq!(
        result.input_cache_hit.coverage,
        ObservationMetricCoverageV2::Unknown
    );

    let partial_day = store
        .observed_value_totals(&local_reader(&fixture, 500), &value_query(None, 1_000), 500)
        .unwrap();
    assert!(partial_day.archive_boundary_partial);
    assert!(
        partial_day
            .usage
            .iter()
            .all(|metric| metric.known_sum.is_none())
    );
}
