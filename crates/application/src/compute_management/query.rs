//! Safe management projection joined with exact runtime health.

use hiroute_application_api::{
    COMPUTE_MANAGEMENT_SNAPSHOT_SCHEMA_V2, ComputeCandidateProvenanceKindV2,
    ComputeConnectionAccessKindV1, ComputeConnectionIdentityV1, ComputeManagedKeyAvailabilityV2,
    ComputeManagedKeyModelStatusV2, ComputeManagedKeyViewV2, ComputeManagedModelViewV2,
    ComputeManagedSourceViewV2, ComputeManagementActionV2, ComputeManagementQueryV2,
    ComputeManagementRuntimeReadStateV2, ComputeManagementSnapshotV2,
    ComputeModelAvailabilityReasonV1, ComputeModelAvailabilityV1, ComputeModelMembershipV2,
    ComputeModelPresentationV1, ComputePriceContextV1,
};
use hiroute_domain::{
    BillingClass, ComputeManagementMembershipV2, ComputeManagementProvenanceV2,
    ComputeManagementRepositoryPort, ComputeManagementSourceV2, ComputeRuntimeHealthV1,
    ComputeRuntimeStateStoreV1, GatewayAuthenticationSemanticsV1, MaterializationState, PortError,
    RevisionSetV1, RuntimeStateIdentityV1, WorkspaceId,
};
use thiserror::Error;

/// Safe catalog/price facts captured before the management repository lock is taken. They carry
/// no credential reference or secret material and are joined only by stable source/binding ids.
#[derive(Clone, Debug, Default)]
pub struct ComputeManagementPresentationFactsV1 {
    pub evaluated_at_ms: i64,
    pub complete: bool,
    pub revisions: Option<RevisionSetV1>,
    pub sources: Vec<ComputeManagementSourcePresentationFactV1>,
    pub models: Vec<ComputeManagementModelPresentationFactV1>,
}

#[derive(Clone, Debug)]
pub struct ComputeManagementSourcePresentationFactV1 {
    pub source_id: String,
    pub identity: ComputeConnectionIdentityV1,
}

#[derive(Clone, Debug)]
pub struct ComputeManagementModelPresentationFactV1 {
    pub binding_id: String,
    pub billing_class: BillingClass,
    pub price_contexts: Vec<ComputePriceContextV1>,
    pub runtime_availability: ComputeManagementModelRuntimeAvailabilityFactV1,
}

/// Runtime authority used by the presentation join. Native connections continue to use the
/// durable binding/key runtime store, while connector-owned models must carry an explicit live
/// connector result and may never become available from a persisted `Ready` bit alone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComputeManagementModelRuntimeAvailabilityFactV1 {
    BindingAndKeys,
    Available,
    SubscriptionUpdating,
    AuthenticationRequired,
    ModelNotAllowed,
    RuntimeUnavailable,
}

/// Build the ordinary-client projection without exposing credential references, account
/// references, protected-input locators, or secret material.
pub fn query_compute_management<R, T>(
    repository: &R,
    runtime: &T,
    workspace: &WorkspaceId,
    query: &ComputeManagementQueryV2,
) -> Result<ComputeManagementSnapshotV2, ComputeManagementQueryErrorV2>
where
    R: ComputeManagementRepositoryPort,
    T: ComputeRuntimeStateStoreV1,
{
    query_compute_management_with_presentation(repository, runtime, workspace, query, None)
}

