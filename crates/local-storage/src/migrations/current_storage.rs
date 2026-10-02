//! Current stable-format initialization, ownership and integrity validation.
use super::*;
use hiroute_domain::*;

pub(super) const MARKER_SQL: &str = "CREATE TABLE stable_storage_format(singleton INTEGER PRIMARY KEY CHECK(singleton=1), format_version INTEGER NOT NULL CHECK(format_version=1), converted INTEGER NOT NULL CHECK(converted IN (0,1))); INSERT INTO stable_storage_format VALUES(1,1,1);";

pub(crate) fn acquire_startup_lock(root: &Path) -> Result<std::fs::File, LocalStorageError> {
    use fs2::FileExt;
    let path = root.join("startup.lock");
    if path.exists() {
        validate_owner_file(&path)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
        options.custom_flags(
            (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
        );
    }
    let file = options.open(&path)?;
    validate_owner_file(&path)?;
    file.try_lock_exclusive()
        .map_err(|_| LocalStorageError::InvalidData)?;
    Ok(file)
}

pub(crate) fn validate_current_storage(
    control: &crate::ControlStore,
    runtime: &crate::RuntimeStore,
    secrets: &crate::LocalSecretStore,
) -> Result<(), LocalStorageError> {
    crate::secrets::validate_stable_grants(secrets)?;
    let connection = control.connection.borrow();
    let mut statement=connection.prepare("SELECT workspace_id,plan_id,content_revision,content_digest,version_json FROM plan_versions")?;
    for row in statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, u64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
        ))
    })? {
        let (workspace, plan, revision, digest, json) = row?;
        let value: PlanVersionV1 =
            serde_json::from_str(&json).map_err(|_| LocalStorageError::InvalidData)?;
        value
            .validate()
            .map_err(|_| LocalStorageError::InvalidData)?;
        if value.reference.workspace_id.as_str() != workspace
            || value.reference.plan_id.as_str() != plan
            || value.reference.content_revision != revision
            || value.reference.content_digest.as_str() != digest
        {
            return Err(LocalStorageError::InvalidData);
        }
    }
    let invalid:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM plan_version_holds h LEFT JOIN plan_versions v USING(workspace_id,plan_id,content_revision) WHERE v.content_digest IS NULL OR h.content_digest!=v.content_digest)",[],|r|r.get(0))?;
    if invalid {
        return Err(LocalStorageError::InvalidData);
    }
    let invalid: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM plan_heads h LEFT JOIN plan_versions v ON v.workspace_id=h.workspace_id AND v.plan_id=h.plan_id AND v.content_revision=json_extract(h.head_json,'$.reference.content_revision') WHERE v.content_digest IS NULL OR v.content_digest IS NOT json_extract(h.head_json,'$.reference.content_digest'))", [], |r| r.get(0))?;
    if invalid {
        return Err(LocalStorageError::InvalidData);
    }
    let mut statement=connection.prepare("SELECT workspace_id,publication_revision,digest,publication_bytes FROM gateway_publications")?;
    let mut publications = Vec::new();
    for row in statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, u64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Vec<u8>>(3)?,
        ))
    })? {
        let (workspace, revision, digest, bytes) = row?;
        let publication =
            GatewayPublicationV1::decode(&bytes).map_err(|_| LocalStorageError::InvalidData)?;
        if publication.workspace_id.as_str() != workspace
            || publication.publication_revision.get() != revision
            || CanonicalDigest::of_bytes(&bytes).as_str() != digest
        {
            return Err(LocalStorageError::InvalidData);
        }
        publications.push(publication);
    }
    // Publication/secret equality is required only for an active grant version. Historical
    // revoked grants stay in historical publications without regaining admission authority.
    let secret_connection = secrets.connection.borrow();
    let mut statement=secret_connection.prepare("SELECT v.grant_id,v.generation,v.scope_json,v.material_sha256 FROM agent_access_grant_heads h JOIN agent_access_grant_versions v ON v.connection_id=h.connection_id AND v.generation=h.active_version_generation")?;
    for row in statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, u64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })? {
        let (id, generation, json, material) = row?;
        let scope: AgentAccessGrantScopeV1 =
            serde_json::from_str(&json).map_err(|_| LocalStorageError::InvalidData)?;
        if !publications.iter().any(|p| {
            p.grants.iter().any(|g| {
                g.grant_id == id
                    && g.generation == generation
                    && g.bearer_token_sha256.as_str() == material
                    && g.model_grant == *scope.model_grant()
            })
        }) {
            return Err(LocalStorageError::InvalidData);
        }
    }
    // A source can have a prepared grant with no active publication yet. Exact validation of
    // that installation decision remains the existing coordinator's responsibility.
    for store in [
        &*connection,
        &*runtime.connection.borrow(),
        &*secret_connection,
    ] {
        let result: String = store.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if result != "ok" {
            return Err(LocalStorageError::InvalidData);
        }
    }
    Ok(())
}
