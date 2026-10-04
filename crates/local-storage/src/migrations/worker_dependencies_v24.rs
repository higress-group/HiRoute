//! Extend native ACP installation support without decoding or reserializing stored journals.

pub(super) const CONTROL: &str = r#"
CREATE TABLE worker_dependency_selections_v24 (
    workspace_id TEXT NOT NULL,
    harness TEXT NOT NULL CHECK(harness IN ('codex_cli','claude_code','qoder_cli')),
    revision INTEGER NOT NULL
        CHECK(typeof(revision) = 'integer' AND revision > 0),
    selection_json TEXT NOT NULL,
    owner_operation_id TEXT NOT NULL REFERENCES operations(operation_id),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(workspace_id, harness)
);

CREATE TABLE worker_dependency_selection_effects_v24 (
    operation_id TEXT PRIMARY KEY REFERENCES operations(operation_id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    harness TEXT NOT NULL CHECK(harness IN ('codex_cli','claude_code','qoder_cli')),
    before_revision INTEGER NOT NULL
        CHECK(typeof(before_revision) = 'integer' AND before_revision >= 0),
    before_json TEXT,
    before_owner_operation_id TEXT,
    after_revision INTEGER NOT NULL
        CHECK(typeof(after_revision) = 'integer' AND after_revision > 0),
    after_json TEXT NOT NULL,
    activated INTEGER NOT NULL DEFAULT 0 CHECK(activated IN (0,1)),
    compensated INTEGER NOT NULL DEFAULT 0 CHECK(compensated IN (0,1)),
    UNIQUE(workspace_id, harness, after_revision)
);

INSERT INTO worker_dependency_selections_v24
    SELECT workspace_id, harness, revision, selection_json, owner_operation_id, updated_at
    FROM worker_dependency_selections;
INSERT INTO worker_dependency_selection_effects_v24
    SELECT operation_id, workspace_id, harness, before_revision, before_json,
           before_owner_operation_id, after_revision, after_json, activated, compensated
    FROM worker_dependency_selection_effects;

DROP TABLE worker_dependency_selection_effects;
DROP TABLE worker_dependency_selections;
ALTER TABLE worker_dependency_selections_v24 RENAME TO worker_dependency_selections;
ALTER TABLE worker_dependency_selection_effects_v24 RENAME TO worker_dependency_selection_effects;
CREATE INDEX worker_dependency_selection_effects_state
    ON worker_dependency_selection_effects(activated, compensated, operation_id);
"#;
