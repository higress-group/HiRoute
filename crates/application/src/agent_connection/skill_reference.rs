//! Reference-only shared Skill changes. These update the control record without claiming,
//! rewriting, chmodding, or deleting an identical file owned by the user.

use super::{SkillFileAction, SkillOwnershipPlan, identity};
use hiroute_domain::{
    AgentFacetIntent, AgentSettingsSpecV2, AgentSkillInstallationStorePort, CanonicalDigest,
    CollaborationSkillFileOwnership, ManagedCollaborationSkill, NativeAgentArtifactPort,
    OperationState, OperationV1, OperationValidationError, PortError, PortErrorCode, PortResult,
};
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "hiroute.settings-skill-reference/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillReferenceAction {
    Add,
    Remove,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsSkillReferenceChange {
    schema: String,
    context_id: String,
    target: String,
    observed_fingerprint: Option<CanonicalDigest>,
    before: Option<ManagedCollaborationSkill>,
    next: ManagedCollaborationSkill,
    action: SkillReferenceAction,
}

impl SettingsSkillReferenceChange {
    fn validate(&self) -> Result<(), OperationValidationError> {
        let invalid = || OperationValidationError::UnregisteredEffectPlan;
        if self.schema != SCHEMA || !identity(&self.context_id) || !identity(&self.target) {
            return Err(invalid());
        }
        self.next.validate().map_err(|_| invalid())?;
        let releases_borrowed_reference = self.action == SkillReferenceAction::Remove
            && self.next.file_ownership == CollaborationSkillFileOwnership::BorrowedIdentical;
        if self.observed_fingerprint.is_none() != releases_borrowed_reference {
            return Err(invalid());
        }
        let mut contexts = if let Some(before) = &self.before {
            before.validate().map_err(|_| invalid())?;
            if before.root_ref != self.next.root_ref
                || before.template_revision != self.next.template_revision
                || before.content_digest != self.next.content_digest
                || before.file_ownership != self.next.file_ownership
                || before.file_effect != self.next.file_effect
                || before.revision.checked_add(1) != Some(self.next.revision)
            {
                return Err(invalid());
            }
            before.contexts.clone()
        } else {
            if self.action != SkillReferenceAction::Add
                || self.next.revision != 1
                || self.next.file_ownership != CollaborationSkillFileOwnership::BorrowedIdentical
                || self.next.file_effect.is_some()
            {
                return Err(invalid());
            }
            Default::default()
        };
        let changed = match self.action {
            SkillReferenceAction::Add => contexts.insert(self.context_id.clone()),
            SkillReferenceAction::Remove => contexts.remove(&self.context_id),
        };
        if !changed
            || contexts != self.next.contexts
            || self.action == SkillReferenceAction::Remove
                && self.next.contexts.is_empty()
                && self.next.file_ownership == CollaborationSkillFileOwnership::Managed
        {
            return Err(invalid());
        }
        Ok(())
    }
}

