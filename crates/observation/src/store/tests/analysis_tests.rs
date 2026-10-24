#[path = "trust_cost_tests.rs"]
mod trust_cost_tests;
use super::*;

fn reader() -> ObservationReaderContext {
    ObservationReaderContext::local_user(
        WorkspaceId::default(),
        "local-analysis".into(),
        1,
        20_000,
        true,
        true,
    )
    .unwrap()
}
fn range() -> AnalysisTimeRangeV1 {
    AnalysisTimeRangeV1 {
        requested_from_ms: 0,
        requested_to_ms: 1000,
        effective_from_ms: 0,
        effective_to_ms: 1000,
        from_utc_offset_seconds: 0,
        to_utc_offset_seconds: 0,
        clipped_for_retention: false,
    }
}
fn current_price(binding: &str, model: &str, rate: u64) -> ApiEquivalentPriceV1 {
    let mut quote = price(100, UsageFrameKindV1::Cumulative).quote.unwrap();
    quote.exact_target.valuation_kind = PriceValuationKindV1::ApiEquivalent;
    quote.valuation_kind = PriceValuationKindV1::ApiEquivalent;
    quote.exact_target.model_identity = PriceModelIdentityV1::CatalogModel(model.into());
    quote.rates = TokenRatesV1 {
        input_uncached: TokenRateV1::known(rate),
        output: TokenRateV1::known(rate),
        cache_read: TokenRateV1::known(rate),
        cache_write: TokenRateV1::known(rate),
    };
    quote.quote_digest = quote.computed_digest().unwrap();
    ApiEquivalentPriceV1 {
        binding_id: binding.into(),
        execution_model_configuration_id: None,
        quote,
    }
}
fn ingest(store: &LocalObservationStore, suffix: &str) -> Fixture {
    let f = Fixture::new(suffix);
    for e in facts(&f, UsageFrameKindV1::Cumulative) {
        store.ingest_fact(&e, &[]).unwrap();
    }
    store.settle_pending_valuations(16).unwrap();
    f
}

#[test]
fn analysis_freezes_prices_and_attempt_inputs_and_uses_the_existing_arithmetic() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let f = ingest(&store, "analysis-kernel");
    let actual = current_price("binding-a", "model-a", 1_000_000);
    let reference = current_price("binding-ref", "model-ref", 4_000_000);
    let legacy = store
        .observed_api_equivalent_value(
            &reader(),
            &ObservationValueQueryV2 {
                from_ms: 0,
                to_ms: 1000,
                session_id: None,
                plan_id: None,
                currency: Some("USD".into()),
            },
            &reference,
            std::slice::from_ref(&actual),
            1000,
        )
        .unwrap();
    let capture = AnalysisPricesV1 {
        generation_ref: "test-generation".into(),
        evaluated_at_ms: 1000,
        prices: vec![actual, reference],
        reference_binding_id: Some("binding-ref".into()),
    };
    let mut q = ObservationAnalysisQueryV1::new(AnalysisViewV1::SavingsSummary);
    q.currency = Some("USD".into());
    let page = store
        .observed_analysis(&reader(), &q, Some(&range()), Some(&capture), 1000)
        .unwrap();
    assert_eq!(page.status, AnalysisStatusV1::Ready, "{page:?}");
    let savings = page.savings.unwrap();
    assert_eq!(
        savings.estimated_savings_micros,
        legacy.estimated_savings_micros
    );
    assert_eq!(
        savings.compared_routed_subtotal_micros,
        legacy.compared_routed_subtotal_micros
    );
    assert_eq!(savings.compared_request_count, 1);
    q.analysis_scope = page.analysis_scope;
    q.session_id = Some(f.session.to_string());
    let mut changed = capture;
    changed.prices.clear();
    changed.evaluated_at_ms = 2000;
    let repeated = store
        .observed_analysis(&reader(), &q, None, Some(&changed), 1001)
        .unwrap();
    assert_eq!(
        repeated.savings.unwrap().estimated_savings_micros,
        savings.estimated_savings_micros
    );
    assert_eq!(repeated.price_capture_ref, page.price_capture_ref);
}

