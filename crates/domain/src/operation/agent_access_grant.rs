use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use super::{OperationValidationError, OwnedEffectKind, OwnedEffectV1, validate_scope_identifier};
use crate::{AgentIngressProtocolV1, CanonicalDigest};

pub const AGENT_ACCESS_GRANT_EFFECT_SCHEMA_V1: &str = "hiroute.agent-access-grant-effect/v1";
const MATERIAL_ENTROPY_BYTES: usize = 32;
const MATERIAL_BASE64URL_BYTES: usize = 43;
const BASE64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// A HiRoute-local bearer, generated from 256 bits of CSPRNG entropy or supplied by the user.
///
/// This type deliberately implements neither `Debug`, `Display`, `Clone`, nor serde traits.
/// Callers may only expose it at the explicitly protected helper/header boundary.
///
/// ```compile_fail
/// use hiroute_domain::AgentAccessGrantMaterial;
/// fn requires_debug<T: std::fmt::Debug>() {}
/// requires_debug::<AgentAccessGrantMaterial>();
/// ```
///
/// ```compile_fail
/// use hiroute_domain::AgentAccessGrantMaterial;
/// fn requires_display<T: std::fmt::Display>() {}
/// requires_display::<AgentAccessGrantMaterial>();
/// ```
///
/// ```compile_fail
/// use hiroute_domain::AgentAccessGrantMaterial;
/// fn requires_serialize<T: serde::Serialize>() {}
/// requires_serialize::<AgentAccessGrantMaterial>();
/// ```
pub struct AgentAccessGrantMaterial(Zeroizing<Vec<u8>>);

impl AgentAccessGrantMaterial {
    /// Encodes exactly 256 bits supplied by the storage authority. The storage implementation is
    /// responsible for filling `entropy` directly from the operating system CSPRNG.
    pub fn from_csprng_entropy(mut entropy: [u8; MATERIAL_ENTROPY_BYTES]) -> Self {
        let encoded = encode_base64url_no_pad(&entropy);
        entropy.zeroize();
        Self(Zeroizing::new(encoded))
    }

    /// Reconstructs material only after an authenticated AEAD record has been opened.
    pub fn from_authenticated_storage(bytes: Vec<u8>) -> Result<Self, OperationValidationError> {
        let bytes = Zeroizing::new(bytes);
        if !valid_user_agent_token(&bytes) {
            return Err(OperationValidationError::InvalidAgentAccessGrantMaterial);
        }
        Ok(Self(bytes))
    }

    pub fn from_user_input(bytes: Vec<u8>) -> Result<Self, OperationValidationError> {
        Self::from_authenticated_storage(bytes)
    }

    pub fn expose(&self) -> &[u8] {
        self.0.as_slice()
    }

    pub fn sha256(&self) -> CanonicalDigest {
        CanonicalDigest::of_bytes(self.expose())
    }
}

/// A custom bearer must be safe in HTTP headers and both native client configuration formats.
/// The minimum length avoids accidentally accepting a short human password as an API bearer.
pub fn valid_user_agent_token(bytes: &[u8]) -> bool {
    (16..=128).contains(&bytes.len())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._~-".contains(byte))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentAccessGrantScopeV1 {
    connection_id: String,
    model_grant: crate::AgentModelGrantV2,
}

impl AgentAccessGrantScopeV1 {
    pub fn new(
        connection_id: impl Into<String>,
        model_grant: crate::AgentModelGrantV2,
    ) -> Result<Self, OperationValidationError> {
        let scope = Self {
            connection_id: connection_id.into(),
            model_grant,
        };
        scope.validate()?;
        Ok(scope)
    }

