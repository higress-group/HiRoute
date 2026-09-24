use hiroute_diagnostics::publication::{PublicationStage, measure};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use hiroute_domain::{
    BeginOperationOutcome, CanonicalDigest, ChangeSpecV1, CompensationOutcome,
    ControlRepositoryPort, CredentialPoolMutationV1, CredentialRefV1, EffectReconciliation,
    ExternalEffectIntentV1, ExternalEffectPort, IdempotencyScopeV1, OperationId, OperationState,
    OperationStepV1, OperationV1, OwnedEffectKind, OwnedEffectV1, PortError, PortErrorCode,
    PortResult, ProtectedApplyCapability, RevisionMismatch, RevisionSetV1, RuntimeMutationV1,
    SecretFingerprintAlgorithm, SecretMutationKind, SecretMutationV1, TransactionPlanV1,
    VerifiedApplyAuthorizationV1, WorkerDependencySelectionChangeV1, WorkspaceId,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::migrations::{DatabaseKind, open_database, open_database_from_set};
use crate::{DaemonStorageAuthority, LocalStorageError};

#[cfg(test)]
mod agent_transaction_tests;
#[cfg(test)]
mod surface_check_tests;
#[cfg(test)]
mod transaction_v2_tests;

mod agent_check;
#[path = "../agents/collaboration_store.rs"]
mod collaboration_store;
mod compute;
mod journal;
#[path = "../agents/native_artifacts.rs"]
mod native_artifacts;
#[path = "../agents/native_directories.rs"]
mod native_directories;
mod operation_status;
mod plans;
mod prices;
mod worker_dependencies;
use native_directories::CreatedNativeDirectory;
#[cfg(all(test, unix))]
#[path = "../agents/native_file_tests.rs"]
mod native_file_tests;
#[path = "../agents/plan_references.rs"]
mod plan_references;
mod publication;
#[path = "../agents/skill_store.rs"]
mod skill_store;

pub use compute::{
    ComputeSubscriptionValidationRecordV1, ComputeSubscriptionValidationStateV1,
    InventorySnapshotV1,
};

pub struct ControlStore {
    diagnostics: RefCell<hiroute_diagnostics::DiagnosticsPort>,
    connection: RefCell<Connection>,
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
        if registration.authorization.expires_at_unix() <= now
            || registration.authorization.expires_at_unix() > now.saturating_add(300)
        {
            return Err(port(
                PortErrorCode::PermissionDenied,
                "control.capability.expiry",
            ));
        }
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
}

impl ControlStore {
    pub fn set_diagnostics(&self, port: hiroute_diagnostics::DiagnosticsPort) {
        *self.diagnostics.borrow_mut() = port;
    }
    pub(crate) fn open(
        _authority: &DaemonStorageAuthority,
        path: impl AsRef<Path>,
        migration_backup_root: impl AsRef<Path>,
    ) -> Result<Self, LocalStorageError> {
        Ok(Self {
            diagnostics: RefCell::new(Default::default()),
            connection: RefCell::new(open_database(
                _authority,
                path,
                DatabaseKind::Control,
                migration_backup_root,
            )?),
        })
    }

    pub(crate) fn open_from_migration_set(
        authority: &DaemonStorageAuthority,
        path: &Path,
        migration_backup_root: &Path,
        expected_store_uuid: &str,
    ) -> Result<Self, LocalStorageError> {
        Ok(Self {
            diagnostics: RefCell::new(Default::default()),
            connection: RefCell::new(open_database_from_set(
                authority,
                path,
                DatabaseKind::Control,
                migration_backup_root,
                Some(expected_store_uuid),
                None,
            )?),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_connection<T>(&self, function: impl FnOnce(&Connection) -> T) -> T {
        function(&self.connection.borrow())
    }

    /// Read the retained writer owner even when it has reached NeedsAttention.
    pub fn writer_claim_operation(&self) -> PortResult<Option<OperationV1>> {
        load_operation_where(
            &self.diagnostics.borrow(),
            &self.connection.borrow(),
            "operation_id = (SELECT operation_id FROM writer_claim WHERE singleton = 1)",
            [],
        )
    }

    pub fn desired_state(&self, workspace: &WorkspaceId) -> PortResult<Option<Value>> {
        let connection = self.connection.borrow();
        let encoded: Option<String> = connection
            .query_row(
                "SELECT desired_json FROM workspace_state WHERE workspace_id = ?1",
                params![workspace.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.desired.read"))?;
        encoded
            .map(|value| {
                let mut envelope: Value = serde_json::from_str(&value)
                    .map_err(|_| port(PortErrorCode::Corrupt, "control.desired.decode"))?;
                if envelope.get("schema").and_then(Value::as_str)
                    != Some("hiroute.control-desired/v1")
                {
                    return Err(port(PortErrorCode::Corrupt, "control.desired.schema"));
                }
                envelope
                    .as_object_mut()
                    .and_then(|object| object.remove("value"))
                    .ok_or_else(|| port(PortErrorCode::Corrupt, "control.desired.value"))
            })
            .transpose()
    }

    #[cfg(test)]
    pub(crate) fn set_dependency_revision(
        &self,
        workspace: &WorkspaceId,
        key: &str,
        revision: u64,
    ) -> PortResult<()> {
        self.connection
            .borrow()
            .execute(
                "INSERT INTO dependency_revisions(workspace_id, dependency_key, revision)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(workspace_id, dependency_key)
                 DO UPDATE SET revision = excluded.revision",
                params![workspace.as_str(), key, revision],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "control.dependencies.write"))?;
        Ok(())
    }

    pub fn operation_step_rows(&self, operation_id: &OperationId) -> PortResult<usize> {
        self.connection
            .borrow()
            .query_row(
                "SELECT count(*) FROM operation_steps WHERE operation_id = ?1",
                params![operation_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "control.journal.count"))
    }

    /// Reads completed operations as newest-first immutable projections. Callers still validate
    /// the sealed command-specific plan; this does not create a mutable AgentConnection state
    /// store.
    pub fn succeeded_operations_for_kind(
        &self,
        workspace: &WorkspaceId,
        operation_kind: &str,
    ) -> PortResult<Vec<OperationV1>> {
        self.succeeded_operations_for_kinds(workspace, &[operation_kind])
    }

    pub fn succeeded_operations_for_kinds(
        &self,
        workspace: &WorkspaceId,
        operation_kinds: &[&str],
    ) -> PortResult<Vec<OperationV1>> {
        if operation_kinds.is_empty() || operation_kinds.len() > 32 {
            return Err(port(
                PortErrorCode::InvalidData,
                "control.operation.completed.kinds",
            ));
        }
        let placeholders = (0..operation_kinds.len())
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT operation_json FROM operations WHERE workspace_id = ? AND operation_kind IN ({placeholders}) AND state = 'succeeded' ORDER BY rowid DESC"
        );
        let connection = self.connection.borrow();
        let mut statement = connection.prepare(&sql).map_err(|_| {
            port(
                PortErrorCode::Unavailable,
                "control.operation.completed.prepare",
            )
        })?;
        let rows = statement
            .query_map(
                rusqlite::params_from_iter(
                    std::iter::once(workspace.as_str()).chain(operation_kinds.iter().copied()),
                ),
                |row| row.get::<_, String>(0),
            )
            .map_err(|_| {
                port(
                    PortErrorCode::Unavailable,
                    "control.operation.completed.query",
                )
            })?;
        let mut operations = Vec::new();
        for row in rows {
            let encoded =
                row.map_err(|_| port(PortErrorCode::Corrupt, "control.operation.completed.row"))?;
            operations.push(decode_operation(&self.diagnostics.borrow(), &encoded)?);
        }
        Ok(operations)
    }

    #[cfg(test)]
    pub(crate) fn grant_apply_capability(
        &self,
        capability: &str,
        operation: &OperationV1,
        expires_at: i64,
    ) -> PortResult<()> {
        let capability = ProtectedApplyCapability::new(capability.to_owned())
            .map_err(|_| port(PortErrorCode::InvalidData, "control.capability.value"))?;
        self.insert_apply_capability(
            &capability,
            &operation.idempotency.principal,
            &operation.workspace_id,
            &operation.idempotency.operation_kind,
            &operation.accepted_digest,
            &operation.expected_revisions,
            expires_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_apply_capability(
        &self,
        capability: &ProtectedApplyCapability,
        principal: &str,
        workspace: &WorkspaceId,
        operation_kind: &str,
        accepted_digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
        expires_at: i64,
    ) -> PortResult<()> {
        let capability_digest = CanonicalDigest::of_bytes(capability.expose());
        let revisions_digest = CanonicalDigest::of(revisions)
            .map_err(|_| port(PortErrorCode::InvalidData, "control.capability.revisions"))?;
        self.connection
            .borrow()
            .execute(
                "INSERT INTO apply_capabilities(
                    capability_digest, principal, workspace_id, operation_kind,
                    accepted_digest, expected_revisions_digest, capability_scope,
                    expires_at, revoked, consumed_operation_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'apply:one-shot', ?7, 0, NULL)",
                params![
                    capability_digest.as_str(),
                    principal,
                    workspace.as_str(),
                    operation_kind,
                    accepted_digest.as_str(),
                    revisions_digest.as_str(),
                    expires_at,
                ],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "control.capability.grant"))?;
        Ok(())
    }
}

impl ControlStore {
    fn begin_operation_admitted(
        &self,
        operation: &OperationV1,
        authorization: Option<&VerifiedApplyAuthorizationV1>,
    ) -> PortResult<BeginOperationOutcome> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port(PortErrorCode::Unavailable, "control.operation.begin"))?;
        if let Some(existing) = load_operation_where(
            &self.diagnostics.borrow(),
            &transaction,
            "workspace_id = ?1 AND principal = ?2 AND operation_kind = ?3 AND idempotency_key = ?4",
            params![
                operation.workspace_id.as_str(),
                &operation.idempotency.principal,
                &operation.idempotency.operation_kind,
                &operation.idempotency.key
            ],
        )? {
            return if existing.accepted_digest == operation.accepted_digest {
                Ok(BeginOperationOutcome::ExistingSame(Box::new(existing)))
            } else {
                Ok(BeginOperationOutcome::ExistingDifferent)
            };
        }

        // Admission and the final target/dependency revision check share one SQLite transaction.
        if let Some(change) = operation.plan.worker_dependency_selection() {
            let current = Self::worker_dependency_selection_revision_in(
                &transaction,
                &operation.workspace_id,
                change.after_selection.harness,
            )?;
            if current != operation.expected_revisions.target || current != change.before_revision {
                return Ok(BeginOperationOutcome::RevisionChanged(
                    RevisionMismatch::Target,
                ));
            }
            if !operation.expected_revisions.dependencies.is_empty() {
                return Ok(BeginOperationOutcome::RevisionChanged(
                    RevisionMismatch::Dependencies,
                ));
            }
        } else {
            let current = read_revisions(&transaction, &operation.workspace_id)?;
            if current.target != operation.expected_revisions.target {
                return Ok(BeginOperationOutcome::RevisionChanged(
                    RevisionMismatch::Target,
                ));
            }
            if current.dependencies != operation.expected_revisions.dependencies {
                return Ok(BeginOperationOutcome::RevisionChanged(
                    RevisionMismatch::Dependencies,
                ));
            }
        }

        if let Some(authorization) = authorization
            && (!authorization.matches_operation(operation)
                || authorization.scope() != "apply:one-shot")
        {
            return Err(port(
                PortErrorCode::PermissionDenied,
                "control.capability.binding",
            ));
        }
        let claimed = transaction
            .execute(
                "INSERT INTO writer_claim(singleton, operation_id, admitted_at)
                 VALUES (1, ?1, unixepoch())",
                params![operation.operation_id.as_str()],
            )
            .map_err(|_| port(PortErrorCode::Conflict, "control.writer.claim"))?;
        if claimed != 1 {
            return Err(port(PortErrorCode::Conflict, "control.writer.claim"));
        }
        if let Some(authorization) = authorization {
            let consumed = transaction
                .execute(
                    "UPDATE apply_capabilities SET consumed_operation_id = ?2
                     WHERE capability_digest = ?1
                       AND principal = ?3 AND workspace_id = ?4 AND operation_kind = ?5
                       AND accepted_digest = ?6 AND expected_revisions_digest = ?7
                       AND capability_scope = 'apply:one-shot'
                       AND expires_at > unixepoch() AND revoked = 0
                       AND consumed_operation_id IS NULL",
                    params![
                        authorization.capability_digest().as_str(),
                        operation.operation_id.as_str(),
                        authorization.principal(),
                        authorization.workspace_id().as_str(),
                        authorization.operation_kind(),
                        authorization.accepted_digest().as_str(),
                        authorization.expected_revisions_digest().as_str(),
                    ],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "control.capability.consume"))?;
            if consumed != 1 {
                return Err(port(
                    PortErrorCode::PermissionDenied,
                    "control.capability.consumed",
                ));
            }
        }

        let encoded = serde_json::to_string(operation)
            .map_err(|_| port(PortErrorCode::InvalidData, "control.operation.encode"))?;
        transaction
            .execute(
                "INSERT INTO operations(
                    operation_id, workspace_id, principal, operation_kind, idempotency_key,
                    request_digest, accepted_change_digest, state, generation, operation_json,
                    created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, unixepoch(), unixepoch())",
                params![
                    operation.operation_id.as_str(),
                    operation.workspace_id.as_str(),
                    &operation.idempotency.principal,
                    &operation.idempotency.operation_kind,
                    &operation.idempotency.key,
                    operation.request_digest.as_str(),
                    operation.accepted_digest.as_str(),
                    operation.state.as_str(),
                    encoded,
                ],
            )
            .map_err(|_| port(PortErrorCode::Conflict, "control.operation.insert"))?;
        store_steps(&transaction, operation)?;
        compute::begin_save_handoff_in(&transaction, operation)?;
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.operation.commit"))?;
        Ok(BeginOperationOutcome::Created)
    }
}

