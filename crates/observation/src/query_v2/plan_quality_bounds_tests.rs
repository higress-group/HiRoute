use super::{
    plan_quality::{SAMPLE_SELECT, SCOPE, summary_sql},
    routing_observations::SUMMARY_SQL,
};
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

const PLAN: &str = "plan/codex-daily";

fn seed() -> (
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

fn measured(db: &Connection, sql: &str) -> (Vec<Vec<Value>>, i32) {
    measured_result(db, sql).unwrap()
}

fn measured_result(db: &Connection, sql: &str) -> rusqlite::Result<(Vec<Vec<Value>>, i32)> {
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
        .replace(" INDEXED BY execution_facts_request_identity", "")
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

#[test]
fn quality_reasoning_filters_known_unrelated_payloads_before_opening_them() {
    let (_root, store) = seed();
    let db = store.connection.lock();
    let reads = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&reads);
    db.create_scalar_function(
        "quality_payload_probe",
        1,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            count.fetch_add(1, Ordering::Relaxed);
            ctx.get::<String>(0)
        },
    )
    .unwrap();
    let sql = summary_sql().replace("p.body_json", "quality_payload_probe(p.body_json)");
    let (rows, _) = measured(&db, &sql);
    let bounded = reads.swap(0, Ordering::Relaxed);
    let unfiltered = sql.replace(
        "LEFT JOIN observation_safe_facts_v2 f INDEXED BY observation_safe_kind_fields ON f.workspace_id=e.workspace_id AND f.request_id=e.request_id AND f.original_digest=e.envelope_digest", ""
    ).replace(
        "AND (f.original_digest IS NULL OR (CASE WHEN json_valid(f.body_json) THEN CASE WHEN json_valid(f.body_json) THEN json_extract(f.body_json,'$.event_kind') END ELSE json_extract(f.body_json,'$.event_kind') END)='candidate_decision')", ""
    );
    let (previous_rows, _) = measured(&db, &unfiltered);
    let previous = reads.swap(0, Ordering::Relaxed);
    assert_eq!(rows, previous_rows);
    assert!(
        bounded * 2 < previous,
        "reasoning opened {bounded} payload expressions; unfiltered read opened {previous}"
    );

    // A missing safe projection remains a gap, not permission to invent a
    // different profile. Retain the original payload lookup as its fallback.
    db.execute("DELETE FROM observation_safe_facts_v2", [])
        .unwrap();
    assert_eq!(measured(&db, &sql).0, rows);
    db.execute("DELETE FROM observation_sensitive_payloads_v2", [])
        .unwrap();
    let deleted = measured(&db, &sql).0;
    assert_eq!(
        deleted[0][7],
        Value::Null,
        "deleted reasoning must stay unavailable"
    );
}

#[test]
fn quality_selection_bounds_payload_reads_and_preserves_legacy_gaps() {
    let (_root, store) = seed();
    let db = store.connection.lock();
    let sql = format!("WITH quality AS ({SCOPE}) {SAMPLE_SELECT}");
    let plan = db
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .raw_query()
        .mapped(|row| row.get::<_, String>(3))
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(
        plan.iter().any(|step| {
            step.starts_with("SEARCH chosen USING COVERING INDEX")
                && step.contains("workspace_id=? AND request_id=?")
        }),
        "selection identity must not fetch event rows: {plan:?}"
    );
    let reads = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&reads);
    db.create_scalar_function(
        "quality_selection_probe",
        1,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            count.fetch_add(1, Ordering::Relaxed);
            ctx.get::<String>(0)
        },
    )
    .unwrap();
    let sql = sql.replace("pick.body_json", "quality_selection_probe(pick.body_json)");
    let (rows, _) = measured(&db, &sql);
    assert_eq!(rows.len(), 128);
    assert!(matches!(rows[0].last(), Some(Value::Text(_))));
    let bounded = reads.swap(0, Ordering::Relaxed);
    let unfiltered = sql.replace(
        "AND (chosen_safe.original_digest IS NULL OR (CASE WHEN json_valid(chosen_safe.body_json) THEN CASE WHEN json_valid(chosen_safe.body_json) THEN json_extract(chosen_safe.body_json,'$.event_kind') END ELSE json_extract(chosen_safe.body_json,'$.event_kind') END)='route_decision')",
        "",
    );
    let (previous_rows, _) = measured(&db, &unfiltered);
    let previous = reads.swap(0, Ordering::Relaxed);
    assert_eq!(rows, previous_rows);
    assert!(
        bounded > 0 && bounded * 2 < previous,
        "selection opened {bounded} payload expressions; unfiltered read opened {previous}"
    );
    db.execute("DELETE FROM observation_safe_facts_v2", [])
        .unwrap();
    assert_eq!(measured(&db, &sql).0, rows);
    db.execute("DELETE FROM observation_sensitive_payloads_v2", [])
        .unwrap();
    assert!(
        measured(&db, &sql)
            .0
            .iter()
            .all(|row| row.last() == Some(&Value::Null)),
        "deleted selection payloads must remain unavailable"
    );
}

