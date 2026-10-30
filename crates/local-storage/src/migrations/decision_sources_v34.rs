//! Journal ownership and keyed runtime projection for source-owned decision credentials.
pub(crate) const CONTROL: &str = r#"
CREATE TABLE decision_source_effects (
    operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    workspace_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    before_owner_operation_id TEXT NOT NULL,
    PRIMARY KEY(operation_id, source_id)
);
CREATE TABLE decision_source_runtime (
    workspace_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    lineage_digest TEXT NOT NULL,
    source_revision INTEGER NOT NULL CHECK(source_revision > 0),
    source_owner_operation_id TEXT NOT NULL,
    state TEXT NOT NULL,
    targets_digest TEXT NOT NULL,
    credentials_json TEXT NOT NULL,
    PRIMARY KEY(workspace_id, source_id),
    FOREIGN KEY(workspace_id, source_id) REFERENCES compute_management_sources(workspace_id, source_id) ON DELETE CASCADE
);
CREATE TABLE decision_consumer_runtime (
    workspace_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    consumer_binding_id TEXT NOT NULL,
    service_id TEXT NOT NULL,
    service_revision INTEGER NOT NULL CHECK(service_revision > 0),
    model TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    wire_protocol TEXT NOT NULL,
    destination_id TEXT NOT NULL,
    PRIMARY KEY(workspace_id, source_id, consumer_binding_id),
    UNIQUE(workspace_id, source_id, service_id, service_revision),
    FOREIGN KEY(workspace_id, source_id) REFERENCES decision_source_runtime(workspace_id, source_id) ON DELETE CASCADE
);
CREATE INDEX decision_consumer_service_idx ON decision_consumer_runtime(workspace_id, service_id, source_id);

-- A deletion checks only the requested service. Populate historical versions and
-- drafts once during migration, then maintain their references with each write.
CREATE TABLE decision_plan_references (
    workspace_id TEXT NOT NULL,
    service_id TEXT NOT NULL,
    owner_kind TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    owner_revision INTEGER NOT NULL,
    PRIMARY KEY(workspace_id, service_id, owner_kind, owner_id, owner_revision)
) WITHOUT ROWID;
INSERT OR IGNORE INTO decision_plan_references
SELECT v.workspace_id, json_extract(node.value, '$.service.id'), 'version', v.plan_id, v.content_revision
FROM plan_versions v, json_tree(v.version_json) node
WHERE node.type='object' AND json_extract(node.value, '$.kind') IN ('decision_service', 'decision_service_v2')
  AND json_type(node.value, '$.service.id')='text';
INSERT OR IGNORE INTO decision_plan_references
SELECT d.workspace_id, json_extract(node.value, '$.service.id'), 'draft', d.draft_id, 0
FROM plan_drafts d, json_tree(d.draft_json) node
WHERE node.type='object' AND json_extract(node.value, '$.kind') IN ('decision_service', 'decision_service_v2')
  AND json_type(node.value, '$.service.id')='text';
CREATE TRIGGER decision_version_references_insert AFTER INSERT ON plan_versions BEGIN
    INSERT OR IGNORE INTO decision_plan_references
    SELECT NEW.workspace_id, json_extract(node.value, '$.service.id'), 'version', NEW.plan_id, NEW.content_revision
    FROM json_tree(NEW.version_json) node
    WHERE node.type='object' AND json_extract(node.value, '$.kind') IN ('decision_service', 'decision_service_v2')
      AND json_type(node.value, '$.service.id')='text';
END;
CREATE TRIGGER decision_version_references_update AFTER UPDATE OF workspace_id, plan_id, content_revision, version_json ON plan_versions BEGIN
    DELETE FROM decision_plan_references WHERE workspace_id=OLD.workspace_id AND owner_kind='version' AND owner_id=OLD.plan_id AND owner_revision=OLD.content_revision;
    INSERT OR IGNORE INTO decision_plan_references
    SELECT NEW.workspace_id, json_extract(node.value, '$.service.id'), 'version', NEW.plan_id, NEW.content_revision
    FROM json_tree(NEW.version_json) node
    WHERE node.type='object' AND json_extract(node.value, '$.kind') IN ('decision_service', 'decision_service_v2')
      AND json_type(node.value, '$.service.id')='text';
END;
CREATE TRIGGER decision_version_references_delete AFTER DELETE ON plan_versions BEGIN
    DELETE FROM decision_plan_references WHERE workspace_id=OLD.workspace_id AND owner_kind='version' AND owner_id=OLD.plan_id AND owner_revision=OLD.content_revision;
END;
CREATE TRIGGER decision_draft_references_insert AFTER INSERT ON plan_drafts BEGIN
    INSERT OR IGNORE INTO decision_plan_references
    SELECT NEW.workspace_id, json_extract(node.value, '$.service.id'), 'draft', NEW.draft_id, 0
    FROM json_tree(NEW.draft_json) node
    WHERE node.type='object' AND json_extract(node.value, '$.kind') IN ('decision_service', 'decision_service_v2')
      AND json_type(node.value, '$.service.id')='text';
END;
CREATE TRIGGER decision_draft_references_update AFTER UPDATE OF workspace_id, draft_id, draft_json ON plan_drafts BEGIN
    DELETE FROM decision_plan_references WHERE workspace_id=OLD.workspace_id AND owner_kind='draft' AND owner_id=OLD.draft_id AND owner_revision=0;
    INSERT OR IGNORE INTO decision_plan_references
    SELECT NEW.workspace_id, json_extract(node.value, '$.service.id'), 'draft', NEW.draft_id, 0
    FROM json_tree(NEW.draft_json) node
    WHERE node.type='object' AND json_extract(node.value, '$.kind') IN ('decision_service', 'decision_service_v2')
      AND json_type(node.value, '$.service.id')='text';
