use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::*;

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialPoolMutationKind {
    Add,
    Replace,
    Remove,
    Reorder,
}

/// A command-specific desired pool transition. Secret bytes are absent; the HMAC fingerprint,
/// exact target credential, and Offer-bound desired pool bind the control CAS.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialPoolMutationV1 {
    kind: CredentialPoolMutationKind,
    expected_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    before_digest: Option<CanonicalDigest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credential_id: Option<String>,
    desired: CredentialPoolV1,
}

impl CredentialPoolMutationV1 {
    pub fn from_registered_planner(
        kind: CredentialPoolMutationKind,
        current: Option<&CredentialPoolV1>,
        desired: CredentialPoolV1,
    ) -> Result<Self, ComputeContractError> {
        let mutation = Self {
            kind,
            expected_revision: current.map_or(0, |pool| pool.revision),
            before_digest: current
                .map(CanonicalDigest::of)
                .transpose()
                .map_err(|_| ComputeContractError::InvalidCredentialPool)?,
            credential_id: transition_credential_id(kind, current, &desired),
            desired,
        };
        mutation.validate_against(current)?;
        Ok(mutation)
    }

    pub fn validate_shape(&self) -> Result<(), ComputeContractError> {
        self.desired.validate()?;
        if self
            .credential_id
            .as_deref()
            .is_some_and(|id| validate_identifier(id).is_err())
            || (self.kind == CredentialPoolMutationKind::Reorder) != self.credential_id.is_none()
        {
            return Err(ComputeContractError::InvalidCredentialPool);
        }
        if self.expected_revision.checked_add(1) != Some(self.desired.revision)
            || (self.expected_revision == 0) != self.before_digest.is_none()
        {
            return Err(ComputeContractError::GenerationConflict);
        }
        Ok(())
    }

    pub fn validate_against(
        &self,
        current: Option<&CredentialPoolV1>,
    ) -> Result<(), ComputeContractError> {
        self.validate_shape()?;
        let Some(current) = current else {
            return if self.kind == CredentialPoolMutationKind::Add
                && self.expected_revision == 0
                && self.desired.credentials.len() == 1
                && self.credential_id.as_deref()
                    == Some(self.desired.credentials[0].credential.credential_id())
            {
                Ok(())
            } else {
                Err(ComputeContractError::GenerationConflict)
            };
        };
        current.validate()?;
        if current.revision != self.expected_revision
            || self.before_digest.as_ref()
                != Some(
                    &CanonicalDigest::of(current)
                        .map_err(|_| ComputeContractError::InvalidCredentialPool)?,
                )
            || !same_pool_identity(current, &self.desired)
        {
            return Err(ComputeContractError::GenerationConflict);
        }
        let credential_id = self.credential_id.as_deref();
        let valid = match self.kind {
            CredentialPoolMutationKind::Add => {
                validate_add_transition(current, &self.desired, credential_id)
            }
            CredentialPoolMutationKind::Replace => {
                validate_replace_transition(current, &self.desired, credential_id)
            }
            CredentialPoolMutationKind::Remove => {
                validate_remove_transition(current, &self.desired, credential_id)
            }
            CredentialPoolMutationKind::Reorder => {
                validate_reorder_transition(current, &self.desired)
            }
        };
        if valid {
            Ok(())
        } else {
            Err(ComputeContractError::InvalidCredentialPool)
        }
    }

    pub const fn kind(&self) -> CredentialPoolMutationKind {
        self.kind
    }

    pub const fn expected_revision(&self) -> u64 {
        self.expected_revision
    }

    pub fn before_digest(&self) -> Option<&CanonicalDigest> {
        self.before_digest.as_ref()
    }

    pub fn credential_id(&self) -> Option<&str> {
        self.credential_id.as_deref()
    }

    pub fn desired(&self) -> &CredentialPoolV1 {
        &self.desired
    }
}

