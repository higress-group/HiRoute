//! Registration is launcher-only and durable; nonblocking admission never grants authority.
use super::*;
use std::sync::{Arc, Mutex, TryLockError};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplyCapabilityRegistrationOutcome {
    Registered,
    Busy,
}

/// One exact grant received over the daemon launcher's protected inherited channel. The raw
/// capability is zeroized by the domain wrapper and this type is intentionally neither Clone nor
/// serializable, so it cannot become an ambient socket/API credential.
pub struct ApplyCapabilityRegistrationV1 {
    capability: ProtectedApplyCapability,
    authorization: VerifiedApplyAuthorizationV1,
    revisions: RevisionSetV1,
}

impl ApplyCapabilityRegistrationV1 {
    /// Reject stale/invalid lifetime before attempting a busy shared writer.
    /// The admitted transaction repeats this check against SQLite's clock.
    pub fn check_current(&self) -> PortResult<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|value| i64::try_from(value.as_secs()).ok())
            .ok_or_else(|| port(PortErrorCode::Unavailable, "control.capability.clock"))?;
        check_expiry(self.authorization.expires_at_unix(), now)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_protected_launcher(
        capability: String,
        principal: impl Into<String>,
        workspace: WorkspaceId,
        operation_kind: impl Into<String>,
        accepted_digest: CanonicalDigest,
        revisions: RevisionSetV1,
        expires_at_unix: i64,
    ) -> Result<Self, LocalStorageError> {
        let principal = principal.into();
        if principal == "daemon-subscription-maintenance" {
            return Err(LocalStorageError::InvalidData);
        }
        Self::build(
            capability,
            principal,
            workspace,
            operation_kind.into(),
            accepted_digest,
            revisions,
            expires_at_unix,
        )
    }

    pub fn from_subscription_maintenance(
        capability: String,
        workspace: WorkspaceId,
        operation_kind: impl Into<String>,
        accepted_digest: CanonicalDigest,
        revisions: RevisionSetV1,
        expires_at_unix: i64,
    ) -> Result<Self, LocalStorageError> {
        let operation_kind = operation_kind.into();
        if !matches!(
            operation_kind.as_str(),
            "ApplySubscriptionCheck" | "ApplyComputeSave"
        ) {
            return Err(LocalStorageError::InvalidData);
        }
        Self::build(
            capability,
            "daemon-subscription-maintenance".to_owned(),
            workspace,
            operation_kind,
            accepted_digest,
            revisions,
            expires_at_unix,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        capability: String,
        principal: String,
        workspace: WorkspaceId,
        operation_kind: String,
        accepted_digest: CanonicalDigest,
        revisions: RevisionSetV1,
        expires_at_unix: i64,
    ) -> Result<Self, LocalStorageError> {
        let capability = ProtectedApplyCapability::new(capability)
            .map_err(|_| LocalStorageError::InvalidData)?;
        if capability.expose().len() < 32 || expires_at_unix <= 0 {
            return Err(LocalStorageError::InvalidData);
        }
        let revisions_digest =
            CanonicalDigest::of(&revisions).map_err(|_| LocalStorageError::InvalidData)?;
        let authorization = VerifiedApplyAuthorizationV1::from_capability_verifier(
            CanonicalDigest::of_bytes(capability.expose()),
            principal,
            workspace,
            operation_kind,
            accepted_digest,
            revisions_digest,
            "apply:one-shot",
            expires_at_unix,
        )
        .map_err(|_| LocalStorageError::InvalidData)?;
        Ok(Self {
            capability,
            authorization,
            revisions,
        })
    }
}

/// Borrowed registrar available only from the coordinated daemon storage set. It can persist a
/// launcher-verified grant, but cannot mint, clone, serialize, or return a capability.
pub struct ApplyCapabilityRegistrar<'a> {
    control: &'a ControlStore,
}

/// Launcher-only writer for the admitted Control database. Unrelated Runtime/Secrets
/// reads cannot refuse a grant; only this registrar or the actual SQLite writer can.
pub struct LauncherCapabilityRegistrar {
    connection: Mutex<Connection>,
    // Drop the connection before releasing coordinated startup ownership.
    _startup_lock: Arc<crate::migrations::StartupLock>,
}

impl LauncherCapabilityRegistrar {
    pub(crate) fn open(control: &ControlStore) -> Result<Self, LocalStorageError> {
        let startup_lock = control
            .startup_lock
            .clone()
            .ok_or(LocalStorageError::InvalidData)?;
        let current = control.connection.borrow();
        if !current.is_autocommit() {
            return Err(LocalStorageError::InvalidData);
        }
        let uuid = |db: &Connection| -> Result<String, LocalStorageError> {
            Ok(db.query_row(
                "SELECT store_uuid FROM storage_meta WHERE singleton=1",
                [],
                |r| r.get(0),
            )?)
        };
        let expected = uuid(&current)?;
        let schema: u32 = current.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let connection = control.physical_binding.open_peer()?;
        if expected.is_empty()
            || uuid(&connection)? != expected
            || connection.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))?
                != schema
        {
            return Err(LocalStorageError::InvalidData);
        }
        let journal: String = connection.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
        if journal != "wal" {
            return Err(LocalStorageError::InvalidData);
        }
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
        connection.busy_timeout(Duration::ZERO)?;
        Ok(Self {
            connection: Mutex::new(connection),
            _startup_lock: startup_lock,
        })
    }

    pub fn try_register(
        &self,
        registration: ApplyCapabilityRegistrationV1,
    ) -> PortResult<ApplyCapabilityRegistrationOutcome> {
        registration.check_current()?;
        let mut connection = match self.connection.try_lock() {
            Ok(connection) => connection,
            Err(TryLockError::WouldBlock) => return Ok(ApplyCapabilityRegistrationOutcome::Busy),
            Err(TryLockError::Poisoned(_)) => {
                return Err(port(
                    PortErrorCode::Unavailable,
                    "control.capability.registrar",
                ));
            }
        };
        try_register(&mut connection, registration)
    }
}

