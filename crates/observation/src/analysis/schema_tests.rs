use super::*;
use crate::store::tests::support::*;
use crate::writer::ObservationCommitPort;
use hiroute_domain::*;

fn ingest(store: &crate::LocalObservationStore, suffix: &str) -> Fixture {
    let f = Fixture::new(suffix);
    for e in request_facts(&f, "schema-attempt", finished("schema-attempt"), 100) {
        store.ingest_fact(&e, &[]).unwrap();
    }
    f
}

fn drop_triggers(db: &Connection) {
    let names = {
        let mut s=db.prepare("SELECT name FROM sqlite_master WHERE type='trigger' AND (name LIKE 'analysis_dirty_%' OR name='analysis_typed_delete_v1')").unwrap();
        s.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert_eq!(names.len(), 14);
    assert!(
        names
            .iter()
            .any(|name| name == "analysis_dirty_session_agent_v1")
    );
    for name in names {
        db.execute_batch(&format!("DROP TRIGGER {name}")).unwrap();
    }
}

#[test]
fn compatible_core_writes_survive_atomic_auxiliary_table_and_trigger_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let f = ingest(&store, "schema-original");
    {
        let mut db = store.connection.lock();
        let tx = db.transaction().unwrap();
        drop_triggers(&tx);
        for table in [
            "observation_analysis_meta_v1",
            "observation_analysis_dirty_v1",
            "observation_analysis_cuts_v1",
        ] {
            tx.execute_batch(&format!("ALTER TABLE {table} RENAME TO replaced_auxiliary"))
                .unwrap();
            tx.execute_batch(DDL).unwrap();
            tx.execute_batch(&format!("INSERT INTO {table} SELECT * FROM replaced_auxiliary; DROP TABLE replaced_auxiliary")).unwrap();
        }
        tx.execute_batch(DDL).unwrap();
        install_dirty_triggers(&tx).unwrap();
        tx.commit().unwrap();
        // An unaware writer only executes core-table SQL. The installed trigger
        // must still notify the candidate without a Rust projection callback.
        for (sql, identity) in [
            (
                "UPDATE logical_requests SET finished_at_ms=finished_at_ms WHERE workspace_id=?1 AND request_id=?2",
                f.request.as_str(),
            ),
            (
                "UPDATE sessions SET agent_id='agent_claude_default' WHERE workspace_id=?1 AND session_id=?2",
                f.session.as_str(),
            ),
        ] {
            db.execute("DELETE FROM observation_analysis_dirty_v1", [])
                .unwrap();
            db.execute(sql, params![f.workspace.as_str(), identity])
                .unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT COUNT(*) FROM observation_analysis_dirty_v1",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
                1
            );
        }
    }
    let fresh = ingest(&store, "schema-after-replacement");
    let principal = ObservationPrincipalV1::local_user(fresh.workspace.clone());
    let spec = SessionDeletionSpecV1 {
        workspace_id: fresh.workspace.clone(),
        session_id: fresh.session.clone(),
        data_class: DeletionDataClass::FactsAndContent,
        delete_rollups: true,
    };
    let preview = store.preview_session_deletion(&principal, &spec).unwrap();
    store
        .apply_session_deletion(
            &principal,
            &spec,
            preview.store_revision,
            &preview.change_digest,
            1000,
        )
        .unwrap();
    drop(store);
    let store = open_store(dir.path());
    ingest(&store, "schema-after-reopen");
}

#[test]
fn interrupted_auxiliary_replacement_rolls_back_core_writer_dependencies() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    ingest(&store, "schema-before-interruption");
    {
        let mut db = store.connection.lock();
        let tx = db.transaction().unwrap();
        drop_triggers(&tx);
        tx.execute_batch("DROP TABLE observation_analysis_dirty_v1")
            .unwrap();
        tx.rollback().unwrap();
    }
    ingest(&store, "schema-after-interruption");
    drop(store);
    let store = open_store(dir.path());
    ingest(&store, "schema-after-rollback-reopen");
}

#[test]
fn missing_new_auxiliary_table_does_not_block_core_ingest_or_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    ingest(&store, "schema-before-incomplete-admission");
    store
        .connection
        .lock()
        .execute_batch("DROP TABLE observation_native_origins_v1")
        .unwrap();
    assert!(!available(&store.connection.lock()));
    let f = ingest(&store, "schema-with-incomplete-admission");
    let principal = ObservationPrincipalV1::local_user(f.workspace.clone());
    let spec = SessionDeletionSpecV1 {
        workspace_id: f.workspace.clone(),
        session_id: f.session.clone(),
        data_class: DeletionDataClass::FactsAndContent,
        delete_rollups: false,
    };
    let p = store.preview_session_deletion(&principal, &spec).unwrap();
    store
        .apply_session_deletion(&principal, &spec, p.store_revision, &p.change_digest, 1000)
        .unwrap();
    drop(store);
    let store = open_store(dir.path());
    assert!(
        available(&store.connection.lock()),
        "normal admission installs the additive table on retry"
    );
    ingest(&store, "schema-after-admission-retry");
}
