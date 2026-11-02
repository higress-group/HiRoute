use hiroute_domain::{
    FactsCompleteness, ObservationQueryPort, SEVEN_DAYS_MILLIS, VALUE_LEDGER_SCHEMA_V1,
    ValueGroupByV1,
};
use rusqlite::{Connection, params};
use tempfile::tempdir;

use crate::LocalObservationStore;

use super::support::*;

#[test]
fn daily_value_isolated_by_plan_and_currency_with_negative_and_unknown_semantics() {
    let temporary = tempdir().unwrap();
    let plan_a_usd = Fixture::new("plan-a-usd");
    let plan_a_cny = Fixture::new("plan-a-cny");
    let plan_b_usd = Fixture::new("plan-b-usd");
    let store = open_store(temporary.path());
    install_value_request(
        &store,
        &plan_a_usd,
        "receipt-plan-a-usd",
        "attempt-plan-a-usd",
        finished_with_value(
            "attempt-plan-a-usd",
            "plan/a",
            "USD",
            Some(100),
            Some(150),
            Some(175),
        ),
        100,
    );
    install_value_request(
        &store,
        &plan_a_cny,
        "receipt-plan-a-cny",
        "attempt-plan-a-cny",
        finished_with_value(
            "attempt-plan-a-cny",
            "plan/a",
            "CNY",
            Some(1_000),
            Some(800),
            Some(500),
        ),
        200,
    );
    install_value_request(
        &store,
        &plan_b_usd,
        "receipt-plan-b-usd",
        "attempt-plan-b-usd",
        finished_with_value(
            "attempt-plan-b-usd",
            "plan/b",
            "USD",
            None,
            Some(200),
            Some(250),
        ),
        300,
    );
    assert_eq!(store.run_retention(SEVEN_DAYS_MILLIS + 10_000).unwrap(), 3);
    assert_eq!(daily_row_count(&store), 3);

    let mut plan_a_usd_query = value_query("plan/a", "USD", None);
    plan_a_usd_query.group_by = ValueGroupByV1::Day;
    let negative = store
        .get_value(&plan_a_usd.workspace, &plan_a_usd_query)
        .unwrap();
    assert_eq!(negative.group_by, ValueGroupByV1::Day);
    assert_eq!(negative.groups.len(), 1);
    assert_eq!(negative.groups[0].day_number, Some(0));
    assert_eq!(negative.groups[0].routing_savings_micros, Some(-50));
    assert!(!negative.groups[0].detail_available);
    assert_eq!(negative.daily_aggregates.len(), 1);
    assert_eq!(negative.baseline_api_equivalent_cost_micros, Some(100));
    assert_eq!(negative.chosen_api_equivalent_cost_micros, Some(150));
    assert_eq!(negative.actual_incremental_cost_micros, Some(175));
    assert_eq!(negative.routing_savings_micros, Some(-50));
    assert_eq!(negative.entitlement_savings_micros, Some(-25));
    assert_eq!(negative.estimated_total_savings_micros, Some(-75));
    assert!(!negative.detail_available);

    let cny = store
        .get_value(&plan_a_cny.workspace, &value_query("plan/a", "CNY", None))
        .unwrap();
    assert_eq!(cny.daily_aggregates.len(), 1);
    assert_eq!(cny.estimated_total_savings_micros, Some(500));
    assert_eq!(cny.currency, "CNY");

    let unknown = store
        .get_value(&plan_b_usd.workspace, &value_query("plan/b", "USD", None))
        .unwrap();
    assert_eq!(unknown.daily_aggregates.len(), 1);
    assert_eq!(unknown.baseline_api_equivalent_cost_micros, None);
    assert_eq!(unknown.routing_savings_micros, None);
    assert_eq!(unknown.entitlement_savings_micros, Some(-50));
    assert_eq!(unknown.estimated_total_savings_micros, None);
    assert_eq!(unknown.facts_completeness, FactsCompleteness::Complete);

    drop(store);
    let reopened = open_store(temporary.path());
    assert_eq!(daily_row_count(&reopened), 3);
    let after_restart = reopened
        .get_value(&plan_a_usd.workspace, &value_query("plan/a", "USD", None))
        .unwrap();
    assert_eq!(after_restart.routing_savings_micros, Some(-50));
    assert_eq!(after_restart.price_version_refs.len(), 1);
    assert_eq!(after_restart.price_override_revision_refs.len(), 1);
}