#[test]
fn retained_attempt_metadata_is_rebuilt_from_bounded_verified_original_facts() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let f = ingest(&store, "metadata-backfill");
    {
        let db = store.connection.lock();
        // Component admission fixture, not released-writer acceptance. Retain
        // the original serialized facts and remove only the derived metadata.
        for table in [
            "observation_analysis_models_v1",
            "observation_analysis_reasoning_v1",
        ] {
            db.execute(&format!("DELETE FROM {table}"), []).unwrap();
        }
        db.execute(
            "INSERT OR REPLACE INTO observation_analysis_dirty_v1 VALUES(?1,?2,1)",
            rusqlite::params![f.workspace.as_str(), f.request.as_str()],
        )
        .unwrap();
    }
    store.reconcile_analysis_batch(1000).unwrap();
    {
        let db = store.connection.lock();
        let models: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM observation_analysis_models_v1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(models, 1);
        let credential: String = db
            .query_row(
                "SELECT credential_digest FROM observation_analysis_models_v1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!credential.is_empty());
    }
}

#[test]
fn expired_scope_cleanup_preserves_a_newer_pinned_membership() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    ingest(&store, "scope-lease");
    let q = ObservationAnalysisQueryV1::new(AnalysisViewV1::Conversations);
    let first = store
        .observed_analysis(&reader(), &q, Some(&range()), None, 1000)
        .unwrap();
    let second = store
        .observed_analysis(&reader(), &q, Some(&range()), None, 2000)
        .unwrap();
    store
        .reconcile_analysis_batch(first.expires_at_ms.unwrap())
        .unwrap();
    let mut follow = q.clone();
    follow.analysis_scope = second.analysis_scope;
    assert_eq!(
        store
            .observed_analysis(&reader(), &follow, None, None, 2001)
            .unwrap()
            .conversations
            .len(),
        1
    );
    follow.analysis_scope = first.analysis_scope;
    assert!(matches!(
        store.observed_analysis(&reader(), &follow, None, None, 2001),
        Err(crate::query_v2::ObservationV2Error::Stale)
    ));
    let db = store.connection.lock();
    let maps: u64 = db
        .query_row(
            "SELECT COUNT(DISTINCT scope_id) FROM observation_analysis_members_v1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(maps, 1);
}

#[test]
fn ready_reopen_keeps_checkpoint_and_scopes_do_not_follow_later_insertions() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let f = ingest(&store, "analysis-reopen");
    let q = ObservationAnalysisQueryV1::new(AnalysisViewV1::Conversations);
    let page = store
        .observed_analysis(&reader(), &q, Some(&range()), None, 1000)
        .unwrap();
    assert_eq!(page.conversations.len(), 1);
    drop(store);
    let store = open_store(dir.path());
    let queued: u64 = store
        .connection
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM observation_analysis_dirty_v1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(queued, 0);
    ingest(&store, "analysis-later");
    let mut detail = ObservationAnalysisQueryV1::new(AnalysisViewV1::CostRanking);
    detail.analysis_scope = page.analysis_scope;
    detail.currency = Some("USD".into());
    let result = store
        .observed_analysis(&reader(), &detail, None, None, 1001)
        .unwrap();
    assert_ne!(result.status, AnalysisStatusV1::Pending);
    assert_eq!(result.costs.len(), 1);
    assert_eq!(result.costs[0].id, f.session.to_string());
}

#[test]
fn revision_cache_revalidates_prior_scopes_and_sql_notified_source_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let f = ingest(&store, "source-revision-cache");
    let q = ObservationAnalysisQueryV1::new(AnalysisViewV1::Conversations);
    let page = store
        .observed_analysis(&reader(), &q, Some(&range()), None, 1000)
        .unwrap();
    {
        let db = store.connection.lock();
        // A retained scope need not contain this optional derived cache.
        db.execute(
            "UPDATE observation_analysis_scopes_v1 SET body=json_remove(body,'$.source_revision')",
            [],
        )
        .unwrap();
    }
    let mut follow = ObservationAnalysisQueryV1::new(AnalysisViewV1::CostRanking);
    follow.analysis_scope = page.analysis_scope;
    follow.currency = Some("USD".into());
    let reused = store
        .observed_analysis(&reader(), &follow, None, None, 1001)
        .unwrap();
    assert_eq!(reused.costs.len(), 1);
    {
        let db = store.connection.lock();
        // Component source-guard fixture: even without the normal public
        // deletion's core revision bump, SQL notifications invalidate the cache.
        db.execute(
            "DELETE FROM logical_requests WHERE workspace_id=?1 AND request_id=?2",
            rusqlite::params![f.workspace.as_str(), f.request.as_str()],
        )
        .unwrap();
    }
    assert!(matches!(
        store.observed_analysis(&reader(), &follow, None, None, 1002),
        Err(crate::query_v2::ObservationV2Error::Stale)
    ));
}

