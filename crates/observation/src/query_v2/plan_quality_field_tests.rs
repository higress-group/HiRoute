use super::{
    plan_quality::{SAMPLE_SELECT, SCOPE, summary_sql},
    plan_quality_bounds_tests::{measured, measured_result, seed},
    routing_observations::SUMMARY_SQL,
};
use crate::store::tests::support::open_store;
use rusqlite::{Connection, functions::FunctionFlags, types::Value};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn probe_guarded_fields(sql: &str) -> String {
    let mut sql = sql.to_owned();
    for field in [
        "reasoning_profile_id",
        "kind",
        "model_configuration_id",
        "profile_digest",
    ] {
        let expression = format!(
            "(CASE WHEN json_valid(p.body_json) THEN CASE WHEN json_valid(p.body_json) THEN json_extract(p.body_json,'$.fact.{field}') END ELSE json_extract(p.body_json,'$.fact.{field}') END)"
        );
        sql = sql.replace(
            &expression,
            &expression.replace("p.body_json", "quality_payload_probe(p.body_json)"),
        );
    }
    sql
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
    let sql = probe_guarded_fields(&summary_sql());
    let (rows, _) = measured(&db, &sql);
    let bounded = reads.swap(0, Ordering::Relaxed);
    let unfiltered = sql.replace(
        "LEFT JOIN observation_safe_facts_v2 f INDEXED BY observation_safe_kind_fields ON f.workspace_id=e.workspace_id AND f.request_id=e.request_id AND f.original_digest=e.envelope_digest", ""
    ).replace(
        "AND CASE WHEN (f.original_digest IS NULL OR (CASE WHEN json_valid(f.body_json) THEN CASE WHEN json_valid(f.body_json) THEN json_extract(f.body_json,'$.event_kind') END ELSE json_extract(f.body_json,'$.event_kind') END)='candidate_decision') THEN", "AND"
    ).replace("           ELSE 0 END", "")
      .replace("INDEXED BY observation_payload_candidate_request", "INDEXED BY observation_payload_query_fields")
      .replace("AND p.id GLOB 'fact:*' AND CASE WHEN json_valid(p.body_json) THEN json_extract(p.body_json,'$.fact.kind') ELSE 'candidate_decision' END='candidate_decision'", "AND p.id GLOB 'fact:*'");
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
    let detail = plan(&format!("WITH quality AS ({SCOPE}) {SAMPLE_SELECT}"));
    for alias in ["r", "first_req", "last_req", "trigger_req"] {
        assert!(
            detail.iter().any(|step| step.starts_with(&format!(
                "SEARCH {alias} USING COVERING INDEX requests_observation_fields"
            ))),
            "request ownership lookup must stay covered: {alias}: {detail:?}"
        );
    }
    assert!(
        summary.iter().any(|step| step
            .starts_with("SEARCH p USING INDEX observation_payload_candidate_request")
            && step.contains("workspace_id=? AND request_id=?")),
        "candidate metadata must be scoped before source lookups: {summary:?}"
    );
    for (alias, index) in [
        ("v", "valuation_request_accepted_ordinal"),
        ("a", "observation_attempt_model_names"),
    ] {
        assert!(
            summary
                .iter()
                .any(|step| step
                    .starts_with(&format!("SEARCH {alias} USING COVERING INDEX {index}"))),
            "accepted model lookup must not open table rows: {alias}: {summary:?}"
        );
    }
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
fn accepted_model_covering_indexes_preserve_live_ordinal_and_missing_evidence() {
    let (_root, store) = seed();
    let db = store.connection.lock();
    let sql = summary_sql();
    let previous = sql
        .replace(
            "INDEXED BY valuation_request_accepted_ordinal",
            "INDEXED BY sqlite_autoindex_valuation_requests_v2_1",
        )
        .replace(
            "INDEXED BY observation_attempt_model_names",
            "INDEXED BY sqlite_autoindex_observation_attempt_models_v2_1",
        );
    let (rows, covered_work) = measured(&db, &sql);
    let (old_rows, previous_work) = measured(&db, &previous);
    assert_eq!(rows, old_rows);
    assert!(
        covered_work < previous_work,
        "covering lookups used {covered_work} VM steps; table lookups used {previous_work}"
    );
    db.execute(
        "INSERT INTO observation_attempt_models_v2
         SELECT workspace_id,request_id,99,'native-other' FROM valuation_requests_v2 LIMIT 1",
        [],
    )
    .unwrap();
    assert_eq!(
        measured(&db, &sql).0,
        rows,
        "unaccepted attempts stay excluded"
    );
    db.execute("UPDATE valuation_requests_v2 SET accepted_ordinal=99", [])
        .unwrap();
    let current = measured(&db, &sql).0;
    assert_eq!(current, measured(&db, &previous).0);
    assert_eq!(current[0][6], Value::Text("native-other".into()));
    db.execute(
        "DELETE FROM observation_attempt_models_v2 WHERE ordinal=99",
        [],
    )
    .unwrap();
    let missing = measured(&db, &sql).0;
    assert_eq!(missing, measured(&db, &previous).0);
    assert_eq!(
        missing[0][6],
        Value::Null,
        "do not reuse a rejected model name"
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
                "DROP INDEX observation_payload_candidate_request;
                DROP INDEX observation_payload_query_fields;
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
            assert_eq!(db.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE name IN('observation_payload_candidate_request','observation_payload_query_fields','observation_safe_kind_fields','observation_safe_route_payloads','execution_facts_request_identity')", [], |row| row.get::<_, u64>(0)).unwrap(), 5);
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

fn retained_rows(db: &Connection, table: &str) -> Vec<Vec<Value>> {
    let mut statement = db
        .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
        .unwrap();
    let width = statement.column_count();
    statement
        .query_map([], |row| (0..width).map(|i| row.get(i)).collect())
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn request_field_indexes_reopen_without_rewriting_source_rows() {
    let (root, store) = seed();
    let tables = [
        "logical_requests",
        "execution_fact_events",
        "observation_sensitive_payloads_v2",
        "valuation_requests_v2",
        "observation_attempt_models_v2",
        "plan_quality_segments",
    ];
    let before = {
        let db = store.connection.lock();
        let before = tables
            .iter()
            .map(|t| retained_rows(&db, t))
            .collect::<Vec<_>>();
        db.execute_batch("DROP INDEX requests_observation_fields; DROP INDEX observation_payload_candidate_request; DROP INDEX valuation_request_accepted_ordinal; DROP INDEX observation_attempt_model_names;").unwrap();
        before
    };
    drop(store);
    for _ in 0..2 {
        let store = open_store(root.path());
        let db = store.connection.lock();
        for (table, expected) in tables.iter().zip(&before) {
            assert_eq!(
                &retained_rows(&db, table),
                expected,
                "source changed: {table}"
            );
        }
        assert_eq!(db.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE name IN('requests_observation_fields','observation_payload_candidate_request','valuation_request_accepted_ordinal','observation_attempt_model_names')",[],|r|r.get::<_,u64>(0)).unwrap(),4);
        assert_eq!(
            db.query_row(
                "SELECT value FROM observation_meta WHERE key='schema_version'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "5"
        );
        assert_eq!(
            measured(&db, &summary_sql()).0[0][7],
            Value::Text("reasoning-high".into())
        );
    }
}

#[test]
fn candidate_index_keeps_unrelated_corrupt_carriers_out_of_reasoning_reads() {
    let (root, store) = seed();
    let (id, expected) = {
        let db = store.connection.lock();
        let expected = measured(&db, &summary_sql()).0;
        let id:String=db.query_row("SELECT id FROM observation_sensitive_payloads_v2 WHERE json_extract(body_json,'$.fact.kind')='attempt_started' LIMIT 1",[],|r|r.get(0)).unwrap();
        db.execute(
            "UPDATE observation_sensitive_payloads_v2 SET body_json='{' WHERE id=?1",
            [&id],
        )
        .unwrap();
        db.execute("DROP INDEX observation_payload_candidate_request", [])
            .unwrap();
        (id, expected)
    };
    drop(store);
    for _ in 0..2 {
        let store = open_store(root.path());
        let db = store.connection.lock();
        assert_eq!(measured(&db, &summary_sql()).0, expected);
        assert_eq!(
            db.query_row(
                "SELECT body_json FROM observation_sensitive_payloads_v2 WHERE id=?1",
                [&id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "{"
        );
    }
}
