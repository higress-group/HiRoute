use std::collections::BTreeSet;

use thiserror::Error;

use crate::CanonicalDigest;

/// Provider model values are opaque UTF-8, not HiRoute's internal reference identifiers.
/// Keep the original bytes for saved intent, authorization and protocol serialization.
pub fn valid_upstream_model_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

pub(super) fn validate_upstream_model_id(value: &str) -> Result<(), ComputeContractError> {
    valid_upstream_model_id(value)
        .then_some(())
        .ok_or(ComputeContractError::InvalidIdentifier)
}

pub(super) fn validate_identifier(value: &str) -> Result<(), ComputeContractError> {
    let valid = !value.is_empty()
        && value.len() <= 256
        && !value.contains("//")
        && !value.contains("..")
        && !value.contains('@')
        && !value.contains('?')
        && !value.contains('#')
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        });
    if valid {
        Ok(())
    } else {
        Err(ComputeContractError::InvalidIdentifier)
    }
}

pub(super) fn valid_display_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.trim() == value
        && value.chars().all(|character| !character.is_control())
}

pub(super) fn validate_evidence(value: &str) -> Result<(), ComputeContractError> {
    let digest = CanonicalDigest::parse(value.to_owned())
        .map_err(|_| ComputeContractError::InvalidEvidence)?;
    if digest.as_str() == CanonicalDigest::of_bytes(&[]).as_str() {
        return Err(ComputeContractError::InvalidEvidence);
    }
    Ok(())
}

pub(super) fn validate_nonempty_digest(
    value: &CanonicalDigest,
) -> Result<(), ComputeContractError> {
    if value == &CanonicalDigest::of_bytes(&[]) {
        Err(ComputeContractError::InvalidEvidence)
    } else {
        Ok(())
    }
}

pub(super) fn ensure_unique<'a>(
    values: impl IntoIterator<Item = &'a str>,
) -> Result<(), ComputeContractError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(ComputeContractError::DuplicateIdentity);
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ComputeContractError {
    #[error("unsupported compute schema")]
    UnsupportedSchema,
    #[error("invalid stable identifier")]
    InvalidIdentifier,
    #[error("duplicate stable identity")]
    DuplicateIdentity,
    #[error("invalid Connector Registry")]
    InvalidRegistry,
    #[error("invalid trusted endpoint")]
    InvalidEndpoint,
    #[error("invalid ModelData")]
    InvalidModelData,
    #[error("invalid catalog evidence reference")]
    InvalidEvidence,
    #[error("trusted bundle cross-reference is not closed")]
    CrossReference,
    #[error("mixed Release slices are forbidden")]
    MixedReleaseSlice,
    #[error("unknown connection option")]
    UnknownConnectionOption,
    #[error("direct free offer lacks verified evidence")]
    UnverifiedDirectOffer,
    #[error("invalid Source identity or state")]
    InvalidSource,
    #[error("paid/subscription Source requires explicit materialization")]
    ExplicitMaterializationRequired,
    #[error("invalid or heterogeneous credential pool")]
    InvalidCredentialPool,
    #[error("generation compare-and-set failed")]
    GenerationConflict,
    #[error("credential not found")]
    CredentialNotFound,
    #[error("cannot remove the last usable credential")]
    LastUsableCredential,
    #[error("invalid price fact or override")]
    InvalidPrice,
    #[error("no effective price")]
    PriceNotFound,
    #[error("effective price selection is ambiguous")]
    AmbiguousPrice,
    #[error("effective price was explicitly disabled")]
    PriceDisabled,
    #[error("observed inventory metadata is invalid or unbounded")]
    InvalidInventory,
    #[error("duplicate observed inventory rows disagree")]
    ConflictingInventory,
    #[error("runtime availability state is internally inconsistent")]
    InvalidRuntimeState,
}