impl ControlRepositoryPort for ControlStore {
    fn current_revisions(&self, workspace: &WorkspaceId) -> PortResult<RevisionSetV1> {
        read_revisions(&self.connection.borrow(), workspace)
    }

    fn operation_for_idempotency(
        &self,
        workspace: &WorkspaceId,
        scope: &IdempotencyScopeV1,
    ) -> PortResult<Option<OperationV1>> {
        let connection = self.connection.borrow();
        load_operation_where(
            &self.diagnostics.borrow(),
            &connection,
            "workspace_id = ?1 AND principal = ?2 AND operation_kind = ?3 AND idempotency_key = ?4",
            params![
                workspace.as_str(),
                &scope.principal,
                &scope.operation_kind,
                &scope.key
            ],
        )
    }

    fn verify_apply_authorization(
        &self,
        capability: &ProtectedApplyCapability,
        workspace: &WorkspaceId,
        principal: &str,
        operation_kind: &str,
        accepted_digest: &CanonicalDigest,
        expected_revisions: &RevisionSetV1,
    ) -> PortResult<VerifiedApplyAuthorizationV1> {
        let digest = CanonicalDigest::of_bytes(capability.expose());
        let expected_revisions_digest = CanonicalDigest::of(expected_revisions)
            .map_err(|_| port(PortErrorCode::InvalidData, "control.capability.revisions"))?;
        let row = self
            .connection
            .borrow()
            .query_row(
                "SELECT principal, workspace_id, operation_kind, accepted_digest,
                        expected_revisions_digest, capability_scope, expires_at,
                        revoked, consumed_operation_id
                 FROM apply_capabilities WHERE capability_digest = ?1",
                params![digest.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, bool>(7)?,
                        row.get::<_, Option<String>>(8)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.capability.read"))?
            .ok_or_else(|| {
                port(
                    PortErrorCode::PermissionDenied,
                    "control.capability.missing",
                )
            })?;
        let now: i64 = self
            .connection
            .borrow()
            .query_row("SELECT unixepoch()", [], |row| row.get(0))
            .map_err(|_| port(PortErrorCode::Unavailable, "control.capability.clock"))?;
        if row.0 != principal
            || row.1 != workspace.as_str()
            || row.2 != operation_kind
            || row.3 != accepted_digest.as_str()
            || row.4 != expected_revisions_digest.as_str()
            || row.5 != "apply:one-shot"
            || row.6 <= now
            || row.7
            || row.8.is_some()
        {
            return Err(port(
                PortErrorCode::PermissionDenied,
                "control.capability.denied",
            ));
        }
        VerifiedApplyAuthorizationV1::from_capability_verifier(
            digest,
            row.0,
            workspace.clone(),
            row.2,
            accepted_digest.clone(),
            expected_revisions_digest,
            row.5,
            row.6,
        )
        .map_err(|_| port(PortErrorCode::Corrupt, "control.capability.record"))
    }

    fn begin_operation(
        &self,
        operation: &OperationV1,
        authorization: &VerifiedApplyAuthorizationV1,
    ) -> PortResult<BeginOperationOutcome> {
        self.begin_operation_admitted(operation, Some(authorization))
    }

    fn begin_local_operation(&self, operation: &OperationV1) -> PortResult<BeginOperationOutcome> {
        if operation.idempotency.principal != "interactive-user" {
            return Err(port(
                PortErrorCode::PermissionDenied,
                "control.local_operation.denied",
            ));
        }
        self.begin_operation_admitted(operation, None)
    }

    fn operation_is_current(&self, operation: &OperationV1) -> PortResult<bool> {
        measure(
            &self.diagnostics.borrow(),
            PublicationStage::OperationCurrent,
            Some(operation.operation_id.as_str()),
            None,
            || journal::is_current(&self.connection.borrow(), operation),
        )
    }

    fn load_operation(&self, operation_id: &OperationId) -> PortResult<Option<OperationV1>> {
        load_operation_where(
            &self.diagnostics.borrow(),
            &self.connection.borrow(),
            "operation_id = ?1",
            params![operation_id.as_str()],
        )
    }

    fn recoverable_operations(&self) -> PortResult<Vec<OperationV1>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "SELECT operation_json FROM operations
                 WHERE state NOT IN ('succeeded', 'rolled_back', 'needs_attention')
                    OR (state = 'needs_attention' AND operation_id =
                        (SELECT operation_id FROM writer_claim WHERE singleton = 1))
                 ORDER BY created_at, operation_id",
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "control.recovery.prepare"))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| port(PortErrorCode::Unavailable, "control.recovery.query"))?;
        let mut operations = Vec::new();
        for row in rows {
            let encoded = row.map_err(|_| port(PortErrorCode::Corrupt, "control.recovery.row"))?;
            operations.push(decode_operation(&self.diagnostics.borrow(), &encoded)?);
        }
        Ok(operations)
    }

    fn writer_recovery_required(&self) -> PortResult<bool> {
        self.connection
            .borrow()
            .query_row("SELECT EXISTS(SELECT 1 FROM writer_claim)", [], |row| {
                row.get(0)
            })
            .map_err(|_| port(PortErrorCode::Unavailable, "control.writer.recovery"))
    }

    fn save_operation(&self, operation: &mut OperationV1) -> PortResult<()> {
        let id = operation.operation_id.to_string();
        measure(
            &self.diagnostics.borrow(),
            PublicationStage::OperationSave,
            Some(&id),
            None,
            || {
                let mut connection = self.connection.borrow_mut();
                let transaction = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|_| port(PortErrorCode::Unavailable, "control.journal.begin"))?;
                let generation = journal::save(&transaction, operation)?;
                transaction
                    .commit()
                    .map_err(|_| port(PortErrorCode::Unavailable, "control.journal.commit"))?;
                operation.acknowledge_journal_commit(generation);
                Ok(())
            },
        )
    }

    fn worker_dependency_selection_revision(
        &self,
        workspace: &WorkspaceId,
        harness: hiroute_domain::delegation::WorkerHarnessV1,
    ) -> PortResult<u64> {
        Self::worker_dependency_selection_revision_in(&self.connection.borrow(), workspace, harness)
    }

    fn apply_worker_dependency_selection(
        &self,
        operation_id: &OperationId,
        workspace: &WorkspaceId,
        change: &WorkerDependencySelectionChangeV1,
    ) -> PortResult<OwnedEffectV1> {
        self.stage_worker_dependency_selection(operation_id, workspace, change)
    }

    fn apply_control(
        &self,
        operation_id: &OperationId,
        workspace: &WorkspaceId,
        expected_revision: u64,
        desired: &Value,
    ) -> PortResult<OwnedEffectV1> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port(PortErrorCode::Unavailable, "control.effect.begin"))?;
        let existing_effect: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM control_effects WHERE operation_id = ?1)",
                params![operation_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "control.effect.lookup"))?;
        if existing_effect {
            drop(transaction);
            drop(connection);
            return match self.observe_control(operation_id, workspace)? {
                EffectReconciliation::Staged(effect) | EffectReconciliation::Applied(effect) => {
                    Ok(effect)
                }
                EffectReconciliation::Missing => {
                    Err(port(PortErrorCode::Conflict, "control.effect.compensated"))
                }
                EffectReconciliation::OwnershipLost(_) => {
                    Err(port(PortErrorCode::Conflict, "control.effect.ownership"))
                }
            };
        }

        let before = transaction
            .query_row(
                "SELECT desired_json, target_revision, desired_digest, owner_operation_id
                 FROM workspace_state WHERE workspace_id = ?1",
                params![workspace.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.effect.before"))?;
        let current_revision = read_control_head(&transaction, workspace)?;
        if current_revision != expected_revision {
            return Err(port(PortErrorCode::Conflict, "control.effect.revision"));
        }
        compute::management::stage_control(&transaction, workspace, operation_id, desired)?;

        let desired_json = serde_json::to_string(&json!({
            "schema": "hiroute.control-desired/v1",
            "value": desired,
        }))
        .map_err(|_| port(PortErrorCode::InvalidData, "control.effect.encode"))?;
        let after_digest = CanonicalDigest::of(desired)
            .map_err(|_| port(PortErrorCode::InvalidData, "control.effect.digest"))?;
        let after_revision = expected_revision
            .checked_add(1)
            .ok_or_else(|| port(PortErrorCode::Conflict, "control.effect.revision_overflow"))?;
        transaction
            .execute(
                "INSERT INTO control_effects(
                    operation_id, workspace_id, before_exists, before_json, before_revision,
                    before_digest, before_owner_operation_id, after_revision, after_digest,
                    compensated, staged_json, activated
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, 0)",
                params![
                    operation_id.as_str(),
                    workspace.as_str(),
                    i64::from(before.is_some()),
                    before.as_ref().map(|value| value.0.as_str()),
                    current_revision,
                    before.as_ref().map(|value| value.2.as_str()),
                    before.as_ref().and_then(|value| value.3.as_deref()),
                    after_revision,
                    after_digest.as_str(),
                    desired_json,
                ],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "control.effect.journal"))?;
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.effect.commit"))?;
        Ok(control_effect(
            operation_id,
            workspace,
            before.as_ref().and_then(|value| parse_digest(&value.2)),
            after_digest,
            after_revision,
        ))
    }

    fn observe_control(
        &self,
        operation_id: &OperationId,
        workspace: &WorkspaceId,
    ) -> PortResult<EffectReconciliation> {
        if let Some(selection) =
            self.observe_worker_dependency_selection(operation_id, workspace)?
        {
            return Ok(selection);
        }
        let connection = self.connection.borrow();
        let effect = connection
            .query_row(
                "SELECT before_digest, after_revision, after_digest, compensated,
                        staged_json, activated, compute_pool_id, compute_pool_after_json,
                        compute_source_id, compute_source_after_json
                 FROM control_effects WHERE operation_id = ?1 AND workspace_id = ?2",
                params![operation_id.as_str(), workspace.as_str()],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, bool>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, bool>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.effect.observe"))?;
        let Some((
            before_digest,
            after_revision,
            after_digest,
            compensated,
            staged_json,
            activated,
            compute_pool_id,
            compute_pool_after_json,
            compute_source_id,
            compute_source_after_json,
        )) = effect
        else {
            return Ok(EffectReconciliation::Missing);
        };
        if compensated {
            return Ok(EffectReconciliation::Missing);
        }
        let after = CanonicalDigest::parse(after_digest.clone())
            .map_err(|_| port(PortErrorCode::Corrupt, "control.effect.digest"))?;
        let owned = control_effect(
            operation_id,
            workspace,
            before_digest.as_deref().and_then(parse_digest),
            after,
            after_revision,
        );
        if !activated {
            let staged_json =
                staged_json.ok_or_else(|| port(PortErrorCode::Corrupt, "control.effect.stage"))?;
            let mut staged: Value = serde_json::from_str(&staged_json)
                .map_err(|_| port(PortErrorCode::Corrupt, "control.effect.stage"))?;
            let staged_value = staged
                .as_object_mut()
                .and_then(|object| object.remove("value"))
                .ok_or_else(|| port(PortErrorCode::Corrupt, "control.effect.stage"))?;
            let staged_digest = CanonicalDigest::of(&staged_value)
                .map_err(|_| port(PortErrorCode::Corrupt, "control.effect.stage_digest"))?;
            if staged_digest.as_str() != after_digest {
                return Err(port(PortErrorCode::Corrupt, "control.effect.stage_digest"));
            }
            return Ok(EffectReconciliation::Staged(owned));
        }
        let current = connection
            .query_row(
                "SELECT target_revision, desired_digest, owner_operation_id
                 FROM workspace_state WHERE workspace_id = ?1",
                params![workspace.as_str()],
                |row| {
                    Ok((
                        row.get::<_, u64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.effect.current"))?;
        let workspace_owned = current.as_ref().is_some_and(|current| {
            current.0 == after_revision
                && current.1 == after_digest
                && current.2.as_deref() == Some(operation_id.as_str())
        });
        let pool_owned = match (compute_pool_id, compute_pool_after_json) {
            (None, None) => true,
            (Some(pool_id), Some(after_json)) => connection
                .query_row(
                    "SELECT pool_json FROM credential_pools WHERE pool_id=?1 AND active=1",
                    params![pool_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| port(PortErrorCode::Unavailable, "control.effect.pool"))?
                .is_some_and(|current| current == after_json),
            _ => return Err(port(PortErrorCode::Corrupt, "control.effect.pool_stage")),
        };
        let source_owned = match (compute_source_id, compute_source_after_json) {
            (None, None) => true,
            (Some(source_id), Some(after_json))
                if compute::projection::is_projection_state(&after_json) =>
            {
                compute::projection::current_matches(&connection, &source_id, &after_json)?
            }
            (Some(source_id), Some(after_json)) => connection
                .query_row(
                    "SELECT source_json FROM compute_sources WHERE source_id=?1",
                    params![source_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| port(PortErrorCode::Unavailable, "control.effect.source"))?
                .is_some_and(|current| current == after_json),
            _ => return Err(port(PortErrorCode::Corrupt, "control.effect.source_stage")),
        };
        let management_owned = compute::management::current_matches(
            &connection,
            workspace,
            staged_json.as_deref(),
            operation_id,
        )?;
        if workspace_owned && pool_owned && source_owned && management_owned {
            Ok(EffectReconciliation::Applied(owned))
        } else {
            Ok(EffectReconciliation::OwnershipLost(owned))
        }
    }

    fn activate_control(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1> {
        if let Some(selection) = self.activate_worker_dependency_selection(effect)? {
            return Ok(selection);
        }
        if effect.kind != OwnedEffectKind::Control {
            return Err(port(PortErrorCode::InvalidData, "control.activate.kind"));
        }
        let operation_id = effect
            .compensation
            .get("operation_id")
            .and_then(Value::as_str)
            .ok_or_else(|| port(PortErrorCode::InvalidData, "control.activate.metadata"))?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port(PortErrorCode::Unavailable, "control.activate.begin"))?;
        let record = transaction
            .query_row(
                "SELECT workspace_id, before_revision, after_revision, after_digest,
                        staged_json, activated, compensated, compute_pool_id,
                        compute_pool_expected_revision, compute_pool_after_json,
                        compute_source_id, compute_source_expected_revision,
                        compute_source_before_json, compute_source_after_json
                 FROM control_effects WHERE operation_id = ?1",
                params![operation_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, bool>(5)?,
                        row.get::<_, bool>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<u64>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, Option<u64>>(11)?,
                        row.get::<_, Option<String>>(12)?,
                        row.get::<_, Option<String>>(13)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.activate.lookup"))?
            .ok_or_else(|| port(PortErrorCode::NotFound, "control.activate.effect"))?;
        if record.6 {
            return Err(port(
                PortErrorCode::Conflict,
                "control.activate.compensated",
            ));
        }
        if record.5 {
            return Ok(effect.clone());
        }
        let workspace = WorkspaceId::parse(record.0.clone())
            .map_err(|_| port(PortErrorCode::Corrupt, "control.activate.workspace"))?;
        if read_control_head(&transaction, &workspace)? != record.1 {
            return Err(port(PortErrorCode::Conflict, "control.activate.revision"));
        }
        let staged = record
            .4
            .ok_or_else(|| port(PortErrorCode::Corrupt, "control.activate.stage"))?;
        match (&record.7, record.8, &record.9) {
            (None, None, None) => {}
            (Some(pool_id), Some(expected_revision), Some(after_json)) => {
                let current_revision = transaction
                    .query_row(
                        "SELECT revision FROM credential_pools WHERE pool_id=?1 AND active=1",
                        params![pool_id],
                        |row| row.get::<_, u64>(0),
                    )
                    .optional()
                    .map_err(|_| port(PortErrorCode::Unavailable, "control.activate.pool"))?
                    .unwrap_or(0);
                let pool: hiroute_domain::CredentialPoolV1 = serde_json::from_str(after_json)
                    .map_err(|_| port(PortErrorCode::Corrupt, "control.activate.pool_stage"))?;
                if current_revision != expected_revision
                    || pool.pool_id != *pool_id
                    || pool.revision != expected_revision.saturating_add(1)
                {
                    return Err(port(PortErrorCode::Conflict, "control.activate.pool_cas"));
                }
                compute::write_credential_pool_in(&transaction, &pool)?;
            }
            _ => {
                return Err(port(
                    PortErrorCode::Corrupt,
                    "control.activate.pool_metadata",
                ));
            }
        }
        if record.7.is_some() && record.10.is_some() {
            return Err(port(
                PortErrorCode::Corrupt,
                "control.activate.compute_metadata",
            ));
        }
        match (&record.10, record.11, &record.12, &record.13) {
            (None, None, None, None) => {}
            (Some(source_id), Some(expected_revision), before_json, Some(after_json)) => {
                if compute::projection::is_projection_state(after_json) {
                    compute::projection::activate(
                        &transaction,
                        source_id,
                        expected_revision,
                        before_json.as_deref(),
                        after_json,
                    )?;
                } else {
                    let current_json = transaction
                        .query_row(
                            "SELECT source_json FROM compute_sources WHERE source_id=?1",
                            params![source_id],
                            |row| row.get::<_, String>(0),
                        )
                        .optional()
                        .map_err(|_| port(PortErrorCode::Unavailable, "control.activate.source"))?;
                    if current_json.as_deref() != before_json.as_deref() {
                        return Err(port(PortErrorCode::Conflict, "control.activate.source_cas"));
                    }
                    let source: hiroute_domain::ComputeSourceV1 = serde_json::from_str(after_json)
                        .map_err(|_| {
                            port(PortErrorCode::Corrupt, "control.activate.source_stage")
                        })?;
                    let desired_revision = expected_revision.checked_add(1).ok_or_else(|| {
                        port(
                            PortErrorCode::Conflict,
                            "control.activate.source_revision_overflow",
                        )
                    })?;
                    if source.source_id != *source_id || source.revision != desired_revision {
                        return Err(port(PortErrorCode::Conflict, "control.activate.source_cas"));
                    }
                    compute::write_compute_source_in(&transaction, &source)?;
                }
            }
            _ => {
                return Err(port(
                    PortErrorCode::Corrupt,
                    "control.activate.source_metadata",
                ));
            }
        }
        compute::management::activate(&transaction, &workspace, &staged, operation_id)?;
        prices::activate(&transaction, &workspace, &staged)?;
        transaction
            .execute(
                "INSERT INTO workspace_state(
                    workspace_id, desired_json, target_revision, desired_digest,
                    owner_operation_id, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, unixepoch())
                 ON CONFLICT(workspace_id) DO UPDATE SET
                    desired_json = excluded.desired_json,
                    target_revision = excluded.target_revision,
                    desired_digest = excluded.desired_digest,
                    owner_operation_id = excluded.owner_operation_id,
                    updated_at = excluded.updated_at",
                params![&record.0, staged, record.2, &record.3, operation_id],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "control.activate.write"))?;
        transaction
            .execute(
                "INSERT INTO workspace_revision_heads(workspace_id, revision) VALUES (?1, ?2)
                 ON CONFLICT(workspace_id) DO UPDATE SET revision = excluded.revision",
                params![&record.0, record.2],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "control.activate.head"))?;
        transaction
            .execute(
                "UPDATE control_effects SET activated = 1 WHERE operation_id = ?1",
                params![operation_id],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "control.activate.mark"))?;
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.activate.commit"))?;
        Ok(effect.clone())
    }

    fn compensate_control(&self, effect: &OwnedEffectV1) -> PortResult<CompensationOutcome> {
        if let Some(selection) = self.compensate_worker_dependency_selection(effect)? {
            return Ok(selection);
        }
        if effect.kind != OwnedEffectKind::Control {
            return Err(port(PortErrorCode::InvalidData, "control.compensate.kind"));
        }
        let operation_id = effect
            .compensation
            .get("operation_id")
            .and_then(Value::as_str)
            .ok_or_else(|| port(PortErrorCode::InvalidData, "control.compensate.metadata"))?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port(PortErrorCode::Unavailable, "control.compensate.begin"))?;
        let record = transaction
            .query_row(
                "SELECT workspace_id, before_exists, before_json, before_revision,
                        before_digest, before_owner_operation_id, after_revision,
                        after_digest, compensated, activated, compute_pool_id,
                        compute_pool_before_json, compute_pool_after_json,
                        compute_source_id, compute_source_before_json, compute_source_after_json
                 FROM control_effects WHERE operation_id = ?1",
                params![operation_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, bool>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, u64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, u64>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, bool>(8)?,
                        row.get::<_, bool>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, Option<String>>(11)?,
                        row.get::<_, Option<String>>(12)?,
                        row.get::<_, Option<String>>(13)?,
                        row.get::<_, Option<String>>(14)?,
                        row.get::<_, Option<String>>(15)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.compensate.lookup"))?
            .ok_or_else(|| port(PortErrorCode::NotFound, "control.compensate.effect"))?;
        if record.8 {
            return Ok(CompensationOutcome::AlreadyCompensated);
        }
        if !record.9 {
            transaction
                .execute(
                    "UPDATE control_effects SET compensated = 1 WHERE operation_id = ?1",
                    params![operation_id],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "control.compensate.stage"))?;
            transaction
                .commit()
                .map_err(|_| port(PortErrorCode::Unavailable, "control.compensate.commit"))?;
            return Ok(CompensationOutcome::Compensated);
        }
        let current = transaction
            .query_row(
                "SELECT target_revision, desired_digest, owner_operation_id
                 FROM workspace_state WHERE workspace_id = ?1",
                params![&record.0],
                |row| {
                    Ok((
                        row.get::<_, u64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.compensate.current"))?;
        if !current.as_ref().is_some_and(|current| {
            current.0 == record.6
                && current.1 == record.7
                && current.2.as_deref() == Some(operation_id)
        }) {
            return Ok(CompensationOutcome::OwnershipLost);
        }
        let workspace = WorkspaceId::parse(record.0.clone())
            .map_err(|_| port(PortErrorCode::Corrupt, "control.compensate.workspace"))?;
        if !compute::management::compensate(&transaction, &workspace, operation_id)? {
            return Ok(CompensationOutcome::OwnershipLost);
        }
        if !prices::compensate(&transaction, operation_id)? {
            return Ok(CompensationOutcome::OwnershipLost);
        }
        match (&record.10, &record.11, &record.12) {
            (None, None, None) => {}
            (Some(pool_id), before_json, Some(after_json)) => {
                let current_pool = transaction
                    .query_row(
                        "SELECT pool_json FROM credential_pools WHERE pool_id=?1 AND active=1",
                        params![pool_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(|_| port(PortErrorCode::Unavailable, "control.compensate.pool"))?;
                if current_pool.as_deref() != Some(after_json.as_str()) {
                    return Ok(CompensationOutcome::OwnershipLost);
                }
                if let Some(before_json) = before_json {
                    let pool: hiroute_domain::CredentialPoolV1 = serde_json::from_str(before_json)
                        .map_err(|_| {
                            port(PortErrorCode::Corrupt, "control.compensate.pool_before")
                        })?;
                    if &pool.pool_id != pool_id {
                        return Err(port(
                            PortErrorCode::Corrupt,
                            "control.compensate.pool_identity",
                        ));
                    }
                    compute::write_credential_pool_in(&transaction, &pool)?;
                } else {
                    transaction
                        .execute(
                            "DELETE FROM credential_pools WHERE pool_id=?1",
                            params![pool_id],
                        )
                        .map_err(|_| {
                            port(PortErrorCode::Unavailable, "control.compensate.pool_delete")
                        })?;
                }
            }
            _ => {
                return Err(port(
                    PortErrorCode::Corrupt,
                    "control.compensate.pool_metadata",
                ));
            }
        }
        if record.10.is_some() && record.13.is_some() {
            return Err(port(
                PortErrorCode::Corrupt,
                "control.compensate.compute_metadata",
            ));
        }
        match (&record.13, &record.14, &record.15) {
            (None, None, None) => {}
            (Some(source_id), before_json, Some(after_json)) => {
                if compute::projection::is_projection_state(after_json) {
                    if !compute::projection::compensate(
                        &transaction,
                        source_id,
                        before_json.as_deref(),
                        after_json,
                    )? {
                        return Ok(CompensationOutcome::OwnershipLost);
                    }
                } else {
                    let current_source = transaction
                        .query_row(
                            "SELECT source_json FROM compute_sources WHERE source_id=?1",
                            params![source_id],
                            |row| row.get::<_, String>(0),
                        )
                        .optional()
                        .map_err(|_| {
                            port(PortErrorCode::Unavailable, "control.compensate.source")
                        })?;
                    if current_source.as_deref() != Some(after_json.as_str()) {
                        return Ok(CompensationOutcome::OwnershipLost);
                    }
                    if let Some(before_json) = before_json {
                        let source: hiroute_domain::ComputeSourceV1 =
                            serde_json::from_str(before_json).map_err(|_| {
                                port(PortErrorCode::Corrupt, "control.compensate.source_before")
                            })?;
                        if &source.source_id != source_id {
                            return Err(port(
                                PortErrorCode::Corrupt,
                                "control.compensate.source_identity",
                            ));
                        }
                        compute::write_compute_source_in(&transaction, &source)?;
                    } else {
                        transaction
                            .execute(
                                "DELETE FROM compute_sources WHERE source_id=?1",
                                params![source_id],
                            )
                            .map_err(|_| {
                                port(
                                    PortErrorCode::Unavailable,
                                    "control.compensate.source_delete",
                                )
                            })?;
                    }
                }
            }
            _ => {
                return Err(port(
                    PortErrorCode::Corrupt,
                    "control.compensate.source_metadata",
                ));
            }
        }
        let restored_revision = record.6.checked_add(1).ok_or_else(|| {
            port(
                PortErrorCode::Conflict,
                "control.compensate.revision_overflow",
            )
        })?;
        if record.1 {
            transaction
                .execute(
                    "UPDATE workspace_state SET desired_json = ?2, target_revision = ?3,
                        desired_digest = ?4, owner_operation_id = ?5, updated_at = unixepoch()
                     WHERE workspace_id = ?1",
                    params![
                        &record.0,
                        record.2,
                        restored_revision,
                        record.4,
                        operation_id
                    ],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "control.compensate.restore"))?;
        } else {
            transaction
                .execute(
                    "DELETE FROM workspace_state WHERE workspace_id = ?1",
                    params![&record.0],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "control.compensate.delete"))?;
        }
        transaction
            .execute(
                "INSERT INTO workspace_revision_heads(workspace_id, revision) VALUES (?1, ?2)
                 ON CONFLICT(workspace_id) DO UPDATE SET revision = excluded.revision",
                params![&record.0, restored_revision],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "control.compensate.head"))?;
        transaction
            .execute(
                "UPDATE control_effects SET compensated = 1 WHERE operation_id = ?1",
                params![operation_id],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "control.compensate.mark"))?;
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.compensate.commit"))?;
        Ok(CompensationOutcome::Compensated)
    }

    fn finish_operation(&self, operation: &mut OperationV1) -> PortResult<u64> {
        if !operation.state.is_terminal() {
            return Err(port(
                PortErrorCode::InvalidData,
                "control.finish.non_terminal",
            ));
        }
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port(PortErrorCode::Unavailable, "control.finish.begin"))?;
        plans::finish_draft_operation(&transaction, operation)?;
        compute::finish_save_handoff_in(&transaction, operation)?;
        let generation = journal::save(&transaction, operation)?;
        if operation.state != OperationState::NeedsAttention {
            let released = transaction
                .execute(
                    "DELETE FROM writer_claim WHERE singleton = 1 AND operation_id = ?1",
                    params![operation.operation_id.as_str()],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "control.finish.release"))?;
            if released != 1 {
                return Err(port(PortErrorCode::Conflict, "control.finish.claim"));
            }
        }
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.finish.commit"))?;
        operation.acknowledge_journal_commit(generation);
        read_control_head(&connection, &operation.workspace_id)
    }

    fn save_operation_tail(&self, operation: &mut OperationV1) -> PortResult<()> {
        let id = operation.operation_id.to_string();
        measure(
            &self.diagnostics.borrow(),
            PublicationStage::OperationSaveTail,
            Some(&id),
            None,
            || {
                if operation.state.is_terminal() {
                    return Err(port(PortErrorCode::InvalidData, "control.tail.terminal"));
                }
                let mut connection = self.connection.borrow_mut();
                let transaction = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|_| port(PortErrorCode::Unavailable, "control.tail.begin"))?;
                let generation = journal::save(&transaction, operation)?;
                transaction
                    .execute(
                        "DELETE FROM writer_claim WHERE singleton = 1 AND operation_id = ?1",
                        params![operation.operation_id.as_str()],
                    )
                    .map_err(|_| port(PortErrorCode::Unavailable, "control.tail.release"))?;
                let held: bool = transaction
                    .query_row("SELECT EXISTS(SELECT 1 FROM writer_claim)", [], |row| {
                        row.get(0)
                    })
                    .map_err(|_| port(PortErrorCode::Unavailable, "control.tail.held"))?;
                if held {
                    // The journal save above is not committed: another operation's claim stays intact.
                    return Err(port(PortErrorCode::Conflict, "control.tail.claim"));
                }
                transaction
                    .commit()
                    .map_err(|_| port(PortErrorCode::Unavailable, "control.tail.commit"))?;
                operation.acknowledge_journal_commit(generation);
                Ok(())
            },
        )
    }

    fn reclaim_operation_writer(&self, operation_id: &OperationId) -> PortResult<()> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port(PortErrorCode::Unavailable, "control.tail.reclaim.begin"))?;
        let holder: Option<String> = transaction
            .query_row(
                "SELECT operation_id FROM writer_claim WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.tail.reclaim.read"))?;
        if holder
            .as_deref()
            .is_some_and(|holder| holder != operation_id.as_str())
        {
            return Err(port(PortErrorCode::Conflict, "control.tail.reclaim.busy"));
        }
        if holder.is_none() {
            transaction
                .execute(
                    "INSERT INTO writer_claim(singleton, operation_id, admitted_at)
                     VALUES (1, ?1, unixepoch())",
                    params![operation_id.as_str()],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "control.tail.reclaim.insert"))?;
        }
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "control.tail.reclaim.commit"))?;
        Ok(())
    }

    fn save_agent_surface_check(
        &self,
        workspace: &WorkspaceId,
        record: &hiroute_domain::AgentSurfaceCheckRecordV1,
    ) -> PortResult<bool> {
        record
            .validate()
            .map_err(|_| port(PortErrorCode::InvalidData, "surface-check.record"))?;
        let surface = serde_json::to_value(record.surface)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(|| port(PortErrorCode::InvalidData, "surface-check.surface"))?;
        let encoded = serde_json::to_string(record)
            .map_err(|_| port(PortErrorCode::InvalidData, "surface-check.encode"))?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port(PortErrorCode::Unavailable, "surface-check.begin"))?;
        let active: Option<u64> = transaction
            .query_row(
                "SELECT publication_revision FROM gateway_publications
                 WHERE workspace_id = ?1 AND state = 'active'",
                params![workspace.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "surface-check.active"))?;
        if active != Some(record.applied_revision.get()) {
            // A newer publication already replaced the checked revision: the stale result is
            // dropped instead of polluting the new release's verification state.
            return Ok(false);
        }
        transaction
            .execute(
                "INSERT INTO agent_surface_checks
                    (workspace_id, context_id, surface, applied_revision, record_json, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, unixepoch())
                 ON CONFLICT(workspace_id, context_id, surface) DO UPDATE SET
                    applied_revision = excluded.applied_revision,
                    record_json = excluded.record_json,
                    updated_at = excluded.updated_at",
                params![
                    workspace.as_str(),
                    record.context_id,
                    surface,
                    record.applied_revision.get(),
                    encoded
                ],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "surface-check.write"))?;
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "surface-check.commit"))?;
        Ok(true)
    }

    fn agent_surface_checks(
        &self,
        workspace: &WorkspaceId,
        context_id: &str,
    ) -> PortResult<Vec<hiroute_domain::AgentSurfaceCheckRecordV1>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "SELECT record_json FROM agent_surface_checks
                 WHERE workspace_id = ?1 AND context_id = ?2 ORDER BY surface",
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "surface-check.prepare"))?;
        let rows = statement
            .query_map(params![workspace.as_str(), context_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|_| port(PortErrorCode::Unavailable, "surface-check.query"))?;
        let mut records = Vec::new();
        for row in rows {
            let encoded = row.map_err(|_| port(PortErrorCode::Unavailable, "surface-check.row"))?;
            let record: hiroute_domain::AgentSurfaceCheckRecordV1 = serde_json::from_str(&encoded)
                .map_err(|_| port(PortErrorCode::Corrupt, "surface-check.decode"))?;
            record
                .validate()
                .map_err(|_| port(PortErrorCode::Corrupt, "surface-check.record"))?;
            records.push(record);
        }
        Ok(records)
    }
}