#[test]
fn quality_identity_reads_use_covering_request_and_exact_payload_lookup() {
    let (_root, store) = seed();
    let db = store.connection.lock();
    let plan = |sql: &str| {
        let mut statement = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        statement
            .raw_query()
            .mapped(|row| row.get::<_, String>(3))
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let summary = plan(&summary_sql());
    assert!(
        summary
            .iter()
            .any(|step| step.starts_with("SEARCH e USING COVERING INDEX")
                && step.contains("workspace_id=? AND request_id=?")),
        "reasoning identity lookup must not open the event payload row: {summary:?}"
    );
    let routing = plan(SUMMARY_SQL);
    assert!(
        routing
            .iter()
            .any(|step| step.starts_with("SEARCH e USING COVERING INDEX")
                && step.contains("workspace_id=? AND request_id=? AND envelope_digest=?")),
        "routing identity lookup must locate the exact retained event: {routing:?}"
    );
}

#[test]
fn quality_request_index_rebuild_preserves_existing_event_bytes_on_reopen() {
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
    let before = {
        let db = store.connection.lock();
        let mut statement = db.prepare("SELECT request_id,envelope_json,envelope_digest FROM execution_fact_events ORDER BY rowid").unwrap();
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        // An existing released store has the request and session indexes but
        // no covering payload index. Reopening must add it without rewriting
        // immutable event identities or their original content carriers.
        db.execute_batch("DROP INDEX IF EXISTS execution_facts_request_payload; DROP INDEX IF EXISTS execution_facts_request_identity")
            .unwrap();
        rows
    };
    drop(store);
    for _ in 0..2 {
        let reopened = open_store(root.path());
        let db = reopened.connection.lock();
        let mut statement = db.prepare("SELECT request_id,envelope_json,envelope_digest FROM execution_fact_events ORDER BY rowid").unwrap();
        let after = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(after, before);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE name IN('execution_facts_request_payload','execution_facts_request_identity')", [], |row| row.get::<_, u64>(0)).unwrap(), 2);
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

#[test]
fn quality_payload_indexes_preserve_missing_fields_and_corruption_on_reopen() {
    for (kind, field, sql, column) in [
        (
            "candidate_decision",
            "reasoning_profile_id",
            summary_sql(),
            7,
        ),
        (
            "route_decision",
            "complexity",
            format!("WITH quality AS ({SCOPE}) {SAMPLE_SELECT}"),
            36,
        ),
    ] {
        let (root, store) = seed();
        let (id, retained, version) = {
            let db = store.connection.lock();
            let (rows, _) = measured(&db, &sql);
            assert!(matches!(&rows[0][column], Value::Text(_)));
            let id: String = db.query_row(
                "SELECT id FROM observation_sensitive_payloads_v2 WHERE json_extract(body_json,'$.fact.kind')=?1 LIMIT 1",
                [kind], |row| row.get(0),
            ).unwrap();
            db.execute("UPDATE observation_sensitive_payloads_v2 SET body_json=json_remove(body_json,?1) WHERE id=?2", rusqlite::params![format!("$.fact.{field}"), id]).unwrap();
            assert_eq!(measured(&db, &sql).0[0][column], Value::Null);
            // A malformed carrier must not become an absent indexed value.
            db.execute(
                "UPDATE observation_sensitive_payloads_v2 SET body_json='{' WHERE id=?1",
                [&id],
            )
            .unwrap();
            db.execute_batch(
                "DROP INDEX observation_payload_query_fields;
                DROP INDEX observation_safe_kind_fields;
                DROP INDEX observation_safe_route_payloads;
                DROP INDEX execution_facts_request_identity;",
            )
            .unwrap();
            let mut statement = db
                .prepare("SELECT id,body_json FROM observation_sensitive_payloads_v2 ORDER BY id")
                .unwrap();
            let retained = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            let version: String = db
                .query_row(
                    "SELECT value FROM observation_meta WHERE key='schema_version'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            (id, retained, version)
        };
        assert!(retained.iter().any(|row| row.0 == id && row.1 == "{"));
        drop(store);
        for _ in 0..2 {
            let reopened = open_store(root.path());
            let db = reopened.connection.lock();
            let mut statement = db
                .prepare("SELECT id,body_json FROM observation_sensitive_payloads_v2 ORDER BY id")
                .unwrap();
            let after = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(after, retained);
            assert_eq!(db.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE name IN('observation_payload_query_fields','observation_safe_kind_fields','observation_safe_route_payloads','execution_facts_request_identity')", [], |row| row.get::<_, u64>(0)).unwrap(), 4);
            assert_eq!(
                db.query_row(
                    "SELECT value FROM observation_meta WHERE key='schema_version'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
                version
            );
            assert!(
                measured_result(&db, &sql)
                    .unwrap_err()
                    .to_string()
                    .contains("malformed JSON")
            );
        }
    }
}
