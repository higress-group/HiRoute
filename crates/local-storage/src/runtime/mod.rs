use std::cell::RefCell;
use std::path::Path;

use hiroute_domain::{
    CanonicalDigest, CompensationOutcome, EffectReconciliation, OperationId, OwnedEffectKind,
    OwnedEffectV1, PortError, PortErrorCode, PortResult, RuntimeMutationV1, RuntimeStatePort,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};

use crate::migrations::{DatabaseKind, open_database, open_database_from_set};
use crate::{DaemonStorageAuthority, LocalStorageError};

mod compute_state;
mod delegation;

#[cfg(test)]
mod compute_state_tests;

pub struct RuntimeStore {
    pub(crate) connection: RefCell<Connection>,
    pub(crate) startup_lock: Option<std::sync::Arc<std::fs::File>>,
}

impl RuntimeStore {
    pub(crate) fn open(
        _authority: &DaemonStorageAuthority,
        path: impl AsRef<Path>,
        migration_backup_root: impl AsRef<Path>,
    ) -> Result<Self, LocalStorageError> {
        Ok(Self {
            startup_lock: None,
            connection: RefCell::new(open_database(
                _authority,
                path,
                DatabaseKind::Runtime,
                migration_backup_root,
            )?),
        })
    }

    pub(crate) fn open_from_migration_set(
        authority: &DaemonStorageAuthority,
        path: &Path,
        migration_backup_root: &Path,
        expected_store_uuid: &str,
        target_schema_version: u32,
    ) -> Result<Self, LocalStorageError> {
        Ok(Self {
            startup_lock: None,
            connection: RefCell::new(open_database_from_set(
                authority,
                path,
                DatabaseKind::Runtime,
                migration_backup_root,
                Some(expected_store_uuid),
                None,
                target_schema_version,
            )?),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_connection<T>(&self, function: impl FnOnce(&Connection) -> T) -> T {
        function(&self.connection.borrow())
    }

    /// Runtime resolvers see only the committed generation. Staged operation values live solely
    /// in `runtime_effects` until Activate publishes them.
    pub fn value(&self, key: &str) -> PortResult<Option<Value>> {
        let encoded: Option<String> = self
            .connection
            .borrow()
            .query_row(
                "SELECT value_json FROM runtime_state WHERE state_key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.value.read"))?;
        encoded.map(decode_runtime_value).transpose()
    }
}

impl RuntimeStatePort for RuntimeStore {
    fn generation(&self, key: &str) -> PortResult<u64> {
        read_head(&self.connection.borrow(), key)
    }

    fn apply_runtime(
        &self,
        operation_id: &OperationId,
        mutation: &RuntimeMutationV1,
    ) -> PortResult<OwnedEffectV1> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.effect.begin"))?;
        let existing: bool = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM runtime_effects WHERE operation_id = ?1 AND state_key = ?2
                 )",
                params![operation_id.as_str(), mutation.key()],
                |row| row.get(0),
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.effect.lookup"))?;
        if existing {
            drop(transaction);
            drop(connection);
            return match self.observe_runtime(operation_id, mutation)? {
                EffectReconciliation::Staged(effect) | EffectReconciliation::Applied(effect) => {
                    Ok(effect)
                }
                EffectReconciliation::Missing => {
                    Err(port(PortErrorCode::Conflict, "runtime.effect.compensated"))
                }
                EffectReconciliation::OwnershipLost(_) => {
                    Err(port(PortErrorCode::Conflict, "runtime.effect.ownership"))
                }
            };
        }
        let before = transaction
            .query_row(
                "SELECT value_json, value_digest, generation, owner_operation_id
                 FROM runtime_state WHERE state_key = ?1",
                params![mutation.key()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.effect.before"))?;
        let current_generation = read_head(&transaction, mutation.key())?;
        if current_generation != mutation.expected_generation() {
            return Err(port(PortErrorCode::Conflict, "runtime.effect.generation"));
        }
        let encoded = serde_json::to_string(&json!({
            "schema": "hiroute.runtime-state/v1",
            "value": mutation.value(),
        }))
        .map_err(|_| port(PortErrorCode::InvalidData, "runtime.effect.encode"))?;
        let after_digest = CanonicalDigest::of(mutation.value())
            .map_err(|_| port(PortErrorCode::InvalidData, "runtime.effect.digest"))?;
        let after_generation = current_generation.checked_add(1).ok_or_else(|| {
            port(
                PortErrorCode::Conflict,
                "runtime.effect.generation_overflow",
            )
        })?;
        transaction
            .execute(
                "INSERT INTO runtime_effects(
                    operation_id, state_key, before_exists, before_json, before_digest,
                    before_generation, before_owner_operation_id, after_digest,
                    after_generation, compensated, staged_json, activated
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, 0)",
                params![
                    operation_id.as_str(),
                    mutation.key(),
                    i64::from(before.is_some()),
                    before.as_ref().map(|before| before.0.as_str()),
                    before.as_ref().map(|before| before.1.as_str()),
                    current_generation,
                    before.as_ref().and_then(|before| before.3.as_deref()),
                    after_digest.as_str(),
                    after_generation,
                    encoded,
                ],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.effect.journal"))?;
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.effect.commit"))?;
        Ok(runtime_effect(
            operation_id,
            mutation.key(),
            before
                .as_ref()
                .and_then(|before| CanonicalDigest::parse(&before.1).ok()),
            after_digest,
            after_generation,
        ))
    }

    fn observe_runtime(
        &self,
        operation_id: &OperationId,
        mutation: &RuntimeMutationV1,
    ) -> PortResult<EffectReconciliation> {
        let connection = self.connection.borrow();
        let effect = connection
            .query_row(
                "SELECT before_digest, after_digest, after_generation, compensated,
                        staged_json, activated
                 FROM runtime_effects WHERE operation_id = ?1 AND state_key = ?2",
                params![operation_id.as_str(), mutation.key()],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, bool>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, bool>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.effect.observe"))?;
        let Some((before, after, generation, compensated, staged, activated)) = effect else {
            return Ok(EffectReconciliation::Missing);
        };
        if compensated {
            return Ok(EffectReconciliation::Missing);
        }
        let after_digest = CanonicalDigest::parse(after.clone())
            .map_err(|_| port(PortErrorCode::Corrupt, "runtime.effect.digest"))?;
        let owned = runtime_effect(
            operation_id,
            mutation.key(),
            before
                .as_deref()
                .and_then(|value| CanonicalDigest::parse(value).ok()),
            after_digest,
            generation,
        );
        if !activated {
            let value = decode_runtime_value(
                staged.ok_or_else(|| port(PortErrorCode::Corrupt, "runtime.effect.stage"))?,
            )?;
            let digest = CanonicalDigest::of(&value)
                .map_err(|_| port(PortErrorCode::Corrupt, "runtime.effect.stage_digest"))?;
            if digest.as_str() != after {
                return Err(port(PortErrorCode::Corrupt, "runtime.effect.stage_digest"));
            }
            return Ok(EffectReconciliation::Staged(owned));
        }
        let current = connection
            .query_row(
                "SELECT value_digest, generation, owner_operation_id
                 FROM runtime_state WHERE state_key = ?1",
                params![mutation.key()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.effect.current"))?;
        if current.as_ref().is_some_and(|current| {
            current.0 == after
                && current.1 == generation
                && current.2.as_deref() == Some(operation_id.as_str())
        }) {
            Ok(EffectReconciliation::Applied(owned))
        } else {
            Ok(EffectReconciliation::OwnershipLost(owned))
        }
    }

    fn activate_runtime(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1> {
        if effect.kind != OwnedEffectKind::RuntimeState {
            return Err(port(PortErrorCode::InvalidData, "runtime.activate.kind"));
        }
        let operation_id = compensation_text(effect, "operation_id", "runtime.activate.operation")?;
        let key = compensation_text(effect, "state_key", "runtime.activate.key")?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.activate.begin"))?;
        let record = transaction
            .query_row(
                "SELECT before_generation, after_digest, after_generation, staged_json,
                        activated, compensated
                 FROM runtime_effects WHERE operation_id = ?1 AND state_key = ?2",
                params![operation_id, key],
                |row| {
                    Ok((
                        row.get::<_, u64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, bool>(4)?,
                        row.get::<_, bool>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.activate.lookup"))?
            .ok_or_else(|| port(PortErrorCode::NotFound, "runtime.activate.effect"))?;
        if record.5 {
            return Err(port(
                PortErrorCode::Conflict,
                "runtime.activate.compensated",
            ));
        }
        if record.4 {
            return Ok(effect.clone());
        }
        if read_head(&transaction, key)? != record.0 {
            return Err(port(PortErrorCode::Conflict, "runtime.activate.generation"));
        }
        let staged = record
            .3
            .ok_or_else(|| port(PortErrorCode::Corrupt, "runtime.activate.stage"))?;
        transaction
            .execute(
                "INSERT INTO runtime_state(
                    state_key, value_json, value_digest, generation, owner_operation_id, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, unixepoch())
                 ON CONFLICT(state_key) DO UPDATE SET
                    value_json = excluded.value_json, value_digest = excluded.value_digest,
                    generation = excluded.generation,
                    owner_operation_id = excluded.owner_operation_id, updated_at = excluded.updated_at",
                params![key, staged, &record.1, record.2, operation_id],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.activate.write"))?;
        write_head(&transaction, key, record.2)?;
        transaction
            .execute(
                "UPDATE runtime_effects SET activated = 1
                 WHERE operation_id = ?1 AND state_key = ?2",
                params![operation_id, key],
            )
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.activate.mark"))?;
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.activate.commit"))?;
        Ok(effect.clone())
    }

    fn compensate_runtime(&self, effect: &OwnedEffectV1) -> PortResult<CompensationOutcome> {
        if effect.kind != OwnedEffectKind::RuntimeState {
            return Err(port(PortErrorCode::InvalidData, "runtime.compensate.kind"));
        }
        let operation_id =
            compensation_text(effect, "operation_id", "runtime.compensate.operation")?;
        let key = compensation_text(effect, "state_key", "runtime.compensate.key")?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.compensate.begin"))?;
        let record = transaction
            .query_row(
                "SELECT before_exists, before_json, before_digest, after_digest,
                        after_generation, compensated, activated
                 FROM runtime_effects WHERE operation_id = ?1 AND state_key = ?2",
                params![operation_id, key],
                |row| {
                    Ok((
                        row.get::<_, bool>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, u64>(4)?,
                        row.get::<_, bool>(5)?,
                        row.get::<_, bool>(6)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.compensate.lookup"))?
            .ok_or_else(|| port(PortErrorCode::NotFound, "runtime.compensate.effect"))?;
        if record.5 {
            return Ok(CompensationOutcome::AlreadyCompensated);
        }
        if !record.6 {
            mark_compensated(&transaction, operation_id, key)?;
            transaction
                .commit()
                .map_err(|_| port(PortErrorCode::Unavailable, "runtime.compensate.commit"))?;
            return Ok(CompensationOutcome::Compensated);
        }
        let current = transaction
            .query_row(
                "SELECT value_digest, generation, owner_operation_id
                 FROM runtime_state WHERE state_key = ?1",
                params![key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.compensate.current"))?;
        if !current.as_ref().is_some_and(|current| {
            current.0 == record.3
                && current.1 == record.4
                && current.2.as_deref() == Some(operation_id)
        }) {
            return Ok(CompensationOutcome::OwnershipLost);
        }
        let restored_generation = record.4.checked_add(1).ok_or_else(|| {
            port(
                PortErrorCode::Conflict,
                "runtime.compensate.generation_overflow",
            )
        })?;
        if record.0 {
            transaction
                .execute(
                    "UPDATE runtime_state SET value_json = ?2, value_digest = ?3,
                        generation = ?4, owner_operation_id = ?5, updated_at = unixepoch()
                     WHERE state_key = ?1",
                    params![key, record.1, record.2, restored_generation, operation_id],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "runtime.compensate.restore"))?;
        } else {
            transaction
                .execute(
                    "DELETE FROM runtime_state WHERE state_key = ?1",
                    params![key],
                )
                .map_err(|_| port(PortErrorCode::Unavailable, "runtime.compensate.delete"))?;
        }
        write_head(&transaction, key, restored_generation)?;
        mark_compensated(&transaction, operation_id, key)?;
        transaction
            .commit()
            .map_err(|_| port(PortErrorCode::Unavailable, "runtime.compensate.commit"))?;
        Ok(CompensationOutcome::Compensated)
    }
}

fn decode_runtime_value(encoded: impl AsRef<str>) -> PortResult<Value> {
    let mut envelope: Value = serde_json::from_str(encoded.as_ref())
        .map_err(|_| port(PortErrorCode::Corrupt, "runtime.value.decode"))?;
    if envelope.get("schema").and_then(Value::as_str) != Some("hiroute.runtime-state/v1") {
        return Err(port(PortErrorCode::Corrupt, "runtime.value.schema"));
    }
    envelope
        .as_object_mut()
        .and_then(|object| object.remove("value"))
        .ok_or_else(|| port(PortErrorCode::Corrupt, "runtime.value.value"))
}

fn read_head(connection: &Connection, key: &str) -> PortResult<u64> {
    connection
        .query_row(
            "SELECT generation FROM runtime_generation_heads WHERE state_key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map(|generation| generation.unwrap_or(0))
        .map_err(|_| port(PortErrorCode::Unavailable, "runtime.generation"))
}

fn write_head(connection: &Connection, key: &str, generation: u64) -> PortResult<()> {
    connection
        .execute(
            "INSERT INTO runtime_generation_heads(state_key, generation) VALUES (?1, ?2)
             ON CONFLICT(state_key) DO UPDATE SET generation = excluded.generation",
            params![key, generation],
        )
        .map_err(|_| port(PortErrorCode::Unavailable, "runtime.generation.write"))?;
    Ok(())
}

fn mark_compensated(connection: &Connection, operation_id: &str, key: &str) -> PortResult<()> {
    connection
        .execute(
            "UPDATE runtime_effects SET compensated = 1
             WHERE operation_id = ?1 AND state_key = ?2",
            params![operation_id, key],
        )
        .map_err(|_| port(PortErrorCode::Unavailable, "runtime.compensate.mark"))?;
    Ok(())
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

fn runtime_effect(
    operation_id: &OperationId,
    key: &str,
    before: Option<CanonicalDigest>,
    after: CanonicalDigest,
    generation: u64,
) -> OwnedEffectV1 {
    OwnedEffectV1 {
        effect_id: format!("runtime:{key}"),
        kind: OwnedEffectKind::RuntimeState,
        target: key.to_owned(),
        before_fingerprint: before,
        after_fingerprint: Some(after),
        compensation: json!({
            "schema": "hiroute.runtime-compensation/v1",
            "operation_id": operation_id.as_str(),
            "state_key": key,
            "after_generation": generation,
        })
        .into(),
    }
}

fn port(code: PortErrorCode, context: &'static str) -> PortError {
    PortError::new(code, context)
}

#[cfg(test)]
mod tests {
    use crate::test_tempdir as tempdir;
    use hiroute_domain::RuntimeStatePort;
    use serde_json::json;

    use super::*;

    fn operation(value: char) -> OperationId {
        OperationId::parse(format!("op_{}", value.to_string().repeat(32))).unwrap()
    }

    #[test]
    fn runtime_stage_is_invisible_and_compensation_advances_generation() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("data");
        let store = RuntimeStore::open(
            &crate::test_storage_authority(),
            root.join("runtime.db"),
            root.join("backups"),
        )
        .unwrap();
        let first =
            RuntimeMutationV1::from_registered_planner("active/setup", json!({"ready": true}), 0)
                .unwrap();
        let first_effect = store.apply_runtime(&operation('1'), &first).unwrap();
        assert!(store.value("active/setup").unwrap().is_none());
        assert_eq!(store.generation("active/setup").unwrap(), 0);
        assert!(matches!(
            store.observe_runtime(&operation('1'), &first).unwrap(),
            EffectReconciliation::Staged(_)
        ));
        store.activate_runtime(&first_effect).unwrap();
        assert_eq!(
            store.value("active/setup").unwrap(),
            Some(json!({"ready": true}))
        );
        assert_eq!(store.generation("active/setup").unwrap(), 1);

        let second =
            RuntimeMutationV1::from_registered_planner("active/setup", json!({"ready": true}), 1)
                .unwrap();
        let second_effect = store.apply_runtime(&operation('2'), &second).unwrap();
        assert_eq!(
            store.value("active/setup").unwrap(),
            Some(json!({"ready": true}))
        );
        store.activate_runtime(&second_effect).unwrap();
        assert_eq!(store.generation("active/setup").unwrap(), 2);
        assert_eq!(
            store.compensate_runtime(&second_effect).unwrap(),
            CompensationOutcome::Compensated
        );
        assert_eq!(
            store.value("active/setup").unwrap(),
            Some(json!({"ready": true}))
        );
        assert_eq!(store.generation("active/setup").unwrap(), 3);
    }
}
