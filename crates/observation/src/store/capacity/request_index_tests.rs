//! A bounded row count is not bounded work when its request lookup scans a workspace.
use super::*;
use crate::store::tests::support::open_store;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const TABLES: &[&str] = &[
    "observation_sensitive_payloads_v2",
    "observation_link_events",
    "observation_native_roots_v1",
];
const INDEXES: &[&str] = &[
    "observation_capacity_sensitive_request_v5",
    "observation_capacity_link_request_v5",
    "observation_capacity_native_request_v5",
];

fn populated(store: &LocalObservationStore) {
    let db = store.connection.lock();
    db.execute_batch(
        "BEGIN;
         WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000)
         INSERT INTO observation_sensitive_payloads_v2
         SELECT 'unrelated-payload-'||x,'selected-workspace','shared-session',
                'unrelated-request-'||x,0,'{}' FROM n;
         WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000)
         INSERT INTO observation_link_events
         SELECT 'selected-workspace','shared-epoch','unrelated-link-'||x,
                'unrelated-request-'||x,'unchanged-digest' FROM n;
         WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000)
         INSERT INTO observation_native_roots_v1
         SELECT 'selected-workspace','shared-session','unrelated-root-'||x,
                'unrelated-request-'||x,0 FROM n;
         INSERT INTO observation_sensitive_payloads_v2 VALUES
           ('selected-payload','selected-workspace','shared-session','selected-request',0,'{}'),
           ('survivor-payload','other-workspace','shared-session','selected-request',0,'{}');
         INSERT INTO observation_link_events VALUES
           ('selected-workspace','zzzz-selected-epoch','selected-link','selected-request','unchanged-digest'),
           ('other-workspace','shared-epoch','survivor-link','selected-request','unchanged-digest');
         INSERT INTO observation_native_roots_v1 VALUES
           ('selected-workspace','zzzz-selected-session','selected-root','selected-request',0),
           ('other-workspace','shared-session','survivor-root','selected-request',0);
         COMMIT;",
    )
    .unwrap();
}

fn sql_budget(db: &Connection, maximum: usize) -> Arc<AtomicBool> {
    let exceeded = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&exceeded);
    let mut steps = 0;
    db.progress_handler(
        100,
        Some(move || {
            steps += 100;
            if steps > maximum {
                observed.store(true, Ordering::Relaxed);
                true
            } else {
                false
            }
        }),
    );
    exceeded
}

fn selected_ids(db: &Connection, table: &str, request: &str) -> rusqlite::Result<Vec<i64>> {
    // Preserve the production selector and its 128-row bound. A VM-instruction
    // allowance proves independence from unrelated rows without a wall-clock
    // assertion that can expire while the test process is descheduled.
    let mut steps = 0;
    db.progress_handler(
        100,
        Some(move || {
            steps += 100;
            steps > 1000
        }),
    );
    let result = db
        .prepare(&format!(
            "SELECT e.rowid FROM {table} e WHERE e.workspace_id=?1 AND e.request_id=?2 LIMIT 128"
        ))
        .and_then(|mut statement| {
            statement
                .query_map(params!["selected-workspace", request], |row| row.get(0))?
                .collect()
        });
    db.progress_handler(0, None::<fn() -> bool>);
    result
}

