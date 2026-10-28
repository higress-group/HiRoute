use super::{plan_quality::summary_sql, routing_observations::SUMMARY_SQL};
use crate::WriterCycleOutcome;
use crate::store::tests::support::{
    Fixture, fact_channel, finished, offer_fact, open_store, request_facts, writer,
};
use hiroute_domain::CanonicalDigest;
use rusqlite::{Connection, StatementStatus, functions::FunctionFlags, named_params, types::Value};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub(super) const PLAN: &str = "plan/codex-daily";

pub(super) fn seed() -> (
    tempfile::TempDir,
    std::sync::Arc<crate::LocalObservationStore>,
) {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(root.path());
    let fixture = Fixture::new("quality-read-work");
    let channel = fact_channel(&fixture, 256 * 1024);
    let writer = writer(&store);
    for fact in request_facts(&fixture, "attempt-bounds", finished("attempt-bounds"), 1000) {
        assert!(matches!(
            offer_fact(&writer, &channel, fact),
            WriterCycleOutcome::Ack(_)
        ));
    }
    {
        let db = store.connection.lock();
        // Many stages can share one request across compaction boundaries. Keep
        // native-name and reasoning attribution in the aggregate being measured.
        db.execute("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<128)
            INSERT INTO plan_quality_segments(workspace_id,segment_id,session_id,plan_id,plan_revision,
              selected_branch_id,executed_branch_id,model_configuration_id,profile_digest,attribution,
              first_at_ms,last_at_ms,first_request_id,last_request_id,score,assessment_partial)
            SELECT ?1,'stage-'||i,?2,?3,17,'writing','writing','model-a',?4,'single',1000,1100,?5,?5,0.8,0 FROM n",
            rusqlite::params![fixture.workspace.as_str(),fixture.session.as_str(),PLAN,
                CanonicalDigest::of_bytes(b"profile-a").to_string(),fixture.request.as_str()]).unwrap();
        // Retained execution facts need not all have a safe projection. They
        // belong to other requests and must not multiply this Plan's lookup work.
        db.execute_batch(
            "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<8192)
            INSERT INTO execution_fact_events
            SELECT workspace_id,session_id,turn_id,'unrelated-'||i,producer_id,producer_epoch,
              'retained-unrelated',i,'event-unrelated-'||i,envelope_json,'digest-unrelated-'||i
            FROM (SELECT * FROM execution_fact_events LIMIT 1),n;",
        )
        .unwrap();
    }
    drop(writer);
    (root, store)
}

pub(super) fn measured(db: &Connection, sql: &str) -> (Vec<Vec<Value>>, i32) {
    measured_result(db, sql).unwrap()
}

pub(super) fn measured_result(
    db: &Connection,
    sql: &str,
) -> rusqlite::Result<(Vec<Vec<Value>>, i32)> {
    let mut statement = db.prepare(sql)?;
    let columns = statement.column_count();
    let rows = statement
        .query_map(
            named_params! {
                ":workspace":"personal/default", ":plan":PLAN, ":session":Option::<String>::None,
                ":revision":17, ":from":0, ":to":5000, ":runs":Option::<String>::None,
            },
            |row| {
                (0..columns)
                    .map(|i| row.get(i))
                    .collect::<rusqlite::Result<Vec<Value>>>()
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((rows, statement.get_status(StatementStatus::VmStep)))
}

#[test]
fn quality_summary_bounds_repeated_execution_projection_work() {
    let (_root, store) = seed();
    let db = store.connection.lock();
    let sql = summary_sql();
    let (rows, bounded) = measured(&db, &sql);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][6], Value::Text("native-a".into()));
    assert_eq!(rows[0][7], Value::Text("reasoning-high".into()));
    assert_eq!(rows[0][8], Value::Integer(128));
    // Negative control: the former flattened CTE repeats correlated execution
    // lookups inside the aggregate. Compare VM work, never machine wall time.
    let (previous_rows, previous) = measured(&db, &sql.replace("AS MATERIALIZED (", "AS ("));
    assert_eq!(rows, previous_rows);
    assert!(
        bounded * 4 < previous * 3,
        "summary used {bounded} VM steps; repeated projections used {previous}"
    );
}

#[test]
fn routing_summary_bounds_unrelated_execution_history_work() {
    let (_root, store) = seed();
    let db = store.connection.lock();
    let (rows, bounded) = measured(&db, SUMMARY_SQL);
    assert_eq!(
        rows.len(),
        1,
        "the one real route decision must remain included"
    );
    // Reproduce the former join, filter and ordering choices. Leaving the new
    // key order can itself make SQLite choose a safe-fact-first plan, so that
    // would no longer be a negative control for the workspace execution scan.
    let previous_sql = SUMMARY_SQL
        .replace(" INDEXED BY observation_safe_route_payloads", "")
        .replace(" INDEXED BY execution_facts_request_payload", "")
        .replace(" INDEXED BY requests_observation_fields", "")
        .replace("CROSS JOIN execution_fact_events", "JOIN execution_fact_events")
        .replace("ORDER BY f.request_id,f.original_digest", "ORDER BY f.rowid")
        .replace("          AND CASE WHEN json_valid(f.body_json) THEN json_extract(f.body_json,'$.event_kind') ELSE 'route_decision' END='route_decision'\n", "");
    let (previous_rows, previous) = measured(&db, &previous_sql);
    assert_eq!(rows, previous_rows);
    assert!(
        bounded * 4 < previous,
        "routing summary used {bounded} VM steps; workspace event scan used {previous}"
    );
}

