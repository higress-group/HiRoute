//! Derived current Agent lookup. Coverage is published with its initial members.
//! The projection is a locator, never authority or a replacement for the journal.
pub(crate) const CONTROL: &str = r#"
CREATE TABLE agent_locator_coverage (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    format_version INTEGER NOT NULL CHECK(format_version=1),
    store_uuid TEXT NOT NULL CHECK(length(store_uuid)>0),
    schema_version INTEGER NOT NULL,
    member_count INTEGER NOT NULL CHECK(member_count>=0),
    pending_count INTEGER NOT NULL CHECK(pending_count>=0),
    head_count INTEGER NOT NULL CHECK(head_count>=0),
    tampered INTEGER NOT NULL CHECK(tampered IN (0,1))
);
CREATE TABLE agent_locator_members (
    ordinal INTEGER PRIMARY KEY,
    operation_id TEXT,
    workspace_id TEXT,
    context_id TEXT,
    generation INTEGER,
    accepted_digest TEXT,
    plan_digest TEXT,
    model_action TEXT,
    collaboration_action TEXT,
    management_action TEXT,
    resident_declaration INTEGER NOT NULL CHECK(resident_declaration IN (0,1)),
    grant_ref_json TEXT,
    grant_unresolved INTEGER NOT NULL CHECK(grant_unresolved IN (0,1)),
    classification TEXT NOT NULL CHECK(classification IN ('settings','other','unresolved')),
    CHECK(model_action IS NULL OR model_action IN ('configure','restore')),
    CHECK(collaboration_action IS NULL OR collaboration_action IN ('configure','restore')),
    CHECK(management_action IS NULL OR management_action IN ('configure','restore'))
);
CREATE UNIQUE INDEX agent_locator_member_operation ON agent_locator_members(operation_id)
    WHERE operation_id IS NOT NULL;
CREATE INDEX agent_locator_unresolved ON agent_locator_members(classification,workspace_id,ordinal);
CREATE INDEX agent_locator_grant_unresolved ON agent_locator_members(grant_unresolved,workspace_id,context_id,ordinal);
CREATE TABLE agent_locator_pending (
    operation_id TEXT PRIMARY KEY,
    ordinal INTEGER NOT NULL UNIQUE,
    workspace_id TEXT,
    context_id TEXT,
    generation INTEGER,
    accepted_digest TEXT,
    plan_digest TEXT,
    state TEXT,
    finalization_pending INTEGER,
    model_action TEXT,
    collaboration_action TEXT,
    management_action TEXT,
    resident_declaration INTEGER NOT NULL CHECK(resident_declaration IN (0,1)),
    classification TEXT NOT NULL CHECK(classification IN ('settings','other','unresolved')),
    CHECK(model_action IS NULL OR model_action IN ('configure','restore')),
    CHECK(collaboration_action IS NULL OR collaboration_action IN ('configure','restore')),
    CHECK(management_action IS NULL OR management_action IN ('configure','restore'))
);
CREATE INDEX agent_locator_pending_scope ON agent_locator_pending(workspace_id,context_id,classification,ordinal);
CREATE TABLE agent_locator_heads (
    workspace_id TEXT NOT NULL,
    scope TEXT NOT NULL CHECK(scope IN ('model','collaboration','management','grant','resident')),
    context_id TEXT NOT NULL,
    grant_key TEXT NOT NULL,
    ordinal INTEGER NOT NULL REFERENCES agent_locator_members(ordinal),
    action TEXT NOT NULL,
    PRIMARY KEY(workspace_id,scope,context_id,grant_key)
);
CREATE TABLE agent_locator_dirty (
    event_id INTEGER PRIMARY KEY,
    operation_id TEXT,
    old_ordinal INTEGER,
    new_ordinal INTEGER,
    old_workspace TEXT,
    new_workspace TEXT,
    flags INTEGER NOT NULL CHECK(flags>0)
);
CREATE INDEX agent_locator_dirty_operation ON agent_locator_dirty(operation_id);
"#;

