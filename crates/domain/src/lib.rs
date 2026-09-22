#![forbid(unsafe_code)]

//! Pure, serializable product-domain primitives.
//!
//! This crate intentionally has no storage, transport, gateway, or operating-system
//! dependency. Cross-process DTOs live in `hiroute-application-api`; this crate owns only
//! values whose canonical form is part of the product contract.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub mod agent_connection;
pub mod agent_plan;
pub mod agents;
pub mod change;
pub mod compute;
pub mod delegation;
pub mod error;
pub mod observation;
pub mod operation;
pub mod publication;
pub mod revision;
pub mod routing;
pub mod workspace;

pub use agent_connection::*;
pub use agent_plan::*;
pub use agents::*;
pub use change::{ChangeValidationError, NormalizedChangeV1, canonicalize_json, normalize_change};
pub use compute::*;
pub use error::{PortError, PortErrorCode, PortResult};
pub use observation::*;
pub use operation::{
    AGENT_ACCESS_GRANT_EFFECT_SCHEMA_V1, ActiveAgentConnectionV1, AgentAccessGrantMaterial,
    AgentAccessGrantMaterialActionV1, AgentAccessGrantMutationKindV1, AgentAccessGrantMutationV1,
    AgentAccessGrantRefV1, AgentAccessGrantScopeV1, AgentConfigPermissionIntentV1,
    AgentConnectionControlIntentV1, AgentConnectionEffectRoleV1, AgentConnectionTransactionKindV1,
    AgentConnectionTransactionSubjectV1, BeginOperationOutcome,
    COMPUTE_SUBSCRIPTION_CHECK_COMMAND_ID_V2, COMPUTE_SUBSCRIPTION_EFFECT_ID_V2,
    CompensationOutcome, ComputeSourceControlPort, ComputeSourceMutationV1, ConsumedPlanDraftV1,
    ControlRepositoryPort, CredentialPoolControlPort, CredentialRefV1, EffectReconciliation,
    ExternalEffectIntentV1, ExternalEffectPort, IdempotencyScopeV1, OPERATION_SCHEMA_VERSION,
    OperationId, OperationState, OperationStatus, OperationStepKind, OperationStepStatus,
    OperationStepV1, OperationV1, OperationValidationError, OwnedEffectKind, OwnedEffectV1,
    PlanContentControlV2, ProtectedApplyCapability, ProtectedSecret, RuntimeMutationV1,
    RuntimeStatePort, SETTINGS_SERVICE_COMPLETION_SCHEMA, SecretFingerprintAlgorithm,
    SecretMutationKind, SecretMutationV1, SecretStorePort, SettingsServiceCompletionV1,
    SubscriptionCheckIntentV2, TransactionPlanV1, VerifiedApplyAuthorizationV1,
    VerifiedSecretSubjectV1, WorkerDependencySelectionChangeV1, WorkerDependencySelectionRecordV1,
    agent_connection_publication_record, agent_connection_restore_publication_record,
    decode_subscription_check_intent, is_agent_access_grant_effect,
    is_settings_managed_configuration, is_settings_publication, is_subscription_check_effect,
    routing_publication_record, settings_model_publication_intent,
    settings_model_publication_record, valid_user_agent_token, validate_idempotency_key,
    validate_settings_model_publication_intent,
};
pub use publication::*;
pub use revision::{RevisionMismatch, compare_revisions};
pub use routing::*;
pub use workspace::{WorkspaceId, WorkspaceIdError};

pub const PRODUCT_CONTRACT_REVISION: &str = "487a807c8dc25cfb24e8b097882cd3bb9716a009";
pub const PROPOSAL_MAP_REVISION: &str = "ab4da93cc49c6ec56a01e046230411ff1ca82300";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SchemaVersion {
    pub major: u16,
    pub minor: u16,
}

impl SchemaVersion {
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// Additive minor revisions negotiate to the lower minor. A different major always
    /// fails closed before an operation can reach Application code.
    pub const fn negotiate(self, peer: Self) -> Option<Self> {
        if self.major == peer.major {
            Some(Self::new(
                self.major,
                if self.minor < peer.minor {
                    self.minor
                } else {
                    peer.minor
                },
            ))
        } else {
            None
        }
    }
}

pub const CHANGE_SPEC_SCHEMA_V1: SchemaVersion = SchemaVersion::new(1, 0);

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct CanonicalDigest(String);

