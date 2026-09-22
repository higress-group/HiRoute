use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use hiroute_domain::{
    AgentAccessGrantMaterial, AgentAccessGrantMutationV1, AgentAccessGrantRefV1, CanonicalDigest,
    CompensationOutcome, CredentialRefV1, EffectReconciliation, OperationId, OwnedEffectKind,
    OwnedEffectV1, PortError, PortErrorCode, PortResult, ProtectedSecret, SecretMutationKind,
    SecretMutationV1, SecretStorePort, VerifiedSecretSubjectV1,
};
use hmac::{Hmac, Mac};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::migrations::{
    DatabaseKind, MigrationSecretBinding, open_database_from_set, open_database_with_key_id,
};
use crate::{DaemonStorageAuthority, LocalStorageError};

#[path = "../agents/collaboration_bootstrap.rs"]
mod collaboration_bootstrap;
#[path = "../agents/collaboration_credentials.rs"]
mod collaboration_credentials;
mod grant;
#[cfg(test)]
mod grant_tests;
mod runtime;
#[cfg(test)]
mod runtime_tests;

pub use runtime::LocalNativeCredentialAuthority;

const MASTER_KEY_HEADER: &[u8; 8] = b"HIRKEY2\0";
const MASTER_KEY_BYTES: usize = 64;
const STORE_UUID_BYTES: usize = 16;
const STORE_UUID_HEX_BYTES: usize = STORE_UUID_BYTES * 2;
const DIGEST_TEXT_BYTES: usize = 71;
const NONCE_BYTES: usize = 12;
const AAD_SCHEMA: &str = "hiroute.secret-entry/v2";
const LEGACY_AAD_SCHEMA: &str = "hiroute.secret-entry/v1";
const KEY_VERSION: u32 = 1;

struct KeyMaterial {
    bytes: Zeroizing<Vec<u8>>,
    store_uuid: String,
    key_id: CanonicalDigest,
    verifier: CanonicalDigest,
}

impl KeyMaterial {
    fn encryption(&self) -> &[u8] {
        &self.bytes[..32]
    }

    fn fingerprint(&self) -> &[u8] {
        &self.bytes[32..]
    }
}

pub struct LocalSecretStore {
    connection: RefCell<Connection>,
    keys: KeyMaterial,
    #[cfg(test)]
    database_path: PathBuf,
    #[cfg(test)]
    master_key_path: PathBuf,
}

impl LocalSecretStore {
    pub(crate) fn open(
        authority: &DaemonStorageAuthority,
        database_path: impl AsRef<Path>,
        master_key_path: impl AsRef<Path>,
        migration_backup_root: impl AsRef<Path>,
    ) -> Result<Self, LocalStorageError> {
        Self::open_internal(
            authority,
            database_path.as_ref(),
            master_key_path.as_ref(),
            migration_backup_root.as_ref(),
            None,
        )
    }

    pub(crate) fn migration_binding(
        database_path: &Path,
        master_key_path: &Path,
    ) -> Result<Option<MigrationSecretBinding>, LocalStorageError> {
        if !database_path.exists() {
            return Ok(None);
        }
        let keys = load_key_material(master_key_path)?.ok_or(LocalStorageError::Locked)?;
        preflight_existing_store(database_path, &keys)?;
        Ok(Some(MigrationSecretBinding {
            store_uuid: keys.store_uuid,
            key_id: keys.key_id,
            key_verifier: keys.verifier,
        }))
    }

    pub(crate) fn open_from_migration_set(
        authority: &DaemonStorageAuthority,
        database_path: &Path,
        master_key_path: &Path,
        migration_backup_root: &Path,
        binding: &MigrationSecretBinding,
    ) -> Result<Self, LocalStorageError> {
        Self::open_internal(
            authority,
            database_path,
            master_key_path,
            migration_backup_root,
            Some(binding),
        )
    }