fn read_revisions(connection: &Connection, workspace: &WorkspaceId) -> PortResult<RevisionSetV1> {
    let target = read_control_head(connection, workspace)?;
    let mut statement = connection
        .prepare(
            "SELECT dependency_key, revision FROM dependency_revisions
             WHERE workspace_id = ?1 ORDER BY dependency_key",
        )
        .map_err(|_| port(PortErrorCode::Unavailable, "control.revisions.prepare"))?;
    let rows = statement
        .query_map(params![workspace.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })
        .map_err(|_| port(PortErrorCode::Unavailable, "control.revisions.query"))?;
    let mut dependencies = BTreeMap::new();
    for row in rows {
        let (key, revision) =
            row.map_err(|_| port(PortErrorCode::Corrupt, "control.revisions.row"))?;
        dependencies.insert(key, revision);
    }
    Ok(RevisionSetV1 {
        target,
        dependencies,
    })
}

fn read_control_head(connection: &Connection, workspace: &WorkspaceId) -> PortResult<u64> {
    connection
        .query_row(
            "SELECT revision FROM workspace_revision_heads WHERE workspace_id = ?1",
            params![workspace.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map(|revision| revision.unwrap_or(0))
        .map_err(|_| port(PortErrorCode::Unavailable, "control.revisions.target"))
}

fn store_steps(transaction: &Transaction<'_>, operation: &OperationV1) -> PortResult<()> {
    for step in &operation.steps {
        store_step(transaction, operation, step)?;
    }
    Ok(())
}

fn store_step(
    transaction: &Transaction<'_>,
    operation: &OperationV1,
    step: &OperationStepV1,
) -> PortResult<()> {
    let encoded = serde_json::to_string(&json!({
        "schema": "hiroute.operation-step/v1",
        "step": step,
    }))
    .map_err(|_| port(PortErrorCode::InvalidData, "control.step.encode"))?;
    transaction
        .execute(
            "INSERT INTO operation_steps(operation_id, step_no, step_kind, state, step_json)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(operation_id, step_no) DO UPDATE SET
                step_kind = excluded.step_kind,
                state = excluded.state,
                step_json = excluded.step_json",
            params![
                operation.operation_id.as_str(),
                step.sequence,
                serde_json::to_value(step.kind)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "invalid".to_owned()),
                serde_json::to_value(step.status)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "invalid".to_owned()),
                encoded,
            ],
        )
        .map_err(|_| port(PortErrorCode::Unavailable, "control.step.write"))?;
    Ok(())
}

