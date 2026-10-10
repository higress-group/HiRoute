//! Authenticated proof of the current reference after an exact owned compensation.
use super::*;

pub(super) fn restored_reference(
    store: &LocalSecretStore,
    operation: &OperationId,
    mutation: &SecretMutationV1,
) -> PortResult<Option<CredentialRefV1>> {
    let connection = store.connection.borrow();
    let original = mutation.credential();
    let record = read_effect(&connection, operation.as_str(), original.credential_id())?;
    let Some(record) = record else {
        // A failure before staging did not mutate this key. Authenticate the exact
        // original reference; never follow a newer generation just because it exists.
        let Some(row) = read_entry(&connection, original.credential_id())? else {
            return Ok(None);
        };
        store.validate_reference(original, &row)?;
        if row.generation != original.generation()
            || read_head(&connection, original.credential_id())? != row.generation
        {
            return Err(port(
                PortErrorCode::Conflict,
                "secret.compensation_reference.unstaged",
            ));
        }
        store.authenticate_row(&row)?;
        return Ok(Some(original.clone()));
    };
    if !record.compensated
        || record.owner_scope != original.owner_scope()
        || record.before_generation != mutation.expected_generation()
        || record.after_exists != (mutation.kind() != SecretMutationKind::Delete)
    {
        return Err(port(
            PortErrorCode::Conflict,
            "secret.compensation_reference.effect",
        ));
    }
    if !record.before_exists {
        return Ok(None);
    }
    store.authenticate_before(&record)?;
    store.authenticate_staged(&record)?;
    let generation = if record.activated {
        record.after_generation.checked_add(1).ok_or_else(|| {
            port(
                PortErrorCode::Conflict,
                "secret.compensation_reference.overflow",
            )
        })?
    } else {
        record.before_generation
    };
    let before = record
        .before_row(generation, operation.as_str())?
        .ok_or_else(|| {
            port(
                PortErrorCode::Corrupt,
                "secret.compensation_reference.before",
            )
        })?;
    let reference = CredentialRefV1::new(
        &before.credential_id,
        &before.owner_scope,
        &before.subject,
        &before.purpose,
        parse_destinations(&before.allowed_destinations_json)?,
        generation,
    )
    .map_err(|_| {
        port(
            PortErrorCode::Corrupt,
            "secret.compensation_reference.authority",
        )
    })?;
    let row = read_entry(&connection, original.credential_id())?.ok_or_else(|| {
        port(
            PortErrorCode::NotFound,
            "secret.compensation_reference.missing",
        )
    })?;
    store.validate_reference(&reference, &row)?;
    if row.generation != generation
        || read_head(&connection, original.credential_id())? != generation
        || row.fingerprint != before.fingerprint
        || row.owner_operation_id
            != if record.activated {
                operation.as_str()
            } else {
                before.owner_operation_id.as_str()
            }
    {
        return Err(port(
            PortErrorCode::Conflict,
            "secret.compensation_reference.owner",
        ));
    }
    store.authenticate_row(&row)?;
    Ok(Some(reference))
}
