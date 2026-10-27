use super::support::*;
use crate::{
    LocalObservationStore,
    valuation::ValuationRecordV2,
    writer::{IngestOutcome, ObservationCommitPort},
};
use hiroute_domain::*;

#[path = "analysis_tests.rs"]
mod analysis_tests;
#[path = "valuation_equal_rate.rs"]
mod equal_rate;
#[path = "native_analysis_tests.rs"]
mod native_analysis_tests;

fn price(at_ms: i64, frame_kind: UsageFrameKindV1) -> ExecutionPricingEvidenceV1 {
    let target = PriceTargetV1 {
        workspace_id: WorkspaceId::default(),
        source_id: "source-a".into(),
        source_identity_digest: CanonicalDigest::of_bytes(b"source-a"),
        model_identity: PriceModelIdentityV1::CatalogModel("model-a".into()),
        currency: "USD".into(),
        valuation_kind: PriceValuationKindV1::UsageEstimate,
    };
    let generation = PriceGenerationRefV1 {
        id: "generation-one".into(),
        digest: CanonicalDigest::of_bytes(b"generation-one"),
        configuration_revision: 1,
        catalog_refs: vec![],
    };
    let mut quote = FrozenPriceQuoteV1 {
        generation_ref: Some(generation.clone()),
        actual_source_ref: target.source_id.clone(),
        exact_target: target,
        actual_offer_ref: None,
        reference_model_offer_ref: None,
        valuation_kind: PriceValuationKindV1::UsageEstimate,
        currency: "USD".into(),
        unit: PriceUnitV1::MicrosPerMillionTokens,
        rates: TokenRatesV1 {
            input_uncached: TokenRateV1::known(1_000_000),
            output: TokenRateV1::known(2_000_000),
            cache_read: TokenRateV1::known(100_000),
            cache_write: TokenRateV1::known(1_500_000),
        },
        origin: PriceOriginV1::Manual,
        applied_override_refs: vec![],
        selected_rule_refs: vec![],
        attempt_execution_at: at_ms / 1000,
        applied_schedule: None,
        billing_context: PriceBillingContextV1::StandardTokens,
        unknown_reasons: vec![],
        quote_digest: CanonicalDigest::of_bytes(b"pending"),
    };
    quote.quote_digest = quote.computed_digest().unwrap();
    ExecutionPricingEvidenceV1 {
        schema_version: EXECUTION_PRICING_SCHEMA_V1.into(),
        request_generation: Some(generation),
        captured_at_ms: 100,
        attempt_execution_at_ms: at_ms,
        quote: Some(quote),
        reference_quote: None,
        unknown_reason: None,
        usage_semantics: UsageSemanticsV1 {
            frame_kind,
            input: InputUsageMeaningV1::IncludesExclusiveCache,
            output: OutputUsageMeaningV1::IncludesReasoning,
            cache_buckets_exclusive: true,
        },
    }
}

fn facts(fixture: &Fixture, frame: UsageFrameKindV1) -> Vec<ExecutionFactEnvelopeV1> {
    let inputs = vec![
        route_decision("plan/codex-daily"),
        candidate("model-a"),
        credential(),
        runtime_state(),
        attempt_started("model-a"),
        semantic_commit(),
        usage_and_cache(),
        usage_and_cache(),
        attempt_finished(),
        request_finished(),
    ];
    inputs
        .into_iter()
        .enumerate()
        .map(|(index, fact)| {
            let attempt = matches!(
                fact,
                ExecutionFactV1::AttemptStarted { .. }
                    | ExecutionFactV1::SemanticCommit { .. }
                    | ExecutionFactV1::UsageAndCache { .. }
                    | ExecutionFactV1::AttemptFinished(_)
            );
            let mut envelope = fixture.fact_for_plan(
                index as u64 + 1,
                fact,
                100 + index as i64,
                "plan/codex-daily",
                attempt.then(|| AttemptId::parse("attempt-a").unwrap()),
            );
            if matches!(envelope.fact, ExecutionFactV1::AttemptStarted { .. }) {
                envelope.schema_version = EXECUTION_FACT_SCHEMA_V2.into();
                envelope.schema_digest =
                    CanonicalDigest::parse(EXECUTION_FACT_PORT_DIGEST_V2).unwrap();
                envelope.pricing = Some(price(100 + index as i64, frame));
            }
            envelope
        })
        .collect()
}