impl CanonicalDigest {
    pub fn of<T: Serialize>(value: &T) -> Result<Self, CanonicalDigestError> {
        // Preserve the original default/BTreeMap digest regardless of transitive
        // serde_json/preserve_order. Arrays and scalars retain their serialized values.
        let value = canonicalize_json(serde_json::to_value(value)?);
        let bytes = serde_json::to_vec(&value)?;
        Ok(Self::of_bytes(&bytes))
    }

    pub fn of_bytes(bytes: &[u8]) -> Self {
        let digest = Sha256::digest(bytes);
        let mut encoded = String::with_capacity(7 + digest.len() * 2);
        encoded.push_str("sha256:");
        for byte in digest {
            write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
        }
        Self(encoded)
    }

    pub fn parse(value: impl Into<String>) -> Result<Self, CanonicalDigestError> {
        let value = value.into();
        let valid = value.len() == 71
            && value.starts_with("sha256:")
            && value[7..].bytes().all(|byte| byte.is_ascii_hexdigit());
        if valid {
            Ok(Self(value.to_ascii_lowercase()))
        } else {
            Err(CanonicalDigestError::InvalidFormat)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CanonicalDigest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Error)]
pub enum CanonicalDigestError {
    #[error("value cannot be encoded as canonical JSON: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("digest must use sha256:<64 lowercase hexadecimal characters>")]
    InvalidFormat,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RevisionSetV1 {
    pub target: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ChangeSpecV1 {
    pub schema_version: SchemaVersion,
    pub command_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    pub desired_state: serde_json::Value,
}

impl ChangeSpecV1 {
    pub fn canonical_digest(
        &self,
        revisions: &RevisionSetV1,
    ) -> Result<CanonicalDigest, CanonicalDigestError> {
        #[derive(Serialize)]
        struct DigestInput<'a> {
            spec: &'a ChangeSpecV1,
            revisions: &'a RevisionSetV1,
        }

        CanonicalDigest::of(&DigestInput {
            spec: self,
            revisions,
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectChannel {
    ControlDatabase,
    OperationJournal,
    SecretStore,
    AgentArtifacts,
    Publication,
    ProviderNetwork,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SideEffectSnapshotV1 {
    #[serde(default)]
    pub generations: BTreeMap<EffectChannel, u64>,
}

impl SideEffectSnapshotV1 {
    pub fn changed_channels(&self, after: &Self) -> Vec<EffectChannel> {
        let mut channels = self
            .generations
            .keys()
            .chain(after.generations.keys())
            .copied()
            .collect::<Vec<_>>();
        channels.sort_unstable();
        channels.dedup();
        channels
            .into_iter()
            .filter(|channel| self.generations.get(channel) != after.generations.get(channel))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_digest_is_independent_of_object_key_insertion_order() {
        let first = json!({"b": 2, "a": 1});
        let second = json!({"a": 1, "b": 2});
        assert_eq!(
            CanonicalDigest::of(&first).unwrap(),
            CanonicalDigest::of(&second).unwrap()
        );
    }

    #[test]
    fn canonical_digest_keeps_nested_key_order_and_candidate_array_semantics() {
        let input: serde_json::Value = serde_json::from_str(
            r#"{"z":[{"b":2,"a":1},{"d":null,"c":"思考"}],"a":{"z":false,"a":-7}}"#,
        )
        .unwrap();
        let expected = r#"{"a":{"a":-7,"z":false},"z":[{"a":1,"b":2},{"c":"思考","d":null}]}"#;
        assert_eq!(
            serde_json::to_string(&canonicalize_json(input.clone())).unwrap(),
            expected
        );
        assert_eq!(
            CanonicalDigest::of(&input).unwrap(),
            CanonicalDigest::of_bytes(expected.as_bytes())
        );
        let mut reversed = input.clone();
        reversed["z"].as_array_mut().unwrap().reverse();
        assert_ne!(
            CanonicalDigest::of(&input).unwrap(),
            CanonicalDigest::of(&reversed).unwrap()
        );
    }

    #[test]
    fn unknown_major_fails_closed() {
        assert_eq!(
            SchemaVersion::new(1, 0).negotiate(SchemaVersion::new(2, 0)),
            None
        );
        assert_eq!(
            SchemaVersion::new(1, 0).negotiate(SchemaVersion::new(1, 3)),
            Some(SchemaVersion::new(1, 0))
        );
    }

    #[test]
    fn side_effect_diff_is_typed() {
        let before = SideEffectSnapshotV1::default();
        let mut after = before.clone();
        after.generations.insert(EffectChannel::SecretStore, 1);
        assert_eq!(
            before.changed_channels(&after),
            vec![EffectChannel::SecretStore]
        );
    }
}