fn transition_credential_id(
    kind: CredentialPoolMutationKind,
    current: Option<&CredentialPoolV1>,
    desired: &CredentialPoolV1,
) -> Option<String> {
    match kind {
        CredentialPoolMutationKind::Add => desired
            .credentials
            .iter()
            .find(|entry| {
                current.is_none_or(|pool| {
                    !pool.credentials.iter().any(|existing| {
                        existing.credential.credential_id() == entry.credential.credential_id()
                    })
                })
            })
            .map(|entry| entry.credential.credential_id().to_owned()),
        CredentialPoolMutationKind::Replace => current.and_then(|pool| {
            pool.credentials
                .iter()
                .zip(&desired.credentials)
                .find(|(before, after)| before != after)
                .map(|(_, after)| after.credential.credential_id().to_owned())
        }),
        CredentialPoolMutationKind::Remove => current.and_then(|pool| {
            pool.credentials
                .iter()
                .find(|entry| {
                    !desired.credentials.iter().any(|remaining| {
                        remaining.credential.credential_id() == entry.credential.credential_id()
                    })
                })
                .map(|entry| entry.credential.credential_id().to_owned())
        }),
        CredentialPoolMutationKind::Reorder => None,
    }
}

fn same_pool_identity(left: &CredentialPoolV1, right: &CredentialPoolV1) -> bool {
    left.identity() == right.identity()
}

fn validate_add_transition(
    current: &CredentialPoolV1,
    desired: &CredentialPoolV1,
    credential_id: Option<&str>,
) -> bool {
    desired.credentials.len() == current.credentials.len() + 1
        && desired.credentials[..current.credentials.len()] == current.credentials
        && credential_id
            == desired
                .credentials
                .last()
                .map(|entry| entry.credential.credential_id())
}

fn validate_replace_transition(
    current: &CredentialPoolV1,
    desired: &CredentialPoolV1,
    credential_id: Option<&str>,
) -> bool {
    if desired.credentials.len() != current.credentials.len() {
        return false;
    }
    let changed = current
        .credentials
        .iter()
        .zip(&desired.credentials)
        .filter(|(left, right)| left != right)
        .collect::<Vec<_>>();
    let [(left, right)] = changed.as_slice() else {
        return false;
    };
    left.credential.credential_id() == right.credential.credential_id()
        && credential_id == Some(right.credential.credential_id())
        && left.credential.generation().checked_add(1) == Some(right.credential.generation())
        && left.ordinal == right.ordinal
        && left.enabled == right.enabled
        && left.fingerprint != right.fingerprint
}

fn validate_remove_transition(
    current: &CredentialPoolV1,
    desired: &CredentialPoolV1,
    credential_id: Option<&str>,
) -> bool {
    if current.credentials.len() != desired.credentials.len() + 1 {
        return false;
    }
    let current_by_id = current
        .credentials
        .iter()
        .map(|entry| (entry.credential.credential_id(), entry))
        .collect::<BTreeMap<_, _>>();
    credential_id.is_some_and(|removed_id| {
        current_by_id.contains_key(removed_id)
            && !desired
                .credentials
                .iter()
                .any(|entry| entry.credential.credential_id() == removed_id)
    }) && desired.credentials.iter().all(|entry| {
        current_by_id
            .get(entry.credential.credential_id())
            .is_some_and(|current| same_entry_except_ordinal(current, entry))
    })
}

fn validate_reorder_transition(current: &CredentialPoolV1, desired: &CredentialPoolV1) -> bool {
    if current.credentials.len() != desired.credentials.len()
        || current.credentials == desired.credentials
    {
        return false;
    }
    let current_by_id = current
        .credentials
        .iter()
        .map(|entry| (entry.credential.credential_id(), entry))
        .collect::<BTreeMap<_, _>>();
    desired.credentials.iter().all(|entry| {
        current_by_id
            .get(entry.credential.credential_id())
            .is_some_and(|current| same_entry_except_ordinal(current, entry))
    })
}

fn same_entry_except_ordinal(left: &PoolCredentialV1, right: &PoolCredentialV1) -> bool {
    left.credential == right.credential
        && left.fingerprint == right.fingerprint
        && left.enabled == right.enabled
}