fn latest(store: &LocalObservationStore, fixture: &Fixture) -> ValuationRecordV2 {
    let json:String=store.connection.lock().query_row("SELECT body_json FROM valuation_records_v2 WHERE workspace_id=?1 AND request_id=?2 ORDER BY revision DESC LIMIT 1",
        rusqlite::params![fixture.workspace.as_str(),fixture.request.as_str()],|row|row.get(0)).unwrap();
    serde_json::from_str(&json).unwrap()
}

#[test]
fn durable_pending_restarts_and_cumulative_frames_do_not_double_count() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("valuation-restart");
    let first = open_store(directory.path());
    let facts = facts(&fixture, UsageFrameKindV1::Cumulative);
    for fact in &facts {
        assert!(matches!(
            first.ingest_fact(fact, &[]).unwrap(),
            IngestOutcome::Ack(_)
        ));
    }
    drop(first);
    let store = open_store(directory.path());
    assert_eq!(store.settle_pending_valuations(16).unwrap(), 1);
    let record = latest(&store, &fixture);
    assert!(record.terminal_observed);
    assert_eq!(record.amounts[0].known_sum_micros, Some(117));
    assert_eq!(
        record.amounts[0].coverage,
        crate::valuation::MetricCoverage::Complete
    );
    for fact in &facts {
        assert!(matches!(
            store.ingest_fact(fact, &[]).unwrap(),
            IngestOutcome::Ack(_)
        ));
    }
    assert_eq!(store.settle_pending_valuations(16).unwrap(), 0);
    assert_eq!(latest(&store, &fixture), record);
    let count: i64 = store
        .connection
        .lock()
        .query_row(
            "SELECT count(*) FROM valuation_contributions_v2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn fixed_requests_settle_and_archive_without_a_synthetic_plan() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("valuation-fixed");
    let store = open_store(directory.path());
    for mut fact in facts(&fixture, UsageFrameKindV1::Cumulative) {
        let route = ModelRequestRouteV2::Fixed {
            binding_digest: CanonicalDigest::of_bytes(b"fixed-binding"),
        };
        fact.trust.agent_plan_id = None;
        fact.trust.plan_display_name = None;
        fact.trust.route = route.clone();
        if let ExecutionFactV1::RouteDecision(decision) = &mut fact.fact {
            decision.plan_id = None;
            decision.route = route;
        }
        assert!(matches!(
            store.ingest_fact(&fact, &[]).unwrap(),
            IngestOutcome::Ack(_)
        ));
    }
    assert_eq!(store.settle_pending_valuations(1).unwrap(), 1);
    assert_eq!(
        latest(&store, &fixture).amounts[0].known_sum_micros,
        Some(117)
    );
    {
        let connection = store.connection.lock();
        let contribution: (Option<String>, u64) = connection
            .query_row(
                "SELECT plan_id,known_micros FROM valuation_contributions_v2",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(contribution, (None, 117));
    }
    let principal = ObservationPrincipalV1::local_user(fixture.workspace.clone());
    let spec = SessionDeletionSpecV1 {
        workspace_id: fixture.workspace.clone(),
        session_id: fixture.session.clone(),
        data_class: DeletionDataClass::FactsAndContent,
        delete_rollups: false,
    };
    let preview = store.preview_session_deletion(&principal, &spec).unwrap();
    store
        .apply_session_deletion(
            &principal,
            &spec,
            preview.store_revision,
            &preview.change_digest,
            1000,
        )
        .unwrap();
    drop(store);
    let reopened = open_store(directory.path());
    let connection = reopened.connection.lock();
    let archived: (Option<String>, u64) = connection
        .query_row(
            "SELECT plan_id,known_micros FROM valuation_archives_v2",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(archived, (None, 117));
    let active: u64 = connection
        .query_row("SELECT COUNT(*) FROM valuation_requests_v2", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(active, 0);
}

#[test]
fn explicit_delta_revisions_replace_prior_contribution_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("valuation-delta");
    let store = open_store(directory.path());
    let facts = facts(&fixture, UsageFrameKindV1::Delta);
    for fact in &facts[..7] {
        store.ingest_fact(fact, &[]).unwrap();
    }
    store.settle_pending_valuations(1).unwrap();
    let first = latest(&store, &fixture);
    assert!(!first.terminal_observed);
    for fact in &facts[7..] {
        store.ingest_fact(fact, &[]).unwrap();
    }
    store.settle_pending_valuations(1).unwrap();
    let last = latest(&store, &fixture);
    assert_eq!(last.supersedes, Some(first.input_revision));
    assert_eq!(last.amounts[0].known_sum_micros, Some(233));
    let sum: i64 = store
        .connection
        .lock()
        .query_row(
            "SELECT sum(known_micros) FROM valuation_contributions_v2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(sum, 233);
}

#[test]
fn all_record_cleanup_archives_once_and_removes_price_inputs_and_reverse_details() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("valuation-delete");
    let store = open_store(directory.path());
    for fact in facts(&fixture, UsageFrameKindV1::Cumulative) {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    store.settle_pending_valuations(1).unwrap();
    let principal = ObservationPrincipalV1::local_user(fixture.workspace.clone());
    let spec = SessionDeletionSpecV1 {
        workspace_id: fixture.workspace.clone(),
        session_id: fixture.session.clone(),
        data_class: DeletionDataClass::FactsAndContent,
        delete_rollups: false,
    };
    let preview = store.preview_session_deletion(&principal, &spec).unwrap();
    store
        .apply_session_deletion(
            &principal,
            &spec,
            preview.store_revision,
            &preview.change_digest,
            1000,
        )
        .unwrap();
    let connection = store.connection.lock();
    for table in [
        "valuation_attempt_inputs_v2",
        "valuation_records_v2",
        "valuation_pending_v2",
        "valuation_requests_v2",
        "valuation_contributions_v2",
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
    let sum: i64 = connection
        .query_row(
            "SELECT sum(known_micros) FROM valuation_archives_v2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(sum, 117);
}

#[test]
fn scoped_value_summary_reports_pending_then_settlement_without_raw_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("valuation-reader");
    let store = open_store(directory.path());
    for fact in facts(&fixture, UsageFrameKindV1::Cumulative) {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    store
        .link_observed_request(&RunObservationLink {
            workspace_id: fixture.workspace.clone(),
            request_id: fixture.request.clone(),
            task_id: "task".into(),
            run_id: "run-a".into(),
            producer_epoch: "trusted-epoch".into(),
            source_event_id: "admission-event".into(),
            plan_id: "plan/codex-daily".into(),
            plan_revision: "revision".into(),
            publication_ref: "publication".into(),
            harness_id: "codex".into(),
            protocol_kind: "acp".into(),
            native_session_id: None,
            native_turn_id: None,
            parent_context_ref: None,
            continued_from_run_id: None,
        })
        .unwrap();
    let reader = |run: &str| {
        ObservationReaderContext::run_scoped(
            fixture.workspace.clone(),
            "worker".into(),
            1,
            10_000,
            [run.to_owned()].into_iter().collect(),
            false,
            false,
        )
        .unwrap()
    };
    assert_eq!(
        store.observed_valuation_summary(&reader("run-b"), &fixture.request, 1000),
        Err(ObservationQueryError::Unauthorized)
    );
    let pending = store
        .observed_valuation_summary(&reader("run-a"), &fixture.request, 1000)
        .unwrap();
    assert!(pending.pending);
    assert_eq!(pending.settled_revision, None);
    assert!(pending.amounts.is_empty());
    store.settle_pending_valuations(1).unwrap();
    let settled = store
        .observed_valuation_summary(&reader("run-a"), &fixture.request, 1000)
        .unwrap();
    assert!(!settled.pending);
    assert_eq!(settled.settled_revision, Some(settled.input_revision));
    assert_eq!(settled.amounts[0].known_sum_micros, Some(117));
    assert_eq!(
        settled.amounts[0].coverage,
        ObservationMetricCoverageV2::Complete
    );
    assert_eq!(
        store.observed_valuation_summary(&reader("run-a"), &fixture.request, 10_000),
        Err(ObservationQueryError::Unauthorized)
    );
}

#[test]
fn settled_record_compares_only_the_accepted_usage_with_its_frozen_reference() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("valuation-reference");
    let store = open_store(directory.path());
    for mut fact in facts(&fixture, UsageFrameKindV1::Cumulative) {
        if let Some(pricing) = &mut fact.pricing {
            let mut reference = pricing.quote.clone().unwrap();
            reference.rates = TokenRatesV1 {
                input_uncached: TokenRateV1::known(0),
                output: TokenRateV1::known(0),
                cache_read: TokenRateV1::known(0),
                cache_write: TokenRateV1::known(0),
            };
            reference.attempt_execution_at = pricing.captured_at_ms / 1000;
            reference.quote_digest = reference.computed_digest().unwrap();
            pricing.reference_quote = Some(reference);
        }
        store.ingest_fact(&fact, &[]).unwrap();
    }
    store.settle_pending_valuations(1).unwrap();
    let record = latest(&store, &fixture);
    let comparison = record.reference_comparison.unwrap();
    assert_eq!(comparison.baseline_micros, Some(0));
    assert_eq!(comparison.observed_estimate_micros, Some(117));
    assert_eq!(comparison.savings_micros, Some(-117));
    assert_eq!(comparison.actual_cash_micros, None);
}

#[test]
fn content_deletion_erases_raw_carriers_preserves_digests_models_and_numeric_value() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("sensitive-delete");
    let store = open_store(directory.path());
    for fact in facts(&fixture, UsageFrameKindV1::Cumulative) {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    store.settle_pending_valuations(1).unwrap();
    let before: Vec<String> = {
        let connection = store.connection.lock();
        let count: u64 = connection
            .query_row(
                "SELECT count(*) FROM observation_sensitive_payloads_v2",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count > 0);
        let mut statement = connection
            .prepare("SELECT envelope_digest FROM execution_fact_events ORDER BY sequence")
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let principal = ObservationPrincipalV1::local_user(fixture.workspace.clone());
    let spec = SessionDeletionSpecV1 {
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
            1000,
        )
        .unwrap();
    assert_eq!(
        store
            .get_receipt(
                &fixture.workspace,
                &ReceiptId::parse(fixture.request.as_str()).unwrap()
            )
            .unwrap_err(),
        ObservationQueryError::NotFound
    );
    let detail = store
        .get_session(&fixture.workspace, &fixture.session, ContentMode::None)
        .unwrap();
    assert_eq!(detail.summary.model_switch, Some(false));
    assert_eq!(
        latest(&store, &fixture).amounts[0].known_sum_micros,
        Some(117)
    );
    let reader = ObservationReaderContext::local_user(
        fixture.workspace.clone(),
        "user".into(),
        1,
        10_000,
        false,
        false,
    )
    .unwrap();
    let page = store
        .observed_facts(
            &reader,
            &ObservationFactsQueryV2 {
                include_routing_observations: false,
                request_id: fixture.request.clone(),
                limit: 200,
                cursor: None,
            },
            1000,
        )
        .unwrap();
    assert_eq!(page.facts.len(), 10);
    assert!(page.facts.iter().all(|fact| fact.sensitive_fields_deleted));
    assert!(page.facts.iter().any(|fact| fact.input_tokens.is_some()));
    let safe_json = serde_json::to_string(&page).unwrap();
    assert!(!safe_json.contains("authority-local"));
    assert!(!safe_json.contains("credential-ref-a"));
    let connection = store.connection.lock();
    let count: u64 = connection
        .query_row(
            "SELECT count(*) FROM observation_sensitive_payloads_v2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    let mut statement = connection
        .prepare(
            "SELECT envelope_digest,envelope_json FROM execution_fact_events ORDER BY sequence",
        )
        .unwrap();
    let after: Vec<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        before,
        after.iter().map(|row| row.0.clone()).collect::<Vec<_>>()
    );
    assert!(
        after.iter().all(
            |row| row.1.contains("managed_sensitive_ref") && !row.1.contains("authority-local")
        )
    );
}

#[test]
fn long_session_expires_only_old_request_archives_latest_input_and_blocks_late_replay() {
    let directory = tempfile::tempdir().unwrap();
    let old = Fixture::new("old-in-long-session");
    let mut recent = Fixture::new("recent-in-long-session");
    recent.session = old.session.clone();
    let store = open_store(directory.path());
    let old_facts = facts(&old, UsageFrameKindV1::Cumulative);
    for fact in &old_facts {
        store.ingest_fact(fact, &[]).unwrap();
    }
    let shift = SEVEN_DAYS_MILLIS;
    for mut fact in facts(&recent, UsageFrameKindV1::Cumulative) {
        fact.occurred_at_unix_nanos += (shift as u64) * 1_000_000;
        if fact.pricing.is_some() {
            fact.pricing = Some(price(
                fact.occurred_at_ms().unwrap(),
                UsageFrameKindV1::Cumulative,
            ));
        }
        store.ingest_fact(&fact, &[]).unwrap();
    }
    assert_eq!(store.expire_request_details(shift + 500, 1).unwrap(), 1);
    assert_eq!(store.expire_request_details(shift + 500, 1).unwrap(), 0);
    assert!(matches!(
        store.ingest_fact(&old_facts[0], &[]).unwrap(),
        IngestOutcome::Nack(_)
    ));
    let connection = store.connection.lock();
    let ids: Vec<String> = {
        let mut s = connection
            .prepare("SELECT request_id FROM logical_requests")
            .unwrap();
        s.query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(ids, vec![recent.request.to_string()]);
    let archived: u64 = connection
        .query_row(
            "SELECT known_micros FROM valuation_archives_v2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(archived, 117);
    let old_carriers: u64 = connection
        .query_row(
            "SELECT count(*) FROM observation_sensitive_payloads_v2 WHERE request_id=?1",
            [old.request.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    let recent_carriers: u64 = connection
        .query_row(
            "SELECT count(*) FROM observation_sensitive_payloads_v2 WHERE request_id=?1",
            [recent.request.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_carriers, 0);
    assert!(recent_carriers > 0);
}

#[test]
fn corrupt_attempt_is_unknown_and_does_not_block_retention_batch() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("corrupt-expiry");
    for fact in facts(&fixture, UsageFrameKindV1::Cumulative) {
        assert!(matches!(
            store.ingest_fact(&fact, &[]).unwrap(),
            IngestOutcome::Ack(_)
        ));
    }
    store
        .connection
        .lock()
        .execute(
            "UPDATE valuation_attempt_inputs_v2 SET pricing_json='{broken'",
            [],
        )
        .unwrap();
    assert_eq!(store.settle_pending_valuations(16).unwrap(), 1);
    let record = latest(&store, &fixture);
    assert!(record.facts_partial);
    assert_eq!(record.unknown_attempt_count, 1);
    assert!(record.amounts.is_empty());
    assert_eq!(
        store
            .expire_request_details(SEVEN_DAYS_MILLIS + 1000, 16)
            .unwrap(),
        1
    );
}

#[path = "valuation_query_tests.rs"]
mod query_tests;

#[path = "api_equivalent_tests.rs"]
mod api_equivalent_tests;
