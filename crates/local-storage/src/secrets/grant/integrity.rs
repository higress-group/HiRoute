//! Authenticate current grants and their exact persisted effect references.
use super::*;
use crate::LocalStorageError;

pub(crate) fn validate(store: &LocalSecretStore) -> Result<(), LocalStorageError> {
    let connection = store.connection.borrow();
    let mut statement =
        connection.prepare("SELECT connection_id,generation FROM agent_access_grant_versions")?;
    for row in statement.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))? {
        let (id, generation) = row?;
        let version = read_version(&connection, &id, generation)
            .map_err(|_| LocalStorageError::InvalidData)?
            .ok_or(LocalStorageError::InvalidData)?;
        authenticate_version(store, &version).map_err(|_| LocalStorageError::InvalidData)?;
    }
    validate_version_links(&connection)
}

fn validate_version_links(connection: &Connection) -> Result<(), LocalStorageError> {
    let invalid:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM agent_access_grant_heads h LEFT JOIN agent_access_grant_versions v ON v.connection_id=h.connection_id AND v.generation=h.active_version_generation WHERE h.active_version_generation IS NOT NULL AND (v.connection_id IS NULL OR h.generation!=v.generation OR h.owner_scope!=v.owner_scope))",[],|r|r.get(0))?;
    if invalid {
        return Err(LocalStorageError::InvalidData);
    }
    // Every persisted effect's metadata must still name its exact version, including revoked
    // and compensated versions; neither a newer head nor a revoked head changes its evidence.
    let invalid:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM agent_access_grant_effects e LEFT JOIN agent_access_grant_versions v ON v.connection_id=e.connection_id AND v.generation=e.after_active_version_generation WHERE e.after_active_version_generation IS NOT NULL AND (v.connection_id IS NULL OR e.after_grant_id IS NOT v.grant_id OR e.after_scope_hash IS NOT v.scope_hash OR e.after_material_sha256 IS NOT v.material_sha256))",[],|r|r.get(0))?;
    if invalid {
        return Err(LocalStorageError::InvalidData);
    }
    Ok(())
}
