use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use hiroute_domain::{
    AgentAccessGrantMaterial, AgentAccessGrantMaterialActionV1, AgentAccessGrantMutationKindV1,
    AgentAccessGrantMutationV1, AgentAccessGrantRefV1, AgentAccessGrantScopeV1, CanonicalDigest,
    CompensationOutcome, EffectReconciliation, OperationId, OwnedEffectV1, PortErrorCode,
    PortResult, ProtectedSecret, is_agent_access_grant_effect,
};
use rusqlite::{Connection, TransactionBehavior, params};
use serde_json::Value;

use super::{KEY_VERSION, LocalSecretStore, NONCE_BYTES, port};

mod prepared;
mod records;

use records::{
    AfterMetadata, GrantEffect, GrantHead, GrantVersion, head_matches, insert_effect,
    insert_version, read_effect, read_head, read_version, restore_head, write_head,
};

const AAD_SCHEMA: &str = "hiroute.agent-access-grant-entry/v1";
const GRANT_ID_RANDOM_BYTES: usize = 16;

pub(super) fn inspect(
    store: &LocalSecretStore,
    owner_scope: &str,
    connection_id: &str,
) -> PortResult<Option<AgentAccessGrantRefV1>> {
    let connection = store.connection.borrow();
    let Some(head) = read_head(&connection, connection_id)? else {
        return Ok(None);
    };
    if head.owner_scope != owner_scope {
        return Err(port(
            PortErrorCode::PermissionDenied,
            "agent_access_grant.inspect.owner",
        ));
    }
    let Some(generation) = head.active_version_generation else {
        return Ok(None);
    };
    let version = read_version(&connection, connection_id, generation)?
        .ok_or_else(|| port(PortErrorCode::Corrupt, "agent_access_grant.inspect.version"))?;
    let (reference, _material) = authenticate_version(store, &version)?;
    if reference.generation() != head.generation {
        return Err(port(
            PortErrorCode::Corrupt,
            "agent_access_grant.inspect.generation",
        ));
    }
    Ok(Some(reference))
}

pub(super) fn resolve(
    store: &LocalSecretStore,
    reference: &AgentAccessGrantRefV1,
) -> PortResult<AgentAccessGrantMaterial> {
    reference.validate().map_err(|_| {
        port(
            PortErrorCode::InvalidData,
            "agent_access_grant.resolve.reference",
        )
    })?;
    let connection = store.connection.borrow();
    let head = read_head(&connection, reference.connection_id())?
        .ok_or_else(|| port(PortErrorCode::NotFound, "agent_access_grant.resolve.head"))?;
    if head.owner_scope != reference.owner_scope()
        || head.generation != reference.generation()
        || head.active_version_generation != Some(reference.generation())
    {
        return Err(port(
            PortErrorCode::PermissionDenied,
            "agent_access_grant.resolve.active",
        ));
    }
    let version = read_version(
        &connection,
        reference.connection_id(),
        reference.generation(),
    )?
    .ok_or_else(|| {
        port(
            PortErrorCode::NotFound,
            "agent_access_grant.resolve.version",
        )
    })?;
    let (stored, material) = authenticate_version(store, &version)?;
    if stored != *reference {
        return Err(port(
            PortErrorCode::PermissionDenied,
            "agent_access_grant.resolve.binding",
        ));
    }
    Ok(material)
}

