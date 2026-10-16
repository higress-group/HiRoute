//! A self-contained Skill file effect in the original Operation journal.
//! The template is non-secret product content; recovery never selects a newer installed bundle.
use super::*;
use hiroute_domain::{
    AgentConnectionControlIntentV1, AgentConnectionEffectRoleV1, AgentConnectionTransactionKindV1,
    OperationValidationError, SkillFileEffectRef,
};
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "hiroute.settings-skill-file/v1";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SkillFilePayload {
    schema: String,
    context_id: String,
    before: Option<ManagedCollaborationSkill>,
    next: ManagedCollaborationSkill,
    action: SkillFileAction,
    content: Option<String>,
}

impl SkillFilePayload {
    fn validate(&self) -> Result<(), OperationValidationError> {
        let invalid = || OperationValidationError::UnregisteredEffectPlan;
        if self.schema != SCHEMA || !identity(&self.context_id) {
            return Err(invalid());
        }
        self.next.validate().map_err(|_| invalid())?;
        let mut contexts = std::collections::BTreeSet::new();
        let mut revision = 0;
        if let Some(before) = &self.before {
            before.validate().map_err(|_| invalid())?;
            if before.root_ref != self.next.root_ref
                || before.file_ownership != self.next.file_ownership
                || before.file_effect != self.next.file_effect
            {
                return Err(invalid());
            }
            contexts = before.contexts.clone();
            revision = before.revision;
        } else if self.next.file_effect.is_some() {
            return Err(invalid());
        }
        if revision.checked_add(1) != Some(self.next.revision) {
            return Err(invalid());
        }
        match self.action {
            SkillFileAction::Install => {
                if self.before.is_none()
                    && self.next.file_ownership != CollaborationSkillFileOwnership::Managed
                {
                    return Err(invalid());
                }
                contexts.insert(self.context_id.clone());
                let content = self.content.as_deref().ok_or_else(invalid)?;
                if content.is_empty()
                    || content.len() > 16 * 1024
                    || content.contains('\0')
                    || CanonicalDigest::of_bytes(content.as_bytes()) != self.next.content_digest
                {
                    return Err(invalid());
                }
            }
            SkillFileAction::Remove => {
                let before = self.before.as_ref().ok_or_else(invalid)?;
                contexts.remove(&self.context_id);
                if self.content.is_some()
                    || !contexts.is_empty()
                    || before.file_ownership != CollaborationSkillFileOwnership::Managed
                    || before.template_revision != self.next.template_revision
                    || before.content_digest != self.next.content_digest
                {
                    return Err(invalid());
                }
            }
            SkillFileAction::Keep | SkillFileAction::Conflict => return Err(invalid()),
        }
        if contexts != self.next.contexts {
            return Err(invalid());
        }
        Ok(())
    }
}

/// Called by the settings planner's renderer after confirmation. A reference-only change has
/// no external file effect and belongs to the control mutation. Unknown/user-owned files are
/// rejected by the ownership planner before reaching this constructor.
pub fn settings_skill_file_intent(
    control: &AgentConnectionControlIntentV1,
    context_id: &str,
    before: Option<&ManagedCollaborationSkill>,
    plan: &SkillOwnershipPlan,
    template: Option<&CollaborationSkillTemplate>,
    before_fingerprint: Option<CanonicalDigest>,
) -> Result<ExternalEffectIntentV1, OperationValidationError> {
    let facet = if control.subject().agent_id() == "agent_hermes_default" {
        hiroute_domain::AgentSettingsFacet::ManagementSkill
    } else {
        hiroute_domain::AgentSettingsFacet::Collaboration
    };
    settings_skill_file_intent_for_facet(
        control,
        context_id,
        facet,
        before,
        plan,
        template,
        before_fingerprint,
    )
}

