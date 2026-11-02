//! V3 seals the already previewed context snapshot, including its metadata identity.
use super::*;
use crate::{DecisionExpectedSourceRevisionV1, DecisionServiceChangeV3, DecisionServiceRecord};

pub const DECISION_SERVICE_OPERATION_INPUT_SCHEMA_V3: &str =
    "hiroute.decision-service-operation-input/v3";
pub const DECISION_SERVICE_OPERATION_SCHEMA_V3: &str = "hiroute.decision-service-operation/v3";

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServiceOperationInputV3 {
    pub schema: String,
    pub change: DecisionServiceChangeV3,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_slot: Option<String>,
}

impl DecisionServiceOperationInputV3 {
    pub fn validate(&self) -> bool {
        self.schema == DECISION_SERVICE_OPERATION_INPUT_SCHEMA_V3 && self.change.validate()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServiceOperationV3 {
    pub schema: String,
    pub input: DecisionServiceOperationInputV3,
    pub sources: Vec<DecisionSourceMutationV1>,
}

/// In-process normalization only. The complete V3 service survives normalization;
/// this type is never an alternative wire or persisted format.
pub struct DecisionServiceMutationState {
    pub id: String,
    pub expected_revision: u64,
    pub service: Option<DecisionServiceRecord>,
    pub expected_source_revisions: Vec<DecisionExpectedSourceRevisionV1>,
}
pub struct DecisionServiceInputState {
    pub change: DecisionServiceMutationState,
    pub input_slot: Option<String>,
}
pub struct DecisionServiceOperationState {
    pub input: DecisionServiceInputState,
    pub sources: Vec<DecisionSourceMutationV1>,
}

impl DecisionServiceOperationV2 {
    pub fn into_state(self) -> DecisionServiceOperationState {
        let change = self.input.change;
        DecisionServiceOperationState {
            input: DecisionServiceInputState {
                change: DecisionServiceMutationState {
                    id: change.id,
                    expected_revision: change.expected_revision,
                    service: change.service.map(DecisionServiceRecord::V2),
                    expected_source_revisions: change.expected_source_revisions,
                },
                input_slot: self.input.input_slot,
            },
            sources: self.sources,
        }
    }
}
impl DecisionServiceOperationV3 {
    pub fn into_state(self) -> DecisionServiceOperationState {
        let change = self.input.change;
        DecisionServiceOperationState {
            input: DecisionServiceInputState {
                change: DecisionServiceMutationState {
                    id: change.id,
                    expected_revision: change.expected_revision,
                    service: change.service.map(DecisionServiceRecord::V3),
                    expected_source_revisions: change.expected_source_revisions,
                },
                input_slot: self.input.input_slot,
            },
            sources: self.sources,
        }
    }
}

impl DecisionServiceOperationState {
    pub fn from_control(control: &Value) -> Result<Option<Self>, OperationValidationError> {
        let invalid = || OperationValidationError::UnregisteredEffectPlan;
        match (
            control.get("decision_service_operation_v2"),
            control.get("decision_service_operation_v3"),
        ) {
            (Some(value), None) => {
                let operation: DecisionServiceOperationV2 =
                    serde_json::from_value(value.clone()).map_err(|_| invalid())?;
                if operation.schema != DECISION_SERVICE_OPERATION_SCHEMA_V2
                    || !operation.input.change.validate()
                {
                    return Err(invalid());
                }
                Ok(Some(operation.into_state()))
            }
            (None, Some(value)) => {
                let operation: DecisionServiceOperationV3 =
                    serde_json::from_value(value.clone()).map_err(|_| invalid())?;
                if operation.schema != DECISION_SERVICE_OPERATION_SCHEMA_V3
                    || !operation.input.validate()
                {
                    return Err(invalid());
                }
                Ok(Some(operation.into_state()))
            }
            (None, None) => Ok(None),
            _ => Err(invalid()),
        }
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
    let input: DecisionServiceOperationInputV3 =
        serde_json::from_value(spec.desired_state.clone()).map_err(|_| invalid())?;
    let operation: DecisionServiceOperationV3 = serde_json::from_value(
        control
            .get("decision_service_operation_v3")
            .cloned()
            .ok_or_else(invalid)?,
    )
    .map_err(|_| invalid())?;
    if operation.schema != DECISION_SERVICE_OPERATION_SCHEMA_V3
        || !input.validate()
        || input != operation.input
        || control != &json!({"decision_service_operation_v3":operation})
        || spec.resource_id.as_deref() != Some(input.change.id.as_str())
        || credential_pool.is_some()
        || !runtime.is_empty()
        || !external.is_empty()
    {
        return Err(invalid());
    }
    super::decision_service_v2::validate_effects(&operation.into_state(), secrets)
}