// The producer clears only its own dirty member in the same transaction as the
// journal write. An unknown writer leaves a durable ordered uncertainty.
pub(crate) const TRIGGERS: &str = r#"
CREATE TRIGGER agent_locator_operation_insert AFTER INSERT ON operations
BEGIN
    INSERT INTO agent_locator_dirty(operation_id,new_ordinal,new_workspace,flags) VALUES(NEW.operation_id,NEW.rowid,NEW.workspace_id,1);
END;
CREATE TRIGGER agent_locator_operation_update AFTER UPDATE ON operations
BEGIN
    INSERT INTO agent_locator_dirty(operation_id,old_ordinal,new_ordinal,old_workspace,new_workspace,flags) VALUES(OLD.operation_id,OLD.rowid,NEW.rowid,OLD.workspace_id,NEW.workspace_id,1);
    INSERT INTO agent_locator_dirty(operation_id,old_ordinal,new_ordinal,old_workspace,new_workspace,flags) SELECT NEW.operation_id,OLD.rowid,NEW.rowid,OLD.workspace_id,NEW.workspace_id,1 WHERE NEW.operation_id IS NOT OLD.operation_id;
END;
CREATE TRIGGER agent_locator_operation_delete AFTER DELETE ON operations
BEGIN
    INSERT INTO agent_locator_dirty(operation_id,old_ordinal,old_workspace,flags) VALUES(OLD.operation_id,OLD.rowid,OLD.workspace_id,1);
END;
CREATE TRIGGER agent_locator_step_insert AFTER INSERT ON operation_steps
BEGIN
    INSERT INTO agent_locator_dirty(operation_id,old_ordinal,new_ordinal,old_workspace,new_workspace,flags) SELECT NEW.operation_id,rowid,rowid,workspace_id,workspace_id,CASE WHEN NEW.step_no=1 THEN 2 ELSE 8 END FROM operations WHERE operation_id=NEW.operation_id;
END;
CREATE TRIGGER agent_locator_step_update AFTER UPDATE ON operation_steps
BEGIN
    INSERT INTO agent_locator_dirty(operation_id,old_ordinal,new_ordinal,old_workspace,new_workspace,flags) SELECT OLD.operation_id,rowid,rowid,workspace_id,workspace_id,CASE WHEN OLD.step_no=1 OR NEW.step_no=1 THEN 2 ELSE 8 END FROM operations WHERE operation_id=OLD.operation_id;
    INSERT INTO agent_locator_dirty(operation_id,old_ordinal,new_ordinal,old_workspace,new_workspace,flags) SELECT NEW.operation_id,rowid,rowid,workspace_id,workspace_id,CASE WHEN OLD.step_no=1 OR NEW.step_no=1 THEN 2 ELSE 8 END FROM operations WHERE operation_id=NEW.operation_id AND NEW.operation_id IS NOT OLD.operation_id;
END;
CREATE TRIGGER agent_locator_step_delete AFTER DELETE ON operation_steps
BEGIN
    INSERT INTO agent_locator_dirty(operation_id,old_ordinal,new_ordinal,old_workspace,new_workspace,flags) SELECT OLD.operation_id,rowid,rowid,workspace_id,workspace_id,CASE WHEN OLD.step_no=1 THEN 2 ELSE 8 END FROM operations WHERE operation_id=OLD.operation_id;
END;
CREATE TRIGGER agent_locator_payload_insert AFTER INSERT ON operation_payloads
BEGIN
    INSERT INTO agent_locator_dirty(operation_id,new_ordinal,new_workspace,flags) VALUES(NEW.operation_id,(SELECT rowid FROM operations WHERE operation_id=NEW.operation_id),(SELECT workspace_id FROM operations WHERE operation_id=NEW.operation_id),4);
