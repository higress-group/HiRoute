//! One service edit, including its optional protected credential, uses the existing journal.
use super::*;
use crate::CHANGE_SPEC_SCHEMA_V1;
use crate::DecisionServiceV1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServiceChangeV1 {
    pub id: String,
    pub expected_revision: u64,
    /// None deletes an unreferenced service. Old versions otherwise remain immutable.
    pub service: Option<DecisionServiceV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_slot: Option<String>,
}

impl DecisionServiceChangeV1 {
    pub fn validate(&self) -> Result<(), OperationValidationError> {
        validate_scope_identifier(&self.id)?;
        if let Some(slot) = &self.input_slot {
            validate_scope_identifier(slot)?;
        }
        if let Some(service) = &self.service {
            if !service.validate()
                || service.id != self.id
                || self.expected_revision.checked_add(1) != Some(service.revision)
                || (self.input_slot.is_some() && service.connection.transport().2.is_none())
            {
                return Err(OperationValidationError::UnregisteredEffectPlan);
            }
        } else if self.expected_revision == 0 || self.input_slot.is_some() {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        Ok(())
    }
}

pub(super) fn validate_plan(
    spec: &ChangeSpecV1,
    control: &Value,
    credential_pool: Option<&CredentialPoolMutationV1>,
    secrets: &[SecretMutationV1],
    runtime: &[RuntimeMutationV1],
    external: &[ExternalEffectIntentV1],
) -> Result<(), OperationValidationError> {
    let invalid = || OperationValidationError::UnregisteredEffectPlan;
    let change: DecisionServiceChangeV1 =
        serde_json::from_value(spec.desired_state.clone()).map_err(|_| invalid())?;
    change.validate()?;
    if spec.resource_id.as_deref() != Some(change.id.as_str())
        || credential_pool.is_some()
        || !runtime.is_empty()
        || !external.is_empty()
        || control != &json!({"decision_service_change":change})
    {
        return Err(invalid());
    }
    match &change.input_slot {
        None if secrets.is_empty() => Ok(()),
        Some(slot) if secrets.len() == 1 => {
            let header = change
                .service
                .as_ref()
                .and_then(|s| s.connection.transport().2)
                .ok_or_else(invalid)?;
            let secret_spec = ChangeSpecV1 {
                schema_version: CHANGE_SPEC_SCHEMA_V1,
                command_id: "routing.classifier.secret.apply".into(),
                resource_id: Some("personal/default".into()),
                desired_state: json!({"secret_id":header.value_secret_ref,"input_slot":slot,"expected_generation":0}),
            };
            super::validate_classifier_header_secret_plan(
                &secret_spec,
                &json!({"classifier_header_secret":header.value_secret_ref}),
                None,
                secrets,
                &[],
                &[],
            )
        }
        _ => Err(invalid()),
    }
}