#[test]
fn routing_summary_bounds_unrelated_safe_projection_work() {
    let (_root, store) = seed();
    let db = store.connection.lock();
    let (before, baseline) = measured(&db, SUMMARY_SQL);
    db.execute_batch(
        "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<8192)
         INSERT INTO observation_safe_facts_v2
         SELECT workspace_id,'unrelated-'||i,'digest-unrelated-'||i,
           json_set(body_json,'$.event_kind','request_started')
         FROM (SELECT * FROM observation_safe_facts_v2 LIMIT 1),n;",
    )
    .unwrap();
    let (after, retained) = measured(&db, SUMMARY_SQL);
    assert_eq!(after, before);
    assert!(
        retained < baseline * 2 + 1000,
        "routing summary grew from {baseline} to {retained} VM steps for unrelated safe history"
    );
}

// Copy the writer's real route projection and matching event, rather than
// unrelated events: every added row must qualify for the routing summary.
fn extend_routing_history(db: &Connection, first: u32, last: u32) {
    let transaction = db.unchecked_transaction().unwrap();
    transaction.execute(
        "WITH RECURSIVE n(i) AS (VALUES(?1) UNION ALL SELECT i+1 FROM n WHERE i<?2)
         INSERT INTO execution_fact_events
         SELECT workspace_id,session_id,turn_id,request_id,producer_id,producer_epoch,
           'routing-tail',i,'routing-event-'||i,envelope_json,'routing-digest-'||printf('%06d',i)
         FROM (SELECT e.* FROM execution_fact_events e JOIN observation_safe_facts_v2 f
           ON f.workspace_id=e.workspace_id AND f.request_id=e.request_id AND f.original_digest=e.envelope_digest
           WHERE json_extract(f.body_json,'$.event_kind')='route_decision' LIMIT 1),n",
        [first, last],
    ).unwrap();
    transaction
        .execute(
            "WITH RECURSIVE n(i) AS (VALUES(?1) UNION ALL SELECT i+1 FROM n WHERE i<?2)
         INSERT INTO observation_safe_facts_v2
         SELECT workspace_id,request_id,'routing-digest-'||printf('%06d',i),
           json_set(body_json,'$.event_id','routing-event-'||i,'$.sequence',i,
             '$.original_digest','routing-digest-'||printf('%06d',i))
         FROM (SELECT * FROM observation_safe_facts_v2
           WHERE json_extract(body_json,'$.event_kind')='route_decision' LIMIT 1),n",
            [first, last],
        )
        .unwrap();
    transaction.commit().unwrap();
}

fn limited_routing_summary(db: &Connection) -> hiroute_domain::RoutingObservationsSummaryV1 {
    let reader = hiroute_domain::ObservationReaderContext::local_user(
        hiroute_domain::WorkspaceId::default(),
        "user".into(),
        1,
        20_000,
        false,
        false,
    )
    .unwrap();
    let query = serde_json::from_value(serde_json::json!({
        "plan_id":PLAN,"plan_revision":17,"include_routing_observations":true,"limit":50
    }))
    .unwrap();
    let transaction = db.unchecked_transaction().unwrap();
    super::routing_observations::summarize(&transaction, &reader, &query, 0, 5000, &None).unwrap()
}

fn routing_scale_connection() -> Connection {
    let (_root, store) = seed();
    let mut db = Connection::open_in_memory().unwrap();
    // These regressions measure SQLite work, not fsync latency. Clone the real
    // schema/seed without making bulk fixture writes compete with capacity tests.
    let source = store.connection.lock();
    assert!(matches!(
        rusqlite::backup::Backup::new(&source, &mut db)
            .unwrap()
            .step(-1)
            .unwrap(),
        rusqlite::backup::StepResult::Done
    ));
    db
}

