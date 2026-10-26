//! Frozen aggregate control versus the actual reader, without model calls.
use super::*;
use rusqlite::{Connection, named_params, params};
use std::{cell::Cell, time::Instant};

thread_local! {
    static PROOF_QUERIES: Cell<usize> = const { Cell::new(0) };
}

fn count_proof_query(sql: &str) {
    if sql.starts_with("WITH routes AS MATERIALIZED") {
        PROOF_QUERIES.with(|n| n.set(n.get() + 1));
    }
}

fn clone_requests(db: &Connection, table: &str, seed: &str, count: usize) {
    let columns: Vec<String> = db
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let names = columns
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(",");
    let values = columns
        .iter()
        .map(|s| {
            if s == "request_id" {
                "'unresolved-'||n".into()
            } else {
                format!("seed.\"{s}\"")
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    db.execute(&format!("WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<?1)
        INSERT INTO {table}({names}) SELECT {values} FROM {table} seed,numbers WHERE seed.request_id=?2"),
        params![count-1,seed]).unwrap();
}

#[test]
fn trial_cost_missing_purpose_has_bounded_work_without_truncating_unknowns() {
    let mut failures = Vec::new();
    for mode in ["missing", "deleted", "invalid"] {
        for count in [128usize, 2048, 16384] {
            let dir = tempfile::tempdir().unwrap();
            let store = open_store(dir.path());
            let f = ingest(&store, "unresolved-seed");
            let mut db = store.connection.lock();
            let tx = db.transaction().unwrap();
            for table in [
                "logical_requests",
                "valuation_requests_v2",
                "observation_analysis_requests_v1",
            ] {
                clone_requests(&tx, table, f.request.as_str(), count);
            }
            tx.execute_batch(
                "UPDATE sessions SET updated_at_ms=0; DELETE FROM observation_analysis_dirty_v1;",
            )
            .unwrap();
            if mode != "missing" {
                // Retained route indexes without original carriers are not
                // authenticated purpose evidence. This also exercises index hits.
                tx.execute("WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<?1)
                    INSERT INTO observation_safe_facts_v2(workspace_id,request_id,original_digest,body_json)
                    SELECT ?2,'unresolved-'||n,'unretained-'||n,json_object('event_kind','route_decision') FROM numbers",
                    params![count-1,f.workspace.as_str()]).unwrap();
                if mode == "deleted" {
                    tx.execute_batch("DELETE FROM observation_sensitive_payloads_v2;")
                        .unwrap();
                } else {
                    // A present carrier without a retained authenticated fact
                    // also costs lookup work without consuming body bytes.
                    tx.execute("WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<?1)
                        INSERT INTO observation_sensitive_payloads_v2(id,workspace_id,session_id,request_id,created_ms,body_json)
                        SELECT 'fact:unretained-'||n,?2,?3,'unresolved-'||n,100,'{}' FROM numbers",
                        params![count-1,f.workspace.as_str(),f.session.as_str()]).unwrap();
                }
            } else {
                tx.execute_batch("DELETE FROM observation_safe_facts_v2;")
                    .unwrap();
            }
            tx.commit().unwrap();
            let now = 1_000_000;
            let scope = TrustCostScopeV1 {
                plan_id: "plan/codex-daily".into(),
                revision: Some(17),
                branch_id: Some("smart_saving".into()),
                group: ExecutionGroupV1::Regular,
                from_ms: 0,
                to_ms: 1000,
                currency: "USD".into(),
            };
            let reader = ObservationReaderContext::local_user(
                WorkspaceId::default(),
                "cost-work".into(),
                1,
                now + 1000,
                false,
                false,
            )
            .unwrap();
            let reference = current_price("reference", "model-ref", 4_000_000);
            let prices = [current_price("actual", "model-a", 1_000_000)];
            let tx = db.transaction().unwrap();
            let started = Instant::now();
            let (pending, gaps): (bool, u64) = tx.query_row(include_str!("fixtures/trust_cost_aggregate_d061.sql"), named_params! {
                ":workspace":f.workspace.as_str(),":plan":scope.plan_id,":revision":scope.revision,":branch":scope.branch_id,
                ":group":"regular",":from":0,":to":1000,":idle":now-300_000,
            }, |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
            let aggregate_us = started.elapsed().as_micros();
            assert!(!pending);
            assert_eq!(gaps, count as u64);
            tx.commit().unwrap();
            PROOF_QUERIES.with(|n| n.set(0));
            db.trace(Some(count_proof_query));
            let tx = db.transaction().unwrap();
            let started = Instant::now();
            let measured = crate::analysis::trust_comparison(
                &tx, &reader, &scope, &reference, &prices, 0, now,
            )
            .unwrap();
            let reader_us = started.elapsed().as_micros();
            tx.commit().unwrap();
            db.trace(None);
            let queries = PROOF_QUERIES.with(Cell::get);
            assert_eq!(
                (measured.total, measured.excluded, measured.compared),
                (count as u64, count as u64, 0)
            );
            drop(db);
            let started = Instant::now();
            let bounded = store.observed_trust_cost(&reader, &scope, &reference, &prices, now);
            let product_us = started.elapsed().as_micros();
            let status = match &bounded {
                Ok(report) => {
                    assert_eq!(
                        (
                            report.request_count,
                            report.missing_request_count,
                            report.compared_request_count
                        ),
                        (count as u64, count as u64, 0)
                    );
                    assert_eq!(report.coverage, ObservationMetricCoverageV2::Unknown);
                    "ok".into()
                }
                Err(error) => format!("{error:?}"),
            };
            println!(
                "purpose_work mode={mode} requests={count} aggregate_us={aggregate_us} reader_us={reader_us} proof_queries={queries} product_us={product_us} product={status}"
            );
            if queries > 128 || bounded.is_err() {
                failures.push(format!(
                    "mode={mode}, requests={count}, proof_queries={queries}, product={status}"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "unbounded proof lookup or cost reader failure: {failures:?}"
    );
}