pub(super) fn apply(
    store: &LocalSecretStore,
    operation_id: &OperationId,
    mutation: &AgentAccessGrantMutationV1,
    input: Option<&ProtectedSecret>,
) -> PortResult<OwnedEffectV1> {
    mutation.validate().map_err(|_| {
        port(
            PortErrorCode::InvalidData,
            "agent_access_grant.apply.mutation",
        )
    })?;
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| port(PortErrorCode::Unavailable, "agent_access_grant.apply.begin"))?;
    if let Some(existing) = read_effect(
        &transaction,
        operation_id.as_str(),
        mutation.connection_id(),
    )? {
        drop(transaction);
        drop(connection);
        return existing_effect(store, operation_id, mutation, existing);
    }

    let before_head = read_head(&transaction, mutation.connection_id())?;
    validate_head_owner(&before_head, mutation.owner_scope())?;
    let current_generation = before_head.as_ref().map_or(0, |head| head.generation);
    if current_generation != mutation.expected_generation() {
        return Err(port(
            PortErrorCode::Conflict,
            "agent_access_grant.apply.generation",
        ));
    }
    let before_version =
        active_version(&transaction, before_head.as_ref(), mutation.connection_id())?;
    let before_material = before_version
        .as_ref()
        .map(|version| authenticate_version(store, version).map(|(_, material)| material))
        .transpose()?;

    let (action, after_generation, after_active, after_metadata) = match mutation.kind() {
        AgentAccessGrantMutationKindV1::Ensure => {
            let scope = mutation.desired_scope().ok_or_else(|| {
                port(PortErrorCode::InvalidData, "agent_access_grant.apply.scope")
            })?;
            let scope_hash = scope_digest(scope)?;
            let selected_material = match mutation.material_action() {
                AgentAccessGrantMaterialActionV1::Preserve => {
                    if input.is_some() {
                        return Err(port(
                            PortErrorCode::InvalidData,
                            "agent_access_grant.apply.unexpected_input",
                        ));
                    }
                    None
                }
                AgentAccessGrantMaterialActionV1::Regenerate => {
                    if input.is_some() || before_material.is_none() {
                        return Err(port(
                            PortErrorCode::InvalidData,
                            "agent_access_grant.apply.regenerate",
                        ));
                    }
                    Some(generate_material()?)
                }
                AgentAccessGrantMaterialActionV1::Set { fingerprint, .. } => {
                    let input = input.ok_or_else(|| {
                        port(PortErrorCode::NotFound, "agent_access_grant.apply.input")
                    })?;
                    if store.fingerprint_secret(input)? != *fingerprint {
                        return Err(port(
                            PortErrorCode::Conflict,
                            "agent_access_grant.apply.input_fingerprint",
                        ));
                    }
                    Some(
                        AgentAccessGrantMaterial::from_user_input(input.expose().to_vec())
                            .map_err(|_| {
                                port(
                                    PortErrorCode::InvalidData,
                                    "agent_access_grant.apply.input_encoding",
                                )
                            })?,
                    )
                }
            };
            let same_material = selected_material.as_ref().is_none_or(|selected| {
                before_material
                    .as_ref()
                    .is_some_and(|before| selected.expose() == before.expose())
            });
            if let Some(before) = &before_version
                && before.scope_hash == scope_hash.as_str()
                && same_material
            {
                (
                    "reuse",
                    current_generation,
                    Some(before.generation),
                    Some(AfterMetadata::from_version(before)),
                )
            } else {
                let next = current_generation.checked_add(1).ok_or_else(|| {
                    port(
                        PortErrorCode::Conflict,
                        "agent_access_grant.apply.generation_overflow",
                    )
                })?;
                let grant_id = match &before_version {
                    Some(version) => version.grant_id.clone(),
                    None => generate_grant_id()?,
                };
                let material = match selected_material {
                    Some(material) => material,
                    None => match before_material {
                        Some(material) => material,
                        None => generate_material()?,
                    },
                };
                let material_sha256 = material.sha256();
                let scope_json = serde_json::to_string(scope).map_err(|_| {
                    port(
                        PortErrorCode::InvalidData,
                        "agent_access_grant.apply.scope_encode",
                    )
                })?;
                let (ciphertext, nonce) = encrypt(
                    store,
                    &grant_id,
                    mutation.owner_scope(),
                    mutation.connection_id(),
                    &scope_json,
                    &scope_hash,
                    next,
                    &material,
                )?;
                let version = GrantVersion {
                    connection_id: mutation.connection_id().to_owned(),
                    generation: next,
                    grant_id,
                    owner_scope: mutation.owner_scope().to_owned(),
                    scope_json,
                    scope_hash: scope_hash.as_str().to_owned(),
                    ciphertext,
                    nonce,
                    aad_schema: AAD_SCHEMA.to_owned(),
                    key_version: KEY_VERSION,
                    material_sha256: material_sha256.as_str().to_owned(),
                    owner_operation_id: operation_id.as_str().to_owned(),
                };
                insert_version(&transaction, &version)?;
                let action = if before_version.is_some() {
                    "rotate"
                } else {
                    "create"
                };
                (
                    action,
                    next,
                    Some(next),
                    Some(AfterMetadata::from_version(&version)),
                )
            }
        }
        AgentAccessGrantMutationKindV1::Revoke => {
            if before_version.is_some() {
                (
                    "revoke",
                    current_generation.checked_add(1).ok_or_else(|| {
                        port(
                            PortErrorCode::Conflict,
                            "agent_access_grant.apply.generation_overflow",
                        )
                    })?,
                    None,
                    None,
                )
            } else {
                ("revoke_noop", current_generation, None, None)
            }
        }
    };
    let record = GrantEffect {
        operation_id: operation_id.as_str().to_owned(),
        connection_id: mutation.connection_id().to_owned(),
        action: action.to_owned(),
        owner_scope: mutation.owner_scope().to_owned(),
        before_generation: current_generation,
        before_active_version_generation: before_head
            .as_ref()
            .and_then(|head| head.active_version_generation),
        before_owner_operation_id: before_head.map(|head| head.owner_operation_id),
        after_generation,
        after_active_version_generation: after_active,
        after_grant_id: after_metadata.as_ref().map(|value| value.grant_id.clone()),
        after_scope_hash: after_metadata
            .as_ref()
            .map(|value| value.scope_hash.clone()),
        after_material_sha256: after_metadata.map(|value| value.material_sha256),
        compensated: false,
        activated: false,
    };
    insert_effect(&transaction, &record)?;
    transaction.commit().map_err(|_| {
        port(
            PortErrorCode::Unavailable,
            "agent_access_grant.apply.commit",
        )
    })?;
    Ok(record.owned_effect())
}