    fn open_internal(
        authority: &DaemonStorageAuthority,
        database_path: &Path,
        master_key_path: &Path,
        migration_backup_root: &Path,
        migration_binding: Option<&MigrationSecretBinding>,
    ) -> Result<Self, LocalStorageError> {
        let existing_store = database_path.exists();
        if !existing_store && has_backup_evidence(database_path, migration_backup_root)? {
            // A lost database with an exact local restore point is not a fresh store. Recovery
            // must be explicit; silently creating a new key/database would make the backup
            // undecryptable and destroy the stable locked-state signal.
            return Err(LocalStorageError::Locked);
        }
        let keys = match load_key_material(master_key_path) {
            Ok(Some(keys)) => keys,
            Ok(None) if existing_store => return Err(LocalStorageError::Locked),
            Ok(None) => create_key_material(master_key_path)?,
            Err(error) if existing_store => {
                let _ = error;
                return Err(LocalStorageError::Locked);
            }
            Err(error) => return Err(error),
        };

        // A v3 store has an exact non-secret key binding. It is checked read-only before any WAL,
        // migration, or metadata write. Older stores with data prove the candidate key by opening
        // and authenticating one current ciphertext before migration.
        if existing_store {
            preflight_existing_store(database_path, &keys)?;
        }
        if migration_binding.is_some_and(|binding| {
            binding.store_uuid != keys.store_uuid
                || binding.key_id != keys.key_id
                || binding.key_verifier != keys.verifier
        }) {
            return Err(LocalStorageError::Locked);
        }
        let mut connection = if let Some(binding) = migration_binding {
            open_database_from_set(
                authority,
                database_path,
                DatabaseKind::Secrets,
                migration_backup_root,
                None,
                Some(binding),
            )?
        } else {
            open_database_with_key_id(
                authority,
                database_path,
                DatabaseKind::Secrets,
                migration_backup_root,
                Some(&keys.key_id),
            )?
        };
        bind_store_metadata(&connection, &keys)?;
        migrate_legacy_secret_aad(&mut connection, &keys)?;
        Ok(Self {
            connection: RefCell::new(connection),
            keys,
            #[cfg(test)]
            database_path: database_path.to_path_buf(),
            #[cfg(test)]
            master_key_path: master_key_path.to_path_buf(),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_connection<T>(&self, function: impl FnOnce(&Connection) -> T) -> T {
        function(&self.connection.borrow())
    }

    #[cfg(test)]
    pub(crate) fn database_path(&self) -> &Path {
        &self.database_path
    }

    #[cfg(test)]
    pub(crate) fn master_key_path(&self) -> &Path {
        &self.master_key_path
    }

    #[cfg(test)]
    fn read_secret_for_test(&self, credential_id: &str) -> PortResult<Option<ProtectedSecret>> {
        read_entry(&self.connection.borrow(), credential_id)?
            .map(|row| self.authenticate_row(&row))
            .transpose()
    }

    #[cfg(test)]
    pub(crate) fn checkpoint(&self) -> PortResult<()> {
        self.connection
            .borrow()
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .map_err(|_| port(PortErrorCode::Unavailable, "secret.checkpoint"))?;
        Ok(())
    }

    fn fingerprint_secret(&self, secret: &ProtectedSecret) -> PortResult<CanonicalDigest> {
        fingerprint_with_key(self.keys.fingerprint(), secret.expose())
    }

    fn encrypt(
        &self,
        credential: &CredentialRefV1,
        kind: &str,
        generation: u64,
        input: &ProtectedSecret,
    ) -> PortResult<(Vec<u8>, Vec<u8>)> {
        let cipher = Aes256Gcm::new_from_slice(self.keys.encryption())
            .map_err(|_| port(PortErrorCode::Crypto, "secret.encrypt.key"))?;
        let mut nonce = vec![0_u8; NONCE_BYTES];
        getrandom::fill(&mut nonce)
            .map_err(|_| port(PortErrorCode::Crypto, "secret.encrypt.nonce"))?;
        let aad = entry_aad(
            &self.keys.store_uuid,
            credential.credential_id(),
            credential.owner_scope(),
            credential.subject(),
            credential.purpose(),
            &destinations_json(credential.allowed_destinations())?,
            kind,
            generation,
        );
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: input.expose(),
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| port(PortErrorCode::Crypto, "secret.encrypt"))?;
        Ok((ciphertext, nonce))
    }

    fn authenticate_row(&self, row: &SecretRow) -> PortResult<ProtectedSecret> {
        authenticate_secret_row(&self.keys, row, false)
    }

    fn validate_reference(&self, credential: &CredentialRefV1, row: &SecretRow) -> PortResult<()> {
        let allowed = destinations_json(credential.allowed_destinations())?;
        if row.credential_id != credential.credential_id()
            || row.owner_scope != credential.owner_scope()
            || row.subject != credential.subject()
            || row.purpose != credential.purpose()
            || row.allowed_destinations_json != allowed
        {
            return Err(port(
                PortErrorCode::PermissionDenied,
                "secret.reference.binding",
            ));
        }
        Ok(())
    }
}

fn has_backup_evidence(
    database_path: &Path,
    backup_root: &Path,
) -> Result<bool, LocalStorageError> {
    if !backup_root.exists() {
        return Ok(false);
    }
    let database_name = database_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(LocalStorageError::InvalidData)?;
    for entry in fs::read_dir(backup_root)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name == database_name
            || name.starts_with(&format!("{database_name}."))
            || name.starts_with(&format!(".{database_name}."))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

impl SecretStorePort for LocalSecretStore {
    fn generation(&self, credential: &CredentialRefV1) -> PortResult<u64> {
        let connection = self.connection.borrow();
        if let Some(row) = read_entry(&connection, credential.credential_id())? {
            self.validate_reference(credential, &row)?;
        } else if let Some(absence) = read_absence(&connection, credential.credential_id())?
            && absence.owner_scope != credential.owner_scope()
        {
            return Err(port(
                PortErrorCode::PermissionDenied,
                "secret.generation.owner",
            ));
        }
        read_head(&connection, credential.credential_id())
    }

    fn fingerprint(&self, secret: &ProtectedSecret) -> PortResult<CanonicalDigest> {
        self.fingerprint_secret(secret)
    }

    fn resolve_secret(
        &self,
        subject: &VerifiedSecretSubjectV1,
        credential: &CredentialRefV1,
        purpose: &str,
        destination: &str,
        expected_generation: u64,
    ) -> PortResult<ProtectedSecret> {
        // All authorization and generation checks happen before ciphertext is selected/decrypted.
        if subject.subject() != credential.subject()
            || subject.owner_scope() != credential.owner_scope()
            || purpose != credential.purpose()
            || !credential.allowed_destinations().contains(destination)
            || expected_generation != credential.generation()
        {
            return Err(port(
                PortErrorCode::PermissionDenied,
                "secret.resolve.authorization",
            ));
        }
        let connection = self.connection.borrow();
        if read_head(&connection, credential.credential_id())? != expected_generation {
            return Err(port(PortErrorCode::Conflict, "secret.resolve.generation"));
        }
        let row = read_entry(&connection, credential.credential_id())?
            .ok_or_else(|| port(PortErrorCode::NotFound, "secret.resolve.missing"))?;
        self.validate_reference(credential, &row)?;
        if row.generation != expected_generation {
            return Err(port(PortErrorCode::Conflict, "secret.resolve.generation"));
        }
        self.authenticate_row(&row)
    }

    fn apply_secret(
        &self,
        operation_id: &OperationId,
        mutation: &SecretMutationV1,
        input: Option<&ProtectedSecret>,
    ) -> PortResult<OwnedEffectV1> {
        if mutation.credential().generation() != mutation.expected_generation() {
            return Err(port(
                PortErrorCode::Conflict,
                "secret.apply.reference_generation",
            ));
        }
        let fingerprint = match mutation.kind() {
            SecretMutationKind::Upsert => {
                let input = input.ok_or_else(|| {
                    port(PortErrorCode::InvalidData, "secret.apply.missing_input")
                })?;
                let actual = self.fingerprint_secret(input)?;
                if mutation.fingerprint() != Some(&actual) {
                    return Err(port(PortErrorCode::Conflict, "secret.apply.fingerprint"));
                }
                Some(actual)
            }
            SecretMutationKind::Delete => {
                if input.is_some() {
                    return Err(port(
                        PortErrorCode::InvalidData,
                        "secret.apply.unexpected_input",
                    ));
                }
                None
            }
        };
        let credential = mutation.credential();
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port(PortErrorCode::Unavailable, "secret.effect.begin"))?;
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM secret_effects
                    WHERE operation_id = ?1 AND credential_id = ?2)",
                params![operation_id.as_str(), credential.credential_id()],
                |row| row.get(0),
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "secret.effect.lookup"))?;
        if exists {
            drop(transaction);
            drop(connection);
            return match self.observe_secret(operation_id, mutation)? {
                EffectReconciliation::Staged(effect) | EffectReconciliation::Applied(effect) => {
                    Ok(effect)
                }
                EffectReconciliation::Missing => {
                    Err(port(PortErrorCode::Conflict, "secret.effect.compensated"))
                }
                EffectReconciliation::OwnershipLost(_) => {
                    Err(port(PortErrorCode::Conflict, "secret.effect.ownership"))
                }
            };
        }
        let before = read_entry(&transaction, credential.credential_id())?;
        let absence = read_absence(&transaction, credential.credential_id())?;
        if let Some(row) = &before {
            self.validate_reference(credential, row)?;
            // AEAD-open + HMAC verification precedes snapshotting or any durable write.
            self.authenticate_row(row)?;
        }
        if absence
            .as_ref()
            .is_some_and(|marker| marker.owner_scope != credential.owner_scope())
        {
            return Err(port(PortErrorCode::PermissionDenied, "secret.apply.owner"));
        }
        let current_generation = read_head(&transaction, credential.credential_id())?;
        if current_generation != mutation.expected_generation() {
            return Err(port(PortErrorCode::Conflict, "secret.apply.generation"));
        }
        let after_generation = current_generation
            .checked_add(1)
            .ok_or_else(|| port(PortErrorCode::Conflict, "secret.apply.generation_overflow"))?;
        let allowed = destinations_json(credential.allowed_destinations())?;
        let (ciphertext, nonce) = match (mutation.kind(), input) {
            (SecretMutationKind::Upsert, Some(input)) => {
                let encrypted =
                    self.encrypt(credential, "provider-api-key", after_generation, input)?;
                (Some(encrypted.0), Some(encrypted.1))
            }
            (SecretMutationKind::Delete, None) => (None, None),
            _ => return Err(port(PortErrorCode::InvalidData, "secret.apply.input")),
        };
        transaction
            .execute(
                "INSERT INTO secret_effects(
                    operation_id, credential_id, before_exists, before_owner_scope, before_kind,
                    before_ciphertext, before_nonce, before_aad_schema, before_key_version,
                    before_fingerprint, before_generation, before_owner_operation_id,
                    after_exists, after_fingerprint, after_generation, compensated,
                    staged_owner_scope, before_subject, before_purpose,
                    before_allowed_destinations_json, staged_subject, staged_purpose,
                    staged_allowed_destinations_json, staged_kind, staged_ciphertext, staged_nonce,
                    staged_aad_schema, staged_key_version, activated
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, 0, ?16, ?17, ?18, ?19, ?20, ?21, ?22,
                    ?23, ?24, ?25, ?26, ?27, 0
                 )",
                params![
                    operation_id.as_str(),
                    credential.credential_id(),
                    i64::from(before.is_some()),
                    before.as_ref().map(|row| row.owner_scope.as_str()),
                    before.as_ref().map(|row| row.kind.as_str()),
                    before.as_ref().map(|row| row.ciphertext.as_slice()),
                    before.as_ref().map(|row| row.nonce.as_slice()),
                    before.as_ref().map(|row| row.aad_schema.as_str()),
                    before.as_ref().map(|row| row.key_version),
                    before.as_ref().map(|row| row.fingerprint.as_str()),
                    current_generation,
                    before.as_ref().map(|row| row.owner_operation_id.as_str()),
                    i64::from(mutation.kind() == SecretMutationKind::Upsert),
                    fingerprint.as_ref().map(CanonicalDigest::as_str),
                    after_generation,
                    credential.owner_scope(),
                    before.as_ref().map(|row| row.subject.as_str()),
                    before.as_ref().map(|row| row.purpose.as_str()),
                    before
                        .as_ref()
                        .map(|row| row.allowed_destinations_json.as_str()),
                    credential.subject(),
                    credential.purpose(),
                    allowed,
                    "provider-api-key",
                    ciphertext,
                    nonce,
                    AAD_SCHEMA,
                    KEY_VERSION,
                ],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "secret.effect.journal"))?;
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "secret.effect.commit"))?;
        Ok(secret_effect(
            operation_id,
            credential,
            before
                .as_ref()
                .and_then(|row| CanonicalDigest::parse(&row.fingerprint).ok()),
            fingerprint,
            mutation.kind() == SecretMutationKind::Upsert,
            after_generation,
        ))
    }

    fn observe_secret(
        &self,
        operation_id: &OperationId,
        mutation: &SecretMutationV1,
    ) -> PortResult<EffectReconciliation> {
        let connection = self.connection.borrow();
        let Some(record) = read_effect(
            &connection,
            operation_id.as_str(),
            mutation.credential().credential_id(),
        )?
        else {
            return Ok(EffectReconciliation::Missing);
        };
        if record.compensated {
            return Ok(EffectReconciliation::Missing);
        }
        if record.owner_scope != mutation.credential().owner_scope()
            || record.after_generation != mutation.expected_generation() + 1
            || record.after_exists != (mutation.kind() == SecretMutationKind::Upsert)
        {
            return Ok(EffectReconciliation::OwnershipLost(
                record.effect(operation_id)?,
            ));
        }
        self.authenticate_before(&record)?;
        let effect = record.effect(operation_id)?;
        if !record.activated {
            self.authenticate_staged(&record)?;
            if !self.current_matches_before(&connection, &record)? {
                return Ok(EffectReconciliation::OwnershipLost(effect));
            }
            return Ok(EffectReconciliation::Staged(effect));
        }
        if self.current_matches_after(&connection, operation_id.as_str(), &record)? {
            Ok(EffectReconciliation::Applied(effect))
        } else {
            Ok(EffectReconciliation::OwnershipLost(effect))
        }
    }

    fn activate_secret(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1> {
        if effect.kind != OwnedEffectKind::Secret {
            return Err(port(PortErrorCode::InvalidData, "secret.activate.kind"));
        }
        let operation_id = compensation_text(effect, "operation_id", "secret.activate.operation")?;
        let credential_id =
            compensation_text(effect, "credential_id", "secret.activate.credential")?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port(PortErrorCode::Unavailable, "secret.activate.begin"))?;
        let record = read_effect(&transaction, operation_id, credential_id)?
            .ok_or_else(|| port(PortErrorCode::NotFound, "secret.activate.effect"))?;
        if record.compensated {
            return Err(port(PortErrorCode::Conflict, "secret.activate.compensated"));
        }
        if record.activated {
            return Ok(effect.clone());
        }
        self.authenticate_before(&record)?;
        self.authenticate_staged(&record)?;
        if !self.current_matches_before(&transaction, &record)? {
            return Err(port(PortErrorCode::Conflict, "secret.activate.current"));
        }
        if record.after_exists {
            transaction
                .execute(
                    "INSERT INTO secret_entries(
                        credential_id, owner_scope, kind, ciphertext, nonce, aad_schema,
                        key_version, fingerprint, generation, owner_operation_id,
                        created_at, updated_at, subject, purpose, allowed_destinations_json
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                               unixepoch(), unixepoch(), ?11, ?12, ?13)
                     ON CONFLICT(credential_id) DO UPDATE SET
                        owner_scope = excluded.owner_scope, kind = excluded.kind,
                        ciphertext = excluded.ciphertext, nonce = excluded.nonce,
                        aad_schema = excluded.aad_schema, key_version = excluded.key_version,
                        fingerprint = excluded.fingerprint, generation = excluded.generation,
                        owner_operation_id = excluded.owner_operation_id,
                        subject = excluded.subject, purpose = excluded.purpose,
                        allowed_destinations_json = excluded.allowed_destinations_json,
                        updated_at = excluded.updated_at",
                    params![
                        credential_id,
                        &record.owner_scope,
                        record.staged_kind.as_deref(),
                        record.staged_ciphertext.as_deref(),
                        record.staged_nonce.as_deref(),
                        record.staged_aad_schema.as_deref(),
                        record.staged_key_version,
                        record.after_fingerprint.as_deref(),
                        record.after_generation,
                        operation_id,
                        record.subject.as_deref(),
                        record.purpose.as_deref(),
                        record.allowed_destinations_json.as_deref(),
                    ],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "secret.activate.write"))?;
            transaction
                .execute(
                    "DELETE FROM secret_absence_markers WHERE credential_id = ?1",
                    params![credential_id],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "secret.activate.absence"))?;
        } else {
            transaction
                .execute(
                    "DELETE FROM secret_entries WHERE credential_id = ?1",
                    params![credential_id],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "secret.activate.delete"))?;
            transaction
                .execute(
                    "INSERT INTO secret_absence_markers(
                        credential_id, owner_scope, generation, owner_operation_id, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, unixepoch())
                     ON CONFLICT(credential_id) DO UPDATE SET
                        owner_scope = excluded.owner_scope, generation = excluded.generation,
                        owner_operation_id = excluded.owner_operation_id,
                        updated_at = excluded.updated_at",
                    params![
                        credential_id,
                        &record.owner_scope,
                        record.after_generation,
                        operation_id
                    ],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "secret.activate.absence"))?;
        }
        write_head(&transaction, credential_id, record.after_generation)?;
        transaction
            .execute(
                "UPDATE secret_effects SET activated = 1
                 WHERE operation_id = ?1 AND credential_id = ?2",
                params![operation_id, credential_id],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "secret.activate.mark"))?;
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "secret.activate.commit"))?;
        Ok(effect.clone())
    }

    fn compensate_secret(&self, effect: &OwnedEffectV1) -> PortResult<CompensationOutcome> {
        if effect.kind != OwnedEffectKind::Secret {
            return Err(port(PortErrorCode::InvalidData, "secret.compensate.kind"));
        }
        let operation_id =
            compensation_text(effect, "operation_id", "secret.compensate.operation")?;
        let credential_id =
            compensation_text(effect, "credential_id", "secret.compensate.credential")?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port(PortErrorCode::Unavailable, "secret.compensate.begin"))?;
        let record = read_effect(&transaction, operation_id, credential_id)?
            .ok_or_else(|| port(PortErrorCode::NotFound, "secret.compensate.effect"))?;
        if record.compensated {
            return Ok(CompensationOutcome::AlreadyCompensated);
        }
        let before_plaintext = self.authenticate_before(&record)?;
        self.authenticate_staged(&record)?;
        if !record.activated {
            mark_compensated(&transaction, operation_id, credential_id)?;
            transaction
                .commit()
                .map_err(|_| port(PortErrorCode::Unavailable, "secret.compensate.commit"))?;
            return Ok(CompensationOutcome::Compensated);
        }
        if !self.current_matches_after(&transaction, operation_id, &record)? {
            return Ok(CompensationOutcome::OwnershipLost);
        }
        let restored_generation = record.after_generation.checked_add(1).ok_or_else(|| {
            port(
                PortErrorCode::Conflict,
                "secret.compensate.generation_overflow",
            )
        })?;
        if record.before_exists {
            let before = record
                .before_row(restored_generation, operation_id)?
                .ok_or_else(|| port(PortErrorCode::Corrupt, "secret.compensate.before"))?;
            let plaintext = before_plaintext
                .ok_or_else(|| port(PortErrorCode::Corrupt, "secret.compensate.before"))?;
            let credential = CredentialRefV1::new(
                credential_id,
                before.owner_scope.clone(),
                before.subject.clone(),
                before.purpose.clone(),
                parse_destinations(&before.allowed_destinations_json)?,
                restored_generation,
            )
            .map_err(|_| port(PortErrorCode::Corrupt, "secret.compensate.reference"))?;
            let (ciphertext, nonce) =
                self.encrypt(&credential, &before.kind, restored_generation, &plaintext)?;
            transaction
                .execute(
                    "UPDATE secret_entries SET owner_scope = ?2, kind = ?3, ciphertext = ?4,
                        nonce = ?5, aad_schema = ?6, key_version = ?7, fingerprint = ?8,
                        generation = ?9, owner_operation_id = ?10, subject = ?11,
                        purpose = ?12, allowed_destinations_json = ?13, updated_at = unixepoch()
                     WHERE credential_id = ?1",
                    params![
                        credential_id,
                        &before.owner_scope,
                        &before.kind,
                        ciphertext,
                        nonce,
                        AAD_SCHEMA,
                        KEY_VERSION,
                        &before.fingerprint,
                        restored_generation,
                        operation_id,
                        &before.subject,
                        &before.purpose,
                        &before.allowed_destinations_json,
                    ],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "secret.compensate.restore"))?;
            transaction
                .execute(
                    "DELETE FROM secret_absence_markers WHERE credential_id = ?1",
                    params![credential_id],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "secret.compensate.absence"))?;
        } else {
            transaction
                .execute(
                    "DELETE FROM secret_entries WHERE credential_id = ?1",
                    params![credential_id],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "secret.compensate.delete"))?;
            transaction
                .execute(
                    "INSERT INTO secret_absence_markers(
                        credential_id, owner_scope, generation, owner_operation_id, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, unixepoch())
                     ON CONFLICT(credential_id) DO UPDATE SET
                        owner_scope = excluded.owner_scope, generation = excluded.generation,
                        owner_operation_id = excluded.owner_operation_id,
                        updated_at = excluded.updated_at",
                    params![
                        credential_id,
                        &record.owner_scope,
                        restored_generation,
                        operation_id
                    ],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "secret.compensate.absence"))?;
        }
        write_head(&transaction, credential_id, restored_generation)?;
        mark_compensated(&transaction, operation_id, credential_id)?;
        // Authenticate the restored committed row before allowing ROLLED_BACK.
        if let Some(restored) = read_entry(&transaction, credential_id)? {
            self.authenticate_row(&restored)?;
        }
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "secret.compensate.commit"))?;
        Ok(CompensationOutcome::Compensated)
    }

    fn inspect_agent_access_grant(
        &self,
        owner_scope: &str,
        connection_id: &str,
    ) -> PortResult<Option<AgentAccessGrantRefV1>> {
        grant::inspect(self, owner_scope, connection_id)
    }

    fn resolve_agent_access_grant(
        &self,
        reference: &AgentAccessGrantRefV1,
    ) -> PortResult<AgentAccessGrantMaterial> {
        grant::resolve(self, reference)
    }

    fn apply_agent_access_grant(
        &self,
        operation_id: &OperationId,
        mutation: &AgentAccessGrantMutationV1,
        input: Option<&ProtectedSecret>,
    ) -> PortResult<OwnedEffectV1> {
        grant::apply(self, operation_id, mutation, input)
    }
    fn observe_agent_access_grant(
        &self,
        operation_id: &OperationId,
        mutation: &AgentAccessGrantMutationV1,
    ) -> PortResult<EffectReconciliation> {
        grant::observe(self, operation_id, mutation)
    }

    fn activate_agent_access_grant(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1> {
        grant::activate(self, effect)
    }

    fn compensate_agent_access_grant(
        &self,
        effect: &OwnedEffectV1,
    ) -> PortResult<CompensationOutcome> {
        grant::compensate(self, effect)
    }
}