#[test]
fn routing_summary_stops_work_at_matching_decision_limit() {
    let db = routing_scale_connection();
    extend_routing_history(&db, 1, 19_999);
    let reads = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&reads);
    db.create_scalar_function(
        "routing_time_probe",
        1,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            count.fetch_add(1, Ordering::Relaxed);
            ctx.get::<i64>(0)
        },
    )
    .unwrap();
    let sql = SUMMARY_SQL.replacen(
        "COALESCE(json_extract(f.body_json,'$.occurred_at_ms'),r.started_at_ms)>=:from",
        "routing_time_probe(COALESCE(json_extract(f.body_json,'$.occurred_at_ms'),r.started_at_ms))>=:from", 1,
    );
    let (before, bounded) = measured(&db, &sql);
    assert_eq!(before.len(), 10_001);
    assert_eq!(
        reads.swap(0, Ordering::Relaxed),
        10_001,
        "LIMIT must stop filtering, not just truncate a fully sorted result"
    );
    extend_routing_history(&db, 20_000, 99_999);
    let (after, retained) = measured(&db, &sql);
    assert_eq!(
        after, before,
        "an unread tail must not change the selected prefix"
    );
    assert_eq!(reads.swap(0, Ordering::Relaxed), 10_001);
    assert!(
        retained <= bounded + 1000,
        "routing work grew from {bounded} to {retained} VM steps"
    );
    let (_, previous) = measured(
        &db,
        &sql.replace(
            "ORDER BY f.request_id,f.original_digest",
            "ORDER BY f.rowid",
        ),
    );
    assert_eq!(reads.load(Ordering::Relaxed), 100_000);
    assert!(
        previous > retained * 4,
        "the former full sort must expose the regression"
    );
    let summary = limited_routing_summary(&db);
    assert_eq!(summary.retained_decisions, 10_000);
    assert!(summary.scan_limited);
    assert_eq!(summary.coverage, "partial");
}

#[test]
fn routing_summary_can_stop_at_safe_byte_limit_before_row_limit() {
    let db = routing_scale_connection();
    extend_routing_history(&db, 1, 19_999);
    // Oversized retained safe fields must not make SQLite sort the unread tail
    // before the production reader can enforce its existing byte ceiling.
    db.execute(
        "UPDATE observation_safe_facts_v2 SET body_json=json_set(body_json,'$.native_model',?1)
        WHERE original_digest BETWEEN 'routing-digest-000001' AND 'routing-digest-000300'",
        ["m".repeat(128 * 1024)],
    )
    .unwrap();
    let mut statement = db.prepare(SUMMARY_SQL).unwrap();
    let mut rows = statement
        .query(named_params! {
            ":workspace":"personal/default", ":plan":PLAN, ":session":Option::<String>::None,
            ":revision":17, ":from":0, ":to":5000, ":runs":Option::<String>::None,
        })
        .unwrap();
    let (mut bytes, mut scanned) = (0, 0);
    while let Some(row) = rows.next().unwrap() {
        bytes += row.get::<_, String>(0).unwrap().len();
        scanned += 1;
        if bytes > 32 * 1024 * 1024 {
            break;
        }
    }
    drop(rows);
    assert!(scanned < 300 && bytes > 32 * 1024 * 1024);
    assert_eq!(
        statement.get_status(StatementStatus::Sort),
        0,
        "the byte ceiling must be able to stop a streaming query"
    );
    assert!(statement.get_status(StatementStatus::VmStep) < 100_000);
    let summary = limited_routing_summary(&db);
    assert_eq!(summary.retained_decisions, scanned - 1);
    assert!(summary.scan_limited);
    assert_eq!(summary.coverage, "partial");
}

#[test]
fn routing_index_reopen_preserves_malformed_safe_bytes_and_failed_read() {
    let (root, store) = seed();
    let schema_version_before: String = store
        .connection
        .lock()
        .query_row(
            "SELECT value FROM observation_meta WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let retained = {
        let db = store.connection.lock();
        db.execute_batch(
            "DROP INDEX IF EXISTS observation_safe_route_decisions;
            DROP INDEX IF EXISTS observation_safe_route_payloads;
            DROP INDEX IF EXISTS observation_safe_kind_fields;
            UPDATE observation_safe_facts_v2 SET body_json='{' WHERE rowid=(
              SELECT rowid FROM observation_safe_facts_v2
              WHERE json_extract(body_json,'$.event_kind')='route_decision' LIMIT 1);",
        )
        .unwrap();
        let mut statement = db.prepare("SELECT workspace_id,request_id,original_digest,body_json FROM observation_safe_facts_v2 ORDER BY rowid").unwrap();
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert!(retained.iter().any(|row| row.3 == "{"));
    drop(store);
    for _ in 0..2 {
        let reopened = open_store(root.path());
        let db = reopened.connection.lock();
        let mut statement = db.prepare("SELECT workspace_id,request_id,original_digest,body_json FROM observation_safe_facts_v2 ORDER BY rowid").unwrap();
        let after = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(after, retained);
        assert!(db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='observation_safe_route_decisions')", [], |row| row.get::<_, bool>(0)).unwrap());
        assert!(
            measured_result(&db, SUMMARY_SQL)
                .unwrap_err()
                .to_string()
                .contains("malformed JSON")
        );
        assert_eq!(
            db.query_row(
                "SELECT value FROM observation_meta WHERE key='schema_version'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            schema_version_before
        );
    }
}
