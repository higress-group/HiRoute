//! Independent settings enter the existing journal with a facet-specific effect contract.
use super::*;
use crate::{
    AGENT_SETTINGS_SCHEMA_V2, AgentAccessGrantMaterialActionV1, AgentAccessTokenIntentV1,
    AgentFacetIntent, AgentSettingsSpecV2,
};

impl AgentConnectionControlIntentV1 {
    /// The Application planner supplies the reproduced settings and current shared Skill file
    /// decision. This is not management authorization; admission still requires confirmation,
    /// the protected Apply capability, current dependencies and the existing writer boundary.
    pub fn from_settings_planner<T: Serialize>(
        subject: AgentConnectionTransactionSubjectV1,
        spec: &ChangeSpecV1,
        skill_file_change: bool,
        payload: &T,
    ) -> Result<Self, OperationValidationError> {
        let settings = decode_spec(spec)?;
        if skill_file_change && matches!(settings.collaboration, AgentFacetIntent::Keep) {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        Self::from_registered_planner(
            AgentConnectionTransactionKindV1::Settings,
            subject,
            spec,
            &serde_json::json!({
                "skill_file_change": skill_file_change,
                "state": registered_payload(payload)?,
            }),
        )
    }
}

pub(super) fn decode_spec(
    spec: &ChangeSpecV1,
) -> Result<AgentSettingsSpecV2, OperationValidationError> {
    let settings: AgentSettingsSpecV2 = serde_json::from_value(spec.desired_state.clone())
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    if spec.command_id != AgentConnectionTransactionKindV1::Settings.command_id()
        || settings.schema_version != AGENT_SETTINGS_SCHEMA_V2
        || spec.resource_id.as_deref() != Some(settings.context_id.as_str())
        || !bounded_reference(&settings.context_id)
        || (matches!(settings.model, AgentFacetIntent::Keep)
            && matches!(settings.collaboration, AgentFacetIntent::Keep))
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    match &settings.model {
        AgentFacetIntent::Configure { settings } => {
            settings
                .validate()
                .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        }
        AgentFacetIntent::Restore { restore_point_ref }
            if !bounded_reference(restore_point_ref) =>
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        _ => {}
    }
    if let AgentFacetIntent::Restore { restore_point_ref } = &settings.collaboration
        && !bounded_reference(restore_point_ref)
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    if !matches!(settings.access_token, AgentAccessTokenIntentV1::Keep)
        && !matches!(settings.model, AgentFacetIntent::Configure { .. })
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    if let AgentAccessTokenIntentV1::Set { input_slot } = &settings.access_token
        && (!bounded_reference(input_slot)
            || !input_slot.starts_with("candidate/native/agent-token-"))
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    Ok(settings)
}

fn bounded_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.starts_with('/')
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_./:-".contains(&byte))
}

pub(super) fn validate_effects(
    spec: &ChangeSpecV1,
    payload: &Value,
    roles: &BTreeSet<AgentConnectionEffectRoleV1>,
) -> Result<(), OperationValidationError> {
    let settings = decode_spec(spec)?;
    let object = payload
        .as_object()
        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    if object.len() != 2 || !object.contains_key("state") {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    validate_payload_value(&object["state"])?;
    let skill_file_change = object
        .get("skill_file_change")
        .and_then(Value::as_bool)
        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    if skill_file_change && matches!(settings.collaboration, AgentFacetIntent::Keep) {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let mut required = BTreeSet::new();
    if !matches!(settings.model, AgentFacetIntent::Keep) {
        required.extend([
            AgentConnectionEffectRoleV1::GrantScopedPublication,
            AgentConnectionEffectRoleV1::ManagedConfiguration,
        ]);
    }
    if skill_file_change {
        required.insert(AgentConnectionEffectRoleV1::RoutingSkill);
    }
    // A shared Skill reference or grant-only change may legitimately have no file effect.
    // The sealed planner decision is durable: dropping an expected file on replay is invalid.
    // Legacy native-routing instructions/spawn rewrites never enter independent settings.
    let mut allowed = required.clone();
    if !matches!(settings.model, AgentFacetIntent::Keep) {
        allowed.insert(AgentConnectionEffectRoleV1::ModelCatalog);
        // A grant-creating configure can be the first resident-service connection, and the
        // last managed connection's restore releases the item this feature owns; whether the
        // login-item effect is rendered at all stays with the backend's ownership facts.
        allowed.insert(AgentConnectionEffectRoleV1::LoginItem);
    }
    if !required.is_subset(roles) || !roles.is_subset(&allowed) {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    Ok(())
}

pub(super) fn validate_model_grants(
    spec: &ChangeSpecV1,
    mutations: &[AgentAccessGrantMutationV1],
) -> Result<(), OperationValidationError> {
    let settings = decode_spec(spec)?;
    let expected = match settings.model {
        AgentFacetIntent::Keep => None,
        AgentFacetIntent::Configure { .. } => Some(AgentAccessGrantMutationKindV1::Ensure),
        AgentFacetIntent::Restore { .. } => Some(AgentAccessGrantMutationKindV1::Revoke),
    };
    if mutations.len() != usize::from(expected.is_some()) {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    for mutation in mutations {
        mutation.validate()?;
        if Some(mutation.kind()) != expected
            || mutation.connection_id() != format!("agent-connection/{}", settings.context_id)
            || !matches!(
                (&settings.access_token, mutation.material_action()),
                (
                    AgentAccessTokenIntentV1::Keep,
                    AgentAccessGrantMaterialActionV1::Preserve
                ) | (
                    AgentAccessTokenIntentV1::Regenerate,
                    AgentAccessGrantMaterialActionV1::Regenerate
                ) | (
                    AgentAccessTokenIntentV1::Set { .. },
                    AgentAccessGrantMaterialActionV1::Set { .. }
                )
            )
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        if let (
            AgentAccessTokenIntentV1::Set { input_slot },
            AgentAccessGrantMaterialActionV1::Set {
                input_slot: staged, ..
            },
        ) = (&settings.access_token, mutation.material_action())
            && input_slot != staged
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

/// A renderer cannot silently install/remove a different context's shared Skill reference.
pub(super) fn validate_skill_selection(
    spec: &ChangeSpecV1,
    payload: &Value,
) -> Result<(), OperationValidationError> {
    let settings = decode_spec(spec)?;
    let action = match settings.collaboration {
        AgentFacetIntent::Configure { .. } => "install",
        AgentFacetIntent::Restore { .. } => "remove",
        AgentFacetIntent::Keep => return Err(OperationValidationError::UnregisteredEffectPlan),
    };
    if payload.get("context_id").and_then(Value::as_str) != Some(settings.context_id.as_str())
        || payload.get("action").and_then(Value::as_str) != Some(action)
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    Ok(())
}