#[test]
fn request_cleanup_seeks_late_and_absent_members_without_visiting_unrelated_history() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    populated(&store);
    let db = store.connection.lock();
    for table in TABLES {
        assert_eq!(
            selected_ids(&db, table, "selected-request").unwrap().len(),
            1
        );
        assert!(
            selected_ids(&db, table, "absent-request")
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn current_store_reopen_installs_request_seek_indexes_without_changing_rows_or_policy() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    populated(&store);
    store.set_capacity_limit(Some(10_000_000_000)).unwrap();
    {
        let db = store.connection.lock();
        for index in INDEXES {
            db.execute_batch(&format!("DROP INDEX IF EXISTS {index}"))
                .unwrap();
        }
    }
    drop(store);
    let store = open_store(directory.path());
    assert_eq!(
        store.capacity_status().unwrap().max_active_bytes,
        Some(10_000_000_000)
    );
    let db = store.connection.lock();
    for table in TABLES {
        assert_eq!(
            selected_ids(&db, table, "selected-request").unwrap().len(),
            1
        );
        let count: usize = db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 10_002);
    }
    let version: String = db
        .query_row(
            "SELECT value FROM observation_meta WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, "5");
}

#[test]
fn retired_request_cleanup_commits_progress_and_preserves_shared_workspace_survivors() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    populated(&store);
    {
        let db = store.connection.lock();
        db.execute_batch("INSERT INTO observation_capacity_pending_v5(workspace,request,session,step) VALUES('selected-workspace','selected-request','shared-session',2); INSERT INTO observation_request_tombstones_v2 VALUES('selected-workspace','selected-request',100);").unwrap();
    }
    // The durable pending step and tombstone, rather than a process-local
    // cursor, own recovery across reopen.
    drop(store);
    let store = open_store(directory.path());
    let mut db = store.connection.lock();
    let mut state = State::default();
    for pass in 0..32 {
        let exceeded = sql_budget(&db, 20_000);
        let deadline = Instant::now() + BUDGET;
        state.committed_progress = false;
        let outcome = store.capacity_cleanup(&mut db, &mut state, deadline);
        db.progress_handler(0, None::<fn() -> bool>);
        assert!(
            !exceeded.load(Ordering::Relaxed),
            "unrelated-row VM budget exhausted"
        );
        // The real cycle also yields normally after earlier committed units
        // when its next unit exhausts the unchanged time allowance.
        assert!(outcome.is_ok() || (state.committed_progress && Instant::now() >= deadline));
        if pass == 0 {
            assert!(state.committed_progress);
        }
        let pending: usize = db
            .query_row(
                "SELECT COUNT(*) FROM observation_capacity_pending_v5",
                [],
                |row| row.get(0),
            )
            .unwrap();
        if pending == 0 {
            break;
        }
    }
    let pending: usize = db
        .query_row(
            "SELECT COUNT(*) FROM observation_capacity_pending_v5",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
    for table in TABLES {
        assert!(
            selected_ids(&db, table, "selected-request")
                .unwrap()
                .is_empty()
        );
        let surviving: usize = db.query_row(&format!("SELECT COUNT(*) FROM {table} WHERE workspace_id='other-workspace' AND request_id='selected-request'"), [], |row| row.get(0)).unwrap();
        assert_eq!(surviving, 1);
        let unrelated: usize = db
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE workspace_id='selected-workspace'"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(unrelated, 10_000);
    }
    let barrier: usize = db.query_row("SELECT COUNT(*) FROM observation_request_tombstones_v2 WHERE workspace_id='selected-workspace' AND request_id='selected-request'", [], |row| row.get(0)).unwrap();
    assert_eq!(barrier, 1);
}

#[test]
fn cleanup_commits_exactly_128_members_before_a_later_failure_and_resumes_the_129th() {
    for table in TABLES {
        let directory = tempfile::tempdir().unwrap();
        let store = open_store(directory.path());
        populated(&store);
        let mut db = store.connection.lock();
        // These are lawful non-candidate payload IDs. A partial candidate-only
        // index or ID-prefix filter must not omit their retirement.
        db.execute_batch("WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<128) INSERT INTO observation_sensitive_payloads_v2 SELECT 'batch-payload-'||x,'selected-workspace','shared-session','selected-request',0,'{}' FROM n;
          WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<128) INSERT INTO observation_link_events SELECT 'selected-workspace','zzzz-selected-epoch','batch-link-'||x,'selected-request','unchanged-digest' FROM n;
          WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<128) INSERT INTO observation_native_roots_v1 SELECT 'selected-workspace','zzzz-selected-session','batch-root-'||x,'selected-request',0 FROM n;").unwrap();
        let step = 2 + CLEANUP
            .iter()
            .position(|candidate| candidate == table)
            .unwrap();
        // A restored step means earlier carriers were already cleaned.
        for prior in TABLES.iter().take_while(|candidate| *candidate != table) {
            db.execute(&format!("DELETE FROM {prior} WHERE workspace_id='selected-workspace' AND request_id='selected-request'"), []).unwrap();
        }
        db.execute("INSERT INTO observation_capacity_pending_v5(workspace,request,session,step) VALUES('selected-workspace','selected-request','shared-session',?1)", [step as i64]).unwrap();
        db.execute_batch("INSERT INTO observation_request_tombstones_v2 VALUES('selected-workspace','selected-request',100);").unwrap();
        drop(db);
        drop(store);
        let store = open_store(directory.path());
        let mut db = store.connection.lock();
        db.execute_batch("CREATE TEMP TABLE capacity_deleted_test(n INTEGER); INSERT INTO capacity_deleted_test VALUES(0);").unwrap();
        db.execute_batch(&format!("CREATE TEMP TRIGGER capacity_prefix_test BEFORE DELETE ON {table} WHEN OLD.workspace_id='selected-workspace' AND OLD.request_id='selected-request' BEGIN SELECT CASE WHEN (SELECT n FROM capacity_deleted_test)>=128 THEN RAISE(ABORT,'owned prefix boundary') END; UPDATE capacity_deleted_test SET n=n+1; END;")).unwrap();
        let exceeded = sql_budget(&db, 100_000);
        let mut state = State::default();
        let deadline = Instant::now() + BUDGET;
        let outcome = store.capacity_cleanup(&mut db, &mut state, deadline);
        db.progress_handler(0, None::<fn() -> bool>);
        assert!(
            !exceeded.load(Ordering::Relaxed),
            "full batch scanned unrelated history: {table}"
        );
        assert!(
            state.committed_progress,
            "first batch did not commit: {table}: {outcome:?}"
        );
        let count: usize = db
            .query_row("SELECT n FROM capacity_deleted_test", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            count, 128,
            "a failed later unit cannot roll back the committed prefix"
        );
        assert_eq!(
            selected_ids(&db, table, "selected-request").unwrap().len(),
            1
        );
        let pending_step: usize = db
            .query_row(
                "SELECT step FROM observation_capacity_pending_v5",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            pending_step, step,
            "a failed unit cannot skip the last member"
        );
        db.execute_batch("DROP TRIGGER capacity_prefix_test; DROP TABLE capacity_deleted_test;")
            .unwrap();
        for _ in 0..32 {
            let exceeded = sql_budget(&db, 100_000);
            let deadline = Instant::now() + BUDGET;
            state.committed_progress = false;
            let outcome = store.capacity_cleanup(&mut db, &mut state, deadline);
            db.progress_handler(0, None::<fn() -> bool>);
            assert!(
                !exceeded.load(Ordering::Relaxed),
                "resume exceeded unrelated-row bound"
            );
            assert!(outcome.is_ok() || (state.committed_progress && Instant::now() >= deadline));
            let pending: usize = db
                .query_row(
                    "SELECT COUNT(*) FROM observation_capacity_pending_v5",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            if pending == 0 {
                break;
            }
        }
        let pending: usize = db
            .query_row(
                "SELECT COUNT(*) FROM observation_capacity_pending_v5",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending, 0);
        assert!(
            selected_ids(&db, table, "selected-request")
                .unwrap()
                .is_empty()
        );
        for survivor in TABLES {
            assert!(
                selected_ids(&db, survivor, "selected-request")
                    .unwrap()
                    .is_empty()
            );
            let count: usize = db.query_row(&format!("SELECT COUNT(*) FROM {survivor} WHERE workspace_id='other-workspace' AND request_id='selected-request'"), [], |row| row.get(0)).unwrap();
            assert_eq!(count, 1);
            let count: usize = db.query_row(&format!("SELECT COUNT(*) FROM {survivor} WHERE workspace_id='selected-workspace' AND request_id!='selected-request'"), [], |row| row.get(0)).unwrap();
            assert_eq!(count, 10_000);
        }
    }
}
