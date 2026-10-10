//! Explicit reference refresh preserves current routing authority and uses its normal writer.
use super::*;
use crate::control::RoutingFactsPort;
use hiroute_application_api::*;
use hiroute_domain::{PublicationRecordV1, TransactionPlanV1, WorkspaceId, checkpoint_publication};

fn preview(
    port: &dyn RoutingFactsPort,
    change: &PublicationCheckpointChangeV1,
) -> Result<(PublicationCheckpointPreviewV1, PublicationRecordV1), ErrorCode> {
    if change.schema != PUBLICATION_CHECKPOINT_CHANGE_SCHEMA_V1 {
        return Err(ErrorCode::InvalidArguments);
    }
    let (before, revisions) = port
        .publication_checkpoint_snapshot(&WorkspaceId::default())
        .map_err(map_control_error)?;
    let after = checkpoint_publication(&before).map_err(|_| ErrorCode::InvalidArguments)?;
    let digest = hiroute_domain::CanonicalDigest::of(&(
        PUBLICATION_CHECKPOINT_PREVIEW_SCHEMA_V1,
        change,
        &revisions,
        &before,
        &after,
    ))
    .map_err(|_| ErrorCode::InvalidArguments)?;
    Ok((
        PublicationCheckpointPreviewV1 {
            schema: PUBLICATION_CHECKPOINT_PREVIEW_SCHEMA_V1.into(),
            change_digest: digest,
            expected_revisions: revisions,
            before_digest: before.digest.clone(),
            publication_revision: after.publication_revision,
        },
        before,
    ))
}

pub(super) fn dispatch_preview(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let Some(port) = service.ports.as_ref().and_then(|p| p.routing.as_deref()) else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let payload: PublicationCheckpointPreviewRequestV1 =
        match serde_json::from_value(request.payload) {
            Ok(value) => value,
            Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
        };
    match preview(port, &payload.change) {
        Ok((result, _)) => succeeded(result, request.request_id),
        Err(error) => failed(error, request.request_id),
    }
}

pub(super) fn dispatch_apply(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let Some(ports) = service.ports.as_ref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let (Some(port), Some(mutation)) = (ports.routing.as_ref(), ports.mutation.as_ref()) else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let payload: PublicationCheckpointApplyRequestV1 = match serde_json::from_value(request.payload)
    {
        Ok(value) => value,
        Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    if payload.idempotency_key.is_empty() {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    }
    if let Some(replay) = replay_apply(
        ports.control.as_ref(),
        PrincipalKind::InteractiveUser,
        "ApplyAgentPlanChange",
        &payload.idempotency_key,
        &payload.accept_digest,
        &request.request_id,
    ) {
        return replay;
    }
    let prepared = (|| {
        let (reproduced, before) = preview(port.as_ref(), &payload.change)?;
        if reproduced.change_digest != payload.accept_digest
            || reproduced.expected_revisions != payload.expected_revisions
        {
            return Err(ErrorCode::ChangePreviewStale);
        }
        let spec = hiroute_domain::ChangeSpecV1 {
            schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
            command_id: "routing.apply".into(),
            resource_id: Some("publication/current".into()),
            desired_state: serde_json::to_value(&payload.change)
                .map_err(|_| ErrorCode::InvalidArguments)?,
        };
        let plan = TransactionPlanV1::from_publication_checkpoint(spec.clone(), before)
            .map_err(|_| ErrorCode::InvalidArguments)?;
        let apply = ApplyRequestV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            spec,
            accept_digest: payload.accept_digest.clone(),
            expected_revisions: payload.expected_revisions.clone(),
            idempotency_key: payload.idempotency_key,
            apply_capability: None,
        };
        let prepared =
            crate::PreparedTransactionV1::for_setup(apply, payload.accept_digest.clone(), plan)
                .map_err(|e| e.error_code())?;
        let port = port.clone();
        Ok(prepared.with_revalidation(move || {
            let (current, _) = preview(port.as_ref(), &payload.change)
                .map_err(|_| crate::TransactionError::ChangePreviewStale)?;
            if current.change_digest != payload.accept_digest
                || current.expected_revisions != payload.expected_revisions
            {
                return Err(crate::TransactionError::ChangePreviewStale);
            }
            Ok(())
        }))
    })();
    let prepared = match prepared {
        Ok(value) => value,
        Err(error) => return failed(error, request.request_id),
    };
    match mutation.apply_local_prepared_change(prepared) {
        Ok(operation) => super::accepted_apply(operation, request.request_id),
        Err(error) => failed(error.error_code(), request.request_id),
    }
}