#[test]
fn pinned_scope_reads_survive_an_unrelated_source_write_and_held_writer() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let first = ingest(&store, "scope-held-writer");
    let q = ObservationAnalysisQueryV1::new(AnalysisViewV1::Conversations);
    let initial = store
        .observed_analysis(&reader(), &q, Some(&range()), None, 1000)
        .unwrap();
    ingest(&store, "scope-unrelated-source");
    let mut follow = q;
    follow.analysis_scope = initial.analysis_scope.clone();
    let writer = store.connection.lock();
    let transaction = writer.unchecked_transaction().unwrap();
    transaction
        .execute("UPDATE observation_analysis_meta_v1 SET value=value", [])
        .unwrap();
    let repeated = store
        .observed_analysis(&reader(), &follow, None, None, 1001)
        .unwrap();
    assert_eq!(repeated.analysis_scope, initial.analysis_scope);
    assert_eq!(repeated.conversations.len(), 1);
    assert_eq!(
        repeated.conversations[0].analysis_session_id,
        first.session.to_string()
    );
    transaction.rollback().unwrap();
}

#[test]
fn unknown_analysis_format_does_not_reject_core_facts() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    store
        .connection
        .lock()
        .execute(
            "UPDATE observation_analysis_meta_v1 SET value='future' WHERE key='format'",
            [],
        )
        .unwrap();
    drop(store);
    let store = open_store(dir.path());
    ingest(&store, "analysis-future");
    let q = ObservationAnalysisQueryV1::new(AnalysisViewV1::Conversations);
    assert_eq!(
        store
            .observed_analysis(&reader(), &q, Some(&range()), None, 1000)
            .unwrap_err(),
        crate::query_v2::ObservationV2Error::Unavailable
    );
}

#[test]
fn auto_selected_price_options_survive_pagination_without_repeating_flags() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    ingest(&store, "auto-price-a");
    ingest(&store, "auto-price-b");
    let capture = AnalysisPricesV1 {
        generation_ref: "auto-generation".into(),
        evaluated_at_ms: 1000,
        prices: vec![current_price("actual", "model-a", 1_000_000)],
        reference_binding_id: None,
    };
    let mut q = ObservationAnalysisQueryV1::new(AnalysisViewV1::CostRanking);
    q.limit = 1;
    q.basis = Some(AnalysisCostBasisV1::ApiEquivalent);
    q.currency = Some("USD".into());
    let first = store
        .observed_analysis(&reader(), &q, Some(&range()), Some(&capture), 1000)
        .unwrap();
    assert_eq!(first.costs.len(), 1);
    q.cursor = first.next_cursor;
    q.currency = None;
    q.basis = None;
    let second = store
        .observed_analysis(&reader(), &q, None, None, 1001)
        .unwrap();
    assert_eq!(second.costs.len(), 1);
    assert_ne!(second.costs[0].id, first.costs[0].id);
    assert_eq!(first.analysis_scope, second.analysis_scope);
    assert_eq!(first.price_capture_ref, second.price_capture_ref);
}

