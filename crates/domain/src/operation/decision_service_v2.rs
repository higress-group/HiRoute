//! The decision service and every affected source are one typed Operation.
use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::{
    ComputeManagementSourceV2, DecisionAuthenticationV2, DecisionConnectionV2,
    DecisionServiceChangeV2, register_decision_consumer, remove_decision_consumers,
};

pub const DECISION_SERVICE_OPERATION_SCHEMA_V2: &str = "hiroute.decision-service-operation/v2";

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServiceOperationInputV2 {
    pub change: DecisionServiceChangeV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_slot: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionSourceMutationV1 {
    pub before: ComputeManagementSourceV2,
    pub after: ComputeManagementSourceV2,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServiceOperationV2 {
    pub schema: String,
    pub input: DecisionServiceOperationInputV2,
    pub sources: Vec<DecisionSourceMutationV1>,
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
    let input: DecisionServiceOperationInputV2 =
        serde_json::from_value(spec.desired_state.clone()).map_err(|_| invalid())?;
    let operation: DecisionServiceOperationV2 = serde_json::from_value(
        control
            .get("decision_service_operation_v2")
            .cloned()
            .ok_or_else(invalid)?,
    )
    .map_err(|_| invalid())?;
    if operation.schema != DECISION_SERVICE_OPERATION_SCHEMA_V2
        || operation.input != input
        || control != &json!({"decision_service_operation_v2":operation})
        || !input.change.validate()
        || spec.resource_id.as_deref() != Some(input.change.id.as_str())
        || credential_pool.is_some()
        || !runtime.is_empty()
        || !external.is_empty()
    {
        return Err(invalid());
    }
    if let Some(slot) = &input.input_slot {
        validate_scope_identifier(slot)?;
    }
    let shared_source =
        input
            .change
            .service
            .as_ref()
            .and_then(|service| match &service.connection {
                DecisionConnectionV2::Native {
                    authentication: DecisionAuthenticationV2::SharedSource { source_id, .. },
                    ..
                } => Some(source_id.as_str()),
                _ => None,
            });
    if shared_source.is_some() && input.input_slot.is_some()
        || input.change.service.is_none() && input.input_slot.is_some()
    {
        return Err(invalid());
    }
    if input.change.service.is_some()
        && operation.sources.len() != usize::from(shared_source.is_some())
    {
        return Err(invalid());
    }
    let revisions: BTreeMap<_, _> = input
        .change
        .expected_source_revisions
        .iter()
        .map(|entry| (entry.source_id.as_str(), entry.revision))
        .collect();
    if revisions.len() != operation.sources.len() {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    let mut expected_secrets = BTreeMap::new();
    for source in &operation.sources {
        if !seen.insert(source.before.source_id.as_str())
            || revisions.get(source.before.source_id.as_str()) != Some(&source.before.revision)
        {
            return Err(invalid());
        }
        let expected = match &input.change.service {
            Some(service) if shared_source == Some(source.before.source_id.as_str()) => {
                register_decision_consumer(&source.before, service).map_err(|_| invalid())?
            }
            None => remove_decision_consumers(&source.before, &input.change.id)
                .map_err(|_| invalid())?
                .ok_or_else(invalid)?,
            _ => return Err(invalid()),
        };
        if expected.after != source.after {
            return Err(invalid());
        }
        for secret in expected.secrets {
            if expected_secrets
                .insert(secret.credential().credential_id().to_owned(), secret)
                .is_some()
            {
                return Err(invalid());
            }
        }
    }
    let mut independent = Vec::new();
    let mut secret_ids = BTreeSet::new();
    for secret in secrets {
        let id = secret.credential().credential_id();
        if !secret_ids.insert(id) {
            return Err(invalid());
        }
        if let Some(expected) = expected_secrets.remove(id) {
            if &expected != secret {
                return Err(invalid());
            }
        } else {
            independent.push(secret.clone());
        }
    }
    if !expected_secrets.is_empty() {
        return Err(invalid());
    }
    match &input.input_slot {
        None if independent.is_empty() => Ok(()),
        Some(slot) if independent.len() == 1 => {
            let header = input
                .change
                .service
                .as_ref()
                .and_then(|service| service.connection.transport().2)
                .ok_or_else(invalid)?;
            let secret_spec = ChangeSpecV1 {
                schema_version: crate::CHANGE_SPEC_SCHEMA_V1,
                command_id: "routing.classifier.secret.apply".into(),
                resource_id: Some("personal/default".into()),
                desired_state: json!({"secret_id":header.value_secret_ref,"input_slot":slot,"expected_generation":0}),
            };
            super::validate_classifier_header_secret_plan(
                &secret_spec,
                &json!({"classifier_header_secret":header.value_secret_ref}),
                None,
                &independent,
                &[],
                &[],
            )
        }
        _ => Err(invalid()),
    }
}
