//! Prepare V3 only against the exact context snapshot presented by the server preview.
use super::*;
use hiroute_domain::{
    DECISION_SERVICE_OPERATION_SCHEMA_V3, DecisionConnectionV2, DecisionServiceOperationInputV3,
    DecisionServiceOperationV3, DecisionServiceRecord, DecisionSourceMutationV1,
    register_decision_consumer_record, remove_decision_consumers,
};

pub(super) fn plan<O: ConnectionOptionAuthorizationPort>(
    options: &O,
    input: DecisionServiceOperationInputV3,
) -> Result<RegisteredEffectPlan, ChangePreparationError> {
    if !input.validate() {
        return Err(ChangePreparationError::TypedPlannerUnavailable);
    }
    let change = &input.change;
    if let Some(service) = &change.service
        && let (
            DecisionConnectionV2::Native {
                endpoint,
                wire_protocol,
                model,
                ..
            },
            Some(context),
        ) = (&service.connection, &service.context)
    {
        let resolved =
            options.resolve_decision_context(endpoint, *wire_protocol, model, &context.policy)?;
        if resolved != context.resolved {
            return Err(ChangePreparationError::DecisionSourceRevisionConflict);
        }
    }
    let prior = if change.expected_revision == 0 {
        None
    } else {
        Some(
            options
                .decision_service_record(&change.id, change.expected_revision)?
                .ok_or(ChangePreparationError::SourceIdentityMismatch)?,
        )
    };
    let mut secrets = Vec::new();
    if let Some(service) = &change.service {
        if let Some(slot) = &input.input_slot {
            let header = service
                .connection
                .transport()
                .2
                .ok_or(ChangePreparationError::UnexpectedSecretInput)?;
            secrets.extend(
                plan_classifier_header_secret(ClassifierHeaderSecretInputV1 {
                    secret_id: header.value_secret_ref.clone(),
                    input_slot: slot.clone(),
                    expected_generation: 0,
                })?
                .secrets,
            );
        } else if let Some(header) = service.connection.transport().2
            && !prior
                .as_ref()
                .is_some_and(|old| decision_header_is_reusable(old, &service.connection, header))
        {
            return Err(ChangePreparationError::SecretInputRequired);
        }
    } else if input.input_slot.is_some() {
        return Err(ChangePreparationError::UnexpectedSecretInput);
    }
    let mut sources = Vec::new();
    for expected in &change.expected_source_revisions {
        let source = options
            .decision_source(&expected.source_id)?
            .ok_or(ChangePreparationError::SourceIdentityMismatch)?;
        if source.revision != expected.revision {
            return Err(ChangePreparationError::DecisionSourceRevisionConflict);
        }
        let transition = match &change.service {
            Some(service) => register_decision_consumer_record(
                &source,
                &DecisionServiceRecord::V3(service.clone()),
            )
            .map(Some),
            None => remove_decision_consumers(&source, &change.id),
        }
        .map_err(|_| ChangePreparationError::SourceIdentityMismatch)?
        .ok_or(ChangePreparationError::SourceIdentityMismatch)?;
        secrets.extend(transition.secrets);
        sources.push(DecisionSourceMutationV1 {
            before: transition.before,
            after: transition.after,
        });
    }
    let operation = DecisionServiceOperationV3 {
        schema: DECISION_SERVICE_OPERATION_SCHEMA_V3.into(),
        input,
        sources,
    };
    Ok(RegisteredEffectPlan {
        control: json!({"decision_service_operation_v3": operation}),
        pending_compute_source: None,
        credential_pool: None,
        pending_pool: None,
        secret_sources: Default::default(),
        secrets,
        runtime: Vec::new(),
        external: Vec::new(),
    })
}