impl ApplyCapabilityRegistrar<'_> {
    pub(crate) fn new(control: &ControlStore) -> ApplyCapabilityRegistrar<'_> {
        ApplyCapabilityRegistrar { control }
    }

    pub fn register(self, registration: ApplyCapabilityRegistrationV1) -> PortResult<()> {
        let now: i64 = self
            .control
            .connection
            .borrow()
            .query_row("SELECT unixepoch()", [], |row| row.get(0))
            .map_err(|_| port(PortErrorCode::Unavailable, "control.capability.clock"))?;
        check_expiry(registration.authorization.expires_at_unix(), now)?;
        self.control.insert_apply_capability(
            &registration.capability,
            registration.authorization.principal(),
            registration.authorization.workspace_id(),
            registration.authorization.operation_kind(),
            registration.authorization.accepted_digest(),
            &registration.revisions,
            registration.authorization.expires_at_unix(),
        )
    }

    /// Only writer admission may return Busy. Once admitted, any failed write or
    /// commit stays an error, rather than claiming a known unregistered state.
    pub fn try_register(
        self,
        registration: ApplyCapabilityRegistrationV1,
    ) -> PortResult<ApplyCapabilityRegistrationOutcome> {
        let mut connection = self.control.connection.borrow_mut();
        try_register(&mut connection, registration)
    }
}

fn try_register(
    connection: &mut Connection,
    registration: ApplyCapabilityRegistrationV1,
) -> PortResult<ApplyCapabilityRegistrationOutcome> {
    registration.check_current()?;
    let previous_ms: u64 = connection
        .pragma_query_value(None, "busy_timeout", |row| row.get(0))
        .map_err(|_| port(PortErrorCode::Unavailable, "control.capability.busy_policy"))?;
    connection
        .busy_timeout(Duration::ZERO)
        .map_err(|_| port(PortErrorCode::Unavailable, "control.capability.admission"))?;
    let result = (|| {
        let transaction = match connection.transaction_with_behavior(TransactionBehavior::Immediate)
        {
            Ok(transaction) => transaction,
            Err(error)
                if matches!(
                    error.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
                ) =>
            {
                return Ok(ApplyCapabilityRegistrationOutcome::Busy);
            }
            Err(_) => return Err(port(PortErrorCode::Unavailable, "control.capability.begin")),
        };
        let now: i64 = transaction
            .query_row("SELECT unixepoch()", [], |row| row.get(0))
            .map_err(|_| port(PortErrorCode::Unavailable, "control.capability.clock"))?;
        check_expiry(registration.authorization.expires_at_unix(), now)?;
        let capability_digest = CanonicalDigest::of_bytes(registration.capability.expose());
        let revisions_digest = CanonicalDigest::of(&registration.revisions)
            .map_err(|_| port(PortErrorCode::InvalidData, "control.capability.revisions"))?;
        transaction
            .execute(
                INSERT_SQL,
                params![
                    capability_digest.as_str(),
                    registration.authorization.principal(),
                    registration.authorization.workspace_id().as_str(),
                    registration.authorization.operation_kind(),
                    registration.authorization.accepted_digest().as_str(),
                    revisions_digest.as_str(),
                    registration.authorization.expires_at_unix(),
                ],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "control.capability.grant"))?;
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.capability.commit"))?;
        Ok(ApplyCapabilityRegistrationOutcome::Registered)
    })();
    // The transaction has ended before restoring the connection's original
    // policy, including on rejection. No other Control operation inherits it.
    connection
        .busy_timeout(Duration::from_millis(previous_ms))
        .map_err(|_| port(PortErrorCode::Unavailable, "control.capability.busy_policy"))?;
    result
}

fn check_expiry(expires: i64, now: i64) -> PortResult<()> {
    if expires <= now || expires > now.saturating_add(300) {
        return Err(port(
            PortErrorCode::PermissionDenied,
            "control.capability.expiry",
        ));
    }
    Ok(())
}

pub(super) const INSERT_SQL: &str = "INSERT INTO apply_capabilities(
                    capability_digest, principal, workspace_id, operation_kind,
                    accepted_digest, expected_revisions_digest, capability_scope,
                    expires_at, revoked, consumed_operation_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'apply:one-shot', ?7, 0, NULL)";

#[cfg(test)]
mod tests;
