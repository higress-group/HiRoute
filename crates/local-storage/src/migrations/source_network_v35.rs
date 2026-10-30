//! One-time initialization at the production storage boundary. Current missing rows are errors.
pub(super) const CONTROL: &str = r#"
CREATE TABLE native_source_network (
    workspace_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    record_json TEXT NOT NULL,
    owner_operation_id TEXT NOT NULL,
    PRIMARY KEY(workspace_id, source_id),
    FOREIGN KEY(workspace_id, source_id) REFERENCES compute_management_sources(workspace_id, source_id) ON DELETE CASCADE
);
INSERT INTO native_source_network(workspace_id,source_id,record_json,owner_operation_id)
SELECT workspace_id,source_id,
    json_object('schema','hiroute.native-network-policy/v1',
        'source_id',source_id,'lineage_digest',json_extract(source_json,'$.lineage_digest'),
        'network_revision',1,'policy',json_object('mode','direct')),
    owner_operation_id
FROM compute_management_sources WHERE json_extract(source_json,'$.provenance.kind') IN ('registered','user_configured');
ALTER TABLE compute_management_effects ADD COLUMN network_before TEXT;
ALTER TABLE compute_management_effects ADD COLUMN network_after TEXT;
"#;

pub(super) fn validate(connection: &rusqlite::Connection) -> Result<(), crate::LocalStorageError> {
    use crate::LocalStorageError;
    use hiroute_domain::{ComputeManagementSourceV2, NativeSourceNetworkRecordV1};
    let mut statement = connection.prepare("SELECT s.source_json,n.record_json FROM compute_management_sources s LEFT JOIN native_source_network n USING(workspace_id,source_id)")?;
    for row in statement.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
    })? {
        let (source, record) = row?;
        let source: ComputeManagementSourceV2 =
            serde_json::from_str(&source).map_err(|_| LocalStorageError::InvalidData)?;
        match (source.provenance.is_native(), record) {
            (true, Some(json)) => {
                let record: NativeSourceNetworkRecordV1 =
                    serde_json::from_str(&json).map_err(|_| LocalStorageError::InvalidData)?;
                record
                    .validate_for(&source)
                    .map_err(|_| LocalStorageError::InvalidData)?;
            }
            (false, None) => {}
            _ => return Err(LocalStorageError::InvalidData),
        }
    }
    let orphan: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM native_source_network n LEFT JOIN compute_management_sources s USING(workspace_id,source_id) WHERE s.source_id IS NULL)", [], |r| r.get(0))?;
    if orphan {
        return Err(LocalStorageError::InvalidData);
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn remove_test_shape(db: &rusqlite::Connection) {
    db.execute_batch("DROP TABLE IF EXISTS native_source_network;")
        .unwrap();
    for column in ["network_before", "network_after"] {
        let present: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('compute_management_effects') WHERE name=?1)",
            [column], |row| row.get(0),
        ).unwrap();
        if present {
            db.execute_batch(&format!(
                "ALTER TABLE compute_management_effects DROP COLUMN {column};"
            ))
            .unwrap();
        }
    }
}