#[test]
fn value_query_uses_required_half_open_range() {
    let temporary = tempdir().unwrap();
    let fixture = Fixture::new("half-open");
    let store = open_store(temporary.path());
    install_value_request(
        &store,
        &fixture,
        "receipt-half-open",
        "attempt-half-open",
        finished("attempt-half-open"),
        100,
    );
    let mut query = value_query("plan/codex-daily", "USD", None);
    query.group_by = ValueGroupByV1::Day;
    query.from_ms = 105;
    query.to_ms = 106;
    let included = store.get_value(&fixture.workspace, &query).unwrap();
    assert_eq!(included.entries.len(), 1);
    assert_eq!(included.groups.len(), 1);
    assert_eq!(included.groups[0].day_number, Some(0));
    assert!(included.groups[0].detail_available);
    query.from_ms = 106;
    query.to_ms = 107;
    assert!(
        store
            .get_value(&fixture.workspace, &query)
            .unwrap()
            .entries
            .is_empty()
    );
    query.from_ms = 107;
    query.to_ms = 107;
    assert!(store.get_value(&fixture.workspace, &query).is_err());
}

#[test]
fn legacy_value_data_is_untouched_by_current_format_admission() {
    let temporary = tempdir().unwrap();
    let fixture = Fixture::new("migration");
    let activity_path = temporary.path().join("activity.db");
    let connection = Connection::open(&activity_path).unwrap();
    connection
        .execute_batch(
            r#"
            CREATE TABLE observation_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO observation_meta(key, value) VALUES
                ('schema_version', '1'), ('store_revision', '7');
            CREATE TABLE value_ledger_entries (
                workspace_id TEXT NOT NULL, request_id TEXT NOT NULL, session_id TEXT NOT NULL,
                receipt_id TEXT NOT NULL, body_json TEXT NOT NULL, body_digest TEXT NOT NULL,
                input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
                cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL,
                reasoning_tokens INTEGER NOT NULL, api_equivalent_micros INTEGER NOT NULL,
                incremental_cost_micros INTEGER NOT NULL, savings_micros INTEGER NOT NULL,
                facts_completeness TEXT NOT NULL, frozen_at_ms INTEGER NOT NULL,
                PRIMARY KEY(workspace_id, request_id)
            );
            CREATE TABLE daily_value_rollups (
                workspace_id TEXT NOT NULL, day_number INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
                cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL,
                reasoning_tokens INTEGER NOT NULL, api_equivalent_micros INTEGER NOT NULL,
                incremental_cost_micros INTEGER NOT NULL, savings_micros INTEGER NOT NULL,
                facts_completeness TEXT NOT NULL,
                PRIMARY KEY(workspace_id, day_number)
            );
            "#,
        )
        .unwrap();
    let body = serde_json::json!({
        "schema": VALUE_LEDGER_SCHEMA_V1,
        "workspace_id": fixture.workspace.as_str(),
        "session_id": fixture.session.as_str(),
        "turn_id": fixture.turn.as_str(),
        "request_id": fixture.request.as_str(),
        "receipt_id": "receipt-migration",
        "usage": {
            "input_tokens": 100,
            "output_tokens": 25,
            "cache_read_tokens": 40,
            "cache_write_tokens": 5,
            "reasoning_tokens": 7
        },
        "frozen": {
            "currency": "USD",
            "billing_unit": "micro_usd",
            "price_version": "price-v1",
            "price_override_revision": "override-v1",
            "api_equivalent_micros": 900,
            "incremental_cost_micros": 300,
            "savings_micros": 600
        },
        "facts_completeness": "complete",
        "frozen_at_ms": 105
    })
    .to_string();
    connection
        .execute(
            "INSERT INTO value_ledger_entries VALUES
             (?1, ?2, ?3, 'receipt-migration', ?4, 'legacy-digest',
              100, 25, 40, 5, 7, 900, 300, 600, 'complete', 105)",
            params![
                fixture.workspace.as_str(),
                fixture.request.as_str(),
                fixture.session.as_str(),
                body,
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO daily_value_rollups VALUES
             (?1, 0, 999, 0, 0, 0, 0, 999, 0, 999, 'complete')",
            [fixture.workspace.as_str()],
        )
        .unwrap();
    drop(connection);

    let mut historical = Connection::open(&activity_path).unwrap();
    assert!(super::super::schema::migrate(&mut historical).is_err());
    let retained: String = historical
        .query_row("SELECT body_json FROM value_ledger_entries", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(retained, body);
    drop(historical);
    assert!(LocalObservationStore::open(temporary.path(), authority()).is_err());
}

fn daily_row_count(store: &LocalObservationStore) -> u64 {
    let connection = store.connection.lock();
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM daily_value_rollups", [], |row| {
            row.get(0)
        })
        .unwrap();
    count.try_into().unwrap()
}