pub(super) fn observe(
    store: &LocalSecretStore,
    operation_id: &OperationId,
    mutation: &AgentAccessGrantMutationV1,
) -> PortResult<EffectReconciliation> {
    let connection = store.connection.borrow();
    let Some(record) = read_effect(&connection, operation_id.as_str(), mutation.connection_id())?
    else {
        return Ok(EffectReconciliation::Missing);
    };
    if record.compensated {
        return Ok(EffectReconciliation::Missing);
    }
    let effect = record.owned_effect();
    if !effect_matches_mutation(&record, mutation)? {
        return Ok(EffectReconciliation::OwnershipLost(effect));
    }
    authenticate_effect_versions(store, &connection, &record)?;
    let expected = if record.activated {
        record.after_head()
    } else {
        record.before_head()
    };
    if !head_matches(
        read_head(&connection, mutation.connection_id())?.as_ref(),
        expected.as_ref(),
    ) {
        return Ok(EffectReconciliation::OwnershipLost(effect));
    }
    if record.activated {
        Ok(EffectReconciliation::Applied(effect))
    } else {
        Ok(EffectReconciliation::Staged(effect))
    }
}

pub(super) fn activate(
    store: &LocalSecretStore,
    effect: &OwnedEffectV1,
) -> PortResult<OwnedEffectV1> {
    let (operation_id, connection_id) = effect_identity(effect, "activate")?;
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| {
            port(
                PortErrorCode::Unavailable,
                "agent_access_grant.activate.begin",
            )
        })?;
    let record = read_effect(&transaction, operation_id, connection_id)?.ok_or_else(|| {
        port(
            PortErrorCode::NotFound,
            "agent_access_grant.activate.effect",
        )
    })?;
    if record.compensated || record.owned_effect() != *effect {
        return Err(port(
            PortErrorCode::Conflict,
            "agent_access_grant.activate.ownership",
        ));
    }
    authenticate_effect_versions(store, &transaction, &record)?;
    if record.activated {
        if !head_matches(
            read_head(&transaction, connection_id)?.as_ref(),
            record.after_head().as_ref(),
        ) {
            return Err(port(
                PortErrorCode::Conflict,
                "agent_access_grant.activate.current",
            ));
        }
        return Ok(effect.clone());
    }
    if !head_matches(
        read_head(&transaction, connection_id)?.as_ref(),
        record.before_head().as_ref(),
    ) {
        return Err(port(
            PortErrorCode::Conflict,
            "agent_access_grant.activate.current",
        ));
    }
    write_head(&transaction, &record.after_head_with_owner(operation_id))?;
    transaction
        .execute(
            "UPDATE agent_access_grant_effects SET activated = 1
             WHERE operation_id = ?1 AND connection_id = ?2",
            params![operation_id, connection_id],
        )
        .map_err(|_| {
            port(
                PortErrorCode::Unavailable,
                "agent_access_grant.activate.mark",
            )
        })?;
    transaction.commit().map_err(|_| {
        port(
            PortErrorCode::Unavailable,
            "agent_access_grant.activate.commit",
        )
    })?;
    Ok(effect.clone())
}

