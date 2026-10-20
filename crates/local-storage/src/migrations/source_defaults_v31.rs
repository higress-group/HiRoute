//! Additive source-owned preferences; the stopped database backup includes this entire table.
pub(super) const CONTROL: &str = "CREATE TABLE compute_source_defaults (
    workspace_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    preferences_json TEXT NOT NULL,
    PRIMARY KEY(workspace_id, source_id),
    FOREIGN KEY(workspace_id, source_id) REFERENCES compute_management_sources(workspace_id, source_id) ON DELETE CASCADE
);
ALTER TABLE compute_management_effects ADD COLUMN preferences_before TEXT;
ALTER TABLE compute_management_effects ADD COLUMN preferences_after TEXT;";