impl LocalSecretStore {
    fn authenticate_before(&self, record: &EffectRecord) -> PortResult<Option<ProtectedSecret>> {
        record
            .before_row(record.before_generation, "snapshot")?
            // Legacy AAD is reachable only through an already durable compensation snapshot.
            // Startup rewrites every snapshot whose full v2 identity can be authenticated.
            .map(|row| authenticate_secret_row(&self.keys, &row, true))
            .transpose()
    }

    fn authenticate_staged(&self, record: &EffectRecord) -> PortResult<()> {
        if !record.after_exists {
            if record.staged_ciphertext.is_some() || record.staged_nonce.is_some() {
                return Err(port(PortErrorCode::Corrupt, "secret.stage.delete"));
            }
            return Ok(());
        }
        let row = record.staged_row()?;
        self.authenticate_row(&row)?;
        Ok(())
    }

    fn current_matches_before(
        &self,
        connection: &Connection,
        record: &EffectRecord,
    ) -> PortResult<bool> {
        if read_head(connection, &record.credential_id)? != record.before_generation {
            return Ok(false);
        }
        let current = read_entry(connection, &record.credential_id)?;
        if record.before_exists {
            let Some(current) = current else {
                return Ok(false);
            };
            self.authenticate_row(&current)?;
            Ok(
                current.fingerprint == record.before_fingerprint.as_deref().unwrap_or_default()
                    && current.generation == record.before_generation
                    && current.owner_scope
                        == record.before_owner_scope.as_deref().unwrap_or_default(),
            )
        } else {
            Ok(current.is_none())
        }
    }