pub fn settings_skill_file_intent_for_facet(
    control: &AgentConnectionControlIntentV1,
    context_id: &str,
    facet: hiroute_domain::AgentSettingsFacet,
    before: Option<&ManagedCollaborationSkill>,
    plan: &SkillOwnershipPlan,
    template: Option<&CollaborationSkillTemplate>,
    before_fingerprint: Option<CanonicalDigest>,
) -> Result<ExternalEffectIntentV1, OperationValidationError> {
    if control.transaction() != AgentConnectionTransactionKindV1::Settings {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let payload = SkillFilePayload {
        schema: SCHEMA.into(),
        context_id: context_id.into(),
        before: before.cloned(),
        next: plan.next.clone(),
        action: plan.file_action,
        content: template.map(|value| value.content.to_owned()),
    };
    payload.validate()?;
    if template.is_some_and(|value| {
        value.revision != plan.next.template_revision || value.digest != plan.next.content_digest
    }) {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    ExternalEffectIntentV1::from_agent_connection_planner(
        control,
        AgentConnectionEffectRoleV1::settings_skill(context_id, facet)
            .ok_or(OperationValidationError::UnregisteredEffectPlan)?,
        before_fingerprint,
        &payload,
        0o644,
    )
}

pub fn is_settings_skill_file(intent: &ExternalEffectIntentV1) -> bool {
    matches!(
        intent.effect_id(),
        "agent-connection-routing-skill"
            | "agent-connection-management-skill"
            | "agent-connection-collaboration-skill"
    ) && intent.desired()["transaction"] == "settings"
}

fn decode(intent: &ExternalEffectIntentV1) -> PortResult<SkillFilePayload> {
    let invalid = || PortError::new(PortErrorCode::InvalidData, "skill.journal.invalid");
    // Do not let this decoder turn arbitrary JSON into a registered file target.
    ExternalEffectIntentV1::from_registered_adapter(
        intent.effect_id(),
        intent.kind(),
        intent.target(),
        intent.before_fingerprint().cloned(),
        intent.desired().clone(),
        intent.desired_mode(),
        intent.sensitive(),
    )
    .map_err(|_| invalid())?;
    if !is_settings_skill_file(intent)
        || intent.desired_mode() != 0o644
        || intent
            .desired()
            .get("transaction")
            .and_then(serde_json::Value::as_str)
            != Some("settings")
    {
        return Err(invalid());
    }
    let payload: SkillFilePayload =
        serde_json::from_value(intent.desired().get("payload").ok_or_else(invalid)?.clone())
            .map_err(|_| invalid())?;
    payload.validate().map_err(|_| invalid())?;
    Ok(payload)
}

/// Decode the bounded, registered file intent before consuming a one-shot Apply capability.
pub fn validate_settings_skill_file_intent(intent: &ExternalEffectIntentV1) -> PortResult<()> {
    decode(intent).map(|_| ())
}

/// Reconstruct the exact record to persist through the existing root-reference CAS. This
/// projection does not authorize a grant or prove that a file or control mutation has executed.
pub fn settings_skill_file_record(
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
) -> PortResult<(u64, ManagedCollaborationSkill)> {
    let payload = decode(intent)?;
    let expected_revision = payload.before.as_ref().map_or(0, |before| before.revision);
    let mut next = payload.next;
    next.file_effect = Some(
        SkillFileEffectRef::from_intent(operation, intent)
            .map_err(|_| PortError::new(PortErrorCode::InvalidData, "skill.journal.reference"))?,
    );
    Ok((expected_revision, next))
}

/// Persist the shared-root record from the ORIGINAL admitted plan after its native file is
/// staged. Call inside the existing activation step, after any required durable revocation.
/// The CAS is independently recoverable: a failure never grants authority or rolls back a
/// revoked generation. This does not certify that selected grant/permit/file effects finished.
pub fn persist_settings_skill_file_record(
    store: &dyn hiroute_domain::AgentSkillInstallationStorePort,
    native: &dyn NativeAgentArtifactPort,
    operation: &hiroute_domain::OperationV1,
    intent: &ExternalEffectIntentV1,
) -> PortResult<ManagedCollaborationSkill> {
    if operation.state != hiroute_domain::OperationState::Activating
        || operation.plan.spec().command_id
            != AgentConnectionTransactionKindV1::Settings.command_id()
        || !operation.plan.external().contains(intent)
    {
        return Err(PortError::new(
            PortErrorCode::InvalidData,
            "skill.reference.operation",
        ));
    }
    if !matches!(
        native.observe_artifact(&operation.operation_id, intent)?,
        EffectReconciliation::Staged(_) | EffectReconciliation::Applied(_)
    ) {
        return Err(PortError::new(
            PortErrorCode::Conflict,
            "skill.reference.file_not_staged",
        ));
    }
    let (expected_revision, next) = settings_skill_file_record(&operation.operation_id, intent)?;
    store.store_skill_installation(
        &operation.workspace_id,
        &operation.operation_id,
        expected_revision,
        &next,
    )?;
    if store
        .skill_installation(&operation.workspace_id, &next.root_ref)?
        .as_ref()
        != Some(&next)
    {
        return Err(PortError::new(
            PortErrorCode::Conflict,
            "skill.reference.not_observed",
        ));
    }
    Ok(next)
}

/// The daemon's existing ApplyAgentArtifacts step calls this with its original Operation ID.
/// Removal activation still follows durable revocation; staging alone changes no native file.
pub fn stage_settings_skill_file(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
) -> PortResult<OwnedEffectV1> {
    let payload = decode(intent)?;
    let mut plan = SkillOwnershipPlan {
        next: payload.next,
        file_action: payload.action,
    };
    stage_skill_bytes(
        port,
        operation,
        intent,
        payload.before.as_ref(),
        &mut plan,
        payload.content.as_deref().map(str::as_bytes),
    )?
    .ok_or_else(|| PortError::new(PortErrorCode::InvalidData, "skill.journal.no_file_effect"))
}
