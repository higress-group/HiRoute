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

/// Current grants and both sides of unfinished effects are authoritative startup
/// dependencies. Retired, settled generations are authenticated when accessed/audited.
pub(crate) fn validate_current(
    store: &LocalSecretStore,
    pending: &[String],
) -> Result<(), LocalStorageError> {
    let db = store.connection.borrow();
    let mut required = std::collections::BTreeSet::new();
    let mut heads = db.prepare("SELECT h.connection_id,h.active_version_generation
        FROM agent_access_grant_heads h LEFT JOIN agent_access_grant_versions v
        ON v.connection_id=h.connection_id AND v.generation=h.active_version_generation
        WHERE h.active_version_generation IS NOT NULL
        AND (v.connection_id IS NULL OR h.generation!=v.generation OR h.owner_scope!=v.owner_scope)")?;
    if heads.query([])?.next()?.is_some() {
        return Err(LocalStorageError::InvalidData);
    }
    let mut heads = db.prepare("SELECT connection_id,active_version_generation FROM agent_access_grant_heads WHERE active_version_generation IS NOT NULL")?;
    for row in heads.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))? {
        required.insert(row?);
    }
    // A partial external write must have a current journal owner. Empty journal
    // membership is not proof that an unactivated Secret effect can be ignored.
    let mut unresolved = db.prepare("SELECT DISTINCT operation_id FROM agent_access_grant_effects WHERE compensated=0 AND activated=0")?;
    for row in unresolved.query_map([], |r| r.get::<_, String>(0))? {
        if !pending.contains(&row?) {
            return Err(LocalStorageError::InvalidData);
        }
    }
    for operation in pending {
        let invalid: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_access_grant_effects e
            LEFT JOIN agent_access_grant_versions v ON v.connection_id=e.connection_id AND v.generation=e.after_active_version_generation
            WHERE e.operation_id=?1 AND e.after_active_version_generation IS NOT NULL
            AND NOT (e.compensated=1 AND e.action IN ('create','rotate') AND e.after_active_version_generation IS NOT e.before_active_version_generation)
            AND (v.connection_id IS NULL OR e.after_grant_id IS NOT v.grant_id OR e.after_scope_hash IS NOT v.scope_hash
                OR e.after_material_sha256 IS NOT v.material_sha256))", [operation], |r| r.get(0))?;
        if invalid {
            return Err(LocalStorageError::InvalidData);
        }
        let mut effects = db.prepare("SELECT connection_id,before_active_version_generation,
            CASE WHEN compensated=1 AND action IN ('create','rotate') AND after_active_version_generation IS NOT before_active_version_generation
                THEN NULL ELSE after_active_version_generation END
            FROM agent_access_grant_effects WHERE operation_id=?1")?;
        for row in effects.query_map([operation], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<u64>>(1)?,
                r.get::<_, Option<u64>>(2)?,
            ))
        })? {
            let (id, before, after) = row?;
            for generation in before.into_iter().chain(after) {
                required.insert((id.clone(), generation));
            }
        }
    }
    for (id, generation) in required {
        let version = read_version(&db, &id, generation)
            .map_err(|_| LocalStorageError::InvalidData)?
            .ok_or(LocalStorageError::InvalidData)?;
        authenticate_version(store, &version).map_err(|_| LocalStorageError::InvalidData)?;
    }
    Ok(())
}

fn validate_version_links(connection: &Connection) -> Result<(), LocalStorageError> {
    let invalid:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM agent_access_grant_heads h LEFT JOIN agent_access_grant_versions v ON v.connection_id=h.connection_id AND v.generation=h.active_version_generation WHERE h.active_version_generation IS NOT NULL AND (v.connection_id IS NULL OR h.generation!=v.generation OR h.owner_scope!=v.owner_scope))",[],|r|r.get(0))?;
    if invalid {
        return Err(LocalStorageError::InvalidData);
    }
    // Compensation deliberately removes a create/rotate's new version and restores the
    // before-head. Its cancelled generation can later be reused; the terminal receipt is
    // not authority for that replacement. All other effects retain exact version links,
    // including revoked grants and compensated reuse of an existing version.
    let invalid: bool = connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM agent_access_grant_effects e
            LEFT JOIN agent_access_grant_versions v
              ON v.connection_id=e.connection_id AND v.generation=e.after_active_version_generation
            WHERE e.after_active_version_generation IS NOT NULL
              AND NOT (e.compensated=1 AND e.action IN ('create','rotate')
                       AND e.after_active_version_generation IS NOT e.before_active_version_generation)
              AND (v.connection_id IS NULL OR e.after_grant_id IS NOT v.grant_id
                   OR e.after_scope_hash IS NOT v.scope_hash
                   OR e.after_material_sha256 IS NOT v.material_sha256))",
        [], |r| r.get(0),
    )?;
    if invalid {
        return Err(LocalStorageError::InvalidData);
    }
    Ok(())
}