pub fn query_compute_management_with_presentation<R, T>(
    repository: &R,
    runtime: &T,
    workspace: &WorkspaceId,
    query: &ComputeManagementQueryV2,
    presentation: Option<&ComputeManagementPresentationFactsV1>,
) -> Result<ComputeManagementSnapshotV2, ComputeManagementQueryErrorV2>
where
    R: ComputeManagementRepositoryPort,
    T: ComputeRuntimeStateStoreV1,
{
    let snapshot = repository.compute_management_snapshot(workspace)?;
    let presentation_complete = presentation.is_none_or(|facts| {
        facts.complete && facts.revisions.as_ref() == Some(&snapshot.revisions)
    });
    let mut partial = !presentation_complete;
    let mut sources = snapshot
        .sources
        .iter()
        .filter(|source| {
            query
                .source_id
                .as_ref()
                .is_none_or(|source_id| source_id == &source.source_id)
        })
        .map(|source| {
            project_source(
                source,
                runtime,
                presentation,
                presentation_complete,
                &mut partial,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    sources.sort_by(|left, right| {
        left.display_name
            .to_lowercase()
            .cmp(&right.display_name.to_lowercase())
            .then_with(|| left.source_id.cmp(&right.source_id))
    });
    if query.source_id.is_some() && sources.is_empty() {
        return Err(ComputeManagementQueryErrorV2::NotFound);
    }
    Ok(ComputeManagementSnapshotV2 {
        schema: COMPUTE_MANAGEMENT_SNAPSHOT_SCHEMA_V2.into(),
        revisions: snapshot.revisions,
        runtime_state: if partial {
            ComputeManagementRuntimeReadStateV2::Partial
        } else {
            ComputeManagementRuntimeReadStateV2::Complete
        },
        sources,
    })
}

fn project_source<T: ComputeRuntimeStateStoreV1>(
    source: &ComputeManagementSourceV2,
    runtime: &T,
    presentation: Option<&ComputeManagementPresentationFactsV1>,
    presentation_complete: bool,
    partial: &mut bool,
) -> Result<ComputeManagedSourceViewV2, ComputeManagementQueryErrorV2> {
    source
        .validate()
        .map_err(|_| ComputeManagementQueryErrorV2::Corrupt)?;
    let source_identity_available = presentation.is_none_or(|facts| {
        presentation_complete
            && (matches!(
                source.provenance,
                ComputeManagementProvenanceV2::UserConfigured { .. }
            ) || facts
                .sources
                .iter()
                .any(|fact| fact.source_id == source.source_id))
    });
    if let Some(facts) = presentation {
        let missing_source_identity = !source_identity_available;
        let missing_model_fact = source.models.iter().any(|model| {
            !facts
                .models
                .iter()
                .any(|fact| fact.binding_id == model.binding_id)
        });
        if !presentation_complete || missing_source_identity || missing_model_fact {
            *partial = true;
        }
    }
    let mut model_binding_health = Vec::with_capacity(source.models.len());
    for model in &source.models {
        model_binding_health.push(read_health(
            runtime,
            RuntimeStateIdentityV1::binding(model.binding_id.clone())
                .map_err(|_| ComputeManagementQueryErrorV2::Corrupt)?,
            partial,
        ));
    }

    let keys = source
        .credentials
        .iter()
        .map(|credential| {
            let model_statuses = source
                .models
                .iter()
                .zip(model_binding_health.iter().copied())
                .map(|(model, binding)| {
                    let identity = RuntimeStateIdentityV1::credential(
                        model.binding_id.clone(),
                        credential.credential.credential_id(),
                        credential.key_id.clone(),
                        credential.credential.generation(),
                    )
                    .map_err(|_| ComputeManagementQueryErrorV2::Corrupt)?;
                    let credential_health = read_health(runtime, identity, partial);
                    let (availability, cooldown_until_ms) = key_availability(
                        source.state,
                        credential.enabled,
                        binding,
                        credential_health,
                    );
                    Ok(ComputeManagedKeyModelStatusV2 {
                        binding_id: model.binding_id.clone(),
                        availability,
                        cooldown_until_ms,
                    })
                })
                .collect::<Result<Vec<_>, ComputeManagementQueryErrorV2>>()?;
            Ok(ComputeManagedKeyViewV2 {
                key_id: credential.key_id.clone(),
                generation: credential.credential.generation(),
                fingerprint_hint: fingerprint_hint(credential.fingerprint.as_str()),
                ordinal: credential.ordinal,
                enabled: credential.enabled,
                model_statuses,
            })
        })
        .collect::<Result<Vec<_>, ComputeManagementQueryErrorV2>>()?;

    let models = source
        .models
        .iter()
        .enumerate()
        .map(|(index, model)| {
            let model_fact = if presentation_complete {
                presentation.and_then(|facts| {
                    facts
                        .models
                        .iter()
                        .find(|fact| fact.binding_id == model.binding_id)
                })
            } else {
                None
            };
            let (availability, reason_code) = model_availability(
                source,
                model_binding_health[index],
                &keys,
                index,
                PresentationAvailabilityFacts {
                    requested: presentation.is_some(),
                    complete: presentation_complete,
                    source_identity_available,
                    model_fact_available: model_fact.is_some(),
                    runtime_availability: model_fact.map(|fact| fact.runtime_availability),
                },
            );
            let mut price_contexts =
                model_fact.map_or_else(Vec::new, |fact| fact.price_contexts.clone());
            price_contexts.sort();
            price_contexts.dedup();
            ComputeManagedModelViewV2 {
                model_ref: model.model_ref.clone(),
                binding_id: model.binding_id.clone(),
                revision: model.revision,
                upstream_model_id: model.upstream_model_id.clone(),
                display_name: model.display_name.clone(),
                catalog_configuration_id: model.catalog_configuration_id.clone(),
                membership: match model.membership {
                    ComputeManagementMembershipV2::Catalog => ComputeModelMembershipV2::Catalog,
                    ComputeManagementMembershipV2::Observed => ComputeModelMembershipV2::Observed,
                    ComputeManagementMembershipV2::UserDeclared => {
                        ComputeModelMembershipV2::UserDeclared
                    }
                },
                capabilities: model.capabilities.clone(),
                native_reasoning: model.capabilities.native_reasoning.value.clone(),
                presentation: ComputeModelPresentationV1 {
                    billing_class: model_fact
                        .map_or(BillingClass::Unknown, |fact| fact.billing_class),
                    availability,
                    reason_code,
                    evaluated_at_ms: presentation.map_or(0, |facts| facts.evaluated_at_ms),
                    price_contexts,
                },
            }
        })
        .collect::<Vec<_>>();
    let ready_model_count = if presentation.is_none() {
        source
            .models
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                model_is_ready(source, model_binding_health[*index], &keys, *index)
            })
            .count()
    } else {
        models
            .iter()
            .filter(|model| {
                model.presentation.availability == ComputeModelAvailabilityV1::Available
            })
            .count()
    }
    .try_into()
    .map_err(|_| ComputeManagementQueryErrorV2::Corrupt)?;
    Ok(ComputeManagedSourceViewV2 {
        display_template_id: source
            .native_recheck
            .as_ref()
            .and_then(|descriptor| descriptor.display_template_id.clone()),
        inventory_path: source
            .native_recheck
            .as_ref()
            .and_then(|descriptor| descriptor.inventory_path.clone()),
        source_id: source.source_id.clone(),
        revision: source.revision,
        display_name: source.display_name.clone(),
        provenance: provenance_kind(&source.provenance),
        connection_identity: connection_identity(
            source,
            presentation.filter(|_| presentation_complete),
        ),
        target: hiroute_application_api::ComputeCandidateTargetV2 {
            scheme: source.target.scheme.clone(),
            authority: source.target.authority.clone(),
            port: source.target.port,
            request_path: source.target.request_path.clone(),
            upstream_protocol: source.target.upstream_protocol,
            protocol_profile_id: source.target.protocol_profile_id.clone(),
            protocol_profile_revision: source.target.protocol_profile_revision,
        },
        authentication: source.authentication.clone(),
        additional_native_endpoints: source.additional_native_endpoints.clone(),
        state: source.state,
        models,
        keys,
        ready_model_count,
        actions: source_actions(source),
    })
}

fn connection_identity(
    source: &ComputeManagementSourceV2,
    facts: Option<&ComputeManagementPresentationFactsV1>,
) -> ComputeConnectionIdentityV1 {
    if matches!(
        source.provenance,
        ComputeManagementProvenanceV2::UserConfigured { .. }
    ) {
        return ComputeConnectionIdentityV1 {
            access_kind: ComputeConnectionAccessKindV1::Api,
            connection_option_id: None,
            product_label: Some(source.display_name.clone()),
        };
    }
    if let Some(identity) = facts.and_then(|facts| {
        facts
            .sources
            .iter()
            .find(|fact| fact.source_id == source.source_id)
            .map(|fact| fact.identity.clone())
    }) {
        return identity;
    }
    match &source.provenance {
        ComputeManagementProvenanceV2::Registered {
            connection_option_id,
            ..
        } => ComputeConnectionIdentityV1 {
            access_kind: ComputeConnectionAccessKindV1::Unknown,
            connection_option_id: Some(connection_option_id.clone()),
            product_label: None,
        },
        ComputeManagementProvenanceV2::UserConfigured { .. } => unreachable!("handled above"),
        ComputeManagementProvenanceV2::ConnectorOwned { .. } => ComputeConnectionIdentityV1 {
            access_kind: ComputeConnectionAccessKindV1::Unknown,
            connection_option_id: None,
            product_label: None,
        },
    }
}

#[derive(Clone, Copy)]
struct PresentationAvailabilityFacts {
    requested: bool,
    complete: bool,
    source_identity_available: bool,
    model_fact_available: bool,
    runtime_availability: Option<ComputeManagementModelRuntimeAvailabilityFactV1>,
}

fn model_availability(
    source: &ComputeManagementSourceV2,
    binding: ReadHealth,
    keys: &[ComputeManagedKeyViewV2],
    model_index: usize,
    presentation: PresentationAvailabilityFacts,
) -> (
    ComputeModelAvailabilityV1,
    Option<ComputeModelAvailabilityReasonV1>,
) {
    if source.state == MaterializationState::Disabled {
        return (
            ComputeModelAvailabilityV1::Disabled,
            Some(ComputeModelAvailabilityReasonV1::SourceDisabled),
        );
    }
    if source.provenance.is_connector_owned() {
        match presentation.runtime_availability {
            Some(ComputeManagementModelRuntimeAvailabilityFactV1::SubscriptionUpdating) => {
                return (
                    ComputeModelAvailabilityV1::Unavailable,
                    Some(ComputeModelAvailabilityReasonV1::SubscriptionUpdating),
                );
            }
            Some(ComputeManagementModelRuntimeAvailabilityFactV1::AuthenticationRequired) => {
                return (
                    ComputeModelAvailabilityV1::NeedsCredentials,
                    Some(ComputeModelAvailabilityReasonV1::AuthenticationRequired),
                );
            }
            Some(ComputeManagementModelRuntimeAvailabilityFactV1::RuntimeUnavailable) => {
                return (
                    ComputeModelAvailabilityV1::Unavailable,
                    Some(ComputeModelAvailabilityReasonV1::RuntimeUnavailable),
                );
            }
            Some(
                ComputeManagementModelRuntimeAvailabilityFactV1::BindingAndKeys
                | ComputeManagementModelRuntimeAvailabilityFactV1::Available
                | ComputeManagementModelRuntimeAvailabilityFactV1::ModelNotAllowed,
            )
            | None => {}
        }
    }
    if !source.models[model_index].execution_eligible {
        return (
            ComputeModelAvailabilityV1::Unavailable,
            Some(ComputeModelAvailabilityReasonV1::ModelNotAllowed),
        );
    }
    if matches!(binding, ReadHealth::Disabled) {
        return (
            ComputeModelAvailabilityV1::Disabled,
            Some(ComputeModelAvailabilityReasonV1::BindingDisabled),
        );
    }
    if matches!(binding, ReadHealth::Unknown)
        || !presentation.source_identity_available
        || !presentation.complete
        || (presentation.requested && !presentation.model_fact_available)
    {
        return (
            ComputeModelAvailabilityV1::Unknown,
            Some(ComputeModelAvailabilityReasonV1::FactsUnavailable),
        );
    }
    match source.state {
        MaterializationState::NeedsCredential => {
            return (
                ComputeModelAvailabilityV1::NeedsCredentials,
                Some(ComputeModelAvailabilityReasonV1::CredentialsMissing),
            );
        }
        MaterializationState::NeedsAuthorization => {
            return (
                ComputeModelAvailabilityV1::NeedsCredentials,
                Some(ComputeModelAvailabilityReasonV1::AuthenticationRequired),
            );
        }
        MaterializationState::Disabled => unreachable!("handled above"),
        MaterializationState::Ready => {}
    }
    if matches!(binding, ReadHealth::Cooling(_)) {
        return (
            ComputeModelAvailabilityV1::Unavailable,
            Some(ComputeModelAvailabilityReasonV1::BindingNotReady),
        );
    }
    if source.provenance.is_connector_owned() {
        return match presentation.runtime_availability {
            Some(ComputeManagementModelRuntimeAvailabilityFactV1::Available) => {
                (ComputeModelAvailabilityV1::Available, None)
            }
            Some(ComputeManagementModelRuntimeAvailabilityFactV1::SubscriptionUpdating) => (
                ComputeModelAvailabilityV1::Unavailable,
                Some(ComputeModelAvailabilityReasonV1::SubscriptionUpdating),
            ),
            Some(ComputeManagementModelRuntimeAvailabilityFactV1::AuthenticationRequired) => (
                ComputeModelAvailabilityV1::NeedsCredentials,
                Some(ComputeModelAvailabilityReasonV1::AuthenticationRequired),
            ),
            Some(ComputeManagementModelRuntimeAvailabilityFactV1::ModelNotAllowed) => (
                ComputeModelAvailabilityV1::Unavailable,
                Some(ComputeModelAvailabilityReasonV1::ModelNotAllowed),
            ),
            Some(ComputeManagementModelRuntimeAvailabilityFactV1::RuntimeUnavailable) => (
                ComputeModelAvailabilityV1::Unavailable,
                Some(ComputeModelAvailabilityReasonV1::RuntimeUnavailable),
            ),
            Some(ComputeManagementModelRuntimeAvailabilityFactV1::BindingAndKeys) | None => (
                ComputeModelAvailabilityV1::Unknown,
                Some(ComputeModelAvailabilityReasonV1::FactsUnavailable),
            ),
        };
    }
    if matches!(
        source.authentication,
        GatewayAuthenticationSemanticsV1::None
    ) {
        return (ComputeModelAvailabilityV1::Available, None);
    }
    let participating = keys.iter().filter(|key| key.enabled).collect::<Vec<_>>();
    if participating.is_empty() {
        return (
            ComputeModelAvailabilityV1::NeedsCredentials,
            Some(ComputeModelAvailabilityReasonV1::CredentialsMissing),
        );
    }
    let statuses = participating
        .iter()
        .filter_map(|key| key.model_statuses.get(model_index))
        .map(|status| status.availability)
        .collect::<Vec<_>>();
    if statuses.contains(&ComputeManagedKeyAvailabilityV2::Available) {
        return (ComputeModelAvailabilityV1::Available, None);
    }
    if !statuses.is_empty()
        && statuses
            .iter()
            .all(|status| *status == ComputeManagedKeyAvailabilityV2::CoolingDown)
    {
        return (
            ComputeModelAvailabilityV1::CoolingDown,
            Some(ComputeModelAvailabilityReasonV1::AllCredentialsCooling),
        );
    }
    if statuses.contains(&ComputeManagedKeyAvailabilityV2::Unknown) {
        return (
            ComputeModelAvailabilityV1::Unknown,
            Some(ComputeModelAvailabilityReasonV1::FactsUnavailable),
        );
    }
    (
        ComputeModelAvailabilityV1::Unavailable,
        Some(ComputeModelAvailabilityReasonV1::RuntimeUnavailable),
    )
}

#[derive(Clone, Copy)]
enum ReadHealth {
    Ready,
    Cooling(i64),
    Disabled,
    Unknown,
}

fn read_health<T: ComputeRuntimeStateStoreV1>(
    runtime: &T,
    identity: RuntimeStateIdentityV1,
    partial: &mut bool,
) -> ReadHealth {
    match runtime.runtime_state(&identity) {
        Ok(None) => ReadHealth::Ready,
        Ok(Some(state)) => match state.health() {
            ComputeRuntimeHealthV1::Ready => ReadHealth::Ready,
            ComputeRuntimeHealthV1::CoolingDown { until_unix_millis } => {
                ReadHealth::Cooling(until_unix_millis)
            }
            ComputeRuntimeHealthV1::Disabled => ReadHealth::Disabled,
        },
        Err(_) => {
            *partial = true;
            ReadHealth::Unknown
        }
    }
}

fn key_availability(
    source_state: MaterializationState,
    enabled: bool,
    binding: ReadHealth,
    credential: ReadHealth,
) -> (ComputeManagedKeyAvailabilityV2, Option<i64>) {
    if source_state == MaterializationState::Disabled || !enabled {
        return (ComputeManagedKeyAvailabilityV2::Disabled, None);
    }
    if source_state != MaterializationState::Ready {
        return (ComputeManagedKeyAvailabilityV2::Unavailable, None);
    }
    match (binding, credential) {
        (ReadHealth::Unknown, _) | (_, ReadHealth::Unknown) => {
            (ComputeManagedKeyAvailabilityV2::Unknown, None)
        }
        (ReadHealth::Disabled, _) | (_, ReadHealth::Disabled) => {
            (ComputeManagedKeyAvailabilityV2::Unavailable, None)
        }
        (ReadHealth::Cooling(until), ReadHealth::Cooling(other)) => (
            ComputeManagedKeyAvailabilityV2::CoolingDown,
            Some(until.max(other)),
        ),
        (ReadHealth::Cooling(until), _) | (_, ReadHealth::Cooling(until)) => {
            (ComputeManagedKeyAvailabilityV2::CoolingDown, Some(until))
        }
        (ReadHealth::Ready, ReadHealth::Ready) => {
            (ComputeManagedKeyAvailabilityV2::Available, None)
        }
    }
}

fn model_is_ready(
    source: &ComputeManagementSourceV2,
    binding: ReadHealth,
    keys: &[ComputeManagedKeyViewV2],
    model_index: usize,
) -> bool {
    if source.state != MaterializationState::Ready
        || !source.models[model_index].execution_eligible
        || !matches!(binding, ReadHealth::Ready)
    {
        return false;
    }
    if source.provenance.is_connector_owned()
        || matches!(
            source.authentication,
            GatewayAuthenticationSemanticsV1::None
        )
    {
        return true;
    }
    keys.iter().any(|key| {
        key.enabled
            && key.model_statuses.get(model_index).is_some_and(|status| {
                status.availability == ComputeManagedKeyAvailabilityV2::Available
            })
    })
}

fn provenance_kind(value: &ComputeManagementProvenanceV2) -> ComputeCandidateProvenanceKindV2 {
    match value {
        ComputeManagementProvenanceV2::Registered { .. } => {
            ComputeCandidateProvenanceKindV2::Registered
        }
        ComputeManagementProvenanceV2::UserConfigured { .. } => {
            ComputeCandidateProvenanceKindV2::UserConfigured
        }
        ComputeManagementProvenanceV2::ConnectorOwned { .. } => {
            ComputeCandidateProvenanceKindV2::ConnectorOwned
        }
    }
}

pub(super) fn source_actions(source: &ComputeManagementSourceV2) -> Vec<ComputeManagementActionV2> {
    let mut actions = vec![ComputeManagementActionV2::Edit];
    if source.provenance.is_native() {
        if !matches!(
            source.authentication,
            GatewayAuthenticationSemanticsV1::None
        ) {
            actions.push(ComputeManagementActionV2::AddKey);
        }
        if source.native_recheck.is_some() {
            actions.push(ComputeManagementActionV2::Recheck);
        }
    } else if source.provenance.is_connector_owned() {
        actions.push(ComputeManagementActionV2::Reauthorize);
    }
    actions.push(if source.state == MaterializationState::Disabled {
        ComputeManagementActionV2::Enable
    } else {
        ComputeManagementActionV2::Disable
    });
    actions
}

fn fingerprint_hint(value: &str) -> String {
    let suffix = value.strip_prefix("sha256:").unwrap_or(value);
    format!("sha256:…{}", &suffix[suffix.len().saturating_sub(8)..])
}

#[derive(Debug, Error)]
pub enum ComputeManagementQueryErrorV2 {
    #[error("saved compute source was not found")]
    NotFound,
    #[error("saved compute management state is corrupt")]
    Corrupt,
    #[error("compute management repository failed: {0}")]
    Port(#[from] PortError),
}