pub(super) fn compensate(
    store: &LocalSecretStore,
    effect: &OwnedEffectV1,
) -> PortResult<CompensationOutcome> {
    let (operation_id, connection_id) = effect_identity(effect, "compensate")?;
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| {
            port(
                PortErrorCode::Unavailable,
                "agent_access_grant.compensate.begin",
            )
        })?;
    let record = read_effect(&transaction, operation_id, connection_id)?.ok_or_else(|| {
        port(
            PortErrorCode::NotFound,
            "agent_access_grant.compensate.effect",
        )
    })?;
    if record.compensated {
        return Ok(CompensationOutcome::AlreadyCompensated);
    }
    if record.owned_effect() != *effect {
        return Ok(CompensationOutcome::OwnershipLost);
    }
    authenticate_effect_versions(store, &transaction, &record)?;
    let expected = if record.activated {
        record.after_head()
    } else {
        record.before_head()
    };
    if !head_matches(
        read_head(&transaction, connection_id)?.as_ref(),
        expected.as_ref(),
    ) {
        return Ok(CompensationOutcome::OwnershipLost);
    }
    if record.activated {
        restore_head(&transaction, connection_id, record.before_head().as_ref())?;
    }
    if record.after_active_version_generation != record.before_active_version_generation
        && let Some(generation) = record.after_active_version_generation
    {
        transaction
            .execute(
                "DELETE FROM agent_access_grant_versions
                 WHERE connection_id = ?1 AND generation = ?2 AND owner_operation_id = ?3",
                params![connection_id, generation, operation_id],
            )
            .map_err(|_| {
                port(
                    PortErrorCode::Unavailable,
                    "agent_access_grant.compensate.version",
                )
            })?;
    }
    transaction
        .execute(
            "UPDATE agent_access_grant_effects SET compensated = 1
             WHERE operation_id = ?1 AND connection_id = ?2",
            params![operation_id, connection_id],
        )
        .map_err(|_| {
            port(
                PortErrorCode::Unavailable,
                "agent_access_grant.compensate.mark",
            )
        })?;
    if let Some(before) = record.before_active_version_generation {
        let restored = read_version(&transaction, connection_id, before)?.ok_or_else(|| {
            port(
                PortErrorCode::Corrupt,
                "agent_access_grant.compensate.restored",
            )
        })?;
        authenticate_version(store, &restored)?;
    }
    transaction.commit().map_err(|_| {
        port(
            PortErrorCode::Unavailable,
            "agent_access_grant.compensate.commit",
        )
    })?;
    Ok(CompensationOutcome::Compensated)
}

fn existing_effect(
    store: &LocalSecretStore,
    operation_id: &OperationId,
    mutation: &AgentAccessGrantMutationV1,
    record: GrantEffect,
) -> PortResult<OwnedEffectV1> {
    let effect = record.owned_effect();
    match observe(store, operation_id, mutation)? {
        EffectReconciliation::Staged(_) | EffectReconciliation::Applied(_) => Ok(effect),
        EffectReconciliation::Missing => Err(port(
            PortErrorCode::Conflict,
            "agent_access_grant.apply.compensated",
        )),
        EffectReconciliation::OwnershipLost(_) => Err(port(
            PortErrorCode::Conflict,
            "agent_access_grant.apply.ownership",
        )),
    }
}

