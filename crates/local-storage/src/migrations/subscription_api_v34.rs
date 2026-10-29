//! Durable opt-in import associations share source-save activation and compensation.
pub(super) const CONTROL: &str = "CREATE TABLE compute_subscription_api_links (
    workspace_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    api_source_id TEXT NOT NULL,
    PRIMARY KEY(workspace_id, source_id),
    CHECK(source_id <> api_source_id),
    FOREIGN KEY(workspace_id, source_id) REFERENCES compute_management_sources(workspace_id, source_id) ON DELETE CASCADE
);
ALTER TABLE compute_management_effects ADD COLUMN api_link_before TEXT;
ALTER TABLE compute_management_effects ADD COLUMN api_link_after TEXT;";

pub(crate) fn validate_links(
    connection: &rusqlite::Connection,
) -> Result<(), crate::LocalStorageError> {
    use hiroute_domain::ComputeSubscriptionApiLinkV1;
    let mut statement =
        connection.prepare("SELECT source_id,api_source_id FROM compute_subscription_api_links")?;
    for row in statement.query_map([], |row| {
        Ok(ComputeSubscriptionApiLinkV1 {
            source_id: row.get(0)?,
            api_source_id: row.get(1)?,
        })
    })? {
        row?.validate()
            .map_err(|_| crate::LocalStorageError::InvalidData)?;
    }
    let invalid: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM compute_subscription_api_links l
         LEFT JOIN compute_management_sources s ON s.workspace_id=l.workspace_id AND s.source_id=l.source_id
         LEFT JOIN compute_management_sources a ON a.workspace_id=l.workspace_id AND a.source_id=l.api_source_id
         WHERE s.source_id IS NULL OR json_extract(s.source_json,'$.provenance.kind') IS NOT 'connector_owned'
          OR json_extract(s.source_json,'$.provenance.connector_id') IS NOT 'connector.cpa.codex'
          OR (a.source_id IS NOT NULL AND json_extract(a.source_json,'$.provenance.kind') NOT IN ('registered','user_configured')))",
        [], |row| row.get(0),
    )?;
    if invalid {
        return Err(crate::LocalStorageError::InvalidData);
    }
    Ok(())
}