    pub fn validate(&self) -> Result<(), OperationValidationError> {
        validate_scope_identifier(&self.connection_id)?;
        if !self.connection_id.starts_with("agent-connection/") {
            return Err(OperationValidationError::InvalidAgentAccessGrantScope);
        }
        self.model_grant
            .validate()
            .map_err(|_| OperationValidationError::InvalidAgentAccessGrantScope)
    }

    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }

    pub const fn protocol(&self) -> AgentIngressProtocolV1 {
        self.model_grant.protocol
    }

    pub fn model_grant(&self) -> &crate::AgentModelGrantV2 {
        &self.model_grant
    }

    pub fn digest(&self) -> Result<CanonicalDigest, OperationValidationError> {
        self.validate()?;
        CanonicalDigest::of(self).map_err(OperationValidationError::Digest)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAccessGrantMutationKindV1 {
    Ensure,
    Revoke,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentAccessGrantMaterialActionV1 {
    #[default]
    Preserve,
    Regenerate,
    Set {
        input_slot: String,
        fingerprint: CanonicalDigest,
    },
}

impl AgentAccessGrantMaterialActionV1 {
    pub fn input(&self) -> Option<(&str, &CanonicalDigest)> {
        match self {
            Self::Set {
                input_slot,
                fingerprint,
            } => Some((input_slot, fingerprint)),
            _ => None,
        }
    }
}

/// A durable non-secret request. Material is intentionally absent and can only be generated by
/// the SecretStore after the Operation has crossed durable Apply admission.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentAccessGrantMutationV1 {
    kind: AgentAccessGrantMutationKindV1,
    owner_scope: String,
    connection_id: String,
    expected_generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    desired_scope: Option<AgentAccessGrantScopeV1>,
    #[serde(default)]
    material_action: AgentAccessGrantMaterialActionV1,
}

impl AgentAccessGrantMutationV1 {
    pub fn ensure(
        owner_scope: impl Into<String>,
        desired_scope: AgentAccessGrantScopeV1,
        expected_generation: u64,
    ) -> Result<Self, OperationValidationError> {
        let mutation = Self {
            kind: AgentAccessGrantMutationKindV1::Ensure,
            owner_scope: owner_scope.into(),
            connection_id: desired_scope.connection_id.clone(),
            expected_generation,
            desired_scope: Some(desired_scope),
            material_action: AgentAccessGrantMaterialActionV1::Preserve,
        };
        mutation.validate()?;
        Ok(mutation)
    }

    pub fn with_material_action(
        mut self,
        material_action: AgentAccessGrantMaterialActionV1,
    ) -> Result<Self, OperationValidationError> {
        self.material_action = material_action;
        self.validate()?;
        Ok(self)
    }

    pub fn revoke(
        owner_scope: impl Into<String>,
        connection_id: impl Into<String>,
        expected_generation: u64,
    ) -> Result<Self, OperationValidationError> {
        let mutation = Self {
            kind: AgentAccessGrantMutationKindV1::Revoke,
            owner_scope: owner_scope.into(),
            connection_id: connection_id.into(),
            expected_generation,
            desired_scope: None,
            material_action: AgentAccessGrantMaterialActionV1::Preserve,
        };
        mutation.validate()?;
        Ok(mutation)
    }

    pub fn validate(&self) -> Result<(), OperationValidationError> {
        validate_scope_identifier(&self.owner_scope)?;
        validate_scope_identifier(&self.connection_id)?;
        if !self.connection_id.starts_with("agent-connection/") {
            return Err(OperationValidationError::InvalidAgentAccessGrantMutation);
        }
        if let Some((input_slot, fingerprint)) = self.material_action.input() {
            validate_scope_identifier(input_slot)?;
            if !input_slot.starts_with("candidate/native/agent-token-")
                || CanonicalDigest::parse(fingerprint.as_str().to_owned()).is_err()
            {
                return Err(OperationValidationError::InvalidAgentAccessGrantMutation);
            }
        }
        match (self.kind, &self.desired_scope) {
            (AgentAccessGrantMutationKindV1::Ensure, Some(scope)) => {
                scope.validate()?;
                if scope.connection_id != self.connection_id {
                    return Err(OperationValidationError::InvalidAgentAccessGrantMutation);
                }
            }
            (AgentAccessGrantMutationKindV1::Revoke, None)
                if matches!(
                    self.material_action,
                    AgentAccessGrantMaterialActionV1::Preserve
                ) => {}
            _ => return Err(OperationValidationError::InvalidAgentAccessGrantMutation),
        }
        Ok(())
    }

    pub const fn kind(&self) -> AgentAccessGrantMutationKindV1 {
        self.kind
    }

    pub fn owner_scope(&self) -> &str {
        &self.owner_scope
    }

    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }

    pub const fn expected_generation(&self) -> u64 {
        self.expected_generation
    }

    pub fn desired_scope(&self) -> Option<&AgentAccessGrantScopeV1> {
        self.desired_scope.as_ref()
    }

    pub fn material_action(&self) -> &AgentAccessGrantMaterialActionV1 {
        &self.material_action
    }
}