pub fn settings_skill_reference_change(
    context_id: &str,
    target: &str,
    before: Option<&ManagedCollaborationSkill>,
    plan: &SkillOwnershipPlan,
    observed_fingerprint: Option<CanonicalDigest>,
    action: SkillReferenceAction,
) -> Result<SettingsSkillReferenceChange, OperationValidationError> {
    if plan.file_action != SkillFileAction::Keep || before == Some(&plan.next) {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let observed_fingerprint = if action == SkillReferenceAction::Remove
        && plan.next.file_ownership == CollaborationSkillFileOwnership::BorrowedIdentical
    {
        None
    } else {
        Some(observed_fingerprint.ok_or(OperationValidationError::UnregisteredEffectPlan)?)
    };
    let change = SettingsSkillReferenceChange {
        schema: SCHEMA.into(),
        context_id: context_id.into(),
        target: target.into(),
        observed_fingerprint,
        before: before.cloned(),
        next: plan.next.clone(),
        action,
    };
    change.validate()?;
    Ok(change)
}

/// Persists a reference-only change while the original settings Operation still owns the writer.
/// Additions and managed shared-file edits recheck the exact file. Releasing a borrowed reference
/// deliberately has no file precondition because HiRoute does not own that file.
pub fn persist_settings_skill_reference_change(
    store: &dyn AgentSkillInstallationStorePort,
    native: &dyn NativeAgentArtifactPort,
    operation: &OperationV1,
    expected_target: &str,
) -> PortResult<Option<ManagedCollaborationSkill>> {
    let spec: AgentSettingsSpecV2 =
        serde_json::from_value(operation.plan.spec().desired_state.clone())
            .map_err(|_| PortError::new(PortErrorCode::InvalidData, "skill.reference.intent"))?;
    persist_settings_skill_reference_change_for_facet(
        store,
        native,
        operation,
        spec.skill_facet(),
        expected_target,
    )
}

pub fn persist_settings_skill_reference_change_for_facet(
    store: &dyn AgentSkillInstallationStorePort,
    native: &dyn NativeAgentArtifactPort,
    operation: &OperationV1,
    facet: hiroute_domain::AgentSettingsFacet,
    expected_target: &str,
) -> PortResult<Option<ManagedCollaborationSkill>> {
    let invalid = || PortError::new(PortErrorCode::InvalidData, "skill.reference.intent");
    if operation.state != OperationState::Succeeded
        || operation.plan.spec().command_id != "agents.settings.apply"
    {
        return Err(invalid());
    }
    let spec: AgentSettingsSpecV2 =
        serde_json::from_value(operation.plan.spec().desired_state.clone())
            .map_err(|_| invalid())?;
    let additional = facet != spec.skill_facet();
    let record_key = if additional {
        "additional_skill_record_change"
    } else {
        "skill_record_change"
    };
    let reference_key = if additional {
        "additional_skill_reference_change"
    } else {
        "skill_reference_change"
    };
    let mutations = operation
        .plan
        .control()
        .pointer("/payload/state/mutations")
        .ok_or_else(invalid)?;
    if additional && matches!(spec.skill_intent_for(facet), AgentFacetIntent::Keep) {
        return Ok(None);
    }
    let record_change = mutations
        .get(record_key)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(invalid)?;
    let encoded = mutations.get(reference_key).ok_or_else(invalid)?;
    let role = hiroute_domain::AgentConnectionEffectRoleV1::settings_skill(&spec.context_id, facet)
        .ok_or_else(invalid)?;
    let routing_skill_file_change = operation.plan.external().iter().any(|intent| {
        intent.desired()["role"] == role.as_str() && intent.desired()["transaction"] == "settings"
    });
    if encoded.is_null() {
        return if record_change == routing_skill_file_change {
            Ok(None)
        } else {
            Err(invalid())
        };
    }
    let change: SettingsSkillReferenceChange =
        serde_json::from_value(encoded.clone()).map_err(|_| invalid())?;
    change.validate().map_err(|_| invalid())?;
    let spec: AgentSettingsSpecV2 =
        serde_json::from_value(operation.plan.spec().desired_state.clone())
            .map_err(|_| invalid())?;
    let expected_action = match spec.skill_intent_for(facet) {
        AgentFacetIntent::Configure { .. } => SkillReferenceAction::Add,
        AgentFacetIntent::Restore { .. } => SkillReferenceAction::Remove,
        AgentFacetIntent::Keep => return Err(invalid()),
    };
    if !record_change
        || change.action != expected_action
        || change.context_id != spec.context_id
        || operation.plan.spec().resource_id.as_deref() != Some(change.context_id.as_str())
        || change.target != expected_target
        || routing_skill_file_change
    {
        return Err(invalid());
    }
    if let Some(observed_fingerprint) = &change.observed_fingerprint {
        if native
            .current_external_fingerprint(expected_target)?
            .as_ref()
            != Some(observed_fingerprint)
        {
            return Err(PortError::new(
                PortErrorCode::Conflict,
                "skill.reference.file_changed",
            ));
        }
        let current = native.read_native_target(expected_target)?.ok_or_else(|| {
            PortError::new(PortErrorCode::Conflict, "skill.reference.file_missing")
        })?;
        if native
            .current_external_fingerprint(expected_target)?
            .as_ref()
            != Some(observed_fingerprint)
            || CanonicalDigest::of_bytes(&current) != change.next.content_digest
        {
            return Err(PortError::new(
                PortErrorCode::Conflict,
                "skill.reference.content_changed",
            ));
        }
    }
    let expected_revision = change.before.as_ref().map_or(0, |before| before.revision);
    store.store_skill_installation(
        &operation.workspace_id,
        &operation.operation_id,
        expected_revision,
        &change.next,
    )?;
    if store
        .skill_installation(&operation.workspace_id, &change.next.root_ref)?
        .as_ref()
        != Some(&change.next)
    {
        return Err(PortError::new(
            PortErrorCode::Conflict,
            "skill.reference.not_observed",
        ));
    }
    Ok(Some(change.next))
}
