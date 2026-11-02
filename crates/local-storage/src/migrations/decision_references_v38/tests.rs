use super::super::{DatabaseKind, migrate};
use rusqlite::{Connection, params};
use serde_json::json;
use std::path::Path;

fn upgrade(db: &mut Connection, path: &Path, target: u32) -> Result<(), crate::LocalStorageError> {
    migrate(
        db,
        path,
        DatabaseKind::Control,
        &path.with_extension("backups"),
        None,
        None,
        None,
        Some(target),
    )
}

fn source(path: &Path) -> Connection {
    let mut db = Connection::open(path).unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    upgrade(&mut db, path, 37).unwrap();
    db
}

fn shape(kind: &str, id: &str) -> String {
    json!({"nested":{"classifier":{"kind":kind,"service":{"id":id}}}}).to_string()
}

fn insert(db: &Connection, owner: &str, content: &str) {
    db.execute("INSERT INTO plan_versions(workspace_id,plan_id,content_revision,content_digest,version_json,state,owner_operation_id) VALUES('workspace',?1,1,'digest',?2,'published','owner')", params![owner, content]).unwrap();
    db.execute("INSERT INTO plan_drafts(workspace_id,draft_id,revision,draft_json) VALUES('workspace',?1,1,?2)", params![owner, content]).unwrap();
}

fn count(db: &Connection, workspace: &str, id: &str) -> i64 {
    db.query_row(
        "SELECT count(*) FROM decision_plan_references WHERE workspace_id=?1 AND service_id=?2",
        params![workspace, id],
        |row| row.get(0),
    )
    .unwrap()
}

fn owner_bytes(db: &Connection) -> Vec<String> {
    db.prepare(
        "SELECT version_json FROM plan_versions ORDER BY workspace_id,plan_id,content_revision",
    )
    .unwrap()
    .query_map([], |row| row.get(0))
    .unwrap()
    .chain(
        db.prepare("SELECT draft_json FROM plan_drafts ORDER BY workspace_id,draft_id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap(),
    )
    .collect::<Result<_, _>>()
    .unwrap()
}

#[test]
fn schema37_upgrade_backfills_v3_without_rewriting_owners_and_keeps_writes_keyed() {
    let root = crate::test_tempdir().unwrap();
    let path = root.path().join("control.db");
    let mut db = source(&path);
    for (kind, id) in [
        ("decision_service", "v1"),
        ("decision_service_v2", "v2"),
        ("decision_service_v3", "v3"),
    ] {
        insert(&db, id, &shape(kind, id));
    }
    let before = owner_bytes(&db);
    assert_eq!(count(&db, "workspace", "v2"), 2);
    assert_eq!(count(&db, "workspace", "v3"), 0);
    upgrade(&mut db, &path, 38).unwrap();
    assert_eq!(owner_bytes(&db), before);
    for id in ["v1", "v2", "v3"] {
        assert_eq!(count(&db, "workspace", id), 2);
        assert_eq!(count(&db, "other", id), 0);
    }
    let plan: Vec<String> = db.prepare("EXPLAIN QUERY PLAN SELECT EXISTS(SELECT 1 FROM decision_plan_references WHERE workspace_id='workspace' AND service_id='v3')")
        .unwrap().query_map([], |row| row.get(3)).unwrap().collect::<Result<_, _>>().unwrap();
    assert!(
        plan.iter()
            .any(|line| line.contains("SEARCH decision_plan_references USING PRIMARY KEY")),
        "{plan:?}"
    );
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    upgrade(&mut db, &path, 38).unwrap();
    assert_eq!(owner_bytes(&db), before);
    assert_eq!(count(&db, "workspace", "v3"), 2);
    // Updates must remove the OLD owner key, not just add the new service.
    db.execute("UPDATE plan_versions SET workspace_id='other',plan_id='moved',content_revision=2,version_json=?1 WHERE plan_id='v3'", [shape("decision_service_v3", "moved")]).unwrap();
    db.execute("UPDATE plan_drafts SET workspace_id='other',draft_id='moved',draft_json=?1 WHERE draft_id='v3'", [shape("decision_service_v3", "moved")]).unwrap();
    assert_eq!(count(&db, "workspace", "v3"), 0);
    assert_eq!(count(&db, "other", "moved"), 2);
    assert!(
        db.execute(
            "UPDATE plan_drafts SET draft_json='{' WHERE draft_id='moved'",
            []
        )
        .is_err()
    );
    assert_eq!(count(&db, "other", "moved"), 2);
    for kind in [
        "decision_service",
        "decision_service_v2",
        "decision_service_v3",
    ] {
        let id = format!("new-{kind}");
        insert(&db, &id, &shape(kind, &id));
        assert_eq!(count(&db, "workspace", &id), 2);
    }
    db.execute("DELETE FROM plan_versions", []).unwrap();
    db.execute("DELETE FROM plan_drafts", []).unwrap();
    let remaining: i64 = db
        .query_row("SELECT count(*) FROM decision_plan_references", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(remaining, 0);
}

#[test]
fn failed_backfill_rolls_back_triggers_and_schema_then_resumes() {
    let root = crate::test_tempdir().unwrap();
    let path = root.path().join("control.db");
    let mut db = source(&path);
    insert(&db, "v3", &shape("decision_service_v3", "v3"));
    let before = owner_bytes(&db);
    db.execute_batch("CREATE TEMP TRIGGER reject_reference_backfill BEFORE INSERT ON main.decision_plan_references BEGIN SELECT RAISE(ABORT,'fixture_backfill_failure'); END;").unwrap();
    assert!(upgrade(&mut db, &path, 38).is_err());
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        37
    );
    assert_eq!(owner_bytes(&db), before);
    assert_eq!(count(&db, "workspace", "v3"), 0);
    let original: String = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='decision_version_references_insert'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!original.contains("decision_service_v3"));
    db.execute_batch("DROP TRIGGER reject_reference_backfill")
        .unwrap();
    upgrade(&mut db, &path, 38).unwrap();
    assert_eq!(owner_bytes(&db), before);
    assert_eq!(count(&db, "workspace", "v3"), 2);
    crate::control::agent_locator::validate_coverage(&db).unwrap();
}
