//! V38 single-rate reservations retain their original bytes across the V39 marker.
use super::*;
use hiroute_domain::*;
use serde_json::json;

#[test]
fn schema38_key_spending_preserves_original_estimates_balances_and_pending_holds() {
    let root = crate::test_tempdir().unwrap();
    let path = root.path().join("control.db");
    let mut db = Connection::open(&path).unwrap();
    // Produce the historical SQL format with its own migration target.
    migrate(
        &mut db,
        &path,
        DatabaseKind::Control,
        &root.path().join("backups"),
        None,
        None,
        None,
        Some(38),
    )
    .unwrap();
    let digest = CanonicalDigest::of_bytes(b"schema38-key-spend");
    let rate = json!({"state":"known","micros_per_million_tokens":1_000_000});
    let mut quote: FrozenPriceQuoteV1 = serde_json::from_value(json!({
        "generation_ref":null,
        "exact_target":{"workspace_id":"default", "source_id":"source/old",
            "source_identity_digest":digest, "model_identity":{"kind":"local_model","id":"model/old"},
            "currency":"USD", "valuation_kind":"usage_estimate"},
        "actual_source_ref":"source/old", "actual_offer_ref":null, "reference_model_offer_ref":null,
        "valuation_kind":"usage_estimate", "currency":"USD", "unit":"micros_per_million_tokens",
        "rates":{"input_uncached":rate,"output":rate,"cache_read":rate,"cache_write":rate},
        "origin":"catalog", "applied_override_refs":[], "selected_rule_refs":[],
        "attempt_execution_at":1, "applied_schedule":null, "billing_context":{"kind":"standard_tokens"},
        "unknown_reasons":[], "quote_digest":digest
    })).unwrap();
    quote.quote_digest = quote.computed_digest().unwrap();
    // Explicit released fields: do not derive a legacy fixture by serializing
    // a new estimate or removing fields from a current database.
    let estimate = json!({"input_bound":100,"output_bound":100,"pricing":{
        "schema_version":EXECUTION_PRICING_SCHEMA_V1,"request_generation":null,
        "captured_at_ms":1000,"attempt_execution_at_ms":1000,"quote":quote,
        "reference_quote":null,"unknown_reason":null,
        "usage_semantics":{"frame_kind":"cumulative","input":"includes_exclusive_cache",
            "output":"includes_reasoning","cache_buckets_exclusive":true}
    }})
    .to_string();
    let definition = json!({"budget_id":"budget/old","revision":1,"display_name":"Old budget",
        "policy":{"currency":"USD","amount_micros":1000,"renewal":null}})
    .to_string();
    db.execute("INSERT INTO compute_key_budgets(workspace_id,budget_id,revision,definition_json,balance_micros,reserved_micros,spent_micros,lifetime_spent_micros) VALUES('default','budget/old',1,?1,970,200,30,30)", [&definition]).unwrap();
    db.execute("INSERT INTO compute_key_spend_reservations(workspace_id,reservation_id,budget_id,key_id,key_generation,budget_epoch,amount_micros,pricing_json,state,created_at_ms) VALUES('default','reservation/old','budget/old','key/old',1,0,200,?1,'pending',1000)", [&estimate]).unwrap();
    key_budgets_v37::validate(&db).unwrap();
    migrate(
        &mut db,
        &path,
        DatabaseKind::Control,
        &root.path().join("backups"),
        None,
        None,
        None,
        Some(39),
    )
    .unwrap();
    key_budgets_v37::validate(&db).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        39
    );
    let after: (String, String, i64, i64, i64, i64, String) = db.query_row(
        "SELECT b.definition_json,r.pricing_json,b.balance_micros,b.reserved_micros,b.spent_micros,b.lifetime_spent_micros,r.state FROM compute_key_budgets b JOIN compute_key_spend_reservations r USING(workspace_id,budget_id)", [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).unwrap();
    assert_eq!(
        after,
        (definition, estimate, 970, 200, 30, 30, "pending".into())
    );
}