    fn current_matches_after(
        &self,
        connection: &Connection,
        operation_id: &str,
        record: &EffectRecord,
    ) -> PortResult<bool> {
        if read_head(connection, &record.credential_id)? != record.after_generation {
            return Ok(false);
        }
        if record.after_exists {
            let Some(current) = read_entry(connection, &record.credential_id)? else {
                return Ok(false);
            };
            self.authenticate_row(&current)?;
            Ok(current.generation == record.after_generation
                && current.owner_operation_id == operation_id
                && Some(current.fingerprint.as_str()) == record.after_fingerprint.as_deref())
        } else {
            let absence = read_absence(connection, &record.credential_id)?;
            Ok(read_entry(connection, &record.credential_id)?.is_none()
                && absence.is_some_and(|marker| {
                    marker.generation == record.after_generation
                        && marker.owner_operation_id == operation_id
                        && marker.owner_scope == record.owner_scope
                }))
        }
    }
}

#[derive(Clone)]
struct SecretRow {
    credential_id: String,
    owner_scope: String,
    kind: String,
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
    aad_schema: String,
    key_version: u32,
    fingerprint: String,
    generation: u64,
    owner_operation_id: String,
    subject: String,
    purpose: String,
    allowed_destinations_json: String,
}

struct AbsenceRow {
    owner_scope: String,
    generation: u64,
    owner_operation_id: String,
}

struct EffectRecord {
    credential_id: String,
    before_exists: bool,
    before_owner_scope: Option<String>,
    before_kind: Option<String>,
    before_ciphertext: Option<Vec<u8>>,
    before_nonce: Option<Vec<u8>>,
    before_aad_schema: Option<String>,
    before_key_version: Option<u32>,
    before_fingerprint: Option<String>,
    before_generation: u64,
    before_owner_operation_id: Option<String>,
    after_exists: bool,
    after_fingerprint: Option<String>,
    after_generation: u64,
    compensated: bool,
    owner_scope: String,
    before_subject: Option<String>,
    before_purpose: Option<String>,
    before_allowed_destinations_json: Option<String>,
    subject: Option<String>,
    purpose: Option<String>,
    allowed_destinations_json: Option<String>,
    staged_kind: Option<String>,
    staged_ciphertext: Option<Vec<u8>>,
    staged_nonce: Option<Vec<u8>>,
    staged_aad_schema: Option<String>,
    staged_key_version: Option<u32>,
    activated: bool,
}

impl EffectRecord {
    fn effect(&self, operation_id: &OperationId) -> PortResult<OwnedEffectV1> {
        let before = self
            .before_fingerprint
            .as_deref()
            .map(CanonicalDigest::parse)
            .transpose()
            .map_err(|_| port(PortErrorCode::Corrupt, "secret.effect.before_digest"))?;
        let after = self
            .after_fingerprint
            .as_deref()
            .map(CanonicalDigest::parse)
            .transpose()
            .map_err(|_| port(PortErrorCode::Corrupt, "secret.effect.after_digest"))?;
        let credential = CredentialRefV1::new(
            self.credential_id.clone(),
            self.owner_scope.clone(),
            self.subject.clone().unwrap_or_else(|| "deleted".to_owned()),
            self.purpose.clone().unwrap_or_else(|| "deleted".to_owned()),
            self.allowed_destinations_json
                .as_deref()
                .map(parse_destinations)
                .transpose()?
                .unwrap_or_else(|| ["deleted".to_owned()].into_iter().collect()),
            self.before_generation,
        )
        .map_err(|_| port(PortErrorCode::Corrupt, "secret.effect.reference"))?;
        Ok(secret_effect(
            operation_id,
            &credential,
            before,
            after,
            self.after_exists,
            self.after_generation,
        ))
    }

    fn before_row(
        &self,
        generation: u64,
        owner_operation_id: &str,
    ) -> PortResult<Option<SecretRow>> {
        if !self.before_exists {
            if self.before_ciphertext.is_some() || self.before_nonce.is_some() {
                return Err(port(PortErrorCode::Corrupt, "secret.effect.before_absent"));
            }
            return Ok(None);
        }
        Ok(Some(SecretRow {
            credential_id: self.credential_id.clone(),
            owner_scope: required(&self.before_owner_scope, "secret.effect.before_owner")?,
            kind: required(&self.before_kind, "secret.effect.before_kind")?,
            ciphertext: required(&self.before_ciphertext, "secret.effect.before_ciphertext")?,
            nonce: required(&self.before_nonce, "secret.effect.before_nonce")?,
            aad_schema: required(&self.before_aad_schema, "secret.effect.before_aad")?,
            key_version: self
                .before_key_version
                .ok_or_else(|| port(PortErrorCode::Corrupt, "secret.effect.before_key"))?,
            fingerprint: required(&self.before_fingerprint, "secret.effect.before_fingerprint")?,
            generation,
            owner_operation_id: self
                .before_owner_operation_id
                .clone()
                .unwrap_or_else(|| owner_operation_id.to_owned()),
            subject: required(&self.before_subject, "secret.effect.before_subject")?,
            purpose: required(&self.before_purpose, "secret.effect.before_purpose")?,
            allowed_destinations_json: required(
                &self.before_allowed_destinations_json,
                "secret.effect.before_destinations",
            )?,
        }))
    }

    fn staged_row(&self) -> PortResult<SecretRow> {
        Ok(SecretRow {
            credential_id: self.credential_id.clone(),
            owner_scope: self.owner_scope.clone(),
            kind: required(&self.staged_kind, "secret.stage.kind")?,
            ciphertext: required(&self.staged_ciphertext, "secret.stage.ciphertext")?,
            nonce: required(&self.staged_nonce, "secret.stage.nonce")?,
            aad_schema: required(&self.staged_aad_schema, "secret.stage.aad")?,
            key_version: self
                .staged_key_version
                .ok_or_else(|| port(PortErrorCode::Corrupt, "secret.stage.key"))?,
            fingerprint: required(&self.after_fingerprint, "secret.stage.fingerprint")?,
            generation: self.after_generation,
            owner_operation_id: "staged".to_owned(),
            subject: required(&self.subject, "secret.stage.subject")?,
            purpose: required(&self.purpose, "secret.stage.purpose")?,
            allowed_destinations_json: required(
                &self.allowed_destinations_json,
                "secret.stage.destinations",
            )?,
        })
    }
}