#[test]
fn recorded_basis_choices_and_missing_rows_use_the_same_filtered_population() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let mut fixtures = Vec::new();
    for (suffix, currency, kind) in [
        ("basis-usd", "USD", PriceValuationKindV1::UsageEstimate),
        ("basis-usd-api", "USD", PriceValuationKindV1::ApiEquivalent),
        ("basis-cny", "CNY", PriceValuationKindV1::UsageEstimate),
        ("basis-missing", "", PriceValuationKindV1::UsageEstimate),
    ] {
        let f = Fixture::new(suffix);
        for mut event in facts(&f, UsageFrameKindV1::Cumulative) {
            if currency.is_empty() {
                event.pricing = None;
            } else if let Some(quote) = event.pricing.as_mut().and_then(|p| p.quote.as_mut()) {
                quote.currency = currency.into();
                quote.exact_target.currency = currency.into();
                quote.valuation_kind = kind;
                quote.exact_target.valuation_kind = kind;
                quote.quote_digest = quote.computed_digest().unwrap();
            }
            store.ingest_fact(&event, &[]).unwrap();
        }
        fixtures.push(f);
    }
    store.settle_pending_valuations(16).unwrap();
    let read = |q: &ObservationAnalysisQueryV1| {
        store
            .observed_analysis(&reader(), q, Some(&range()), None, 1000)
            .unwrap()
    };
    let mut q = ObservationAnalysisQueryV1::new(AnalysisViewV1::CostRanking);
    let ambiguous = read(&q);
    assert_eq!(ambiguous.status, AnalysisStatusV1::ChoicesRequired);
    assert!(ambiguous.choices.iter().any(|c| c.field == "currency"));
    q.currency = Some("USD".into());
    assert!(read(&q).choices.iter().any(|c| c.field == "valuation_kind"));
    q.valuation_kind = Some(PriceValuationKindV1::UsageEstimate);
    let selected = read(&q);
    assert!(selected.choices.is_empty());
    assert_eq!(
        selected.costs.iter().map(|r| r.request_count).sum::<u64>(),
        4
    );
    assert_eq!(
        selected.costs.iter().map(|r| r.attempt_count).sum::<u64>(),
        4
    );
    assert_eq!(
        selected
            .costs
            .iter()
            .filter(|r| r.amount_micros.is_some())
            .count(),
        1
    );
    assert!(selected.costs.iter().all(|r| r.currency == "USD"
        && r.valuation_kind == PriceValuationKindV1::UsageEstimate));
    q.currency = None;
    q.valuation_kind = None;
    q.request_id = Some(fixtures[0].request.to_string());
    let narrowed = read(&q);
    assert!(narrowed.choices.is_empty());
    assert_eq!(narrowed.costs.len(), 1);
    assert!(narrowed.costs[0].amount_micros.is_some());
    assert_eq!(narrowed.costs[0].currency, "USD");
}

#[test]
fn request_filter_applies_before_conversation_counts_and_member_previews() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let a = ingest(&store, "member-filter-a");
    ingest(&store, "member-filter-b");
    let mut q = ObservationAnalysisQueryV1::new(AnalysisViewV1::Conversations);
    q.request_id = Some(a.request.to_string());
    let page = store
        .observed_analysis(&reader(), &q, Some(&range()), None, 1000)
        .unwrap();
    assert_eq!(page.conversations.len(), 1);
    let row = &page.conversations[0];
    assert_eq!(row.request_count, 1);
    assert_eq!(row.member_count, 1);
    assert_eq!(row.members[0].session_id, a.session.to_string());
    assert!(!row.members_partial);
}

