//! Current stable-format initialization, ownership and integrity validation.
use super::*;
use hiroute_domain::*;

pub(super) const MARKER_SQL: &str = "CREATE TABLE stable_storage_format(singleton INTEGER PRIMARY KEY CHECK(singleton=1), format_version INTEGER NOT NULL CHECK(format_version=1), converted INTEGER NOT NULL CHECK(converted IN (0,1))); INSERT INTO stable_storage_format VALUES(1,1,1);";

pub(crate) fn acquire_startup_lock(root: &Path) -> Result<std::fs::File, LocalStorageError> {
    use fs2::FileExt;
    let path = root.join("startup.lock");
    #[cfg(windows)]
    let file = {
        use hiroute_platform::windows::security;
        let (expected, created) = match security::open_identity(&path, false) {
            Ok(file) => (security::file_identity(&file)?, false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match security::create_owner_only_file(&path) {
                    Ok(file) => {
                        security::verify_owner_only_handle(&file, false)?;
                        file.sync_all()?;
                        (security::file_identity(&file)?, true)
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (
                        security::file_identity(&security::open_identity(&path, false)?)?,
                        false,
                    ),
                    Err(error) => return Err(error.into()),
                }
            }
            Err(error) => return Err(error.into()),
        };
        let file = security::open_read_write(&path)?;
        if security::file_identity(&file)? != expected {
            return Err(LocalStorageError::Permission);
        }
        if created {
            security::verify_owner_only_handle(&file, false)?;
            hiroute_platform::durable::directory_barrier(root)?;
        }
        file
    };
    #[cfg(not(windows))]
    let file = {
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
        file
    };
    file.try_lock_exclusive().map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            LocalStorageError::Io(error).with_recovery(
                hiroute_host_runtime::StorageRecoveryCause::Locked,
                hiroute_host_runtime::StorageRecoveryAction::RetryCurrent,
            )
        } else {
            LocalStorageError::Io(error)
        }
    })?;
    Ok(file)
}

mod dependencies;

pub(crate) fn validate_current_storage(
    control: &crate::ControlStore,
    runtime: &crate::RuntimeStore,
    secrets: &crate::LocalSecretStore,
) -> Result<(), LocalStorageError> {
    let version: u32 =
        control
            .connection
            .borrow()
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version < 32 {
        return audit_current_storage(control, runtime, secrets);
    }
    dependencies::validate(control, runtime, secrets)
}

pub(crate) fn audit_current_storage(
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
    visit_publications(&connection, |_| {})?;
    // A retained earlier-target batch is validated before the next batch creates newer tables.
    // Current schema still requires the table and all of its reference closure.
    let control_version: u32 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if control_version >= 31 {
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
        drop(defaults);
    }
    // Cross-store grant/publication equality is checked only after Operation recovery.
    // Each store's format and integrity must already be valid before recovery writes.
    let secret_connection = secrets.connection.borrow();
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

/// Validate every retained publication, retaining only one decoded runtime object at a time.
fn visit_publications(
    connection: &rusqlite::Connection,
    mut inspect: impl FnMut(&GatewayPublicationV1),
) -> Result<(), LocalStorageError> {
    let mut statement=connection.prepare("SELECT workspace_id,publication_revision,digest,publication_bytes FROM gateway_publications")?;
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
        inspect(&publication);
    }
    Ok(())
}

/// Active Secret heads may legitimately precede publication compensation on a crash.
/// Run only after journal recovery, before resuming serving admission. Historical revoked
/// grants remain history and never regain admission authority through this check.
pub(crate) fn validate_recovered_grant_publications(
    control: &crate::ControlStore,
    secrets: &crate::LocalSecretStore,
) -> Result<(), LocalStorageError> {
    let version: u32 =
        control
            .connection
            .borrow()
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version >= 32 {
        return dependencies::validate_grant_publications(control, secrets);
    }
    let secret_connection = secrets.connection.borrow();
    let mut statement=secret_connection.prepare("SELECT v.grant_id,v.generation,v.scope_json,v.material_sha256 FROM agent_access_grant_heads h JOIN agent_access_grant_versions v ON v.connection_id=h.connection_id AND v.generation=h.active_version_generation")?;
    let mut unmatched = Vec::new();
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
        unmatched.push((id, generation, scope, material));
    }
    visit_publications(&control.connection.borrow(), |publication| {
        unmatched.retain(|(id, generation, scope, material)| {
            !publication.grants.iter().any(|grant| {
                grant.grant_id == *id
                    && grant.generation == *generation
                    && grant.bearer_token_sha256.as_str() == material
                    && grant.model_grant == *scope.model_grant()
            })
        });
    })?;
    if !unmatched.is_empty() {
        return Err(LocalStorageError::InvalidData);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