/// Authenticates all ciphertext first, then rewrites every legacy row whose complete identity is
/// known in one SQLite transaction. Pre-v3 rows deliberately carry `legacy-locked` placeholders;
/// those bytes remain recoverable under their original AAD but can never enter the live resolve
/// path because no subject, purpose, or destination may be invented during migration.
fn migrate_legacy_secret_aad(
    connection: &mut Connection,
    keys: &KeyMaterial,
) -> Result<(), LocalStorageError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let entry_ids = {
        let mut statement = transaction.prepare("SELECT credential_id FROM secret_entries")?;
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    let entries = entry_ids
        .iter()
        .map(|credential_id| {
            read_entry(&transaction, credential_id)
                .map_err(|_| LocalStorageError::Locked)?
                .ok_or(LocalStorageError::Locked)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let effect_ids = {
        let mut statement = transaction.prepare(
            "SELECT operation_id, credential_id FROM secret_effects
             ORDER BY operation_id, credential_id",
        )?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    let effects = effect_ids
        .iter()
        .map(|(operation_id, credential_id)| {
            read_effect(&transaction, operation_id, credential_id)
                .map_err(|_| LocalStorageError::Locked)?
                .map(|record| (operation_id.clone(), record))
                .ok_or(LocalStorageError::Locked)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut entry_updates = Vec::new();
    for row in &entries {
        if let Some((ciphertext, nonce)) = rewrap_secret_row(keys, row)? {
            entry_updates.push((row.credential_id.clone(), ciphertext, nonce));
        }
    }

    enum EffectSlot {
        Before,
        Staged,
    }
    let mut effect_updates = Vec::new();
    for (operation_id, record) in &effects {
        if record.before_exists {
            let before = SecretRow {
                credential_id: record.credential_id.clone(),
                owner_scope: record
                    .before_owner_scope
                    .clone()
                    .ok_or(LocalStorageError::Locked)?,
                kind: record
                    .before_kind
                    .clone()
                    .ok_or(LocalStorageError::Locked)?,
                ciphertext: record
                    .before_ciphertext
                    .clone()
                    .ok_or(LocalStorageError::Locked)?,
                nonce: record
                    .before_nonce
                    .clone()
                    .ok_or(LocalStorageError::Locked)?,
                aad_schema: record
                    .before_aad_schema
                    .clone()
                    .ok_or(LocalStorageError::Locked)?,
                key_version: record.before_key_version.ok_or(LocalStorageError::Locked)?,
                fingerprint: record
                    .before_fingerprint
                    .clone()
                    .ok_or(LocalStorageError::Locked)?,
                generation: record.before_generation,
                owner_operation_id: record
                    .before_owner_operation_id
                    .clone()
                    .unwrap_or_else(|| operation_id.clone()),
                subject: record
                    .before_subject
                    .clone()
                    .unwrap_or_else(|| "legacy-locked".to_owned()),
                purpose: record
                    .before_purpose
                    .clone()
                    .unwrap_or_else(|| "legacy-locked".to_owned()),
                allowed_destinations_json: record
                    .before_allowed_destinations_json
                    .clone()
                    .unwrap_or_else(|| "[]".to_owned()),
            };
            if let Some((ciphertext, nonce)) = rewrap_secret_row(keys, &before)? {
                effect_updates.push((
                    operation_id.clone(),
                    record.credential_id.clone(),
                    EffectSlot::Before,
                    ciphertext,
                    nonce,
                ));
            }
        }
        let staged_present = record.staged_ciphertext.is_some()
            || record.staged_nonce.is_some()
            || record.staged_aad_schema.is_some()
            || record.staged_key_version.is_some();
        if staged_present && record.after_exists {
            let staged = record.staged_row().map_err(|_| LocalStorageError::Locked)?;
            if let Some((ciphertext, nonce)) = rewrap_secret_row(keys, &staged)? {
                effect_updates.push((
                    operation_id.clone(),
                    record.credential_id.clone(),
                    EffectSlot::Staged,
                    ciphertext,
                    nonce,
                ));
            }
        } else if staged_present || (record.after_exists && !record.activated) {
            return Err(LocalStorageError::Locked);
        }
    }

    for (credential_id, ciphertext, nonce) in entry_updates {
        transaction.execute(
            "UPDATE secret_entries
             SET ciphertext = ?2, nonce = ?3, aad_schema = ?4, updated_at = unixepoch()
             WHERE credential_id = ?1 AND aad_schema = ?5",
            params![
                credential_id,
                ciphertext,
                nonce,
                AAD_SCHEMA,
                LEGACY_AAD_SCHEMA
            ],
        )?;
    }
    for (operation_id, credential_id, slot, ciphertext, nonce) in effect_updates {
        match slot {
            EffectSlot::Before => transaction.execute(
                "UPDATE secret_effects
                 SET before_ciphertext = ?3, before_nonce = ?4, before_aad_schema = ?5
                 WHERE operation_id = ?1 AND credential_id = ?2 AND before_aad_schema = ?6",
                params![
                    operation_id,
                    credential_id,
                    ciphertext,
                    nonce,
                    AAD_SCHEMA,
                    LEGACY_AAD_SCHEMA
                ],
            )?,
            EffectSlot::Staged => transaction.execute(
                "UPDATE secret_effects
                 SET staged_ciphertext = ?3, staged_nonce = ?4, staged_aad_schema = ?5
                 WHERE operation_id = ?1 AND credential_id = ?2 AND staged_aad_schema = ?6",
                params![
                    operation_id,
                    credential_id,
                    ciphertext,
                    nonce,
                    AAD_SCHEMA,
                    LEGACY_AAD_SCHEMA
                ],
            )?,
        };
    }
    transaction.commit()?;
    Ok(())
}

type RewrappedSecretRow = (Vec<u8>, Vec<u8>);

fn rewrap_secret_row(
    keys: &KeyMaterial,
    row: &SecretRow,
) -> Result<Option<RewrappedSecretRow>, LocalStorageError> {
    let plaintext =
        authenticate_secret_row(keys, row, true).map_err(|_| LocalStorageError::Locked)?;
    if row.aad_schema == AAD_SCHEMA {
        return Ok(None);
    }
    if row.subject == "legacy-locked"
        && row.purpose == "legacy-locked"
        && row.allowed_destinations_json == "[]"
    {
        return Ok(None);
    }
    let destinations = parse_destinations(&row.allowed_destinations_json)
        .map_err(|_| LocalStorageError::Locked)?;
    if CredentialRefV1::new(
        row.credential_id.clone(),
        row.owner_scope.clone(),
        row.subject.clone(),
        row.purpose.clone(),
        destinations,
        row.generation,
    )
    .is_err()
    {
        return Err(LocalStorageError::Locked);
    }
    let mut nonce = vec![0_u8; NONCE_BYTES];
    getrandom::fill(&mut nonce).map_err(|_| LocalStorageError::Crypto)?;
    let aad = entry_aad(
        &keys.store_uuid,
        &row.credential_id,
        &row.owner_scope,
        &row.subject,
        &row.purpose,
        &row.allowed_destinations_json,
        &row.kind,
        row.generation,
    );
    let cipher =
        Aes256Gcm::new_from_slice(keys.encryption()).map_err(|_| LocalStorageError::Crypto)?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext.expose(),
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| LocalStorageError::Crypto)?;
    Ok(Some((ciphertext, nonce)))
}

fn authenticate_secret_row(
    keys: &KeyMaterial,
    row: &SecretRow,
    allow_legacy_recovery: bool,
) -> PortResult<ProtectedSecret> {
    if row.nonce.len() != NONCE_BYTES || row.key_version != KEY_VERSION {
        return Err(port(PortErrorCode::Corrupt, "secret.decrypt.metadata"));
    }
    let aad = match row.aad_schema.as_str() {
        AAD_SCHEMA => entry_aad(
            &keys.store_uuid,
            &row.credential_id,
            &row.owner_scope,
            &row.subject,
            &row.purpose,
            &row.allowed_destinations_json,
            &row.kind,
            row.generation,
        ),
        LEGACY_AAD_SCHEMA if allow_legacy_recovery => {
            legacy_aad(&row.credential_id, &row.owner_scope, &row.kind)
        }
        _ => return Err(port(PortErrorCode::Corrupt, "secret.decrypt.aad_schema")),
    };
    let cipher = Aes256Gcm::new_from_slice(keys.encryption())
        .map_err(|_| port(PortErrorCode::Crypto, "secret.decrypt.key"))?;
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&row.nonce),
            Payload {
                msg: &row.ciphertext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| port(PortErrorCode::Crypto, "secret.decrypt"))?;
    let protected = ProtectedSecret::new(plaintext)
        .map_err(|_| port(PortErrorCode::Corrupt, "secret.decrypt.empty"))?;
    if fingerprint_with_key(keys.fingerprint(), protected.expose())?.as_str() != row.fingerprint {
        return Err(port(PortErrorCode::Corrupt, "secret.fingerprint.integrity"));
    }
    Ok(protected)
}

fn read_entry(connection: &Connection, credential_id: &str) -> PortResult<Option<SecretRow>> {
    connection
        .query_row(
            "SELECT credential_id, owner_scope, kind, ciphertext, nonce, aad_schema,
                    key_version, fingerprint, generation, owner_operation_id,
                    subject, purpose, allowed_destinations_json
             FROM secret_entries WHERE credential_id = ?1",
            params![credential_id],
            |row| {
                Ok(SecretRow {
                    credential_id: row.get(0)?,
                    owner_scope: row.get(1)?,
                    kind: row.get(2)?,
                    ciphertext: row.get(3)?,
                    nonce: row.get(4)?,
                    aad_schema: row.get(5)?,
                    key_version: row.get(6)?,
                    fingerprint: row.get(7)?,
                    generation: row.get(8)?,
                    owner_operation_id: row.get(9)?,
                    subject: row.get(10)?,
                    purpose: row.get(11)?,
                    allowed_destinations_json: row.get(12)?,
                })
            },
        )
        .optional()
        .map_err(|_| port(PortErrorCode::Unavailable, "secret.entry.read"))
}

fn read_absence(connection: &Connection, credential_id: &str) -> PortResult<Option<AbsenceRow>> {
    connection
        .query_row(
            "SELECT owner_scope, generation, owner_operation_id
             FROM secret_absence_markers WHERE credential_id = ?1",
            params![credential_id],
            |row| {
                Ok(AbsenceRow {
                    owner_scope: row.get(0)?,
                    generation: row.get(1)?,
                    owner_operation_id: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|_| port(PortErrorCode::Unavailable, "secret.absence.read"))
}

fn read_effect(
    connection: &Connection,
    operation_id: &str,
    credential_id: &str,
) -> PortResult<Option<EffectRecord>> {
    connection
        .query_row(
            "SELECT credential_id, before_exists, before_owner_scope, before_kind,
                    before_ciphertext, before_nonce, before_aad_schema, before_key_version,
                    before_fingerprint, before_generation, before_owner_operation_id,
                    after_exists, after_fingerprint, after_generation, compensated,
                    staged_owner_scope, before_subject, before_purpose,
                    before_allowed_destinations_json, staged_subject, staged_purpose,
                    staged_allowed_destinations_json, staged_kind, staged_ciphertext, staged_nonce,
                    staged_aad_schema, staged_key_version, activated
             FROM secret_effects WHERE operation_id = ?1 AND credential_id = ?2",
            params![operation_id, credential_id],
            |row| {
                Ok(EffectRecord {
                    credential_id: row.get(0)?,
                    before_exists: row.get(1)?,
                    before_owner_scope: row.get(2)?,
                    before_kind: row.get(3)?,
                    before_ciphertext: row.get(4)?,
                    before_nonce: row.get(5)?,
                    before_aad_schema: row.get(6)?,
                    before_key_version: row.get(7)?,
                    before_fingerprint: row.get(8)?,
                    before_generation: row.get(9)?,
                    before_owner_operation_id: row.get(10)?,
                    after_exists: row.get(11)?,
                    after_fingerprint: row.get(12)?,
                    after_generation: row.get(13)?,
                    compensated: row.get(14)?,
                    owner_scope: row.get(15)?,
                    before_subject: row.get(16)?,
                    before_purpose: row.get(17)?,
                    before_allowed_destinations_json: row.get(18)?,
                    subject: row.get(19)?,
                    purpose: row.get(20)?,
                    allowed_destinations_json: row.get(21)?,
                    staged_kind: row.get(22)?,
                    staged_ciphertext: row.get(23)?,
                    staged_nonce: row.get(24)?,
                    staged_aad_schema: row.get(25)?,
                    staged_key_version: row.get(26)?,
                    activated: row.get(27)?,
                })
            },
        )
        .optional()
        .map_err(|_| port(PortErrorCode::Unavailable, "secret.effect.read"))
}

fn read_head(connection: &Connection, credential_id: &str) -> PortResult<u64> {
    connection
        .query_row(
            "SELECT generation FROM secret_generation_heads WHERE credential_id = ?1",
            params![credential_id],
            |row| row.get(0),
        )
        .optional()
        .map(|generation| generation.unwrap_or(0))
        .map_err(|_| port(PortErrorCode::Unavailable, "secret.generation"))
}

fn write_head(connection: &Connection, credential_id: &str, generation: u64) -> PortResult<()> {
    connection
        .execute(
            "INSERT INTO secret_generation_heads(credential_id, generation) VALUES (?1, ?2)
             ON CONFLICT(credential_id) DO UPDATE SET generation = excluded.generation",
            params![credential_id, generation],
        )
        .map_err(|_| port(PortErrorCode::Unavailable, "secret.generation.write"))?;
    Ok(())
}

fn mark_compensated(
    connection: &Connection,
    operation_id: &str,
    credential_id: &str,
) -> PortResult<()> {
    connection
        .execute(
            "UPDATE secret_effects SET compensated = 1
             WHERE operation_id = ?1 AND credential_id = ?2",
            params![operation_id, credential_id],
        )
        .map_err(|_| port(PortErrorCode::Unavailable, "secret.compensate.mark"))?;
    Ok(())
}

fn secret_effect(
    operation_id: &OperationId,
    credential: &CredentialRefV1,
    before: Option<CanonicalDigest>,
    after: Option<CanonicalDigest>,
    after_exists: bool,
    generation: u64,
) -> OwnedEffectV1 {
    OwnedEffectV1 {
        effect_id: format!("secret:{}", credential.credential_id()),
        kind: OwnedEffectKind::Secret,
        target: credential.credential_id().to_owned(),
        before_fingerprint: before,
        after_fingerprint: after,
        compensation: json!({
            "schema": "hiroute.secret-compensation/v1",
            "operation_id": operation_id.as_str(),
            "credential_id": credential.credential_id(),
            "owner_scope": credential.owner_scope(),
            "after_exists": after_exists,
            "after_generation": generation,
        })
        .into(),
    }
}

fn destinations_json(destinations: &BTreeSet<String>) -> PortResult<String> {
    serde_json::to_string(destinations)
        .map_err(|_| port(PortErrorCode::InvalidData, "secret.destinations.encode"))
}

fn parse_destinations(value: &str) -> PortResult<BTreeSet<String>> {
    serde_json::from_str(value)
        .map_err(|_| port(PortErrorCode::Corrupt, "secret.destinations.decode"))
}

#[allow(clippy::too_many_arguments)]
fn entry_aad(
    store_uuid: &str,
    credential_id: &str,
    owner_scope: &str,
    subject: &str,
    purpose: &str,
    allowed_destinations_json: &str,
    kind: &str,
    generation: u64,
) -> String {
    format!(
        "{AAD_SCHEMA}\0{KEY_VERSION}\0{store_uuid}\0{credential_id}\0{owner_scope}\0{subject}\0{purpose}\0{allowed_destinations_json}\0{kind}\0{generation}"
    )
}

fn legacy_aad(credential_id: &str, owner_scope: &str, kind: &str) -> String {
    format!("{LEGACY_AAD_SCHEMA}\0{KEY_VERSION}\0{credential_id}\0{owner_scope}\0{kind}")
}

fn fingerprint_with_key(key: &[u8], bytes: &[u8]) -> PortResult<CanonicalDigest> {
    type HmacSha256 = Hmac<Sha256>;
    let mut hmac = <HmacSha256 as Mac>::new_from_slice(key)
        .map_err(|_| port(PortErrorCode::Crypto, "secret.fingerprint.key"))?;
    hmac.update(b"hiroute-secret-fingerprint-v1\0");
    hmac.update(bytes);
    CanonicalDigest::parse(format!(
        "sha256:{}",
        hex_bytes(&hmac.finalize().into_bytes())
    ))
    .map_err(|_| port(PortErrorCode::Crypto, "secret.fingerprint.digest"))
}

fn preflight_existing_store(path: &Path, keys: &KeyMaterial) -> Result<(), LocalStorageError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| LocalStorageError::Locked)?;
    let version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|_| LocalStorageError::Locked)?;
    if version >= 3 {
        let binding = connection
            .query_row(
                "SELECT store_uuid, key_id, key_verifier FROM secret_store_meta WHERE singleton = 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
            )
            .map_err(|_| LocalStorageError::Locked)?;
        if !binding.0.is_empty()
            && (binding.0 != keys.store_uuid
                || binding.1 != keys.key_id.as_str()
                || binding.2 != keys.verifier.as_str())
        {
            return Err(LocalStorageError::Locked);
        }
        if binding.0.is_empty() {
            let count: u64 = connection
                .query_row(
                    "SELECT
                        (SELECT count(*) FROM secret_entries)
                      + (SELECT count(*) FROM secret_effects)
                      + (SELECT count(*) FROM secret_absence_markers)
                      + (SELECT count(*) FROM secret_generation_heads)",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| LocalStorageError::Locked)?;
            if count != 0 {
                return Err(LocalStorageError::Locked);
            }
        }
    } else if version > 0 {
        // Legacy key proof without writing. A wrong key fails AEAD/HMAC and locks the store.
        let row = connection
            .query_row(
                "SELECT credential_id, owner_scope, kind, ciphertext, nonce, aad_schema,
                        key_version, fingerprint, generation, owner_operation_id
                 FROM secret_entries LIMIT 1",
                [],
                |row| {
                    Ok(SecretRow {
                        credential_id: row.get(0)?,
                        owner_scope: row.get(1)?,
                        kind: row.get(2)?,
                        ciphertext: row.get(3)?,
                        nonce: row.get(4)?,
                        aad_schema: row.get(5)?,
                        key_version: row.get(6)?,
                        fingerprint: row.get(7)?,
                        generation: row.get(8)?,
                        owner_operation_id: row.get(9)?,
                        subject: "legacy-locked".to_owned(),
                        purpose: "legacy-locked".to_owned(),
                        allowed_destinations_json: "[]".to_owned(),
                    })
                },
            )
            .optional()
            .map_err(|_| LocalStorageError::Locked)?;
        if let Some(row) = row {
            authenticate_preflight(keys, &row).map_err(|_| LocalStorageError::Locked)?;
        }
    }
    Ok(())
}

fn authenticate_preflight(keys: &KeyMaterial, row: &SecretRow) -> PortResult<()> {
    if row.nonce.len() != NONCE_BYTES || row.aad_schema != LEGACY_AAD_SCHEMA {
        return Err(port(PortErrorCode::Crypto, "secret.preflight.metadata"));
    }
    let cipher = Aes256Gcm::new_from_slice(keys.encryption())
        .map_err(|_| port(PortErrorCode::Crypto, "secret.preflight.key"))?;
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&row.nonce),
            Payload {
                msg: &row.ciphertext,
                aad: legacy_aad(&row.credential_id, &row.owner_scope, &row.kind).as_bytes(),
            },
        )
        .map_err(|_| port(PortErrorCode::Crypto, "secret.preflight.decrypt"))?;
    if fingerprint_with_key(keys.fingerprint(), &plaintext)?.as_str() != row.fingerprint {
        return Err(port(PortErrorCode::Crypto, "secret.preflight.fingerprint"));
    }
    Ok(())
}

fn bind_store_metadata(
    connection: &Connection,
    keys: &KeyMaterial,
) -> Result<(), LocalStorageError> {
    let current = connection.query_row(
        "SELECT store_uuid, key_id, key_verifier FROM secret_store_meta WHERE singleton = 1",
        [],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    )?;
    if current.0.is_empty() {
        connection.execute(
            "UPDATE secret_store_meta SET store_uuid = ?1, key_id = ?2, key_verifier = ?3,
                    updated_at = unixepoch() WHERE singleton = 1",
            params![
                keys.store_uuid,
                keys.key_id.as_str(),
                keys.verifier.as_str()
            ],
        )?;
    } else if current.0 != keys.store_uuid
        || current.1 != keys.key_id.as_str()
        || current.2 != keys.verifier.as_str()
    {
        return Err(LocalStorageError::Locked);
    }
    Ok(())
}

fn load_key_material(path: &Path) -> Result<Option<KeyMaterial>, LocalStorageError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_owner_file(&metadata)?;
            let mut file = File::open(path)?;
            let mut header = [0_u8; 8];
            file.read_exact(&mut header)?;
            if &header != MASTER_KEY_HEADER {
                return Err(LocalStorageError::InvalidData);
            }
            let mut uuid = vec![0_u8; STORE_UUID_HEX_BYTES];
            let mut key_id = vec![0_u8; DIGEST_TEXT_BYTES];
            let mut verifier = vec![0_u8; DIGEST_TEXT_BYTES];
            let mut bytes = Zeroizing::new(vec![0_u8; MASTER_KEY_BYTES]);
            file.read_exact(&mut uuid)?;
            file.read_exact(&mut key_id)?;
            file.read_exact(&mut verifier)?;
            file.read_exact(&mut bytes)?;
            let mut trailing = [0_u8; 1];
            if file.read(&mut trailing)? != 0 {
                return Err(LocalStorageError::InvalidData);
            }
            let store_uuid = String::from_utf8(uuid).map_err(|_| LocalStorageError::InvalidData)?;
            let key_id = CanonicalDigest::parse(
                String::from_utf8(key_id).map_err(|_| LocalStorageError::InvalidData)?,
            )
            .map_err(|_| LocalStorageError::InvalidData)?;
            let verifier = CanonicalDigest::parse(
                String::from_utf8(verifier).map_err(|_| LocalStorageError::InvalidData)?,
            )
            .map_err(|_| LocalStorageError::InvalidData)?;
            let computed_id = key_id_for(&bytes);
            let computed_verifier = key_verifier_for(&bytes);
            if computed_id != key_id || computed_verifier != verifier {
                return Err(LocalStorageError::Locked);
            }
            Ok(Some(KeyMaterial {
                bytes,
                store_uuid,
                key_id,
                verifier,
            }))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn create_key_material(path: &Path) -> Result<KeyMaterial, LocalStorageError> {
    let parent = path.parent().ok_or(LocalStorageError::InvalidData)?;
    prepare_owner_directory(parent)?;
    let mut bytes = Zeroizing::new(vec![0_u8; MASTER_KEY_BYTES]);
    let mut uuid = [0_u8; STORE_UUID_BYTES];
    getrandom::fill(&mut bytes).map_err(|_| LocalStorageError::Crypto)?;
    getrandom::fill(&mut uuid).map_err(|_| LocalStorageError::Crypto)?;
    let store_uuid = hex_bytes(&uuid);
    let key_id = key_id_for(&bytes);
    let verifier = key_verifier_for(&bytes);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(MASTER_KEY_HEADER)?;
    file.write_all(store_uuid.as_bytes())?;
    file.write_all(key_id.as_str().as_bytes())?;
    file.write_all(verifier.as_str().as_bytes())?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    File::open(parent)?.sync_all()?;
    validate_owner_file(&fs::metadata(path)?)?;
    Ok(KeyMaterial {
        bytes,
        store_uuid,
        key_id,
        verifier,
    })
}

fn key_id_for(bytes: &[u8]) -> CanonicalDigest {
    let mut material = b"hiroute-secret-key-id-v1\0".to_vec();
    material.extend_from_slice(&bytes[..32]);
    CanonicalDigest::of_bytes(&material)
}

fn key_verifier_for(bytes: &[u8]) -> CanonicalDigest {
    let mut material = b"hiroute-secret-key-verifier-v1\0".to_vec();
    material.extend_from_slice(bytes);
    CanonicalDigest::of_bytes(&material)
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

fn validate_owner_file(metadata: &fs::Metadata) -> Result<(), LocalStorageError> {
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

fn required<T: Clone>(value: &Option<T>, context: &'static str) -> PortResult<T> {
    value
        .clone()
        .ok_or_else(|| port(PortErrorCode::Corrupt, context))
}

fn compensation_text<'a>(
    effect: &'a OwnedEffectV1,
    field: &str,
    context: &'static str,
) -> PortResult<&'a str> {
    effect
        .compensation
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| port(PortErrorCode::InvalidData, context))
}

fn hex_bytes(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn port(code: PortErrorCode, context: &'static str) -> PortError {
    PortError::new(code, context)
}

#[cfg(test)]
mod tests {
    use crate::test_tempdir as tempdir;
    use hiroute_domain::VerifiedSecretSubjectV1;

    use super::*;

    fn operation_id(value: char) -> OperationId {
        OperationId::parse(format!("op_{}", value.to_string().repeat(32))).unwrap()
    }

    fn reference(generation: u64) -> CredentialRefV1 {
        CredentialRefV1::new(
            "credential-a",
            "connection-a",
            "hirouted",
            "provider-auth",
            ["provider-api".to_owned()],
            generation,
        )
        .unwrap()
    }

    fn mutation(
        store: &LocalSecretStore,
        secret: &ProtectedSecret,
        generation: u64,
    ) -> SecretMutationV1 {
        SecretMutationV1::upsert(
            reference(generation),
            generation,
            "primary",
            Some(store.fingerprint(secret).unwrap()),
        )
        .unwrap()
    }

    fn store() -> (tempfile::TempDir, LocalSecretStore) {
        let directory = tempdir().unwrap();
        let root = directory.path().join("data");
        let store = LocalSecretStore::open(
            &crate::test_storage_authority(),
            root.join("secrets.db"),
            root.join("master-key"),
            root.join("backups"),
        )
        .unwrap();
        (directory, store)
    }

    fn legacy_ciphertext(
        store: &LocalSecretStore,
        credential_id: &str,
        owner_scope: &str,
        kind: &str,
        plaintext: &[u8],
    ) -> (Vec<u8>, Vec<u8>) {
        let cipher = Aes256Gcm::new_from_slice(store.keys.encryption()).unwrap();
        let mut nonce = vec![0_u8; NONCE_BYTES];
        getrandom::fill(&mut nonce).unwrap();
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: legacy_aad(credential_id, owner_scope, kind).as_bytes(),
                },
            )
            .unwrap();
        (ciphertext, nonce)
    }

    #[test]
    fn staged_secret_is_invisible_then_resolves_only_with_exact_authorization() {
        let (_directory, store) = store();
        let secret = ProtectedSecret::new(b"sentinel-secret".to_vec()).unwrap();
        let mutation = mutation(&store, &secret, 0);
        let effect = store
            .apply_secret(&operation_id('1'), &mutation, Some(&secret))
            .unwrap();
        assert!(matches!(
            store.observe_secret(&operation_id('1'), &mutation).unwrap(),
            EffectReconciliation::Staged(_)
        ));
        assert!(
            store
                .read_secret_for_test("credential-a")
                .unwrap()
                .is_none()
        );
        store.activate_secret(&effect).unwrap();
        let committed = CredentialRefV1::new(
            "credential-a",
            "connection-a",
            "hirouted",
            "provider-auth",
            ["provider-api".to_owned()],
            1,
        )
        .unwrap();
        let subject =
            VerifiedSecretSubjectV1::from_authenticated_transport("hirouted", "connection-a")
                .unwrap();
        assert_eq!(
            store
                .resolve_secret(&subject, &committed, "provider-auth", "provider-api", 1)
                .unwrap()
                .expose(),
            secret.expose()
        );
        for denied in [
            VerifiedSecretSubjectV1::from_authenticated_transport("other", "connection-a").unwrap(),
            VerifiedSecretSubjectV1::from_authenticated_transport("hirouted", "other").unwrap(),
        ] {
            assert_eq!(
                store
                    .resolve_secret(&denied, &committed, "provider-auth", "provider-api", 1)
                    .err()
                    .unwrap()
                    .code,
                PortErrorCode::PermissionDenied
            );
        }
        assert_eq!(
            store
                .resolve_secret(&subject, &committed, "wrong", "provider-api", 1)
                .err()
                .unwrap()
                .code,
            PortErrorCode::PermissionDenied
        );
        assert_eq!(
            store
                .resolve_secret(&subject, &committed, "provider-auth", "wrong", 1)
                .err()
                .unwrap()
                .code,
            PortErrorCode::PermissionDenied
        );
        let wrong_generation = CredentialRefV1::new(
            "credential-a",
            "connection-a",
            "hirouted",
            "provider-auth",
            ["provider-api".to_owned()],
            2,
        )
        .unwrap();
        assert_eq!(
            store
                .resolve_secret(
                    &subject,
                    &wrong_generation,
                    "provider-auth",
                    "provider-api",
                    2,
                )
                .err()
                .unwrap()
                .code,
            PortErrorCode::Conflict
        );
    }

    #[test]
    fn current_stage_and_before_snapshot_tamper_fail_closed() {
        for column in ["staged_ciphertext", "before_ciphertext"] {
            let (_directory, store) = store();
            let first = ProtectedSecret::new(b"first-secret".to_vec()).unwrap();
            let first_mutation = mutation(&store, &first, 0);
            let first_effect = store
                .apply_secret(&operation_id('1'), &first_mutation, Some(&first))
                .unwrap();
            store.activate_secret(&first_effect).unwrap();
            let second = ProtectedSecret::new(b"second-secret".to_vec()).unwrap();
            let second_mutation = mutation(&store, &second, 1);
            store
                .apply_secret(&operation_id('2'), &second_mutation, Some(&second))
                .unwrap();
            store.with_connection(|connection| {
                connection
                    .execute(
                        &format!(
                            "UPDATE secret_effects SET {column} = x'00' WHERE operation_id = ?1"
                        ),
                        params![operation_id('2').as_str()],
                    )
                    .unwrap();
            });
            assert!(
                store
                    .observe_secret(&operation_id('2'), &second_mutation)
                    .is_err()
            );
        }

        let (_directory, store) = store();
        let secret = ProtectedSecret::new(b"current-secret".to_vec()).unwrap();
        let initial_mutation = mutation(&store, &secret, 0);
        let effect = store
            .apply_secret(&operation_id('3'), &initial_mutation, Some(&secret))
            .unwrap();
        store.activate_secret(&effect).unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE secret_entries SET ciphertext = x'00' WHERE credential_id = 'credential-a'",
                    [],
                )
                .unwrap();
        });
        assert!(
            store
                .observe_secret(&operation_id('3'), &initial_mutation)
                .is_err()
        );

        // A damaged committed generation is authenticated before the next journal row is
        // inserted, so corruption cannot be blessed as a compensation snapshot.
        let replacement = ProtectedSecret::new(b"replacement-secret".to_vec()).unwrap();
        let replacement_mutation = mutation(&store, &replacement, 1);
        assert!(
            store
                .apply_secret(
                    &operation_id('4'),
                    &replacement_mutation,
                    Some(&replacement),
                )
                .is_err()
        );
        store.with_connection(|connection| {
            let journal_rows: u64 = connection
                .query_row("SELECT count(*) FROM secret_effects", [], |row| row.get(0))
                .unwrap();
            assert_eq!(journal_rows, 1);
        });
    }

    #[test]
    fn compensation_uses_forward_generation_and_never_reintroduces_aba() {
        let (_directory, store) = store();
        let secret = ProtectedSecret::new(b"rollback-secret".to_vec()).unwrap();
        let mutation = mutation(&store, &secret, 0);
        let effect = store
            .apply_secret(&operation_id('4'), &mutation, Some(&secret))
            .unwrap();
        store.activate_secret(&effect).unwrap();
        assert_eq!(
            store.compensate_secret(&effect).unwrap(),
            CompensationOutcome::Compensated
        );
        let forward = CredentialRefV1::new(
            "credential-a",
            "connection-a",
            "hirouted",
            "provider-auth",
            ["provider-api".to_owned()],
            2,
        )
        .unwrap();
        assert_eq!(store.generation(&forward).unwrap(), 2);
        assert!(
            store
                .read_secret_for_test("credential-a")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn legacy_aad_entries_and_inflight_journal_rewrap_atomically_and_recover() {
        let (directory, store) = store();
        let data_root = directory.path().join("data");
        let database = store.database_path().to_path_buf();
        let master_key = store.master_key_path().to_path_buf();
        let first = ProtectedSecret::new(b"legacy-first".to_vec()).unwrap();
        let first_mutation = mutation(&store, &first, 0);
        let first_effect = store
            .apply_secret(&operation_id('5'), &first_mutation, Some(&first))
            .unwrap();
        store.activate_secret(&first_effect).unwrap();

        let second = ProtectedSecret::new(b"legacy-staged".to_vec()).unwrap();
        let second_mutation = mutation(&store, &second, 1);
        let second_effect = store
            .apply_secret(&operation_id('6'), &second_mutation, Some(&second))
            .unwrap();
        let entry = legacy_ciphertext(
            &store,
            "credential-a",
            "connection-a",
            "provider-api-key",
            first.expose(),
        );
        let before = legacy_ciphertext(
            &store,
            "credential-a",
            "connection-a",
            "provider-api-key",
            first.expose(),
        );
        let staged = legacy_ciphertext(
            &store,
            "credential-a",
            "connection-a",
            "provider-api-key",
            second.expose(),
        );
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE secret_entries
                     SET ciphertext=?1, nonce=?2, aad_schema=?3
                     WHERE credential_id='credential-a'",
                    params![entry.0, entry.1, LEGACY_AAD_SCHEMA],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE secret_effects
                     SET before_ciphertext=?2, before_nonce=?3, before_aad_schema=?4,
                         staged_ciphertext=?5, staged_nonce=?6, staged_aad_schema=?4
                     WHERE operation_id=?1 AND credential_id='credential-a'",
                    params![
                        operation_id('6').as_str(),
                        before.0,
                        before.1,
                        LEGACY_AAD_SCHEMA,
                        staged.0,
                        staged.1
                    ],
                )
                .unwrap();
            connection.pragma_update(None, "user_version", 16).unwrap();
            connection
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .unwrap();
        });
        let key_before = fs::read(&master_key).unwrap();
        drop(store);

        let reopened = LocalSecretStore::open(
            &crate::test_storage_authority(),
            &database,
            &master_key,
            data_root.join("backups"),
        )
        .unwrap();
        assert_eq!(fs::read(&master_key).unwrap(), key_before);
        assert!(data_root.join("backups/secrets.db.before-v17.db").is_file());
        reopened.with_connection(|connection| {
            let entry_aad: String = connection
                .query_row(
                    "SELECT aad_schema FROM secret_entries WHERE credential_id='credential-a'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let journal_aad: (String, String) = connection
                .query_row(
                    "SELECT before_aad_schema, staged_aad_schema FROM secret_effects
                     WHERE operation_id=?1 AND credential_id='credential-a'",
                    params![operation_id('6').as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(entry_aad, AAD_SCHEMA);
            assert_eq!(journal_aad, (AAD_SCHEMA.to_owned(), AAD_SCHEMA.to_owned()));
        });
        assert!(matches!(
            reopened
                .observe_secret(&operation_id('6'), &second_mutation)
                .unwrap(),
            EffectReconciliation::Staged(_)
        ));
        reopened.activate_secret(&second_effect).unwrap();
        assert_eq!(
            reopened.compensate_secret(&second_effect).unwrap(),
            CompensationOutcome::Compensated
        );
        let restored = reference(3);
        let subject =
            VerifiedSecretSubjectV1::from_authenticated_transport("hirouted", "connection-a")
                .unwrap();
        assert_eq!(
            reopened
                .resolve_secret(&subject, &restored, "provider-auth", "provider-api", 3)
                .unwrap()
                .expose(),
            first.expose()
        );
    }

    #[test]
    fn existing_store_missing_or_wrong_key_is_stably_locked_without_database_writes() {
        let (directory, store) = store();
        let database = store.database_path().to_path_buf();
        let key = store.master_key_path().to_path_buf();
        drop(store);
        let before = fs::read(&database).unwrap();
        fs::remove_file(&key).unwrap();
        assert!(matches!(
            LocalSecretStore::open(
                &crate::test_storage_authority(),
                &database,
                &key,
                directory.path().join("data/backups")
            ),
            Err(LocalStorageError::Locked)
        ));
        assert_eq!(fs::read(&database).unwrap(), before);

        // Restore a syntactically valid but different key file from a fresh store.
        let other = directory.path().join("other");
        let other_store = LocalSecretStore::open(
            &crate::test_storage_authority(),
            other.join("secrets.db"),
            other.join("master-key"),
            other.join("backups"),
        )
        .unwrap();
        drop(other_store);
        fs::copy(other.join("master-key"), &key).unwrap();
        assert!(matches!(
            LocalSecretStore::open(
                &crate::test_storage_authority(),
                &database,
                &key,
                directory.path().join("data/backups")
            ),
            Err(LocalStorageError::Locked)
        ));
        assert_eq!(fs::read(&database).unwrap(), before);
    }

    #[test]
    fn restore_point_without_live_database_is_not_reinitialized_as_a_fresh_store() {
        let (directory, store) = store();
        let data_root = directory.path().join("data");
        let database = store.database_path().to_path_buf();
        let key = store.master_key_path().to_path_buf();
        store.checkpoint().unwrap();
        store
            .with_connection(|connection| {
                crate::SqliteBackup::create(
                    &crate::test_storage_authority(),
                    connection,
                    data_root.join("backups/secrets.db.before-v4.db"),
                )
            })
            .unwrap();
        drop(store);
        fs::remove_file(&database).unwrap();
        fs::remove_file(&key).unwrap();

        assert!(matches!(
            LocalSecretStore::open(
                &crate::test_storage_authority(),
                &database,
                &key,
                data_root.join("backups")
            ),
            Err(LocalStorageError::Locked)
        ));
        assert!(!database.exists());
        assert!(!key.exists());
    }
}