END;
CREATE TRIGGER decision_draft_references_delete AFTER DELETE ON plan_drafts BEGIN
    DELETE FROM decision_plan_references WHERE workspace_id=OLD.workspace_id AND owner_kind='draft' AND owner_id=OLD.draft_id AND owner_revision=0;
END;
CREATE INDEX decision_plan_reference_owner_idx ON decision_plan_references(workspace_id, owner_kind, owner_id, owner_revision);
"#;

/// The preceding locator was checked before the schema change. Advance only its
/// schema cookie in the same transaction, then recheck its unchanged coverage.
pub(super) fn refresh_locator(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), crate::LocalStorageError> {
    let schema: i64 = transaction.query_row("PRAGMA main.schema_version", [], |row| row.get(0))?;
    let updated = transaction.execute(
        "UPDATE agent_locator_coverage SET schema_version=?1 WHERE singleton=1 AND tampered=0",
        [schema],
    )?;
    if updated != 1 {
        return Err(crate::LocalStorageError::InvalidData);
    }
    crate::control::agent_locator::validate_coverage(transaction)
        .map_err(|_| crate::LocalStorageError::InvalidData)
}

#[cfg(test)]
pub(super) fn remove_test_shape(db: &rusqlite::Connection) {
    db.execute_batch(
        "DROP TRIGGER IF EXISTS decision_version_references_insert;
         DROP TRIGGER IF EXISTS decision_version_references_update;
         DROP TRIGGER IF EXISTS decision_version_references_delete;
         DROP TRIGGER IF EXISTS decision_draft_references_insert;
         DROP TRIGGER IF EXISTS decision_draft_references_update;
         DROP TRIGGER IF EXISTS decision_draft_references_delete;
         DROP TABLE IF EXISTS decision_plan_references;
         DROP TABLE IF EXISTS decision_consumer_runtime;
         DROP TABLE IF EXISTS decision_source_runtime;
         DROP TABLE IF EXISTS decision_source_effects;",
    )
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::super::{DatabaseKind, migration_sql};
    use rusqlite::{Connection, params};

    #[test]
    fn upgrade_adds_keyed_decision_tables_without_rewriting_existing_rows() {
        let db = Connection::open_in_memory().unwrap();
        for version in 1..=34 {
            db.execute_batch(migration_sql(DatabaseKind::Control, version).unwrap())
                .unwrap();
        }
        for table in [
            "decision_source_effects",
            "decision_source_runtime",
            "decision_consumer_runtime",
            "decision_plan_references",
        ] {
            assert!(db.prepare(&format!("SELECT * FROM {table}")).is_ok());
        }
    }

    #[test]
    fn decision_reference_backfill_and_writes_keep_deletion_lookup_keyed() {
        let db = Connection::open_in_memory().unwrap();
        for version in 1..=33 {
            db.execute_batch(migration_sql(DatabaseKind::Control, version).unwrap())
                .unwrap();
        }
        let old_version = r#"{"nested":[{"kind":"decision_service","service":{"id":"old"}},{"kind":"decision_service_v2","service":{"id":"shared"}}]}"#;
        let draft = r#"{"parked":{"kind":"decision_service_v2","service":{"id":"shared"}}}"#;
        db.execute("INSERT INTO plan_versions(workspace_id,plan_id,content_revision,content_digest,version_json,state,owner_operation_id) VALUES('workspace','plan',1,'digest',?1,'published','owner')", [old_version]).unwrap();
        db.execute("INSERT INTO plan_drafts(workspace_id,draft_id,revision,draft_json) VALUES('workspace','draft',1,?1)", [draft]).unwrap();
        db.execute_batch(migration_sql(DatabaseKind::Control, 34).unwrap())
            .unwrap();
        let count = |workspace: &str, service: &str| -> i64 {
            db.query_row("SELECT count(*) FROM decision_plan_references WHERE workspace_id=?1 AND service_id=?2", params![workspace, service], |row| row.get(0)).unwrap()
        };
        assert_eq!(count("workspace", "old"), 1);
        assert_eq!(count("workspace", "shared"), 2);
        assert_eq!(count("other", "shared"), 0);
        let details: Vec<String> = db.prepare("EXPLAIN QUERY PLAN SELECT EXISTS(SELECT 1 FROM decision_plan_references WHERE workspace_id='workspace' AND service_id='shared')").unwrap().query_map([], |row| row.get(3)).unwrap().collect::<Result<_, _>>().unwrap();
        assert!(
            details
                .iter()
                .any(|line| line.contains("SEARCH decision_plan_references USING PRIMARY KEY")),
            "{details:?}"
        );
        db.execute(
            "UPDATE plan_drafts SET draft_json=?1 WHERE draft_id='draft'",
            [r#"{"kind":"decision_service","service":{"id":"new"}}"#],
        )
        .unwrap();
        assert_eq!(count("workspace", "shared"), 1);
        assert_eq!(count("workspace", "new"), 1);
        assert!(
            db.execute(
                "UPDATE plan_drafts SET draft_json='{' WHERE draft_id='draft'",
                []
            )
            .is_err()
        );
        assert_eq!(count("workspace", "new"), 1);
        db.execute("DELETE FROM plan_versions WHERE plan_id='plan'", [])
            .unwrap();
        db.execute("DELETE FROM plan_drafts WHERE draft_id='draft'", [])
            .unwrap();
        assert_eq!(count("workspace", "old"), 0);
        assert_eq!(count("workspace", "shared"), 0);
        assert_eq!(count("workspace", "new"), 0);
    }
}
