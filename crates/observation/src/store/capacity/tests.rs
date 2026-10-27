use super::*;
use crate::store::tests::support::*;
use hiroute_domain::{ContentMode, ObservationQueryPort};
fn due(store: &LocalObservationStore) {
    store.capacity.lock().next_due = Instant::now();
}
#[test]
fn default_twenty_gb_persists_positive_u64_and_explicit_disable() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    assert_eq!(
        store.capacity_status().unwrap().max_active_bytes,
        Some(20_000_000_000)
    );
    assert!(store.set_capacity_limit(Some(0)).is_err());
    assert_eq!(
        store
            .set_capacity_limit(Some(u64::MAX))
            .unwrap()
            .max_active_bytes,
        Some(u64::MAX)
    );
    drop(store);
    let store = open_store(dir.path());
    assert_eq!(
        store.capacity_status().unwrap().max_active_bytes,
        Some(u64::MAX)
    );
    store.set_capacity_limit(None).unwrap();
    drop(store);
    let store = open_store(dir.path());
    assert_eq!(store.capacity_status().unwrap().max_active_bytes, None);
    assert_eq!(store.capacity_status().unwrap().reason, "disabled");
    assert_eq!(
        store
            .connection
            .lock()
            .query_row(
                "SELECT value FROM observation_meta WHERE key='max_active_bytes'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "null"
    );
}

#[test]
fn page_release_consumes_the_bounded_pragma_instead_of_only_its_first_row() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let db = store.connection.lock();
    db.execute_batch("CREATE TABLE capacity_page_fixture(payload BLOB); WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<512) INSERT INTO capacity_page_fixture SELECT zeroblob(4096) FROM n; DELETE FROM capacity_page_fixture;").unwrap();
    let free = || {
        db.query_row("PRAGMA freelist_count", [], |row| row.get::<_, u64>(0))
            .unwrap()
    };
    let before = free();
    assert!(before > 32);
    assert!(vacuum_pages(&db).unwrap());
    assert!(
        before - free() > 1,
        "execute_batch steps a row-producing pragma only once"
    );
}

