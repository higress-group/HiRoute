use hiroute_application_api::{
    COMPUTE_CONNECTION_CHANGE_SCHEMA_V1, COMPUTE_CONNECTION_PREVIEW_SCHEMA_V1,
    ComputeConnectionChangeV1, ComputeConnectionOptionsResultV1, ComputeConnectionPreviewResultV1,
    ComputeProjectionEffectV1, ComputeScanResultV1, PreviewResultV1,
};
use hiroute_domain::{CHANGE_SPEC_SCHEMA_V1, ChangeSpecV1};

use crate::control::{ComputeFactsPort, ComputeProjectionReadError, ControlReadError};

use hiroute_application_api::ComputeConnectionPlannerPayloadV1;

pub(crate) fn validate_apply_spec(
    port: &dyn ComputeFactsPort,
    spec: &ChangeSpecV1,
) -> Result<(), ComputeControlError> {
    if spec.command_id != "compute.connection.apply" {
        return Err(ComputeControlError::InvalidArguments);
    }
    let payload: ComputeConnectionPlannerPayloadV1 =
        serde_json::from_value(spec.desired_state.clone())
            .map_err(|_| ComputeControlError::InvalidArguments)?;
    let projection = &payload.projection;
    projection
        .validate()
        .map_err(|_| ComputeControlError::InvalidSelection)?;
    if spec.resource_id.as_deref() != Some(payload.source_id.as_str())
        || payload.connection_option_id != projection.desired.source.connection_option_id
        || payload.source_id != projection.desired.source.source_id
        || payload.expected_source_revision != projection.expected.source_revision
        || (projection
            .desired
            .source
            .billing_class
            .requires_explicit_materialization()
            && !payload.explicit_materialization)
    {
        return Err(ComputeControlError::InvalidSelection);
    }
    let change = ComputeConnectionChangeV1 {
        schema: COMPUTE_CONNECTION_CHANGE_SCHEMA_V1.into(),
        discovered_source_ref: projection.desired.scanner.discovered_source_ref.clone(),
        connection_option_id: projection.desired.source.connection_option_id.clone(),
        model_configuration_id: projection.desired.binding.model_configuration_id.clone(),
        expected_source_revision: projection.expected.source_revision,
        expected_binding_revision: projection.expected.binding_revision,
        expected_inventory_revision: projection.expected.inventory_revision,
        explicit_materialization: payload.explicit_materialization,
    };
    let current = port.prepare_compute_projection(&change)?;
    if current != payload.projection {
        return Err(ComputeControlError::InvalidSelection);
    }
    Ok(())
}

pub(crate) fn scan_compute(
    port: &dyn ComputeFactsPort,
) -> Result<ComputeScanResultV1, ComputeControlError> {
    port.scan_compute().map_err(Into::into)
}

pub(crate) fn connection_options(
    port: &dyn ComputeFactsPort,
) -> Result<ComputeConnectionOptionsResultV1, ComputeControlError> {
    port.connection_options().map_err(Into::into)
}

pub(crate) struct PreparedComputeChangeV1 {
    pub(crate) change: ComputeConnectionChangeV1,
    pub(crate) prepared: hiroute_domain::PreparedComputeProjectionV1,
    pub(crate) spec: ChangeSpecV1,
}

pub(crate) fn prepare_compute(
    port: &dyn ComputeFactsPort,
    change: ComputeConnectionChangeV1,
) -> Result<PreparedComputeChangeV1, ComputeControlError> {
    validate_change(&change)?;
    let prepared = port.prepare_compute_projection(&change)?;
    let spec = internal_spec(&prepared, change.explicit_materialization)?;
    Ok(PreparedComputeChangeV1 {
        change,
        prepared,
        spec,
    })
}

pub(crate) fn public_preview(
    prepared: PreparedComputeChangeV1,
    result: PreviewResultV1,
) -> ComputeConnectionPreviewResultV1 {
    let spec = prepared.spec;
    let projection = prepared.prepared.desired;
    ComputeConnectionPreviewResultV1 {
        schema: COMPUTE_CONNECTION_PREVIEW_SCHEMA_V1.into(),
        normalized_change: prepared.change,
        spec,
        effects: projection_effects(&prepared.prepared.expected, &projection),
        projection,
        change_digest: result.change_digest,
        expected_revisions: result.expected_revisions,
    }
}

fn internal_spec(
    prepared: &hiroute_domain::PreparedComputeProjectionV1,
    explicit_materialization: bool,
) -> Result<ChangeSpecV1, ComputeControlError> {
    let payload = ComputeConnectionPlannerPayloadV1 {
        connection_option_id: prepared.desired.source.connection_option_id.clone(),
        source_id: prepared.desired.source.source_id.clone(),
        explicit_materialization,
        expected_source_revision: prepared.expected.source_revision,
        projection: prepared.clone(),
    };
    Ok(ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: "compute.connection.apply".into(),
        resource_id: Some(prepared.desired.source.source_id.clone()),
        desired_state: serde_json::to_value(payload)
            .map_err(|_| ComputeControlError::InvalidArguments)?,
    })
}

fn validate_change(
    change: &hiroute_application_api::ComputeConnectionChangeV1,
) -> Result<(), ComputeControlError> {
    if change.schema != COMPUTE_CONNECTION_CHANGE_SCHEMA_V1
        || change.discovered_source_ref.is_empty()
        || change.connection_option_id.is_empty()
        || change.model_configuration_id.is_empty()
    {
        return Err(ComputeControlError::InvalidArguments);
    }
    Ok(())
}

fn projection_effects(
    expected: &hiroute_domain::ComputeProjectionExpectationV1,
    desired: &hiroute_domain::ComputeControlProjectionV1,
) -> Vec<ComputeProjectionEffectV1> {
    vec![
        ComputeProjectionEffectV1 {
            resource_kind: "compute_source".into(),
            resource_id: desired.source.source_id.clone(),
            expected_revision: expected.source_revision,
            desired_revision: desired.source.revision,
        },
        ComputeProjectionEffectV1 {
            resource_kind: "source_binding".into(),
            resource_id: desired.binding.binding_id.clone(),
            expected_revision: expected.binding_revision,
            desired_revision: desired.binding.revision,
        },
        ComputeProjectionEffectV1 {
            resource_kind: "inventory_snapshot".into(),
            resource_id: format!(
                "{}:{}",
                desired.inventory.source_id, desired.inventory.endpoint_profile_id
            ),
            expected_revision: expected.inventory_revision,
            desired_revision: desired.inventory.inventory_revision,
        },
    ]
}

#[derive(Debug)]
pub(crate) enum ComputeControlError {
    InvalidArguments,
    InvalidSelection,
    RevisionConflict,
    ActionRequired,
    Control(ControlReadError),
}

impl From<ControlReadError> for ComputeControlError {
    fn from(value: ControlReadError) -> Self {
        Self::Control(value)
    }
}

impl From<ComputeProjectionReadError> for ComputeControlError {
    fn from(value: ComputeProjectionReadError) -> Self {
        match value {
            ComputeProjectionReadError::InvalidSelection => Self::InvalidSelection,
            ComputeProjectionReadError::RevisionConflict => Self::RevisionConflict,
            ComputeProjectionReadError::ActionRequired => Self::ActionRequired,
            ComputeProjectionReadError::Control(error) => Self::Control(error),
        }
    }
}
