//! Current dependency discovery is durable state, populated before this format is admitted.
//! SQLite maintains membership in the same transaction as every journal state change.
pub(super) const CONTROL: &str = r#"
CREATE TABLE operation_payloads (
    operation_id TEXT NOT NULL REFERENCES operations(operation_id) DEFERRABLE INITIALLY DEFERRED,
    digest TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    PRIMARY KEY(operation_id,digest)
);
ALTER TABLE operations ADD COLUMN finalization_pending INTEGER NOT NULL DEFAULT 1
    CHECK(finalization_pending IN (0,1));
CREATE TABLE operation_recovery_members (
    operation_id TEXT PRIMARY KEY REFERENCES operations(operation_id)
);
CREATE TABLE operation_recovery_coverage (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    format_version INTEGER NOT NULL CHECK(format_version=1),
    store_uuid TEXT NOT NULL CHECK(length(store_uuid)>0),
    pending_count INTEGER NOT NULL CHECK(pending_count>=0)
);
CREATE TABLE agent_native_target_heads (
    workspace_id TEXT NOT NULL,
    context_id TEXT NOT NULL,
    owner_operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    PRIMARY KEY(workspace_id,context_id)
);
CREATE TABLE agent_native_target_coverage (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    format_version INTEGER NOT NULL CHECK(format_version=1),
    store_uuid TEXT NOT NULL CHECK(length(store_uuid)>0),
    target_count INTEGER NOT NULL CHECK(target_count>=0)
);
CREATE TABLE artifact_heads (
    workspace_id TEXT NOT NULL,
    target TEXT NOT NULL,
    owner_operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    owner_json TEXT NOT NULL,
    PRIMARY KEY(workspace_id,target)
);
CREATE TABLE artifact_head_coverage (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    format_version INTEGER NOT NULL CHECK(format_version=1),
    store_uuid TEXT NOT NULL CHECK(length(store_uuid)>0),
    head_count INTEGER NOT NULL CHECK(head_count>=0)
);
CREATE TABLE artifact_head_transitions (
    operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    target TEXT NOT NULL,
    before_json TEXT NOT NULL,
    PRIMARY KEY(operation_id,target)
);
CREATE TABLE publication_grant_proofs (
    workspace_id TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK(generation>0),
    material_sha256 TEXT NOT NULL,
    publication_revision INTEGER NOT NULL,
    PRIMARY KEY(workspace_id,grant_id,generation,material_sha256,publication_revision),
    FOREIGN KEY(workspace_id,publication_revision)
        REFERENCES gateway_publications(workspace_id,publication_revision) ON DELETE CASCADE
);
CREATE INDEX publication_grant_proofs_identity ON publication_grant_proofs(grant_id,generation,material_sha256,publication_revision);
CREATE INDEX plan_versions_prepared ON plan_versions(workspace_id,plan_id,content_revision) WHERE state='prepared';
CREATE INDEX gateway_publications_state ON gateway_publications(state,workspace_id,publication_revision);
"#;

pub(crate) const TRIGGERS: &str = r#"
CREATE TRIGGER operation_recovery_insert AFTER INSERT ON operations
WHEN NEW.state NOT IN ('succeeded','rolled_back') OR NEW.finalization_pending=1
BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM operation_recovery_coverage c JOIN storage_meta s
        ON s.singleton=c.singleton AND s.store_uuid=c.store_uuid
        WHERE c.singleton=1 AND c.format_version=1
        AND c.pending_count=(SELECT count(*) FROM operation_recovery_members))
        THEN RAISE(ABORT,'incomplete operation recovery coverage') END;
    INSERT INTO operation_recovery_members VALUES(NEW.operation_id);
    UPDATE operation_recovery_coverage SET pending_count=pending_count+1 WHERE singleton=1;
END;
CREATE TRIGGER operation_recovery_guard BEFORE UPDATE OF state,finalization_pending ON operations
WHEN NOT EXISTS(SELECT 1 FROM operation_recovery_coverage c JOIN storage_meta s
        ON s.singleton=c.singleton AND s.store_uuid=c.store_uuid
        WHERE c.singleton=1 AND c.format_version=1
        AND c.pending_count=(SELECT count(*) FROM operation_recovery_members))
    OR (OLD.state NOT IN ('succeeded','rolled_back') OR OLD.finalization_pending=1)
    != EXISTS(SELECT 1 FROM operation_recovery_members WHERE operation_id=OLD.operation_id)
BEGIN
    SELECT RAISE(ABORT,'incomplete operation recovery membership');
END;
CREATE TRIGGER operation_recovery_update AFTER UPDATE OF state,finalization_pending ON operations
BEGIN
    INSERT INTO operation_recovery_members
        SELECT NEW.operation_id
        WHERE (NEW.state NOT IN ('succeeded','rolled_back') OR NEW.finalization_pending=1)
          AND NOT (OLD.state NOT IN ('succeeded','rolled_back') OR OLD.finalization_pending=1);
    DELETE FROM operation_recovery_members WHERE operation_id=NEW.operation_id
        AND NEW.state IN ('succeeded','rolled_back') AND NEW.finalization_pending=0;
    UPDATE operation_recovery_coverage SET pending_count=pending_count
        + (NEW.state NOT IN ('succeeded','rolled_back') OR NEW.finalization_pending=1)
        - (OLD.state NOT IN ('succeeded','rolled_back') OR OLD.finalization_pending=1)
        WHERE singleton=1;
END;
"#;

pub(super) fn initialize(tx: &rusqlite::Transaction<'_>) -> Result<(), crate::LocalStorageError> {
    crate::control::current_dependencies::initialize(tx)?;
    tx.execute_batch(TRIGGERS)?;
    super::routing_read_v32::initialize(tx)?;
    Ok(())
}

#[cfg(test)]
pub(super) fn remove_test_shape(db: &rusqlite::Connection) {
    super::routing_read_v32::remove_test_shape(db);
    db.execute_batch("DROP TRIGGER operation_recovery_insert;
        DROP TRIGGER operation_recovery_guard; DROP TRIGGER operation_recovery_update;
        DROP TABLE agent_native_target_heads; DROP TABLE agent_native_target_coverage;
        DROP TABLE artifact_head_transitions; DROP TABLE artifact_heads; DROP TABLE artifact_head_coverage;
        DROP TABLE operation_payloads; DROP TABLE operation_recovery_coverage; DROP TABLE operation_recovery_members;
        DROP TABLE publication_grant_proofs; DROP INDEX plan_versions_prepared;
        DROP INDEX gateway_publications_state;
        ALTER TABLE operations DROP COLUMN finalization_pending;").unwrap();
}
