//! Add Qoder model verification while retaining existing result bytes and revision authority.

pub(super) const CONTROL: &str = r#"
CREATE TABLE agent_surface_checks_v25 (
    workspace_id TEXT NOT NULL,
    context_id TEXT NOT NULL,
    surface TEXT NOT NULL
        CHECK(surface IN ('codex_cli','codex_desktop','claude_cli','qoder_cli')),
    applied_revision INTEGER NOT NULL
        CHECK(typeof(applied_revision) = 'integer' AND applied_revision > 0),
    record_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(workspace_id, context_id, surface)
);
INSERT INTO agent_surface_checks_v25
    SELECT workspace_id, context_id, surface, applied_revision, record_json, updated_at
    FROM agent_surface_checks;
DROP TABLE agent_surface_checks;
ALTER TABLE agent_surface_checks_v25 RENAME TO agent_surface_checks;
"#;