fn load_operation_where<P: rusqlite::Params>(
    diagnostics: &hiroute_diagnostics::DiagnosticsPort,
    connection: &Connection,
    predicate: &str,
    parameters: P,
) -> PortResult<Option<OperationV1>> {
    let sql = format!("SELECT operation_json FROM operations WHERE {predicate}");
    let encoded: Option<String> = measure(
        diagnostics,
        PublicationStage::OperationRead,
        None,
        None,
        || {
            connection
                .query_row(&sql, parameters, |row| row.get(0))
                .optional()
                .map_err(|_| port(PortErrorCode::Unavailable, "control.operation.read"))
        },
    )?;
    encoded
        .map(|encoded| decode_operation(diagnostics, &encoded))
        .transpose()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableSecretMutation {
    kind: SecretMutationKind,
    credential: CredentialRefV1,
    expected_generation: u64,
    #[serde(default)]
    fingerprint_algorithm: SecretFingerprintAlgorithm,
    input_slot: Option<String>,
    fingerprint: Option<CanonicalDigest>,
    new_allowed_destinations: Option<BTreeSet<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableRuntimeMutation {
    key: String,
    value: Value,
    expected_generation: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableExternalIntent {
    effect_id: String,
    kind: OwnedEffectKind,
    target: String,
    before_fingerprint: Option<CanonicalDigest>,
    desired: Value,
    #[serde(default = "default_artifact_mode")]
    desired_mode: u32,
    #[serde(default)]
    sensitive: bool,
}

const fn default_artifact_mode() -> u32 {
    0o644
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurablePlan {
    spec: ChangeSpecV1,
    control: Value,
    #[serde(default)]
    credential_pool: Option<CredentialPoolMutationV1>,
    #[serde(default)]
    worker_dependency_selection: Option<WorkerDependencySelectionChangeV1>,
    #[serde(default)]
    secrets: Vec<DurableSecretMutation>,
    #[serde(default)]
    runtime: Vec<DurableRuntimeMutation>,
    #[serde(default)]
    external: Vec<DurableExternalIntent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableOperation {
    schema_version: u16,
    operation_id: OperationId,
    workspace_id: WorkspaceId,
    idempotency: IdempotencyScopeV1,
    request_digest: CanonicalDigest,
    accepted_digest: CanonicalDigest,
    expected_revisions: RevisionSetV1,
    plan: DurablePlan,
    state: OperationState,
    generation: u64,
    steps: Vec<OperationStepV1>,
    safe_error_code: Option<String>,
}

fn decode_operation(
    diagnostics: &hiroute_diagnostics::DiagnosticsPort,
    encoded: &str,
) -> PortResult<OperationV1> {
    measure(
        diagnostics,
        PublicationStage::OperationDecode,
        None,
        Some(encoded.len()),
        || {
            let durable: DurableOperation = measure(
                diagnostics,
                PublicationStage::OperationJsonParse,
                None,
                Some(encoded.len()),
                || {
                    serde_json::from_str(encoded)
                        .map_err(|_| port(PortErrorCode::Corrupt, "control.operation.decode"))
                },
            )?;
            if durable.schema_version != hiroute_domain::OPERATION_SCHEMA_VERSION {
                return Err(port(PortErrorCode::Corrupt, "control.operation.schema"));
            }
            let operation_id = durable.operation_id.to_string();
            measure(
                diagnostics,
                PublicationStage::OperationReconstruct,
                Some(&operation_id),
                Some(encoded.len()),
                || {
                    let secrets = durable
                        .plan
                        .secrets
                        .into_iter()
                        .map(|mutation| {
                            if mutation.fingerprint_algorithm
                                != SecretFingerprintAlgorithm::HmacSha256V1
                            {
                                return Err(port(
                                    PortErrorCode::Corrupt,
                                    "control.operation.secret_algorithm",
                                ));
                            }
                            match mutation.kind {
                                SecretMutationKind::Upsert => SecretMutationV1::upsert(
                                    mutation.credential,
                                    mutation.expected_generation,
                                    mutation.input_slot.ok_or_else(|| {
                                        port(
                                            PortErrorCode::Corrupt,
                                            "control.operation.secret_slot",
                                        )
                                    })?,
                                    mutation.fingerprint,
                                ),
                                SecretMutationKind::Delete => {
                                    if mutation.input_slot.is_some()
                                        || mutation.fingerprint.is_some()
                                    {
                                        return Err(port(
                                            PortErrorCode::Corrupt,
                                            "control.operation.secret_delete",
                                        ));
                                    }
                                    SecretMutationV1::delete(
                                        mutation.credential,
                                        mutation.expected_generation,
                                    )
                                }
                                SecretMutationKind::Rebind => {
                                    if mutation.input_slot.is_some() {
                                        return Err(port(
                                            PortErrorCode::Corrupt,
                                            "control.operation.secret_rebind_slot",
                                        ));
                                    }
                                    SecretMutationV1::rebind(
                                        mutation.credential,
                                        mutation.new_allowed_destinations.ok_or_else(|| {
                                            port(
                                                PortErrorCode::Corrupt,
                                                "control.operation.secret_rebind_destinations",
                                            )
                                        })?,
                                        mutation.fingerprint.ok_or_else(|| {
                                            port(
                                                PortErrorCode::Corrupt,
                                                "control.operation.secret_rebind_fingerprint",
                                            )
                                        })?,
                                    )
                                }
                            }
                            .map_err(|_| port(PortErrorCode::Corrupt, "control.operation.secret"))
                        })
                        .collect::<PortResult<Vec<_>>>()?;
                    let runtime = durable
                        .plan
                        .runtime
                        .into_iter()
                        .map(|mutation| {
                            RuntimeMutationV1::from_registered_planner(
                                mutation.key,
                                mutation.value,
                                mutation.expected_generation,
                            )
                            .map_err(|_| port(PortErrorCode::Corrupt, "control.operation.runtime"))
                        })
                        .collect::<PortResult<Vec<_>>>()?;
                    let external = durable
                        .plan
                        .external
                        .into_iter()
                        .map(|intent| {
                            ExternalEffectIntentV1::from_registered_adapter(
                                intent.effect_id,
                                intent.kind,
                                intent.target,
                                intent.before_fingerprint,
                                intent.desired,
                                intent.desired_mode,
                                intent.sensitive,
                            )
                            .map_err(|_| port(PortErrorCode::Corrupt, "control.operation.external"))
                        })
                        .collect::<PortResult<Vec<_>>>()?;
                    let plan = measure(
                        diagnostics,
                        PublicationStage::OperationPlanValidate,
                        Some(&operation_id),
                        Some(encoded.len()),
                        || {
                            if let Some(change) = durable.plan.worker_dependency_selection {
                                if durable.plan.control != json!({})
                                    || durable.plan.credential_pool.is_some()
                                    || !secrets.is_empty()
                                    || !runtime.is_empty()
                                    || !external.is_empty()
                                {
                                    return Err(port(
                                        PortErrorCode::Corrupt,
                                        "control.operation.plan",
                                    ));
                                }
                                let selection = WorkerDependencySelectionChangeV1::new(
                                    change.before_revision,
                                    hiroute_domain::WorkerDependencySelectionRecordV1::new(
                                        change.after_selection.harness,
                                        change.after_selection.adapter_path,
                                        change.after_selection.cli_path,
                                        change.after_selection.node_path,
                                    )
                                    .map_err(|_| {
                                        port(PortErrorCode::Corrupt, "control.operation.plan")
                                    })?,
                                )
                                .map_err(|_| {
                                    port(PortErrorCode::Corrupt, "control.operation.plan")
                                })?;
                                TransactionPlanV1::from_worker_dependency_selection_planner(
                                    durable.plan.spec,
                                    selection,
                                )
                            } else {
                                TransactionPlanV1::from_registered_typed_planner(
                                    durable.plan.spec,
                                    durable.plan.control,
                                    durable.plan.credential_pool,
                                    secrets,
                                    runtime,
                                    external,
                                )
                            }
                            .map_err(|_| port(PortErrorCode::Corrupt, "control.operation.plan"))
                        },
                    )?;
                    let expected_id = OperationId::derive(
                        &durable.workspace_id,
                        &durable.idempotency,
                        &durable.request_digest,
                    );
                    if expected_id != durable.operation_id {
                        return Err(port(PortErrorCode::Corrupt, "control.operation.id"));
                    }
                    measure(
                        diagnostics,
                        PublicationStage::OperationRestore,
                        Some(&operation_id),
                        Some(encoded.len()),
                        || {
                            let mut operation = OperationV1::new(
                                durable.operation_id,
                                durable.workspace_id,
                                durable.idempotency,
                                durable.request_digest,
                                durable.accepted_digest,
                                durable.expected_revisions,
                                plan,
                            )
                            .map_err(|_| port(PortErrorCode::Corrupt, "control.operation.plan"))?;
                            operation
                                .restore_durable_state(
                                    durable.state,
                                    durable.generation,
                                    durable.steps,
                                    durable.safe_error_code,
                                )
                                .map_err(|_| {
                                    port(PortErrorCode::Corrupt, "control.operation.journal")
                                })?;
                            Ok(operation)
                        },
                    )
                },
            )
        },
    )
}

fn control_effect(
    operation_id: &OperationId,
    workspace: &WorkspaceId,
    before: Option<CanonicalDigest>,
    after: CanonicalDigest,
    after_revision: u64,
) -> OwnedEffectV1 {
    OwnedEffectV1 {
        effect_id: format!("control:{workspace}"),
        kind: OwnedEffectKind::Control,
        target: workspace.to_string(),
        before_fingerprint: before,
        after_fingerprint: Some(after),
        compensation: json!({
            "schema": "hiroute.control-compensation/v1",
            "operation_id": operation_id.as_str(),
            "after_revision": after_revision,
        })
        .into(),
    }
}

fn parse_digest(value: &str) -> Option<CanonicalDigest> {
    CanonicalDigest::parse(value).ok()
}

fn port(code: PortErrorCode, context: &'static str) -> PortError {
    PortError::new(code, context)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ArtifactMarker {
    schema: String,
    restore_store_uuid: String,
    restore_key_id: CanonicalDigest,
    operation_id: String,
    effect_id: String,
    kind: OwnedEffectKind,
    target: String,
    before_exists: bool,
    before_digest: Option<CanonicalDigest>,
    before_mode: Option<u32>,
    after_digest: CanonicalDigest,
    #[serde(default = "artifact_present")]
    after_exists: bool,
    after_mode: u32,
    sensitive: bool,
    #[serde(default)]
    rendered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    intent_digest: Option<CanonicalDigest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_path_digest: Option<CanonicalDigest>,
    backup_name: Option<String>,
    #[serde(default)]
    backup_aad: ArtifactBackupAad,
    activated: bool,
    compensated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    created_directories: Vec<CreatedNativeDirectory>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ArtifactBackupAad {
    /// Decodes an authenticated backup created by marker v2 during one-time migration.
    #[default]
    LegacyV2,
    /// Decodes an authenticated backup created by marker v3 during one-time migration.
    LegacyV3,
    CurrentV4,
}

/// Writes only adapter-owned publication/config artifacts. Restore bytes are independently
/// AEAD-sealed in an owner-only restore directory, never in `control.db` or the Operation journal.
pub struct ManagedArtifactStore {
    root: PathBuf,
    restore_root: PathBuf,
    restore_key: Zeroizing<Vec<u8>>,
    restore_store_uuid: String,
    restore_key_id: CanonicalDigest,
    external_targets: BTreeMap<String, PathBuf>,
}

fn artifact_present() -> bool {
    true
}

const RESTORE_KEY_HEADER: &[u8; 8] = b"HIRRST2\0";
const RESTORE_BACKUP_HEADER: &[u8; 8] = b"HIRBAK1\0";
const RESTORE_KEY_BYTES: usize = 32;
const RESTORE_NONCE_BYTES: usize = 12;
const RESTORE_STORE_UUID_BYTES: usize = 16;

struct RestoreKeyMaterial {
    bytes: Zeroizing<Vec<u8>>,
    store_uuid: String,
    key_id: CanonicalDigest,
}

impl ManagedArtifactStore {
    pub fn open(
        _authority: &DaemonStorageAuthority,
        root: impl AsRef<Path>,
        restore_root: impl AsRef<Path>,
    ) -> Result<Self, LocalStorageError> {
        prepare_owner_directory(root.as_ref())?;
        prepare_owner_directory(restore_root.as_ref())?;
        let restore = load_or_create_restore_key(
            &restore_root.as_ref().join(".restore-key"),
            restore_root.as_ref(),
        )?;
        let store = Self {
            root: root.as_ref().to_path_buf(),
            restore_root: restore_root.as_ref().to_path_buf(),
            restore_key: restore.bytes,
            restore_store_uuid: restore.store_uuid,
            restore_key_id: restore.key_id,
            external_targets: BTreeMap::new(),
        };
        store.migrate_legacy_markers()?;
        store.validate_existing_marker_bindings()?;
        Ok(store)
    }

    pub(crate) fn open_with_external_target(
        _authority: &DaemonStorageAuthority,
        root: impl AsRef<Path>,
        restore_root: impl AsRef<Path>,
        target: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Result<Self, LocalStorageError> {
        Self::open_with_external_targets(
            _authority,
            root,
            restore_root,
            [(target.into(), path.into())],
        )
    }

    pub(crate) fn open_with_external_targets(
        _authority: &DaemonStorageAuthority,
        root: impl AsRef<Path>,
        restore_root: impl AsRef<Path>,
        targets: impl IntoIterator<Item = (String, PathBuf)>,
    ) -> Result<Self, LocalStorageError> {
        let mut external_targets = BTreeMap::new();
        for (target, path) in targets {
            validate_external_target_path(&path)?;
            if external_targets.contains_key(&target)
                || external_targets.values().any(|existing| existing == &path)
            {
                return Err(LocalStorageError::InvalidData);
            }
            external_targets.insert(target, path);
        }
        prepare_owner_directory(root.as_ref())?;
        prepare_owner_directory(restore_root.as_ref())?;
        let restore = load_or_create_restore_key(
            &restore_root.as_ref().join(".restore-key"),
            restore_root.as_ref(),
        )?;
        let store = Self {
            root: root.as_ref().to_path_buf(),
            restore_root: restore_root.as_ref().to_path_buf(),
            restore_key: restore.bytes,
            restore_store_uuid: restore.store_uuid,
            restore_key_id: restore.key_id,
            external_targets,
        };
        // Validate every durable marker only after its native path binding is present. This is
        // the restart-safe counterpart to bind_external_target, which is used before first Apply.
        for target in store.external_targets.keys() {
            let _ = store.target_path(target)?;
        }
        store.migrate_legacy_markers()?;
        store.validate_existing_marker_bindings()?;
        Ok(store)
    }

    pub fn read_target(&self, target: &str) -> Result<Option<Vec<u8>>, LocalStorageError> {
        let path = self.target_path(target)?;
        Ok(read_optional_regular_file(&path)?.map(|bytes| bytes.to_vec()))
    }

    /// Binds one registered logical effect target to an exact native file outside the HiRoute
    /// artifact root. The caller must do this during composition, before recovery. The binding is
    /// recorded by digest in every new marker so a changed HOME/layout cannot redirect recovery.
    pub fn bind_external_target(
        &mut self,
        target: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Result<(), LocalStorageError> {
        let target = target.into();
        // Validate the logical target through the normal confined resolver before registering an
        // alternate physical location.
        let _ = self.target_path(&target)?;
        let path = path.into();
        validate_external_target_path(&path)?;
        if self
            .external_targets
            .values()
            .any(|existing| existing == &path)
            || self.external_targets.insert(target, path).is_some()
        {
            return Err(LocalStorageError::InvalidData);
        }
        self.validate_existing_marker_bindings()
    }

    /// Stages already rendered native bytes using the same encrypted restore and atomic
    /// activation machinery as ordinary managed artifacts. Only the renderer bytes differ; the
    /// sealed Operation intent remains the ownership and recovery identity.
    pub fn apply_rendered_external(
        &self,
        operation_id: &OperationId,
        intent: &ExternalEffectIntentV1,
        desired: &[u8],
    ) -> PortResult<OwnedEffectV1> {
        self.apply_external_bytes(operation_id, intent, desired, true, true, true)
    }

    /// Confirms that a rendered external effect still has its activated marker and decryptable
    /// restore point. The native integration separately verifies its owned fields semantically;
    /// this deliberately does not require the entire external file to remain byte-identical.
    pub fn rendered_external_is_recoverable(
        &self,
        operation_id: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<bool> {
        let Some(marker) = self
            .load_marker(operation_id, intent.effect_id())
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.marker.read"))?
        else {
            return Ok(false);
        };
        if marker.compensated
            || !marker.activated
            || !marker.rendered
            || marker.kind != intent.kind()
            || marker.target != intent.target()
            || marker.before_digest.as_ref() != intent.before_fingerprint()
            || marker.after_mode != intent.desired_mode()
            || marker.intent_digest.as_ref()
                != Some(
                    &CanonicalDigest::of(intent.desired())
                        .map_err(|_| port(PortErrorCode::InvalidData, "artifact.intent_digest"))?,
                )
        {
            return Ok(false);
        }
        self.validated_backup(&marker)
            .map_err(|_| port(PortErrorCode::Corrupt, "artifact.backup.validate"))?;
        Ok(true)
    }

    fn target_path(&self, target: &str) -> Result<PathBuf, LocalStorageError> {
        if let Some(path) = self.external_targets.get(target) {
            validate_external_target_path(path)?;
            return Ok(path.clone());
        }
        let relative = Path::new(target);
        if relative.is_absolute()
            || relative.components().any(|component| {
                !matches!(component, Component::Normal(_))
                    || component.as_os_str().to_string_lossy().starts_with('.')
            })
        {
            return Err(LocalStorageError::InvalidData);
        }
        let root_metadata = fs::symlink_metadata(&self.root)?;
        if root_metadata.file_type().is_symlink() || !root_metadata.file_type().is_dir() {
            return Err(LocalStorageError::Permission);
        }

        let mut path = self.root.clone();
        let mut components = relative.components().peekable();
        let mut missing_ancestor = false;
        while let Some(Component::Normal(component)) = components.next() {
            path.push(component);
            if components.peek().is_some() && !missing_ancestor {
                match fs::symlink_metadata(&path) {
                    Ok(metadata) => validate_owner_directory_metadata(&metadata)?,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        // A missing ancestor proves the final target is absent. Preview remains
                        // read-only; Apply creates and validates each directory separately.
                        missing_ancestor = true;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(path)
    }

    fn marker_key(operation_id: &OperationId, effect_id: &str) -> String {
        let digest = CanonicalDigest::of_bytes(
            format!("artifact-marker-v1\0{operation_id}\0{effect_id}").as_bytes(),
        );
        digest.as_str()[7..39].to_owned()
    }

    fn marker_path(&self, operation_id: &OperationId, effect_id: &str) -> PathBuf {
        self.restore_root.join(format!(
            "{}.json",
            Self::marker_key(operation_id, effect_id)
        ))
    }

    fn stage_path(&self, marker: &ArtifactMarker) -> Result<PathBuf, LocalStorageError> {
        let target = self.target_path(&marker.target)?;
        let file_name = target
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(LocalStorageError::InvalidData)?;
        let operation_id = OperationId::parse(marker.operation_id.clone())
            .map_err(|_| LocalStorageError::InvalidData)?;
        let key = Self::marker_key(&operation_id, &marker.effect_id);
        Ok(target.with_file_name(format!(".{file_name}.{key}.hiroute-stage")))
    }

    fn load_marker(
        &self,
        operation_id: &OperationId,
        effect_id: &str,
    ) -> Result<Option<ArtifactMarker>, LocalStorageError> {
        let path = self.marker_path(operation_id, effect_id);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                validate_owner_regular_file(&metadata)?;
                let bytes = fs::read(path)?;
                let encoded: Value =
                    serde_json::from_slice(&bytes).map_err(|_| LocalStorageError::InvalidData)?;
                if encoded.get("backup_aad").is_none() {
                    return Err(LocalStorageError::InvalidData);
                }
                let marker: ArtifactMarker =
                    serde_json::from_value(encoded).map_err(|_| LocalStorageError::InvalidData)?;
                if marker.schema != "hiroute.managed-artifact-marker/v4"
                    || marker.operation_id != operation_id.as_str()
                    || marker.effect_id != effect_id
                    || marker.restore_store_uuid != self.restore_store_uuid
                    || marker.restore_key_id != self.restore_key_id
                {
                    return Err(LocalStorageError::InvalidData);
                }
                self.validate_marker_shape(&marker)?;
                Ok(Some(marker))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Atomically upgrades marker metadata while preserving the exact authenticated backup.
    /// The explicit AAD discriminator prevents a schema-label rewrite from making ciphertext
    /// unrecoverable after a crash.
    fn migrate_legacy_markers(&self) -> Result<(), LocalStorageError> {
        for entry in fs::read_dir(&self.restore_root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let metadata = fs::symlink_metadata(&path)?;
            validate_owner_regular_file(&metadata).map_err(|_| LocalStorageError::Locked)?;
            let encoded: Value =
                serde_json::from_slice(&fs::read(&path)?).map_err(|_| LocalStorageError::Locked)?;
            let has_backup_aad = encoded.get("backup_aad").is_some();
            let mut marker: ArtifactMarker =
                serde_json::from_value(encoded).map_err(|_| LocalStorageError::Locked)?;
            let operation_id = OperationId::parse(marker.operation_id.clone())
                .map_err(|_| LocalStorageError::Locked)?;
            if path != self.marker_path(&operation_id, &marker.effect_id)
                || marker.restore_store_uuid != self.restore_store_uuid
                || marker.restore_key_id != self.restore_key_id
                || marker.external_path_digest != self.external_path_digest(&marker.target)?
            {
                return Err(LocalStorageError::Locked);
            }
            let changed = match marker.schema.as_str() {
                "hiroute.managed-artifact-marker/v2" => {
                    marker.backup_aad = if marker.backup_name.is_some() {
                        ArtifactBackupAad::LegacyV2
                    } else {
                        ArtifactBackupAad::CurrentV4
                    };
                    true
                }
                "hiroute.managed-artifact-marker/v3" => {
                    marker.backup_aad = if marker.backup_name.is_some() {
                        ArtifactBackupAad::LegacyV3
                    } else {
                        ArtifactBackupAad::CurrentV4
                    };
                    true
                }
                "hiroute.managed-artifact-marker/v4" if has_backup_aad => false,
                "hiroute.managed-artifact-marker/v4" => return Err(LocalStorageError::Locked),
                _ => return Err(LocalStorageError::Locked),
            };
            marker.schema = "hiroute.managed-artifact-marker/v4".to_owned();
            self.validate_marker_shape(&marker)
                .map_err(|_| LocalStorageError::Locked)?;
            self.validated_backup(&marker)
                .map_err(|_| LocalStorageError::Locked)?;
            if changed {
                self.save_marker(&marker)
                    .map_err(|_| LocalStorageError::Locked)?;
            }
        }
        Ok(())
    }

    fn validate_existing_marker_bindings(&self) -> Result<(), LocalStorageError> {
        for entry in fs::read_dir(&self.restore_root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let metadata = fs::symlink_metadata(&path)?;
            validate_owner_regular_file(&metadata)?;
            let marker: ArtifactMarker =
                serde_json::from_slice(&fs::read(path)?).map_err(|_| LocalStorageError::Locked)?;
            if marker.schema != "hiroute.managed-artifact-marker/v4"
                || marker.restore_store_uuid != self.restore_store_uuid
                || marker.restore_key_id != self.restore_key_id
                || marker.external_path_digest != self.external_path_digest(&marker.target)?
            {
                return Err(LocalStorageError::Locked);
            }
            self.validate_marker_shape(&marker)
                .map_err(|_| LocalStorageError::Locked)?;
        }
        Ok(())
    }

    fn save_marker(&self, marker: &ArtifactMarker) -> Result<(), LocalStorageError> {
        let operation_id = OperationId::parse(marker.operation_id.clone())
            .map_err(|_| LocalStorageError::InvalidData)?;
        let path = self.marker_path(&operation_id, &marker.effect_id);
        let encoded = serde_json::to_vec(marker).map_err(|_| LocalStorageError::InvalidData)?;
        atomic_write(&path, &encoded, 0o600, true)
    }

    fn validate_marker_shape(&self, marker: &ArtifactMarker) -> Result<(), LocalStorageError> {
        match (
            marker.before_exists,
            marker.before_digest.as_ref(),
            marker.before_mode,
            marker.backup_name.as_deref(),
        ) {
            (true, Some(_), Some(mode), Some(name)) if supported_artifact_mode(mode) => {
                let relative = Path::new(name);
                if relative.components().count() != 1
                    || relative
                        .components()
                        .any(|component| !matches!(component, Component::Normal(_)))
                    || name.starts_with('.')
                {
                    return Err(LocalStorageError::InvalidData);
                }
                Ok(())
            }
            (false, None, None, None) => Ok(()),
            _ => Err(LocalStorageError::InvalidData),
        }?;
        if marker.schema != "hiroute.managed-artifact-marker/v4"
            || (!marker.after_exists
                && (!marker.rendered || marker.kind != OwnedEffectKind::AgentArtifact))
            || !supported_artifact_mode(marker.after_mode)
            || (marker.sensitive && marker.after_mode != 0o600)
            || marker.external_path_digest != self.external_path_digest(&marker.target)?
            || marker.rendered != marker.intent_digest.is_some()
            || (marker.backup_name.is_none() && marker.backup_aad != ArtifactBackupAad::CurrentV4)
        {
            return Err(LocalStorageError::InvalidData);
        }
        Ok(())
    }

    fn external_path_digest(
        &self,
        target: &str,
    ) -> Result<Option<CanonicalDigest>, LocalStorageError> {
        self.external_targets
            .get(target)
            .map(|path| {
                path.to_str()
                    .map(|value| CanonicalDigest::of_bytes(value.as_bytes()))
                    .ok_or(LocalStorageError::InvalidData)
            })
            .transpose()
    }

    fn validated_backup(
        &self,
        marker: &ArtifactMarker,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, LocalStorageError> {
        self.validate_marker_shape(marker)?;
        let Some(name) = marker.backup_name.as_deref() else {
            return Ok(None);
        };
        let path = self.restore_root.join(name);
        let metadata = fs::symlink_metadata(&path)?;
        validate_owner_regular_file(&metadata)?;
        let sealed = fs::read(path)?;
        if sealed.len() <= RESTORE_BACKUP_HEADER.len() + RESTORE_NONCE_BYTES
            || &sealed[..RESTORE_BACKUP_HEADER.len()] != RESTORE_BACKUP_HEADER
        {
            return Err(LocalStorageError::InvalidData);
        }
        let nonce_start = RESTORE_BACKUP_HEADER.len();
        let ciphertext_start = nonce_start + RESTORE_NONCE_BYTES;
        let cipher =
            Aes256Gcm::new_from_slice(&self.restore_key).map_err(|_| LocalStorageError::Crypto)?;
        let bytes = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&sealed[nonce_start..ciphertext_start]),
                    Payload {
                        msg: &sealed[ciphertext_start..],
                        aad: artifact_backup_aad(marker).as_bytes(),
                    },
                )
                .map_err(|_| LocalStorageError::Crypto)?,
        );
        if Some(artifact_fingerprint(
            &bytes,
            marker.before_mode.unwrap_or(0o600),
        )?) != marker.before_digest
        {
            return Err(LocalStorageError::InvalidData);
        }
        Ok(Some(bytes))
    }

    fn seal_backup(
        &self,
        marker: &ArtifactMarker,
        bytes: &[u8],
    ) -> Result<Vec<u8>, LocalStorageError> {
        let cipher =
            Aes256Gcm::new_from_slice(&self.restore_key).map_err(|_| LocalStorageError::Crypto)?;
        let mut nonce = [0_u8; RESTORE_NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| LocalStorageError::Crypto)?;
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: bytes,
                    aad: artifact_backup_aad(marker).as_bytes(),
                },
            )
            .map_err(|_| LocalStorageError::Crypto)?;
        let mut sealed = Vec::with_capacity(
            RESTORE_BACKUP_HEADER.len() + RESTORE_NONCE_BYTES + ciphertext.len(),
        );
        sealed.extend_from_slice(RESTORE_BACKUP_HEADER);
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&ciphertext);
        Ok(sealed)
    }

    fn effect(marker: &ArtifactMarker) -> OwnedEffectV1 {
        OwnedEffectV1 {
            effect_id: marker.effect_id.clone(),
            kind: marker.kind,
            target: marker.target.clone(),
            before_fingerprint: marker.before_digest.clone(),
            after_fingerprint: marker.after_exists.then(|| marker.after_digest.clone()),
            compensation: json!({
                "schema": "hiroute.managed-artifact-compensation/v1",
                "operation_id": marker.operation_id,
                "effect_id": marker.effect_id,
            })
            .into(),
        }
    }

    fn apply_external_bytes(
        &self,
        operation_id: &OperationId,
        intent: &ExternalEffectIntentV1,
        desired: &[u8],
        rendered: bool,
        sensitive: bool,
        after_exists: bool,
    ) -> PortResult<OwnedEffectV1> {
        self.apply_external_bytes_with_parents(
            operation_id,
            intent,
            desired,
            rendered,
            sensitive,
            after_exists,
            Vec::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_external_bytes_with_parents(
        &self,
        operation_id: &OperationId,
        intent: &ExternalEffectIntentV1,
        desired: &[u8],
        rendered: bool,
        sensitive: bool,
        after_exists: bool,
        inherited_parents: Vec<CreatedNativeDirectory>,
    ) -> PortResult<OwnedEffectV1> {
        if !matches!(
            intent.kind(),
            OwnedEffectKind::Publication | OwnedEffectKind::AgentArtifact
        ) || (sensitive && intent.desired_mode() != 0o600)
        {
            return Err(port(PortErrorCode::InvalidData, "artifact.kind_or_mode"));
        }
        let desired_digest = artifact_fingerprint(desired, intent.desired_mode())
            .map_err(|_| port(PortErrorCode::InvalidData, "artifact.after_fingerprint"))?;
        if let Some(marker) = self
            .load_marker(operation_id, intent.effect_id())
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.marker.read"))?
        {
            if marker.after_exists != after_exists
                || marker.rendered != rendered
                || marker.sensitive != sensitive
                || marker.after_digest != desired_digest
                || (rendered
                    && marker.intent_digest.as_ref()
                        != Some(&CanonicalDigest::of(intent.desired()).map_err(|_| {
                            port(PortErrorCode::InvalidData, "artifact.intent_digest")
                        })?))
            {
                return Err(port(PortErrorCode::Conflict, "artifact.ownership"));
            }
            return match self.observe_artifact(operation_id, intent)? {
                EffectReconciliation::Staged(effect) | EffectReconciliation::Applied(effect) => {
                    Ok(effect)
                }
                EffectReconciliation::OwnershipLost(_) => {
                    Err(port(PortErrorCode::Conflict, "artifact.ownership"))
                }
                EffectReconciliation::Missing => {
                    self.validated_backup(&marker)
                        .map_err(|_| port(PortErrorCode::Corrupt, "artifact.backup.validate"))?;
                    let stage = self
                        .stage_path(&marker)
                        .map_err(|_| port(PortErrorCode::InvalidData, "artifact.stage"))?;
                    atomic_write(&stage, desired, marker.after_mode, marker.sensitive)
                        .map_err(|_| port(PortErrorCode::Unavailable, "artifact.stage.write"))?;
                    Ok(Self::effect(&marker))
                }
            };
        }

        let target = self
            .target_path(intent.target())
            .map_err(|_| port(PortErrorCode::InvalidData, "artifact.target"))?;
        let before = read_artifact(&target)
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.read"))?;
        let before_digest = before.as_ref().map(|snapshot| snapshot.fingerprint.clone());
        if before_digest.as_ref() != intent.before_fingerprint() {
            return Err(port(PortErrorCode::Conflict, "artifact.before_fingerprint"));
        }
        let new_directories = if after_exists || before.is_some() {
            native_directories::prepare_parent(&target)
                .map_err(|_| port(PortErrorCode::PermissionDenied, "artifact.target.parent"))?
        } else {
            Vec::new()
        };
        let created_directories =
            native_directories::merge_lineage(inherited_parents, new_directories)
                .map_err(|_| port(PortErrorCode::InvalidData, "native.parents.bound"))?;
        let key = Self::marker_key(operation_id, intent.effect_id());
        let backup_name = before.as_ref().map(|_| format!("{key}.before"));
        let marker = ArtifactMarker {
            schema: "hiroute.managed-artifact-marker/v4".to_owned(),
            restore_store_uuid: self.restore_store_uuid.clone(),
            restore_key_id: self.restore_key_id.clone(),
            operation_id: operation_id.to_string(),
            effect_id: intent.effect_id().to_owned(),
            kind: intent.kind(),
            target: intent.target().to_owned(),
            before_exists: before.is_some(),
            before_digest,
            before_mode: before.as_ref().map(|snapshot| snapshot.mode),
            after_digest: desired_digest,
            after_exists,
            after_mode: intent.desired_mode(),
            sensitive,
            rendered,
            intent_digest: rendered
                .then(|| CanonicalDigest::of(intent.desired()))
                .transpose()
                .map_err(|_| port(PortErrorCode::InvalidData, "artifact.intent_digest"))?,
            external_path_digest: self
                .external_path_digest(intent.target())
                .map_err(|_| port(PortErrorCode::InvalidData, "artifact.external_binding"))?,
            backup_name,
            backup_aad: ArtifactBackupAad::CurrentV4,
            activated: false,
            compensated: false,
            created_directories,
        };
        if let (Some(snapshot), Some(name)) = (&before, &marker.backup_name) {
            let sealed = self
                .seal_backup(&marker, &snapshot.bytes)
                .map_err(|_| port(PortErrorCode::Crypto, "artifact.backup.seal"))?;
            atomic_write(&self.restore_root.join(name), &sealed, 0o600, true)
                .map_err(|_| port(PortErrorCode::Unavailable, "artifact.backup"))?;
        }
        self.save_marker(&marker)
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.marker.prepare"))?;
        self.validated_backup(&marker)
            .map_err(|_| port(PortErrorCode::Corrupt, "artifact.backup.validate"))?;
        if !marker.after_exists && !marker.before_exists {
            return Ok(Self::effect(&marker));
        }
        let stage = self
            .stage_path(&marker)
            .map_err(|_| port(PortErrorCode::InvalidData, "artifact.stage"))?;
        atomic_write(&stage, desired, marker.after_mode, marker.sensitive)
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.stage.write"))?;
        Ok(Self::effect(&marker))
    }
}

impl ManagedArtifactStore {
    pub fn current_external_fingerprint(
        &self,
        target: &str,
    ) -> PortResult<Option<CanonicalDigest>> {
        let path = self
            .target_path(target)
            .map_err(|_| port(PortErrorCode::InvalidData, "artifact.target"))?;
        Ok(read_artifact(&path)
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.fingerprint"))?
            .map(|snapshot| snapshot.fingerprint))
    }

    pub fn apply_artifact(
        &self,
        operation_id: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<OwnedEffectV1> {
        let desired = serde_json::to_vec(intent.desired())
            .map_err(|_| port(PortErrorCode::InvalidData, "artifact.encode"))?;
        self.apply_external_bytes(
            operation_id,
            intent,
            &desired,
            false,
            intent.sensitive(),
            true,
        )
    }

    pub fn observe_artifact(
        &self,
        operation_id: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<EffectReconciliation> {
        let Some(marker) = self
            .load_marker(operation_id, intent.effect_id())
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.marker.observe"))?
        else {
            return Ok(EffectReconciliation::Missing);
        };
        if marker.compensated {
            return Ok(EffectReconciliation::Missing);
        }
        if marker.kind != intent.kind()
            || marker.target != intent.target()
            || marker.before_digest.as_ref() != intent.before_fingerprint()
            || marker.after_mode != intent.desired_mode()
            || (!marker.rendered && marker.sensitive != intent.sensitive())
        {
            return Ok(EffectReconciliation::OwnershipLost(Self::effect(&marker)));
        }
        self.validated_backup(&marker)
            .map_err(|_| port(PortErrorCode::Corrupt, "artifact.backup.validate"))?;
        if marker.rendered {
            if marker.intent_digest.as_ref()
                != Some(
                    &CanonicalDigest::of(intent.desired())
                        .map_err(|_| port(PortErrorCode::InvalidData, "artifact.intent_digest"))?,
                )
            {
                return Ok(EffectReconciliation::OwnershipLost(Self::effect(&marker)));
            }
        } else {
            let desired = serde_json::to_vec(intent.desired())
                .map_err(|_| port(PortErrorCode::InvalidData, "artifact.encode"))?;
            if artifact_fingerprint(&desired, intent.desired_mode())
                .map_err(|_| port(PortErrorCode::Corrupt, "artifact.intent_fingerprint"))?
                != marker.after_digest
            {
                return Ok(EffectReconciliation::OwnershipLost(Self::effect(&marker)));
            }
        }
        let target = self
            .target_path(&marker.target)
            .map_err(|_| port(PortErrorCode::Corrupt, "artifact.target"))?;
        let current = read_artifact(&target)
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.observe"))?;
        let current_digest = current
            .as_ref()
            .map(|snapshot| snapshot.fingerprint.clone());
        if marker.activated
            && current_digest.as_ref() == marker.after_exists.then_some(&marker.after_digest)
        {
            Ok(EffectReconciliation::Applied(Self::effect(&marker)))
        } else if marker.activated {
            Ok(EffectReconciliation::OwnershipLost(Self::effect(&marker)))
        } else if current_digest.as_ref() == marker.after_exists.then_some(&marker.after_digest) {
            // rename succeeded and marker persistence crashed; Activate can safely finalize it.
            Ok(EffectReconciliation::Staged(Self::effect(&marker)))
        } else if current_digest == marker.before_digest
            || (!marker.before_exists && current.is_none())
        {
            let stage = self
                .stage_path(&marker)
                .map_err(|_| port(PortErrorCode::Corrupt, "artifact.stage"))?;
            match read_artifact(&stage)
                .map_err(|_| port(PortErrorCode::Unavailable, "artifact.stage.observe"))?
            {
                Some(snapshot) if snapshot.fingerprint == marker.after_digest => {
                    Ok(EffectReconciliation::Staged(Self::effect(&marker)))
                }
                None => Ok(EffectReconciliation::Missing),
                Some(_) => Ok(EffectReconciliation::OwnershipLost(Self::effect(&marker))),
            }
        } else {
            Ok(EffectReconciliation::OwnershipLost(Self::effect(&marker)))
        }
    }

    pub fn activate_artifact(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1> {
        let operation_id = effect
            .compensation
            .get("operation_id")
            .and_then(Value::as_str)
            .ok_or_else(|| port(PortErrorCode::InvalidData, "artifact.activate.operation"))?;
        let operation_id = OperationId::parse(operation_id)
            .map_err(|_| port(PortErrorCode::InvalidData, "artifact.activate.operation"))?;
        let effect_id = effect
            .compensation
            .get("effect_id")
            .and_then(Value::as_str)
            .ok_or_else(|| port(PortErrorCode::InvalidData, "artifact.activate.effect"))?;
        let mut marker = self
            .load_marker(&operation_id, effect_id)
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.activate.marker"))?
            .ok_or_else(|| port(PortErrorCode::NotFound, "artifact.activate.marker"))?;
        if marker.compensated {
            return Err(port(
                PortErrorCode::Conflict,
                "artifact.activate.compensated",
            ));
        }
        if marker.activated {
            return Ok(Self::effect(&marker));
        }
        self.validated_backup(&marker)
            .map_err(|_| port(PortErrorCode::Corrupt, "artifact.activate.backup"))?;
        let target = self
            .target_path(&marker.target)
            .map_err(|_| port(PortErrorCode::Corrupt, "artifact.activate.target"))?;
        let current = read_artifact(&target)
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.activate.current"))?;
        let current_digest = current
            .as_ref()
            .map(|snapshot| snapshot.fingerprint.clone());
        if current_digest.as_ref() != marker.after_exists.then_some(&marker.after_digest) {
            if current_digest != marker.before_digest
                && !(!marker.before_exists && current.is_none())
            {
                return Err(port(PortErrorCode::Conflict, "artifact.activate.ownership"));
            }
            let stage = self
                .stage_path(&marker)
                .map_err(|_| port(PortErrorCode::Corrupt, "artifact.activate.stage"))?;
            let staged = read_artifact(&stage)
                .map_err(|_| port(PortErrorCode::Unavailable, "artifact.activate.stage"))?
                .ok_or_else(|| port(PortErrorCode::NotFound, "artifact.activate.stage"))?;
            if staged.fingerprint != marker.after_digest {
                return Err(port(
                    PortErrorCode::Conflict,
                    "artifact.activate.stage_ownership",
                ));
            }
            if marker.after_exists {
                fs::rename(&stage, &target)
                    .map_err(|_| port(PortErrorCode::Unavailable, "artifact.activate.rename"))?;
            } else {
                if current.is_some() {
                    fs::remove_file(&target).map_err(|_| {
                        port(PortErrorCode::Unavailable, "artifact.activate.remove")
                    })?;
                }
                fs::remove_file(&stage).map_err(|_| {
                    port(PortErrorCode::Unavailable, "artifact.activate.remove_stage")
                })?;
            }
            sync_parent(&target)
                .map_err(|_| port(PortErrorCode::Unavailable, "artifact.activate.sync"))?;
        }
        let activated = read_artifact(&target)
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.activate.verify"))?;
        if activated.as_ref().map(|snapshot| &snapshot.fingerprint)
            != marker.after_exists.then_some(&marker.after_digest)
        {
            return Err(port(PortErrorCode::Corrupt, "artifact.activate.verify"));
        }
        // A replay may observe the intended absence before consuming the empty stage.
        // Delete only our exact staged artifact, never a replacement at that path.
        if !marker.after_exists {
            let stage = self
                .stage_path(&marker)
                .map_err(|_| port(PortErrorCode::Corrupt, "artifact.activate.stage"))?;
            if let Some(staged) = read_artifact(&stage)
                .map_err(|_| port(PortErrorCode::Unavailable, "artifact.activate.stage"))?
            {
                if staged.fingerprint != marker.after_digest {
                    return Err(port(
                        PortErrorCode::Conflict,
                        "artifact.activate.stage_ownership",
                    ));
                }
                fs::remove_file(&stage).map_err(|_| {
                    port(PortErrorCode::Unavailable, "artifact.activate.remove_stage")
                })?;
                sync_parent(&stage)
                    .map_err(|_| port(PortErrorCode::Unavailable, "artifact.activate.sync"))?;
            }
        }
        marker.activated = true;
        self.save_marker(&marker)
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.activate.mark"))?;
        Ok(Self::effect(&marker))
    }

    pub fn compensate_artifact(&self, effect: &OwnedEffectV1) -> PortResult<CompensationOutcome> {
        let operation_id = effect
            .compensation
            .get("operation_id")
            .and_then(Value::as_str)
            .ok_or_else(|| port(PortErrorCode::InvalidData, "artifact.compensate.metadata"))?;
        let operation_id = OperationId::parse(operation_id)
            .map_err(|_| port(PortErrorCode::InvalidData, "artifact.compensate.operation"))?;
        let effect_id = effect
            .compensation
            .get("effect_id")
            .and_then(Value::as_str)
            .ok_or_else(|| port(PortErrorCode::InvalidData, "artifact.compensate.effect"))?;
        let mut marker = self
            .load_marker(&operation_id, effect_id)
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.compensate.marker"))?
            .ok_or_else(|| port(PortErrorCode::NotFound, "artifact.compensate.marker"))?;
        if marker.compensated {
            return Ok(CompensationOutcome::AlreadyCompensated);
        }
        self.validated_backup(&marker)
            .map_err(|_| port(PortErrorCode::Corrupt, "artifact.compensate.backup"))?;
        let target = self
            .target_path(&marker.target)
            .map_err(|_| port(PortErrorCode::Corrupt, "artifact.compensate.target"))?;
        let current = read_artifact(&target)
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.compensate.read"))?;
        let current_digest = current
            .as_ref()
            .map(|snapshot| snapshot.fingerprint.clone());
        if !marker.activated {
            let stage = self
                .stage_path(&marker)
                .map_err(|_| port(PortErrorCode::Corrupt, "artifact.compensate.stage"))?;
            if let Some(snapshot) = read_artifact(&stage)
                .map_err(|_| port(PortErrorCode::Unavailable, "artifact.compensate.stage"))?
            {
                if snapshot.fingerprint != marker.after_digest {
                    return Ok(CompensationOutcome::OwnershipLost);
                }
                fs::remove_file(&stage)
                    .map_err(|_| port(PortErrorCode::Unavailable, "artifact.compensate.stage"))?;
                sync_parent(&stage)
                    .map_err(|_| port(PortErrorCode::Unavailable, "artifact.compensate.sync"))?;
            }
            if current_digest != marker.before_digest
                && !(!marker.before_exists && current.is_none())
            {
                return Ok(CompensationOutcome::OwnershipLost);
            }
            marker.compensated = true;
            self.save_marker(&marker)
                .map_err(|_| port(PortErrorCode::Unavailable, "artifact.compensate.mark"))?;
            return Ok(CompensationOutcome::Compensated);
        }
        if current_digest != marker.after_exists.then(|| marker.after_digest.clone()) {
            if current_digest == marker.before_digest
                || (!marker.before_exists && current.is_none())
            {
                marker.compensated = true;
                self.save_marker(&marker)
                    .map_err(|_| port(PortErrorCode::Unavailable, "artifact.compensate.mark"))?;
                return Ok(CompensationOutcome::AlreadyCompensated);
            }
            return Ok(CompensationOutcome::OwnershipLost);
        }
        if marker.before_exists {
            let before = self
                .validated_backup(&marker)
                .map_err(|_| port(PortErrorCode::Corrupt, "artifact.compensate.backup"))?
                .ok_or_else(|| port(PortErrorCode::Corrupt, "artifact.compensate.backup"))?;
            atomic_write(
                &target,
                &before,
                marker
                    .before_mode
                    .ok_or_else(|| port(PortErrorCode::Corrupt, "artifact.before_mode"))?,
                false,
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.compensate.write"))?;
        } else if current.is_some() {
            fs::remove_file(&target)
                .map_err(|_| port(PortErrorCode::Unavailable, "artifact.compensate.remove"))?;
            sync_parent(&target)
                .map_err(|_| port(PortErrorCode::Unavailable, "artifact.compensate.sync"))?;
        }
        let restored = read_artifact(&target)
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.compensate.verify"))?;
        let restored_digest = restored
            .as_ref()
            .map(|snapshot| snapshot.fingerprint.clone());
        if restored_digest != marker.before_digest && !(!marker.before_exists && restored.is_none())
        {
            return Err(port(PortErrorCode::Corrupt, "artifact.compensate.verify"));
        }
        marker.compensated = true;
        self.save_marker(&marker)
            .map_err(|_| port(PortErrorCode::Unavailable, "artifact.compensate.mark"))?;
        Ok(CompensationOutcome::Compensated)
    }
}

fn prepare_owner_directory(path: &Path) -> Result<(), LocalStorageError> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
            return Err(LocalStorageError::Permission);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.mode() & 0o777 != 0o700
                || metadata.uid() != rustix::process::getuid().as_raw()
            {
                return Err(LocalStorageError::Permission);
            }
        }
    } else {
        fs::create_dir_all(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

fn validate_external_target_path(path: &Path) -> Result<(), LocalStorageError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(LocalStorageError::InvalidData);
    }
    let parent = path.parent().ok_or(LocalStorageError::InvalidData)?;
    native_directories::validate_parent(parent)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
                return Err(LocalStorageError::Permission);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if metadata.uid() != rustix::process::getuid().as_raw()
                    || metadata.nlink() != 1
                    || !supported_artifact_mode(metadata.mode() & 0o777)
                {
                    return Err(LocalStorageError::Permission);
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn validate_owner_directory_metadata(metadata: &fs::Metadata) -> Result<(), LocalStorageError> {
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(LocalStorageError::Permission);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // The store root is already fixed at 0700. Existing nested directories may predate
        // this renderer, but must still be owned by the daemon uid and never be symlinks.
        if metadata.uid() != rustix::process::getuid().as_raw() || metadata.mode() & 0o022 != 0 {
            return Err(LocalStorageError::Permission);
        }
    }
    Ok(())
}

fn read_optional_regular_file(
    path: &Path,
) -> Result<Option<Zeroizing<Vec<u8>>>, LocalStorageError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(Some(Zeroizing::new(fs::read(path)?))),
        Ok(_) => Err(LocalStorageError::InvalidData),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

struct ArtifactSnapshot {
    bytes: Zeroizing<Vec<u8>>,
    mode: u32,
    fingerprint: CanonicalDigest,
}

fn read_artifact(path: &Path) -> Result<Option<ArtifactSnapshot>, LocalStorageError> {
    let Some(bytes) = read_optional_regular_file(path)? else {
        return Ok(None);
    };
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(path)?.mode() & 0o777
    };
    #[cfg(not(unix))]
    return Err(LocalStorageError::InvalidData);
    if !supported_artifact_mode(mode) {
        return Err(LocalStorageError::Permission);
    }
    let fingerprint = artifact_fingerprint(&bytes, mode)?;
    Ok(Some(ArtifactSnapshot {
        bytes,
        mode,
        fingerprint,
    }))
}

fn artifact_fingerprint(bytes: &[u8], mode: u32) -> Result<CanonicalDigest, LocalStorageError> {
    if !supported_artifact_mode(mode) {
        return Err(LocalStorageError::Permission);
    }
    CanonicalDigest::of(&json!({
        "schema": "hiroute.posix-artifact-fingerprint/v1",
        "content": CanonicalDigest::of_bytes(bytes),
        "mode": mode,
    }))
    .map_err(|_| LocalStorageError::InvalidData)
}

const fn supported_artifact_mode(mode: u32) -> bool {
    matches!(mode, 0o600 | 0o640 | 0o644)
}

fn artifact_backup_aad(marker: &ArtifactMarker) -> String {
    let base = format!(
        "hiroute.managed-artifact-backup/v2\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
        marker.restore_store_uuid,
        marker.restore_key_id,
        marker.operation_id,
        marker.effect_id,
        marker.target,
        marker
            .before_digest
            .as_ref()
            .map_or("absent", CanonicalDigest::as_str),
        marker
            .before_mode
            .map_or_else(|| "absent".to_owned(), |mode| format!("{mode:o}")),
        marker.after_mode,
        marker.sensitive,
    );
    match marker.backup_aad {
        ArtifactBackupAad::LegacyV2 => base,
        ArtifactBackupAad::LegacyV3 => format!("{base}\0v3\0{}", marker.after_exists),
        ArtifactBackupAad::CurrentV4 => format!("{base}\0v4\0{}", marker.after_exists),
    }
}

fn load_or_create_restore_key(
    path: &Path,
    restore_root: &Path,
) -> Result<RestoreKeyMaterial, LocalStorageError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_owner_regular_file(&metadata).map_err(|_| LocalStorageError::Locked)?;
            let mut file = File::open(path)?;
            let mut header = [0_u8; 8];
            file.read_exact(&mut header)?;
            if &header != RESTORE_KEY_HEADER {
                return Err(LocalStorageError::Locked);
            }
            let mut store_uuid = vec![0_u8; RESTORE_STORE_UUID_BYTES * 2];
            let mut key_id = vec![0_u8; 71];
            let mut bytes = Zeroizing::new(vec![0_u8; RESTORE_KEY_BYTES]);
            file.read_exact(&mut store_uuid)?;
            file.read_exact(&mut key_id)?;
            file.read_exact(&mut bytes)?;
            let mut trailing = [0_u8; 1];
            if file.read(&mut trailing)? != 0 {
                return Err(LocalStorageError::Locked);
            }
            let store_uuid =
                String::from_utf8(store_uuid).map_err(|_| LocalStorageError::Locked)?;
            let key_id = CanonicalDigest::parse(
                String::from_utf8(key_id).map_err(|_| LocalStorageError::Locked)?,
            )
            .map_err(|_| LocalStorageError::Locked)?;
            if restore_key_id(&bytes) != key_id {
                return Err(LocalStorageError::Locked);
            }
            Ok(RestoreKeyMaterial {
                bytes,
                store_uuid,
                key_id,
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if fs::read_dir(restore_root)?.next().is_some() {
                return Err(LocalStorageError::Locked);
            }
            let mut bytes = Zeroizing::new(vec![0_u8; RESTORE_KEY_BYTES]);
            let mut uuid = [0_u8; RESTORE_STORE_UUID_BYTES];
            getrandom::fill(&mut bytes).map_err(|_| LocalStorageError::Crypto)?;
            getrandom::fill(&mut uuid).map_err(|_| LocalStorageError::Crypto)?;
            let store_uuid = hex_restore(&uuid);
            let key_id = restore_key_id(&bytes);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(path)?;
            file.write_all(RESTORE_KEY_HEADER)?;
            file.write_all(store_uuid.as_bytes())?;
            file.write_all(key_id.as_str().as_bytes())?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            sync_parent(path)?;
            validate_owner_regular_file(&fs::metadata(path)?)?;
            Ok(RestoreKeyMaterial {
                bytes,
                store_uuid,
                key_id,
            })
        }
        Err(error) => Err(error.into()),
    }
}

fn restore_key_id(bytes: &[u8]) -> CanonicalDigest {
    let mut material = b"hiroute-restore-key-id-v1\0".to_vec();
    material.extend_from_slice(bytes);
    CanonicalDigest::of_bytes(&material)
}

fn hex_restore(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn validate_owner_regular_file(metadata: &fs::Metadata) -> Result<(), LocalStorageError> {
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(LocalStorageError::Permission);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o777 != 0o600 || metadata.uid() != rustix::process::getuid().as_raw()
        {
            return Err(LocalStorageError::Permission);
        }
    }
    Ok(())
}

fn atomic_write(
    path: &Path,
    bytes: &[u8],
    mode: u32,
    sensitive: bool,
) -> Result<(), LocalStorageError> {
    if !supported_artifact_mode(mode) || (sensitive && mode != 0o600) {
        return Err(LocalStorageError::Permission);
    }
    let parent = path.parent().ok_or(LocalStorageError::InvalidData)?;
    let parent_metadata = fs::symlink_metadata(parent)?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.file_type().is_dir() {
        return Err(LocalStorageError::Permission);
    }
    let temporary = path.with_extension("hiroute-tmp");
    if temporary.exists() {
        fs::remove_file(&temporary)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))?;
    }
    fs::rename(&temporary, path)?;
    sync_parent(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if fs::metadata(path)?.mode() & 0o777 != mode {
            return Err(LocalStorageError::Permission);
        }
    }
    Ok(())
}

fn sync_parent(path: &Path) -> Result<(), LocalStorageError> {
    let parent = path.parent().ok_or(LocalStorageError::InvalidData)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

mod delegation;

impl ExternalEffectPort for ManagedArtifactStore {
    fn current_external_fingerprint(&self, target: &str) -> PortResult<Option<CanonicalDigest>> {
        ManagedArtifactStore::current_external_fingerprint(self, target)
    }
    fn apply_external(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<OwnedEffectV1> {
        self.apply_artifact(&operation.operation_id, intent)
    }
    fn observe_external(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<EffectReconciliation> {
        self.observe_artifact(&operation.operation_id, intent)
    }
    fn activate_external(
        &self,
        _operation: &OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<OwnedEffectV1> {
        self.activate_artifact(effect)
    }
    fn compensate_external(
        &self,
        _operation: &OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<CompensationOutcome> {
        self.compensate_artifact(effect)
    }
}