/// The only durable/public projection of grant material.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentAccessGrantRefV1 {
    grant_id: String,
    owner_scope: String,
    scope: AgentAccessGrantScopeV1,
    generation: u64,
    material_sha256: CanonicalDigest,
}

impl AgentAccessGrantRefV1 {
    pub fn new(
        grant_id: impl Into<String>,
        owner_scope: impl Into<String>,
        scope: AgentAccessGrantScopeV1,
        generation: u64,
        material_sha256: CanonicalDigest,
    ) -> Result<Self, OperationValidationError> {
        let reference = Self {
            grant_id: grant_id.into(),
            owner_scope: owner_scope.into(),
            scope,
            generation,
            material_sha256,
        };
        reference.validate()?;
        Ok(reference)
    }

    pub fn validate(&self) -> Result<(), OperationValidationError> {
        validate_scope_identifier(&self.owner_scope)?;
        self.scope.validate()?;
        let valid_id = self.grant_id.len() == 38
            && self.grant_id.starts_with("grant_")
            && self.grant_id[6..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit());
        if !valid_id
            || self.generation == 0
            || CanonicalDigest::parse(self.material_sha256.as_str().to_owned()).is_err()
        {
            return Err(OperationValidationError::InvalidAgentAccessGrantRef);
        }
        Ok(())
    }

    pub fn grant_id(&self) -> &str {
        &self.grant_id
    }

    pub fn owner_scope(&self) -> &str {
        &self.owner_scope
    }

    pub fn connection_id(&self) -> &str {
        self.scope.connection_id()
    }