#[test]
fn a_recorded_stage_is_evidence_for_a_selected_transition_not_a_switch() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let f = Fixture::new("recorded-reason");
    let mut execution = facts(&f, UsageFrameKindV1::Cumulative);
    let mut finished = execution.pop().unwrap();
    for e in execution {
        assert!(matches!(
            store.ingest_fact(&e, &[]).unwrap(),
            IngestOutcome::Ack(_)
        ));
    }
    // A different request can start in the same millisecond without belonging
    // to this native turn. Only the recorded endpoint IDs prove membership.
    let mut neighbor = Fixture::new("same-millisecond-neighbor");
    neighbor.session = f.session.clone();
    for e in facts(&neighbor, UsageFrameKindV1::Cumulative) {
        store.ingest_fact(&e, &[]).unwrap();
    }
    let outcome = store
        .ingest_fact(
            &f.fact(
                10,
                ExecutionFactV1::AgentTurnFinished {
                    history_membership: None,
                    opening_cause: None,
                    branch_execution: None,
                    agent_turn_id: "native-turn-1".into(),
                    segment_id: "stage-1".into(),
                    ordinal: 1,
                    plan_id: AgentPlanId::parse("plan/codex-daily").unwrap(),
                    plan_revision: 17,
                    selected_branch_id: "economy".into(),
                    executed_branch_id: Some("economy".into()),
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
                },
                110,
            ),
            &[],
        )
        .unwrap();
    assert!(matches!(outcome, IngestOutcome::Ack(_)), "{outcome:?}");
    let neighbor_turns: u64 = store
        .connection
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM observation_analysis_turns_v1 WHERE workspace_id=?1 AND request_id=?2",
            rusqlite::params![neighbor.workspace.as_str(), neighbor.request.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(neighbor_turns, 0);
    finished.sequence = 11;
    finished.event_id = EventId::parse("fact-event-11").unwrap();
    finished.occurred_at_unix_nanos = 111_000_000;
    assert!(matches!(
        store.ingest_fact(&finished, &[]).unwrap(),
        IngestOutcome::Ack(_)
    ));
    store.settle_pending_valuations(16).unwrap();
    let mut q = ObservationAnalysisQueryV1::new(AnalysisViewV1::Routing);
    q.session_id = Some(f.session.to_string());
    let page = store
        .observed_analysis(&reader(), &q, Some(&range()), None, 1000)
        .unwrap();
    assert_eq!(page.transitions.len(), 2);
    let t = page
        .transitions
        .iter()
        .find(|t| t.request_id == f.request.as_str())
        .unwrap();
    assert_eq!(t.kind, AnalysisTransitionKindV1::InitialSelection, "{t:?}");
    assert_eq!(t.reason.as_ref().unwrap().segment_id, "stage-1");
    assert!(!page.transition_counts.contains_key("between_turn_change"));
    let timeline = store
        .observed_routing_timeline(
            &reader(),
            &RoutingTimelineQueryV1 {
                session_id: f.session.to_string(),
                after_watermark: None,
                limit: 50,
            },
            1000,
        )
        .unwrap();
    let turn = timeline
        .turns
        .iter()
        .find(|turn| turn.agent_turn_id.as_deref() == Some("native-turn-1"))
        .unwrap();
    assert_eq!(
        turn.requests
            .iter()
            .map(|r| r.request_id.as_str())
            .collect::<Vec<_>>(),
        vec![f.request.as_str()]
    );
    let classified = turn
        .transitions
        .iter()
        .find(|change| change.request_id == f.request.as_str())
        .unwrap();
    assert_eq!(
        classified.kind, t.kind,
        "retained and live readers must agree at a timestamp tie"
    );
}

#[test]
fn numeric_legacy_comparison_preserves_the_original_reader_output() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    for suffix in ["legacy-numeric-a", "legacy-numeric-b", "legacy-numeric-c"] {
        ingest(&store, suffix);
    }
    let q = ObservationValueQueryV2 {
        from_ms: 0,
        to_ms: 1000,
        session_id: None,
        plan_id: None,
        currency: Some("USD".into()),
    };
    let prices = vec![current_price("actual", "model-a", 350_001)];
    let reference = current_price("reference", "another-model", 1_675_531);
    let projected = store
        .observed_api_equivalent_value(&reader(), &q, &reference, &prices, 1000)
        .unwrap();
    store
        .connection
        .lock()
        .execute(
            "UPDATE observation_analysis_meta_v1 SET value='future' WHERE key='format'",
            [],
        )
        .unwrap();
    let original = store
        .observed_api_equivalent_value(&reader(), &q, &reference, &prices, 1000)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&projected).unwrap(),
        serde_json::to_value(&original).unwrap()
    );
    assert_eq!(projected.compared_request_count, 3);
}

#[test]
fn deleting_and_reusing_the_last_source_row_cannot_replace_a_frozen_population() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let a = ingest(&store, "population-old");
    let q = ObservationAnalysisQueryV1::new(AnalysisViewV1::Conversations);
    let first = store
        .observed_analysis(&reader(), &q, Some(&range()), None, 1000)
        .unwrap();
    store
        .connection
        .lock()
        .execute(
            "DELETE FROM logical_requests WHERE workspace_id=?1 AND request_id=?2",
            rusqlite::params![a.workspace.as_str(), a.request.as_str()],
        )
        .unwrap();
    let mut replacement = Fixture::new("population-replacement");
    replacement.session = a.session;
    for e in facts(&replacement, UsageFrameKindV1::Cumulative) {
        store.ingest_fact(&e, &[]).unwrap();
    }
    let mut detail = q;
    detail.analysis_scope = first.analysis_scope;
    assert_eq!(
        store
            .observed_analysis(&reader(), &detail, None, None, 1001)
            .unwrap_err(),
        crate::query_v2::ObservationV2Error::Stale
    );
}
