//! Ordinary startup verifies current roots and unfinished work. Historical records
//! remain immutable evidence and are authenticated when used, not replayed en masse.
use super::*;

#[cfg(test)]
thread_local! {
    pub(super) static PUBLICATION_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn validate(
    control: &crate::ControlStore,
    _runtime: &crate::RuntimeStore,
    secrets: &crate::LocalSecretStore,
) -> Result<(), LocalStorageError> {
    control
        .read_stamp()
        .map_err(|_| LocalStorageError::InvalidData)?;
    control
        .validate_native_target_heads()
        .map_err(|_| LocalStorageError::InvalidData)?;
    control
        .artifact_dependencies()
        .map_err(|_| LocalStorageError::InvalidData)?;
    let connection = control.connection.borrow();
    let pending = crate::control::current_dependencies::validate_recovery_members(&connection)
        .map_err(|_| LocalStorageError::InvalidData)?;
    for id in &pending {
        let id = OperationId::parse(id).map_err(|_| LocalStorageError::InvalidData)?;
        let operation = control
            .load_operation(&id)
            .map_err(|_| LocalStorageError::InvalidData)?
            .ok_or(LocalStorageError::InvalidData)?;
        if !control
            .operation_is_current(&operation)
            .map_err(|_| LocalStorageError::InvalidData)?
        {
            return Err(LocalStorageError::InvalidData);
        }
    }
    crate::secrets::validate_current_grants(secrets, &pending)?;
    let mut heads = connection.prepare("SELECT workspace_id,plan_id,head_json FROM plan_heads")?;
    for row in heads.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (workspace, plan, encoded) = row?;
        let head: PlanHeadV1 =
            serde_json::from_str(&encoded).map_err(|_| LocalStorageError::InvalidData)?;
        head.validate()
            .map_err(|_| LocalStorageError::InvalidData)?;
        if head.reference.workspace_id.as_str() != workspace
            || head.reference.plan_id.as_str() != plan
        {
            return Err(LocalStorageError::InvalidData);
        }
    }
    // Validate required definitions, including retained Worker/continuation versions.
    // LEFT JOIN checks below distinguish a missing required object from an empty set.
    let mut versions = connection.prepare("SELECT v.workspace_id,v.plan_id,v.content_revision,v.content_digest,v.version_json
        FROM (SELECT workspace_id,plan_id,json_extract(head_json,'$.reference.content_revision') AS content_revision FROM plan_heads
            UNION SELECT workspace_id,plan_id,content_revision FROM plan_version_holds
            UNION SELECT workspace_id,plan_id,content_revision FROM plan_versions WHERE state='prepared') r
        JOIN plan_versions v USING(workspace_id,plan_id,content_revision)")?;
    for row in versions.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, u64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
        ))
    })? {
        let (workspace, plan, revision, digest, bytes) = row?;
        let version: PlanVersionV1 =
            serde_json::from_str(&bytes).map_err(|_| LocalStorageError::InvalidData)?;
        version
            .validate()
            .map_err(|_| LocalStorageError::InvalidData)?;
        if version.reference.workspace_id.as_str() != workspace
            || version.reference.plan_id.as_str() != plan
            || version.reference.content_revision != revision
            || version.reference.content_digest.as_str() != digest
        {
            return Err(LocalStorageError::InvalidData);
        }
    }
    for sql in [
        "SELECT EXISTS(SELECT 1 FROM plan_version_holds h LEFT JOIN plan_versions v USING(workspace_id,plan_id,content_revision) WHERE v.content_digest IS NULL OR h.content_digest!=v.content_digest)",
        "SELECT EXISTS(SELECT 1 FROM plan_heads h LEFT JOIN plan_versions v ON v.workspace_id=h.workspace_id AND v.plan_id=h.plan_id AND v.content_revision=json_extract(h.head_json,'$.reference.content_revision') WHERE v.content_digest IS NULL OR v.content_digest IS NOT json_extract(h.head_json,'$.reference.content_digest'))",
        "SELECT EXISTS(SELECT 1 FROM plan_versions v LEFT JOIN operation_recovery_members m ON m.operation_id=v.owner_operation_id WHERE v.state='prepared' AND m.operation_id IS NULL)",
        "SELECT EXISTS(SELECT 1 FROM publication_heads h LEFT JOIN gateway_publications p ON p.workspace_id=h.workspace_id AND p.publication_revision=h.active_revision WHERE h.active_revision IS NOT NULL AND (p.state IS NULL OR p.state!='active'))",
        "SELECT EXISTS(SELECT 1 FROM publication_heads h LEFT JOIN gateway_publications p ON p.workspace_id=h.workspace_id AND p.publication_revision=h.lkg_revision WHERE h.lkg_revision IS NOT NULL AND (p.state IS NULL OR p.state!='lkg'))",
        "SELECT EXISTS(SELECT 1 FROM gateway_publications p LEFT JOIN publication_heads h USING(workspace_id) WHERE (p.state='active' AND p.publication_revision IS NOT h.active_revision) OR (p.state='lkg' AND p.publication_revision IS NOT h.lkg_revision))",
    ] {
        if connection.query_row(sql, [], |r| r.get::<_, bool>(0))? {
            return Err(LocalStorageError::InvalidData);
        }
    }
    let mut publications = connection.prepare("SELECT workspace_id,publication_revision,digest,publication_bytes FROM gateway_publications WHERE state IN ('active','prepared','lkg')")?;
    for row in publications.query_map([], publication_row)? {
        verify_publication(row?)?;
    }
    // Current source defaults are management roots, not historical operation payloads.
    let mut defaults = connection.prepare("SELECT d.source_id,d.preferences_json,s.source_json FROM compute_source_defaults d LEFT JOIN compute_management_sources s USING(workspace_id,source_id)")?;
    for row in defaults.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })? {
        let (id, json, source) = row?;
        let source: ComputeManagementSourceV2 =
            serde_json::from_str(&source.ok_or(LocalStorageError::InvalidData)?)
                .map_err(|_| LocalStorageError::InvalidData)?;
        let preferences: SourceDefaultPreferencesV1 =
            serde_json::from_str(&json).map_err(|_| LocalStorageError::InvalidData)?;
        if id != source.source_id {
            return Err(LocalStorageError::InvalidData);
        }
        validate_source_default_preferences(Some(&source), &preferences)
            .map_err(|_| LocalStorageError::InvalidData)?;
    }
    Ok(())
}

