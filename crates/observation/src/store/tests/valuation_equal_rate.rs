use super::*;

#[test]
fn frozen_equal_rate_amount_is_settled_and_retained_timeline_recovers_without_rewriting() {
    use crate::valuation::{MetricCoverage, UsageBucketsV2, VALUATION_ALGORITHM, value_attempt};
    for (equal_rate, stale_input, pricing_fault) in [
        (true, false, 0),
        (false, false, 0),
        (true, true, 0),
        (true, false, 1),
        (true, false, 2),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fixture = Fixture::new("valuation-equal-write");
        let store = open_store(directory.path());
        let mut frozen = None;
        for mut fact in facts(&fixture, UsageFrameKindV1::Cumulative) {
            if let Some(pricing) = &mut fact.pricing {
                let quote = pricing.quote.as_mut().unwrap();
                quote.rates.cache_write = if equal_rate {
                    quote.rates.input_uncached.clone()
                } else {
                    TokenRateV1::unknown(PriceUnknownReasonV1::CacheRateNotCollected)
                };
                quote.quote_digest = quote.computed_digest().unwrap();
                frozen = Some(pricing.clone());
            }
            if let ExecutionFactV1::UsageAndCache {
                cache_write_tokens,
                cache_write_provenance,
                ..
            } = &mut fact.fact
            {
                *cache_write_tokens = None;
                *cache_write_provenance = UsageProvenanceV1::Unknown;
            }
            assert!(matches!(
                store.ingest_fact(&fact, &[]).unwrap(),
                IngestOutcome::Ack(_)
            ));
        }
        store.settle_pending_valuations(16).unwrap();
        let settled = latest(&store, &fixture);
        assert_eq!(
            settled.amounts[0].known_sum_micros,
            Some(if equal_rate { 114 } else { 54 })
        );
        assert_eq!(
            settled.amounts[0].coverage,
            if equal_rate {
                MetricCoverage::Complete
            } else {
                MetricCoverage::Partial
            }
        );
        let stored_usage: String = store.connection.lock().query_row(
            "SELECT usage_json FROM valuation_attempt_inputs_v2 WHERE workspace_id=?1 AND request_id=?2",
            rusqlite::params![fixture.workspace.as_str(), fixture.request.as_str()],
            |row| row.get(0),
        ).unwrap();
        assert!(
            serde_json::from_str::<serde_json::Value>(&stored_usage).unwrap()["write"].is_null()
        );
        assert_eq!(settled.attempts[0].components[3].tokens, None);
        drop(store);
        let store = open_store(directory.path());
        assert_eq!(latest(&store, &fixture), settled);
        let reader = ObservationReaderContext::local_user(
            fixture.workspace.clone(),
            "user".into(),
            1,
            10_000,
            false,
            false,
        )
        .unwrap();
        let summary = store
            .observed_valuation_summary(&reader, &fixture.request, 1000)
            .unwrap();
        assert_eq!(
            summary.amounts[0].known_sum_micros,
            settled.amounts[0].known_sum_micros
        );

        // Model an existing v2 settlement of these same immutable inputs. The
        // timeline may derive a display amount, but must never rewrite this row.
        let mut legacy = settled.clone();
        legacy.algorithm = VALUATION_ALGORITHM.into();
        legacy.attempts[0] = value_attempt(
            frozen.as_ref().unwrap().quote.as_ref().unwrap(),
            &UsageBucketsV2::from_inclusive_input(Some(100), Some(25), Some(40), None),
        )
        .unwrap();
        legacy.amounts[0].known_sum_micros = Some(54);
        legacy.amounts[0].coverage = MetricCoverage::Partial;
        legacy.amounts[0].missing_attempt_count = 1;
        let before = serde_json::to_string(&legacy).unwrap();
        {
            let db = store.connection.lock();
            db.execute(
                "UPDATE valuation_records_v2 SET body_json=?3 WHERE workspace_id=?1 AND request_id=?2",
                rusqlite::params![fixture.workspace.as_str(), fixture.request.as_str(), before],
            ).unwrap();
            if stale_input {
                db.execute(
                    "UPDATE valuation_requests_v2 SET input_digest='stale' WHERE workspace_id=?1 AND request_id=?2",
                    rusqlite::params![fixture.workspace.as_str(), fixture.request.as_str()],
                ).unwrap();
            }
            if pricing_fault != 0 {
                let corrupted = if pricing_fault == 1 {
                    let mut other = frozen.clone().unwrap();
                    let quote = other.quote.as_mut().unwrap();
                    quote.rates.input_uncached = TokenRateV1::known(2_000_000);
                    quote.rates.cache_write = quote.rates.input_uncached.clone();
                    quote.quote_digest = quote.computed_digest().unwrap();
                    serde_json::to_string(&other).unwrap()
                } else {
                    "{".to_owned()
                };
                db.execute(
                    "UPDATE valuation_attempt_inputs_v2 SET pricing_json=?3 WHERE workspace_id=?1 AND request_id=?2",
                    rusqlite::params![fixture.workspace.as_str(), fixture.request.as_str(), corrupted],
                ).unwrap();
            }
        }
        drop(store);
        let store = open_store(directory.path());
        let page = store
            .observed_routing_timeline(
                &reader,
                &RoutingTimelineQueryV1 {
                    include_presentation: false,
                    session_id: fixture.session.to_string(),
                    after_watermark: None,
                    limit: 50,
                },
                1000,
            )
            .unwrap();
        let request = page
            .turns
            .iter()
            .flat_map(|turn| &turn.requests)
            .find(|request| request.request_id == fixture.request.as_str())
            .unwrap();
        assert_eq!(request.costs.len(), 1);
        let can_derive = equal_rate && !stale_input && pricing_fault == 0;
        assert_eq!(
            request.costs[0].known_micros,
            Some(if can_derive { 114 } else { 54 })
        );
        assert_eq!(
            request.costs[0].unknown_contributions,
            u64::from(!can_derive)
        );
        let after: String = store.connection.lock().query_row(
            "SELECT body_json FROM valuation_records_v2 WHERE workspace_id=?1 AND request_id=?2",
            rusqlite::params![fixture.workspace.as_str(), fixture.request.as_str()], |row| row.get(0),
        ).unwrap();
        assert_eq!(before, after);
    }
}