    pub fn scope(&self) -> &AgentAccessGrantScopeV1 {
        &self.scope
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn material_sha256(&self) -> &CanonicalDigest {
        &self.material_sha256
    }

    /// Reconstructs the non-secret staged reference returned by an operation-bound SecretStore
    /// observation. Callers must obtain `effect` from `observe_agent_access_grant`; this parser
    /// additionally binds every field back to the immutable ensure mutation.
    pub fn from_ensure_effect(
        effect: &OwnedEffectV1,
        mutation: &AgentAccessGrantMutationV1,
    ) -> Result<Self, OperationValidationError> {
        mutation.validate()?;
        if mutation.kind != AgentAccessGrantMutationKindV1::Ensure
            || !is_agent_access_grant_effect(effect)
            || effect.effect_id != format!("agent-access-grant:{}", mutation.connection_id)
            || effect.target != mutation.connection_id
        {
            return Err(OperationValidationError::InvalidAgentAccessGrantRef);
        }
        let metadata = effect
            .compensation
            .as_object()
            .ok_or(OperationValidationError::InvalidAgentAccessGrantRef)?;
        let text = |field: &str| {
            metadata
                .get(field)
                .and_then(serde_json::Value::as_str)
                .ok_or(OperationValidationError::InvalidAgentAccessGrantRef)
        };
        let connection_id = text("connection_id")?;
        let owner_scope = text("owner_scope")?;
        let grant_id = text("grant_id")?;
        let scope_hash = CanonicalDigest::parse(text("scope_hash")?.to_owned())
            .map_err(|_| OperationValidationError::InvalidAgentAccessGrantRef)?;
        let material_sha256 = CanonicalDigest::parse(text("material_sha256")?.to_owned())
            .map_err(|_| OperationValidationError::InvalidAgentAccessGrantRef)?;
        let generation = metadata
            .get("generation")
            .and_then(serde_json::Value::as_u64)
            .ok_or(OperationValidationError::InvalidAgentAccessGrantRef)?;
        let desired_scope = mutation
            .desired_scope
            .as_ref()
            .ok_or(OperationValidationError::InvalidAgentAccessGrantRef)?;
        if connection_id != mutation.connection_id
            || owner_scope != mutation.owner_scope
            || scope_hash != desired_scope.digest()?
            || effect.after_fingerprint.as_ref() != Some(&material_sha256)
            || !matches!(
                generation,
                value if value == mutation.expected_generation
                    || mutation.expected_generation.checked_add(1) == Some(value)
            )
        {
            return Err(OperationValidationError::InvalidAgentAccessGrantRef);
        }
        Self::new(
            grant_id,
            owner_scope,
            desired_scope.clone(),
            generation,
            material_sha256,
        )
    }
}

pub fn is_agent_access_grant_effect(effect: &OwnedEffectV1) -> bool {
    effect.kind == OwnedEffectKind::Secret
        && effect.effect_id.starts_with("agent-access-grant:")
        && effect
            .compensation
            .get("schema")
            .and_then(serde_json::Value::as_str)
            == Some(AGENT_ACCESS_GRANT_EFFECT_SCHEMA_V1)
}

fn encode_base64url_no_pad(input: &[u8; MATERIAL_ENTROPY_BYTES]) -> Vec<u8> {
    let mut output = Vec::with_capacity(MATERIAL_BASE64URL_BYTES);
    let mut index = 0;
    while index + 3 <= input.len() {
        let value = (u32::from(input[index]) << 16)
            | (u32::from(input[index + 1]) << 8)
            | u32::from(input[index + 2]);
        output.push(BASE64URL[((value >> 18) & 0x3f) as usize]);
        output.push(BASE64URL[((value >> 12) & 0x3f) as usize]);
        output.push(BASE64URL[((value >> 6) & 0x3f) as usize]);
        output.push(BASE64URL[(value & 0x3f) as usize]);
        index += 3;
    }
    let value = (u32::from(input[index]) << 16) | (u32::from(input[index + 1]) << 8);
    output.push(BASE64URL[((value >> 18) & 0x3f) as usize]);
    output.push(BASE64URL[((value >> 12) & 0x3f) as usize]);
    output.push(BASE64URL[((value >> 6) & 0x3f) as usize]);
    debug_assert_eq!(output.len(), MATERIAL_BASE64URL_BYTES);
    output
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn material_is_fixed_printable_base64url_and_hash_only_is_serializable() {
        let material = AgentAccessGrantMaterial::from_csprng_entropy([0xa5; 32]);
        assert_eq!(material.expose().len(), 43);
        assert!(
            material
                .expose()
                .iter()
                .all(|byte| BASE64URL.contains(byte))
        );
        let hash = material.sha256();
        assert_eq!(hash.as_str().len(), 71);
    }

    #[test]
    fn custom_material_accepts_only_bounded_header_safe_ascii() {
        for invalid in [
            b"short".as_slice(),
            b"contains a space-and-long".as_slice(),
            b"contains\nnewline-and-long".as_slice(),
            b"contains:colon-and-long".as_slice(),
        ] {
            assert!(!valid_user_agent_token(invalid));
            assert!(AgentAccessGrantMaterial::from_user_input(invalid.to_vec()).is_err());
        }
        let valid = b"custom-token_01.~abcd";
        assert!(valid_user_agent_token(valid));
        assert_eq!(
            AgentAccessGrantMaterial::from_user_input(valid.to_vec())
                .unwrap()
                .expose(),
            valid
        );
    }

    #[test]
    fn ensure_effect_reference_rejects_a_mismatched_effect_identity() {
        let scope = AgentAccessGrantScopeV1::new(
            "agent-connection/claude",
            crate::AgentModelGrantV2::seal(
                AgentIngressProtocolV1::Messages,
                std::collections::BTreeMap::from([(
                    "hiroute/0011223344556677".into(),
                    crate::AgentModelRouteV2::Plan {
                        plan_id: crate::AgentPlanId::parse("plan/test").unwrap(),
                        alias: crate::ModelAlias::parse("hiroute/0011223344556677").unwrap(),
                        revision: 1,
                        semantic_digest: CanonicalDigest::of_bytes(b"plan-grant"),
                    },
                )]),
            )
            .unwrap(),
        )
        .unwrap();
        let mutation =
            AgentAccessGrantMutationV1::ensure("personal/default", scope.clone(), 0).unwrap();
        let material = AgentAccessGrantMaterial::from_csprng_entropy([0x5a; 32]);
        let hash = material.sha256();
        let effect = OwnedEffectV1 {
            effect_id: "agent-access-grant:agent-connection/other".to_owned(),
            kind: OwnedEffectKind::Secret,
            target: "agent-connection/claude".to_owned(),
            before_fingerprint: None,
            after_fingerprint: Some(hash.clone()),
            compensation: json!({
                "schema": AGENT_ACCESS_GRANT_EFFECT_SCHEMA_V1,
                "operation_id": "op_11111111111111111111111111111111",
                "connection_id": "agent-connection/claude",
                "owner_scope": "personal/default",
                "grant_id": "grant_11111111111111111111111111111111",
                "generation": 1,
                "scope_hash": scope.digest().unwrap(),
                "material_sha256": hash,
            })
            .into(),
        };
        assert!(AgentAccessGrantRefV1::from_ensure_effect(&effect, &mutation).is_err());
    }
}