#[test]
fn physical_release_yields_to_a_pinned_reader_then_truncates_after_it_finishes() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let db = store.connection.lock();
    db.busy_timeout(Duration::ZERO).unwrap();
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    let reader = Connection::open(&store.activity_path).unwrap();
    reader.execute_batch("BEGIN").unwrap();
    reader
        .query_row("SELECT COUNT(*) FROM observation_meta", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    db.execute(
        "INSERT INTO observation_meta(key,value) VALUES('capacity-reader-fixture','1')",
        [],
    )
    .unwrap();
    let start = Instant::now();
    assert!(!checkpoint_pages(&db, start + BUDGET).unwrap());
    assert!(start.elapsed() < BUDGET);
    let wal = format!("{}-wal", store.activity_path.display());
    assert!(fs::metadata(&wal).unwrap().len() > 0);
    drop(reader);
    assert!(fs::metadata(&wal).unwrap().len() > 0);
    // Even with no pinned reader, an exhausted pass must defer physical release.
    // PASSIVE may backfill pages; it must not truncate after its deadline.
    let expired = Instant::now() - Duration::from_millis(1);
    assert!(!checkpoint_pages(&db, expired).unwrap());
    assert!(fs::metadata(&wal).unwrap().len() > 0);
    // Production resumes pending release on a later capacity cycle. Keep the
    // original per-pass budget instead of requiring one fsync to finish in it.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut passes = Vec::new();
    loop {
        let start = Instant::now();
        let released = checkpoint_pages(&db, start + BUDGET).unwrap();
        passes.push((start.elapsed(), released, fs::metadata(&wal).unwrap().len()));
        if released {
            break;
        }
        assert!(
            Instant::now() < deadline && passes.len() < 32,
            "released reader left physical release pending (busy_timeout=0): {passes:?}"
        );
        std::thread::yield_now();
    }
    assert_eq!(fs::metadata(&wal).unwrap().len(), 0);
    assert_eq!(
        db.query_row(
            "SELECT value FROM observation_meta WHERE key='capacity-reader-fixture'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "1"
    );
}

#[test]
fn large_wal_recovers_after_reader_release_without_reopening_or_over_retiring() {
    for already_checkpointed in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let store = open_store(directory.path());
        let fixture = Fixture::new("large-wal-retirement");
        let writer = writer(&store);
        let channel = fact_channel(&fixture, 512 * 1024);
        for event in request_facts(&fixture, "large-wal", finished("large-wal"), 100) {
            assert!(matches!(
                offer_fact(&writer, &channel, event),
                crate::WriterCycleOutcome::Ack(_)
            ));
        }
        install_content(&store, &fixture, b"eligible old detail", 200);
        store.set_capacity_limit(Some(1)).unwrap();
        store
            .connection
            .lock()
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let reader = Connection::open(&store.activity_path).unwrap();
        reader.execute_batch("BEGIN").unwrap();
        reader
            .query_row("SELECT COUNT(*) FROM logical_requests", [], |r| {
                r.get::<_, u64>(0)
            })
            .unwrap();
        // A temporary old reader can let a single committed transaction grow
        // the WAL above the former 16 MiB admission guard. The unrelated table
        // remains allocated, so a lower threshold cannot be satisfied by it.
        store.connection.lock().execute_batch("CREATE TABLE capacity_wal_fixture(payload BLOB); INSERT INTO capacity_wal_fixture VALUES(zeroblob(20971520));").unwrap();
        let wal = format!("{}-wal", store.activity_path.display());
        assert!(fs::metadata(&wal).unwrap().len() > 16 * 1024 * 1024);
        due(&store);
        store.capacity_cycle(1000).unwrap();
        assert_eq!(
            store.capacity_status().unwrap().reason,
            "page_release_pending"
        );
        assert_eq!(
            store
                .connection
                .lock()
                .query_row("SELECT COUNT(*) FROM logical_requests", [], |r| r
                    .get::<_, u64>(0))
                .unwrap(),
            1,
            "reader-pinned space must not cause additional logical expiry"
        );
        drop(reader);
        if already_checkpointed {
            let (_, frames, backfilled): (i64, i64, i64) = store
                .connection
                .lock()
                .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .unwrap();
            assert_eq!(frames, backfilled);
            assert!(
                fs::metadata(&wal).unwrap().len() > 16 * 1024 * 1024,
                "checkpointed WAL allocation retains its physical high-water mark"
            );
        }
        for _ in 0..30 {
            due(&store);
            store.capacity_cycle(1000).unwrap();
        }
        let db = store.connection.lock();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM logical_requests", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0,
            "a released reader must allow capacity progress without reopening"
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM observation_capacity_pending_v5",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM observation_capacity_blobs_v5",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        assert!(fs::metadata(&wal).unwrap().len() < 16 * 1024 * 1024);
    }
}

#[test]
fn a_nonpreemptible_overrun_extends_the_completion_based_cooldown() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    store.set_capacity_limit(Some(1)).unwrap();
    store.connection.lock().trace(Some(|sql| {
        if sql.starts_with("SELECT EXISTS(SELECT 1 FROM observation_capacity_pending_v5)") {
            // Model a slow SQLite/filesystem unit that progress callbacks
            // cannot preempt; the next pass must account for its full cost.
            std::thread::sleep(Duration::from_millis(250));
        }
    }));
    let start = Instant::now();
    store.capacity_cycle(1000).unwrap();
    store.connection.lock().trace(None);
    assert!(start.elapsed() >= Duration::from_millis(250));
    assert!(store.capacity.lock().next_due >= Instant::now() + Duration::from_secs(20));
    assert!(!store.capacity_cycle(2000).unwrap());
}