type PublicationRow = (String, u64, String, Vec<u8>);
fn publication_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PublicationRow> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
}
fn verify_publication(
    (workspace, revision, digest, bytes): PublicationRow,
) -> Result<GatewayPublicationV1, LocalStorageError> {
    #[cfg(test)]
    PUBLICATION_READS.with(|count| count.set(count.get() + 1));
    PublicationRecordV1::from_parts(
        WorkspaceId::parse(workspace).map_err(|_| LocalStorageError::InvalidData)?,
        GatewayPublicationRevision::new(revision).map_err(|_| LocalStorageError::InvalidData)?,
        CanonicalDigest::parse(digest).map_err(|_| LocalStorageError::InvalidData)?,
        bytes,
    )
    .and_then(|record| record.verify())
    .map_err(|_| LocalStorageError::InvalidData)
}

pub(super) fn validate_grant_publications(
    control: &crate::ControlStore,
    secrets: &crate::LocalSecretStore,
) -> Result<(), LocalStorageError> {
    let db = control.connection.borrow();
    let secret_db = secrets.connection.borrow();
    let mut heads = secret_db.prepare(
        "SELECT v.owner_scope,v.grant_id,v.generation,v.scope_json,v.material_sha256
        FROM agent_access_grant_heads h LEFT JOIN agent_access_grant_versions v
        ON v.connection_id=h.connection_id AND v.generation=h.active_version_generation
        WHERE h.active_version_generation IS NOT NULL",
    )?;
    let mut verified = std::collections::BTreeMap::new();
    let mut current = db.prepare("SELECT workspace_id,publication_revision,digest,publication_bytes FROM gateway_publications WHERE state IN ('active','lkg')")?;
    for row in current.query_map([], publication_row)? {
        let source = row?;
        verified.insert((source.0.clone(), source.1), verify_publication(source)?);
    }
    for row in heads.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, u64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
        ))
    })? {
        let (_owner_scope, id, generation, scope, material) = row?;
        let scope: AgentAccessGrantScopeV1 =
            serde_json::from_str(&scope).map_err(|_| LocalStorageError::InvalidData)?;
        if verified.values().any(|p| {
            p.grants.iter().any(|grant| {
                grant.grant_id == id
                    && grant.generation == generation
                    && grant.bearer_token_sha256.as_str() == material
                    && grant.model_grant == *scope.model_grant()
            })
        }) {
            continue;
        }
        // A proof reference only locates immutable bytes; it never authenticates a grant.
        let source: PublicationRow = db.query_row("SELECT p.workspace_id,p.publication_revision,p.digest,p.publication_bytes
            FROM publication_grant_proofs r JOIN gateway_publications p USING(workspace_id,publication_revision)
            WHERE r.grant_id=?1 AND r.generation=?2 AND r.material_sha256=?3
            ORDER BY r.publication_revision DESC LIMIT 1", params![id,generation,material], publication_row)?;
        let key = (source.0.clone(), source.1);
        if !verified.contains_key(&key) {
            verified.insert(key.clone(), verify_publication(source)?);
        }
        if !verified[&key].grants.iter().any(|grant| {
            grant.grant_id == id
                && grant.generation == generation
                && grant.bearer_token_sha256.as_str() == material
                && grant.model_grant == *scope.model_grant()
        }) {
            return Err(LocalStorageError::InvalidData);
        }
    }
    Ok(())
}
