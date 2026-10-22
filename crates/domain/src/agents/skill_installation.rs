//! Shared Skill file ownership data, independent of any per-context authorization.
use crate::{CanonicalDigest, ExternalEffectIntentV1, OperationId, OwnedEffectKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const MANAGED_COLLABORATION_SKILL_SCHEMA: &str = "hiroute.managed-collaboration-skill/v2";

/// Shared-root metadata only. Writes belong to the admitted original Operation writer; this
/// port neither authorizes collaboration nor performs native file IO.
pub trait AgentSkillInstallationStorePort {
    fn skill_installation(
        &self,
        workspace: &crate::WorkspaceId,
        root_ref: &str,
    ) -> crate::PortResult<Option<ManagedCollaborationSkill>>;

    fn store_skill_installation(
        &self,
        workspace: &crate::WorkspaceId,
        operation: &OperationId,
        expected_revision: u64,
        next: &ManagedCollaborationSkill,
    ) -> crate::PortResult<()>;
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("Skill ownership record is invalid")]
pub struct InvalidSkillRecord;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedCollaborationSkill {
    pub schema: String,
    pub root_ref: String,
    pub template_revision: String,
    pub content_digest: CanonicalDigest,
    pub file_ownership: CollaborationSkillFileOwnership,
    pub contexts: BTreeSet<String>,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_effect: Option<SkillFileEffectRef>,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationSkillFileOwnership {
    /// HiRoute created the file and may remove its exact managed generation.
    Managed,
    /// An identical file already existed. HiRoute may reference it but never overwrite or remove it.
    BorrowedIdentical,
}
impl ManagedCollaborationSkill {
    pub fn validate(&self) -> Result<(), InvalidSkillRecord> {
        if self.schema != MANAGED_COLLABORATION_SKILL_SCHEMA
            || !identity(&self.root_ref)
            || !identity(&self.template_revision)
            || self.revision == 0
            || self.contexts.len() > 1024
            || !self.contexts.iter().all(|context| identity(context))
            || CanonicalDigest::parse(self.content_digest.as_str().to_owned()).is_err()
        {
            return Err(InvalidSkillRecord);
        }
        if let Some(reference) = &self.file_effect {
            reference.validate()?;
            if !matches!(
                reference.effect_id.as_str(),
                "agent-connection-routing-skill"
                    | "agent-connection-management-skill"
                    | "agent-connection-collaboration-skill"
            ) {
                return Err(InvalidSkillRecord);
            }
        }
        if self.file_ownership == CollaborationSkillFileOwnership::BorrowedIdentical
            && self.file_effect.is_some()
        {
            return Err(InvalidSkillRecord);
        }
        Ok(())
    }
}

fn identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_./:-".contains(&byte))
}

/// Opaque binding to the protected artifact marker, not a filesystem path or authorization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkillFileEffectRef {
    pub operation_id: OperationId,
    pub effect_id: String,
    pub target: String,
    pub intent_digest: CanonicalDigest,
}
impl SkillFileEffectRef {
    pub fn from_intent(
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> Result<Self, InvalidSkillRecord> {
        if intent.kind() != OwnedEffectKind::AgentArtifact
            || intent.desired_mode() != 0o644
            || intent.sensitive()
        {
            return Err(InvalidSkillRecord);
        }
        let value = Self {
            operation_id: operation.clone(),
            effect_id: intent.effect_id().into(),
            target: intent.target().into(),
            intent_digest: CanonicalDigest::of(intent.desired()).map_err(|_| InvalidSkillRecord)?,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), InvalidSkillRecord> {
        if !matches!(
            self.effect_id.as_str(),
            "agent-connection-routing-skill"
                | "agent-connection-management-skill"
                | "agent-connection-collaboration-skill"
                | "agent-connection-hermes-plugin-module"
                | "agent-connection-hermes-plugin-manifest"
                | "agent-connection-native-routing-adapter"
        ) || !identity(&self.target)
            || CanonicalDigest::parse(self.intent_digest.as_str().to_owned()).is_err()
        {
            return Err(InvalidSkillRecord);
        }
        Ok(())
    }
}