#[test]
fn turn_cleanup_progresses_with_unrelated_requests_and_surviving_session_turns() {
    use std::sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    };

    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("turn-cleanup");
    let writer = writer(&store);
    let channel = fact_channel(&fixture, 512 * 1024);
    for event in request_facts(&fixture, "turn-cleanup", finished("turn-cleanup"), 100) {
        assert!(matches!(
            offer_fact(&writer, &channel, event),
            crate::WriterCycleOutcome::Ack(_)
        ));
    }
    let db = store.connection.lock();
    db.execute("WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<50000) INSERT INTO logical_requests(workspace_id,request_id,session_id,turn_id,started_at_ms,finished_at_ms) SELECT ?1,'unrelated-request-'||x,'unrelated-session','unrelated-turn-'||x,200,300 FROM n", [fixture.workspace.as_str()]).unwrap();
    db.execute("WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<500) INSERT INTO turns(workspace_id,turn_id,session_id,started_at_ms) SELECT ?1,'surviving-turn-'||x,?2,200 FROM n", params![fixture.workspace.as_str(),fixture.session.as_str()]).unwrap();
    db.execute("INSERT INTO logical_requests(workspace_id,request_id,session_id,turn_id,started_at_ms,finished_at_ms) SELECT workspace_id,'surviving-request-'||turn_id,session_id,turn_id,200,300 FROM turns WHERE workspace_id=?1 AND session_id=?2 AND turn_id!=?3", params![fixture.workspace.as_str(),fixture.session.as_str(),fixture.turn.as_str()]).unwrap();
    // Reopen an already-current store that predates the required lookup index.
    db.execute_batch("DROP INDEX IF EXISTS observation_capacity_turn_v5")
        .unwrap();
    drop(db);
    drop(writer);
    drop(store);
    let store = open_store(directory.path());
    let mut db = store.connection.lock();
    store
        .capacity_retire(
            &mut db,
            fixture.workspace.as_str(),
            fixture.session.as_str(),
            fixture.request.as_str(),
            1000,
        )
        .unwrap();
    let mut state = State::default();
    // Maintenance may yield between durable phases on a busy host. Preserve
    // both the production time budget and the per-slice scan regression guard.
    for _ in 0..10 {
        let deadline = Instant::now() + BUDGET;
        let interrupted = Arc::new(AtomicU8::new(0));
        let reason = Arc::clone(&interrupted);
        let mut instructions = 0;
        db.progress_handler(
            1000,
            Some(move || {
                instructions += 1000;
                let value = if instructions > 100_000 {
                    2
                } else if Instant::now() >= deadline {
                    1
                } else {
                    return false;
                };
                reason.store(value, Ordering::Relaxed);
                true
            }),
        );
        let result = store.capacity_cleanup(&mut db, &mut state, deadline);
        db.progress_handler(0, None::<fn() -> bool>);
        let reason = interrupted.load(Ordering::Relaxed);
        assert_ne!(reason, 2, "turn cleanup must not scan unrelated history");
        assert!(result.is_ok() || reason == 1, "{result:?}");
        if !db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM observation_capacity_pending_v5)",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap()
        {
            break;
        }
    }
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM observation_capacity_pending_v5",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM turns WHERE workspace_id=?1 AND session_id=?2",
            params![fixture.workspace.as_str(), fixture.session.as_str()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        500
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM logical_requests", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        50_500
    );
}

#[test]
fn request_chunk_cleanup_uses_request_membership_before_unrelated_workspace_history() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("bounded-chunk-selection");
    install_content(&store, &fixture, b"selected request body", 100);
    let mut db = store.connection.lock();
    // Match a large workspace while keeping the selected request tiny. Both
    // carriers have valid matching content membership, preceding it by ID.
    db.execute_batch("WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000) INSERT INTO content_instances_v2 SELECT workspace_id,'aaa-unrelated-'||x,message_instance_id,'unrelated-request',direction,fork_id,part_ordinal,content_kind,canonical_media_type,content_blob_digest,expected_byte_count,has_content_ref,transport_frame_id,downstream_delivery,accumulated_byte_count,state,created_at_unix_nanos FROM content_instances_v2 CROSS JOIN n WHERE content_id LIKE 'content-%' LIMIT 10000;").unwrap();
    db.execute_batch("INSERT INTO content_chunks_v2 SELECT workspace_id,content_id,0,'not-a-selected-object',0,1,'unrelated-event' FROM content_instances_v2 WHERE request_id='unrelated-request';").unwrap();
    db.execute(
        "INSERT INTO observation_capacity_pending_v5(workspace,request,session) VALUES(?1,?2,?3)",
        params![
            fixture.workspace.as_str(),
            fixture.request.as_str(),
            fixture.session.as_str()
        ],
    )
    .unwrap();
    let mut steps = 0;
    db.progress_handler(
        100,
        Some(move || {
            steps += 100;
            steps > 20000
        }),
    );
    let mut state = State::default();
    let _ = store.capacity_cleanup(&mut db, &mut state, Instant::now() + Duration::from_secs(5));
    db.progress_handler(0, None::<fn() -> bool>);
    let step: Option<i64> = db
        .query_row(
            "SELECT step FROM observation_capacity_pending_v5",
            [],
            |r| r.get(0),
        )
        .optional()
        .unwrap();
    assert!(
        step.is_none_or(|step| step > 0),
        "a tiny request cannot exhaust its SQL budget scanning other requests' chunks"
    );
}