END;
CREATE TRIGGER agent_locator_payload_update AFTER UPDATE ON operation_payloads
BEGIN
    INSERT INTO agent_locator_dirty(operation_id,old_ordinal,new_ordinal,old_workspace,new_workspace,flags) SELECT OLD.operation_id,rowid,rowid,workspace_id,workspace_id,4 FROM operations WHERE operation_id=OLD.operation_id;
    INSERT INTO agent_locator_dirty(operation_id,old_ordinal,new_ordinal,old_workspace,new_workspace,flags) SELECT NEW.operation_id,rowid,rowid,workspace_id,workspace_id,4 FROM operations WHERE operation_id=NEW.operation_id AND NEW.operation_id IS NOT OLD.operation_id;
END;
CREATE TRIGGER agent_locator_payload_delete AFTER DELETE ON operation_payloads
BEGIN
    INSERT INTO agent_locator_dirty(operation_id,old_ordinal,old_workspace,flags) VALUES(OLD.operation_id,(SELECT rowid FROM operations WHERE operation_id=OLD.operation_id),(SELECT workspace_id FROM operations WHERE operation_id=OLD.operation_id),4);
END;
CREATE TRIGGER agent_locator_member_insert AFTER INSERT ON agent_locator_members BEGIN UPDATE agent_locator_coverage SET tampered=1 WHERE singleton=1; END;
CREATE TRIGGER agent_locator_member_update AFTER UPDATE ON agent_locator_members BEGIN UPDATE agent_locator_coverage SET tampered=1 WHERE singleton=1; END;
CREATE TRIGGER agent_locator_member_delete AFTER DELETE ON agent_locator_members BEGIN UPDATE agent_locator_coverage SET tampered=1 WHERE singleton=1; END;
CREATE TRIGGER agent_locator_pending_insert AFTER INSERT ON agent_locator_pending BEGIN UPDATE agent_locator_coverage SET tampered=1 WHERE singleton=1; END;
CREATE TRIGGER agent_locator_pending_update AFTER UPDATE ON agent_locator_pending BEGIN UPDATE agent_locator_coverage SET tampered=1 WHERE singleton=1; END;
CREATE TRIGGER agent_locator_pending_delete AFTER DELETE ON agent_locator_pending BEGIN UPDATE agent_locator_coverage SET tampered=1 WHERE singleton=1; END;
CREATE TRIGGER agent_locator_head_insert AFTER INSERT ON agent_locator_heads BEGIN UPDATE agent_locator_coverage SET tampered=1 WHERE singleton=1; END;
CREATE TRIGGER agent_locator_head_update AFTER UPDATE ON agent_locator_heads BEGIN UPDATE agent_locator_coverage SET tampered=1 WHERE singleton=1; END;
CREATE TRIGGER agent_locator_head_delete AFTER DELETE ON agent_locator_heads BEGIN UPDATE agent_locator_coverage SET tampered=1 WHERE singleton=1; END;
CREATE TRIGGER agent_locator_dirty_update AFTER UPDATE ON agent_locator_dirty BEGIN UPDATE agent_locator_coverage SET tampered=1 WHERE singleton=1; END;
CREATE TRIGGER agent_locator_dirty_delete AFTER DELETE ON agent_locator_dirty BEGIN UPDATE agent_locator_coverage SET tampered=1 WHERE singleton=1; END;
"#;

pub(super) fn initialize(tx: &rusqlite::Transaction<'_>) -> Result<(), crate::LocalStorageError> {
    crate::control::agent_locator::initialize(tx)
}

#[cfg(test)]
pub(crate) fn remove_test_shape(db: &rusqlite::Connection) {
    for definition in TRIGGERS.split("CREATE TRIGGER ").skip(1) {
        let (name, _) = definition.split_once(' ').unwrap();
        db.execute_batch(&format!("DROP TRIGGER IF EXISTS {name}"))
            .unwrap();
    }
    db.execute_batch(
        "DROP TABLE IF EXISTS agent_locator_heads;
        DROP TABLE IF EXISTS agent_locator_members; DROP TABLE IF EXISTS agent_locator_pending; DROP TABLE IF EXISTS agent_locator_dirty;
        DROP TABLE IF EXISTS agent_locator_coverage;",
    )
    .unwrap();
}
