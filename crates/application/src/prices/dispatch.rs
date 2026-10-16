use super::control::*;
use crate::{ApplicationService, failed, map_control_error, succeeded};
use hiroute_application_api::*;
use hiroute_domain::TransactionPlanV1;
use serde_json::Value;

pub(crate) fn dispatch(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    let Some(ports) = service.ports.as_ref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let Some(price_port) = ports.prices.as_ref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    if request.operation_id == "ApplyPriceOverrideChange" {
        return apply(service, request);
    }
    if request.protected_grant.is_some() {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    }
    let facts = match price_port.price_control_facts() {
        Ok(f) => f,
        Err(e) => return failed(map_control_error(e), request.request_id),
    };
    if request.operation_id == "PreviewPriceOverrideChange" {
        let payload = match serde_json::from_value(request.payload) {
            Ok(p) => p,
            Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
        };
        return match preview_source_price_change(&facts, &payload) {
            Ok(v) => succeeded(v, request.request_id),
            Err(e) => failed(e, request.request_id),
        };
    }
    let payload: GetEffectivePricesV2 = match serde_json::from_value(request.payload) {
        Ok(p) => p,
        Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    if payload.targets.is_empty() || payload.targets.len() > 256 {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut items = vec![];
    for target in payload.targets {
        if target.query_id.is_empty()
            || target.query_id.len() > 128
            || !ids.insert(target.query_id.clone())
        {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        }
        let (normalized, binding, source_revision) = match resolve_price_target(
            &facts,
            &target.target_locator,
            &target.currency,
            target.valuation_kind,
        ) {
            Ok(resolved) => resolved,
            Err(e) => return failed(e, request.request_id),
        };
        let quote = match facts.effective.freeze_price(
            &normalized,
            facts.evaluated_at,
            hiroute_domain::PriceBillingContextV1::StandardTokens,
        ) {
            Ok(q) => q,
            Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
        };
        let Some(source_origin) = facts.source_bindings.iter().find_map(|candidate| {
            (candidate.source_id == binding.source_id && candidate.binding_id == binding.binding_id)
                .then_some(candidate.source_origin)
        }) else {
            return failed(ErrorCode::Internal, request.request_id);
        };
        let edit_context = (target.valuation_kind
            == hiroute_domain::PriceValuationKindV1::ApiEquivalent
            || (source_origin != hiroute_domain::SourceOrigin::Cpa
                && binding.billing_class != hiroute_domain::BillingClass::Subscription))
            .then(|| PriceEditContextV1 {
                target_locator: PriceTargetLocatorV1::Binding {
                    binding_id: binding.binding_id.clone(),
                },
                expected_source_revision: source_revision,
                expected_binding_revision: binding.binding_revision,
                expected_override_revision: facts
                    .entries
                    .iter()
                    .find(|entry| entry.target == normalized)
                    .and_then(|entry| entry.source_override.as_ref())
                    .map_or(0, |value| value.revision),
            });
        items.push(EffectivePriceResultItemV2 {
            query_id: target.query_id,
            quote,
            edit_context,
        });
    }
    succeeded(
        EffectivePricesResultV2 {
            pending_activation: super::PriceSnapshot::build(
                facts.configuration_revision,
                facts.catalog_refs,
                facts.entries,
            )
            .map(|s| Some(s.generation_ref()) != facts.effective.generation_ref())
            .unwrap_or(true),
            generation_ref: facts.effective.generation_ref().cloned(),
            evaluated_at: facts.evaluated_at,
            items,
        },
        request.request_id,
    )
}
fn apply(service: &ApplicationService, request: LocalControlRequestV2) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let payload: ApplyPriceOverrideChangeV2 = match serde_json::from_value(request.payload) {
        Ok(p) => p,
        Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    let ports = service.ports.as_ref().expect("dispatch checked ports");
    match ports.control.operation_for_idempotency(
        &hiroute_domain::WorkspaceId::default(),
        hiroute_application_api::PrincipalKind::InteractiveUser,
        "ApplyPriceOverrideChange",
        &payload.idempotency_key,
    ) {
        Ok(Some(operation)) if operation.accepted_digest == payload.accept_digest => {
            return operation_result(operation, request.request_id);
        }
        Ok(Some(_)) => return failed(ErrorCode::IdempotencyKeyReused, request.request_id),
        Err(error) => return failed(map_control_error(error), request.request_id),
        Ok(None) => (),
    }
    let port = ports
        .prices
        .as_ref()
        .expect("dispatch checked prices")
        .clone();
    let check_request = match request_from_spec(&payload.spec) {
        Ok(r) => r,
        Err(e) => return failed(e, request.request_id),
    };
    let facts = match port.price_control_facts() {
        Ok(f) => f,
        Err(e) => return failed(map_control_error(e), request.request_id),
    };
    let reproduced = match preview_source_price_change(&facts, &check_request) {
        Ok(p) => p,
        Err(e) => return failed(e, request.request_id),
    };
    if reproduced.spec != payload.spec
        || reproduced.change_digest != payload.accept_digest
        || reproduced.expected_revisions != payload.expected_revisions
    {
        return failed(ErrorCode::ChangePreviewStale, request.request_id);
    }
    let plan = match TransactionPlanV1::from_source_price_planner(payload.spec.clone()) {
        Ok(p) => p,
        Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    let digest = payload.accept_digest.clone();
    let revisions = payload.expected_revisions.clone();
    let apply = ApplyRequestV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        spec: payload.spec,
        accept_digest: payload.accept_digest,
        expected_revisions: payload.expected_revisions,
        idempotency_key: payload.idempotency_key,
        apply_capability: None,
    };
    let prepared = match crate::PreparedTransactionV1::for_setup(apply, digest.clone(), plan) {
        Ok(p) => p.with_revalidation(move || {
            let facts = port
                .price_control_facts()
                .map_err(|_| crate::TransactionError::ChangePreviewStale)?;
            let current = preview_source_price_change(&facts, &check_request)
                .map_err(|_| crate::TransactionError::ChangePreviewStale)?;
            if current.change_digest != digest || current.expected_revisions != revisions {
                return Err(crate::TransactionError::ChangePreviewStale);
            }
            Ok(())
        }),
        Err(e) => return failed(e.error_code(), request.request_id),
    };
    let Some(mutation) = &ports.mutation else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    match mutation.apply_local_prepared_change(prepared) {
        Ok(operation) => operation_result(operation, request.request_id),
        Err(e) => failed(e.error_code(), request.request_id),
    }
}

fn operation_result(
    operation: hiroute_domain::OperationV1,
    request_id: String,
) -> MachineEnvelopeV2<Value> {
    let Some(change) = operation.plan.source_price_change() else {
        return failed(ErrorCode::Internal, request_id);
    };
    let generation = operation
        .step(hiroute_domain::OperationStepKind::Activate)
        .terminal_result
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());
    if operation.state == hiroute_domain::OperationState::Succeeded && generation.is_none() {
        return failed(ErrorCode::Internal, request_id);
    }
    let reference = OperationReferenceV1 {
        operation_id: operation.operation_id.to_string(),
        state: operation.state.as_str().to_owned(),
        sequence: operation.generation,
        cancellable: !operation.state.is_terminal(),
    };
    let mut envelope = MachineEnvelopeV2::accepted(
        serde_json::to_value(PriceApplyOperationV2 {
            operation_id: reference.operation_id.clone(),
            accepted_digest: operation.accepted_digest,
            state: reference.state.clone(),
            override_revision: change.after.revision,
            effective_price_generation: generation,
        })
        .expect("serializable price result"),
        Some(request_id),
    );
    envelope.operation = Some(reference);
    envelope
}