#[cfg(unix)]
#[test]
fn capacity_accounting_preserves_sqlite_writer_locks_against_other_processes() {
    const CHILD_DB: &str = "HIROUTE_CAPACITY_LOCK_CHILD_DB";
    if let Some(path) = std::env::var_os(CHILD_DB) {
        let db = Connection::open(path).unwrap();
        db.busy_timeout(Duration::ZERO).unwrap();
        assert!(
            db.execute_batch("BEGIN IMMEDIATE").is_err(),
            "accounting must not release another SQLite connection's writer lock"
        );
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let db = store.connection.lock();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(db_bytes(&store).unwrap() > 0);
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "store::capacity::tests::capacity_accounting_preserves_sqlite_writer_locks_against_other_processes", "--nocapture"])
        .env(CHILD_DB, &store.activity_path)
        .status()
        .unwrap();
    assert!(
        child.success(),
        "a competing process acquired the active writer transaction after accounting"
    );
    db.execute_batch("ROLLBACK").unwrap();
}
#[test]
fn retires_completed_request_in_slices_preserves_active_shared_history_and_rollup() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let a = Fixture::new("capacity-old");
    let mut b = Fixture::new("capacity-live");
    b.workspace = hiroute_domain::WorkspaceId::parse("other-workspace").unwrap();
    let writer = writer(&store);
    let channel = fact_channel(&a, 512 * 1024);
    for event in request_facts(&a, "attempt-capacity", finished("attempt-capacity"), 100) {
        assert!(matches!(
            offer_fact(&writer, &channel, event),
            crate::WriterCycleOutcome::Ack(_)
        ));
    }
    install_content(&store, &a, b"shared retained body", 200);
    install_content(&store, &b, b"shared retained body", 300);
    let retired_fact =
        crate::store::fact_log::load_request(&store.connection.lock(), &a.workspace, &a.request)
            .unwrap()
            .remove(0);
    // A large request must make committed progress across multiple bounded units.
    {
        let mut db = store.connection.lock();
        let tx = db.transaction().unwrap();
        for i in 0..2048 {
            // Rowid order deliberately differs from the request index order.
            tx.execute("INSERT INTO conversation_content_events_v2(stream_ref,sequence,event_id,workspace_id,request_id,direction,fork_id,phase,metadata_json,envelope_digest) SELECT stream_ref,?1,?2,workspace_id,request_id,direction,fork_id,phase,metadata_json,envelope_digest FROM conversation_content_events_v2 WHERE request_id=?3 LIMIT 1",params![12047-i,format!("bulk-{i}"),a.request.as_str()]).unwrap();
            tx.execute("INSERT INTO observation_feedback_v2(stream_ref,rejected_or_acked_sequence,event_id,envelope_digest,feedback_kind,feedback_json,created_at_unix_nanos) SELECT stream_ref,sequence,event_id,envelope_digest,'ack',(SELECT feedback_json FROM observation_feedback_v2 WHERE feedback_kind='ack' LIMIT 1),100 FROM conversation_content_events_v2 WHERE event_id=?1",[format!("bulk-{i}")]).unwrap();
        }
        tx.commit().unwrap();
    }
    store.set_capacity_limit(Some(1)).unwrap();
    for _ in 0..30 {
        due(&store);
        store.capacity_cycle(1000).unwrap();
    }
    let db = store.connection.lock();
    assert!(
        !db.query_row(
            "SELECT EXISTS(SELECT 1 FROM logical_requests WHERE request_id=?1)",
            [a.request.as_str()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    assert!(
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM logical_requests WHERE request_id=?1)",
            [b.request.as_str()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM conversation_content_events_v2 WHERE request_id=?1",
            [a.request.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM observation_feedback_v2 WHERE event_id LIKE 'bulk-%'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0,
        "retiring a carrier must not strand its accepted feedback"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM observation_analysis_dirty_v1 WHERE request_id=?1",
            [a.request.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM observation_feedback_v2 WHERE event_id=?1",
            [retired_fact.event_id.as_str()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM observation_events WHERE event_id=?1",
            [retired_fact.event_id.as_str()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    let next: u64 = db
        .query_row(
            "SELECT MAX(highest_accounted_sequence)+1 FROM observation_checkpoints",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(db);
    use crate::writer::{IngestOutcome, ObservationCommitPort};
    assert!(matches!(
        store.ingest_fact(&retired_fact, &[]).unwrap(),
        IngestOutcome::Nack(_)
    ));
    let mut reused_id = retired_fact;
    reused_id.sequence = next;
    reused_id.correlation.request_id =
        hiroute_domain::LogicalRequestId::parse("changed-request-cannot-reuse-retired-event")
            .unwrap();
    assert!(
        matches!(store.ingest_fact(&reused_id,&[]).unwrap(), IngestOutcome::Nack(nack) if matches!(nack.detail,hiroute_domain::ObservationNackDetailV1::ImmutableProjectionConflict { .. }))
    );
    let detail = store
        .get_session(&b.workspace, &b.session, ContentMode::Messages)
        .unwrap();
    assert_eq!(detail.summary.request_count, 1);
    let value = store
        .get_value(&a.workspace, &value_query("plan/codex-daily", "USD", None))
        .unwrap();
    assert_eq!(value.input_tokens, 100);
    assert_eq!(value.estimated_total_savings_micros, Some(900));
    assert!(
        store
            .blob_path(
                store
                    .authority
                    .content_blob_digest("text/plain", b"shared retained body")
                    .as_str()
            )
            .exists()
    );
}
#[test]
fn busy_writer_yields_and_schedules_from_completion() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    store.set_capacity_limit(Some(1)).unwrap();
    let lock = store.connection.lock();
    let start = Instant::now();
    store.capacity_cycle(1).unwrap();
    assert!(start.elapsed() < Duration::from_millis(20));
    drop(lock);
    let state = store.capacity.lock();
    assert!(state.next_due >= start + INTERVAL);
    assert_eq!(state.status.reason, "writer_busy");
    drop(state);
    assert!(!store.capacity_cycle(2).unwrap());
}

#[test]
fn disabling_after_committed_retirement_resumes_cleanup_after_restart_without_selecting_another_request()
 {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let a = Fixture::new("disable-retired");
    let b = Fixture::new("disable-survivor");
    let writer = writer(&store);
    for (fixture, at) in [(&a, 100), (&b, 200)] {
        let channel = fact_channel(fixture, 512 * 1024);
        let attempt = format!("attempt-{}", fixture.request);
        for event in request_facts(fixture, &attempt, finished(&attempt), at) {
            assert!(matches!(
                offer_fact(&writer, &channel, event),
                crate::WriterCycleOutcome::Ack(_)
            ));
        }
    }
    install_content(&store, &a, b"already committed retirement", 300);
    install_content(&store, &b, b"completed but policy disabled", 400);
    store.set_capacity_limit(Some(1)).unwrap();
    store
        .capacity_retire(
            &mut store.connection.lock(),
            a.workspace.as_str(),
            a.session.as_str(),
            a.request.as_str(),
            1000,
        )
        .unwrap();
    store.set_capacity_limit(None).unwrap();
    drop(writer);
    drop(store);
    let store = open_store(directory.path());
    assert_eq!(store.capacity_status().unwrap().max_active_bytes, None);
    assert_eq!(
        store
            .connection
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM observation_capacity_pending_v5",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
    for _ in 0..30 {
        due(&store);
        store.capacity_cycle(1000).unwrap();
    }
    let db = store.connection.lock();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM observation_capacity_pending_v5",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM content_instances_v2 WHERE request_id=?1",
            [a.request.as_str()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert!(
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM logical_requests WHERE request_id=?1)",
            [b.request.as_str()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    assert!(db.query_row("SELECT EXISTS(SELECT 1 FROM content_instances_v2 WHERE request_id=?1 AND state='complete')", [b.request.as_str()], |r| r.get::<_, bool>(0)).unwrap());
}
#[test]
fn exhausted_sql_budget_rolls_back_without_deleting_or_running_fast_follow_on_work() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let mut db = store.connection.lock();
    db.progress_handler(1, Some(|| true));
    let result = store.capacity_retire(&mut db, "default", "missing", "missing", 0);
    assert!(result.is_err());
    db.progress_handler(0, None::<fn() -> bool>);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM observation_capacity_pending_v5",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn failed_policy_commit_keeps_the_previous_limit() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    store.set_capacity_limit(Some(20_000_000_000)).unwrap();
    store.connection.lock().execute_batch("CREATE TRIGGER reject_capacity BEFORE UPDATE ON observation_meta WHEN OLD.key='max_active_bytes' BEGIN SELECT RAISE(ABORT,'injected write failure'); END;").unwrap();
    assert!(store.set_capacity_limit(Some(1)).is_err());
    assert!(store.set_capacity_limit(None).is_err());
    assert_eq!(
        store.capacity_status().unwrap().max_active_bytes,
        Some(20_000_000_000)
    );
}