fn generate_material() -> PortResult<AgentAccessGrantMaterial> {
    let mut entropy = [0_u8; 32];
    getrandom::fill(&mut entropy).map_err(|_| {
        port(
            PortErrorCode::Crypto,
            "agent_access_grant.generate.material",
        )
    })?;
    Ok(AgentAccessGrantMaterial::from_csprng_entropy(entropy))
}

fn generate_grant_id() -> PortResult<String> {
    let mut bytes = [0_u8; GRANT_ID_RANDOM_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|_| port(PortErrorCode::Crypto, "agent_access_grant.generate.id"))?;
    Ok(format!("grant_{}", hex(&bytes)))
}

#[allow(clippy::too_many_arguments)]
fn encrypt(
    store: &LocalSecretStore,
    grant_id: &str,
    owner_scope: &str,
    connection_id: &str,
    scope_json: &str,
    scope_hash: &CanonicalDigest,
    generation: u64,
    material: &AgentAccessGrantMaterial,
) -> PortResult<(Vec<u8>, Vec<u8>)> {
    let cipher = Aes256Gcm::new_from_slice(store.keys.encryption())
        .map_err(|_| port(PortErrorCode::Crypto, "agent_access_grant.encrypt.key"))?;
    let mut nonce = vec![0_u8; NONCE_BYTES];
    getrandom::fill(&mut nonce)
        .map_err(|_| port(PortErrorCode::Crypto, "agent_access_grant.encrypt.nonce"))?;
    let aad = aad(
        &store.keys.store_uuid,
        grant_id,
        owner_scope,
        connection_id,
        scope_json,
        scope_hash.as_str(),
        generation,
    );
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: material.expose(),
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| port(PortErrorCode::Crypto, "agent_access_grant.encrypt"))?;
    Ok((ciphertext, nonce))
}

fn authenticate_version(
    store: &LocalSecretStore,
    version: &GrantVersion,
) -> PortResult<(AgentAccessGrantRefV1, AgentAccessGrantMaterial)> {
    if version.nonce.len() != NONCE_BYTES
        || version.key_version != KEY_VERSION
        || version.aad_schema != AAD_SCHEMA
    {
        return Err(port(
            PortErrorCode::Corrupt,
            "agent_access_grant.decrypt.metadata",
        ));
    }
    let scope: AgentAccessGrantScopeV1 = serde_json::from_str(&version.scope_json)
        .map_err(|_| port(PortErrorCode::Corrupt, "agent_access_grant.decrypt.scope"))?;
    scope
        .validate()
        .map_err(|_| port(PortErrorCode::Corrupt, "agent_access_grant.decrypt.scope"))?;
    let scope_hash = scope_digest(&scope)?;
    if scope.connection_id() != version.connection_id || scope_hash.as_str() != version.scope_hash {
        return Err(port(
            PortErrorCode::Corrupt,
            "agent_access_grant.decrypt.scope_hash",
        ));
    }
    let cipher = Aes256Gcm::new_from_slice(store.keys.encryption())
        .map_err(|_| port(PortErrorCode::Crypto, "agent_access_grant.decrypt.key"))?;
    let aad = aad(
        &store.keys.store_uuid,
        &version.grant_id,
        &version.owner_scope,
        &version.connection_id,
        &version.scope_json,
        &version.scope_hash,
        version.generation,
    );
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&version.nonce),
            Payload {
                msg: &version.ciphertext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| port(PortErrorCode::Crypto, "agent_access_grant.decrypt"))?;
    let material =
        AgentAccessGrantMaterial::from_authenticated_storage(plaintext).map_err(|_| {
            port(
                PortErrorCode::Corrupt,
                "agent_access_grant.decrypt.encoding",
            )
        })?;
    let material_sha256 = material.sha256();
    if material_sha256.as_str() != version.material_sha256 {
        return Err(port(
            PortErrorCode::Corrupt,
            "agent_access_grant.decrypt.hash",
        ));
    }
    let reference = AgentAccessGrantRefV1::new(
        version.grant_id.clone(),
        version.owner_scope.clone(),
        scope,
        version.generation,
        material_sha256,
    )
    .map_err(|_| {
        port(
            PortErrorCode::Corrupt,
            "agent_access_grant.decrypt.reference",
        )
    })?;
    Ok((reference, material))
}

fn authenticate_effect_versions(
    store: &LocalSecretStore,
    connection: &Connection,
    record: &GrantEffect,
) -> PortResult<()> {
    for generation in [
        record.before_active_version_generation,
        record.after_active_version_generation,
    ]
    .into_iter()
    .flatten()
    {
        let version = read_version(connection, &record.connection_id, generation)?
            .ok_or_else(|| port(PortErrorCode::Corrupt, "agent_access_grant.effect.version"))?;
        authenticate_version(store, &version)?;
    }
    Ok(())
}

fn effect_matches_mutation(
    record: &GrantEffect,
    mutation: &AgentAccessGrantMutationV1,
) -> PortResult<bool> {
    if record.owner_scope != mutation.owner_scope()
        || record.connection_id != mutation.connection_id()
        || record.before_generation != mutation.expected_generation()
    {
        return Ok(false);
    }
    match mutation.kind() {
        AgentAccessGrantMutationKindV1::Ensure => {
            Ok(record.after_active_version_generation.is_some()
                && record.after_scope_hash.as_deref()
                    == Some(
                        scope_digest(mutation.desired_scope().ok_or_else(|| {
                            port(
                                PortErrorCode::InvalidData,
                                "agent_access_grant.effect.scope",
                            )
                        })?)?
                        .as_str(),
                    ))
        }
        AgentAccessGrantMutationKindV1::Revoke => {
            Ok(record.after_active_version_generation.is_none())
        }
    }
}

fn effect_identity<'a>(
    effect: &'a OwnedEffectV1,
    action: &'static str,
) -> PortResult<(&'a str, &'a str)> {
    if !is_agent_access_grant_effect(effect) {
        return Err(port(
            PortErrorCode::InvalidData,
            if action == "activate" {
                "agent_access_grant.activate.kind"
            } else {
                "agent_access_grant.compensate.kind"
            },
        ));
    }
    let operation_id = effect
        .compensation
        .get("operation_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            port(
                PortErrorCode::InvalidData,
                "agent_access_grant.effect.operation",
            )
        })?;
    let connection_id = effect
        .compensation
        .get("connection_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            port(
                PortErrorCode::InvalidData,
                "agent_access_grant.effect.connection",
            )
        })?;
    Ok((operation_id, connection_id))
}

fn validate_head_owner(head: &Option<GrantHead>, owner_scope: &str) -> PortResult<()> {
    if head
        .as_ref()
        .is_some_and(|head| head.owner_scope != owner_scope)
    {
        Err(port(
            PortErrorCode::PermissionDenied,
            "agent_access_grant.apply.owner",
        ))
    } else {
        Ok(())
    }
}

fn active_version(
    connection: &Connection,
    head: Option<&GrantHead>,
    connection_id: &str,
) -> PortResult<Option<GrantVersion>> {
    head.and_then(|head| head.active_version_generation)
        .map(|generation| {
            read_version(connection, connection_id, generation)?
                .ok_or_else(|| port(PortErrorCode::Corrupt, "agent_access_grant.active.version"))
        })
        .transpose()
}

fn scope_digest(scope: &AgentAccessGrantScopeV1) -> PortResult<CanonicalDigest> {
    scope.digest().map_err(|_| {
        port(
            PortErrorCode::InvalidData,
            "agent_access_grant.scope.digest",
        )
    })
}

fn aad(
    store_uuid: &str,
    grant_id: &str,
    owner_scope: &str,
    connection_id: &str,
    scope_json: &str,
    scope_hash: &str,
    generation: u64,
) -> String {
    format!(
        "{AAD_SCHEMA}\0{KEY_VERSION}\0{store_uuid}\0{grant_id}\0{owner_scope}\0{connection_id}\0{scope_json}\0{scope_hash}\0{generation}"
    )
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}
