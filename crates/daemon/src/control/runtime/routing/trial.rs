//! Trial projection from the routing owner's already captured catalog and CPA batch.
use super::*;
use crate::control::runtime::subscriptions::maintenance::SubscriptionMaintenancePresentation as Maintenance;
use hiroute_application::compute_management::{
    ComputeManagementModelRuntimeAvailabilityFactV1 as Runtime, project_trial_source,
};
use hiroute_application_api::{ComputeConnectionAccessKindV1 as Access, TrialManagementSnapshotV1};
use hiroute_domain::{
    ComputeManagementProvenanceV2, ComputeManagementStoredSnapshotV2, ComputeRuntimeStateStoreV1,
    ConnectionOrigin,
};
use hiroute_integrations::TrustedReleaseCatalog;

struct SourceContext {
    access: Access,
    runtime: Option<Runtime>,
    maintenance: Option<Maintenance>,
}

pub(super) struct CaptureContext(Vec<SourceContext>);

/// Retained approval and successful-save evidence are read by exact source identity.
/// This is separate from connector discovery and is checked again after the CPA fence.
pub(super) fn trusted_sources(
    adapter: &LocalControlAdapter,
    catalog: &TrustedReleaseCatalog,
    management: &ComputeManagementStoredSnapshotV2,
) -> Vec<bool> {
    management
        .sources
        .iter()
        .map(|source| {
            !source.provenance.is_connector_owned()
                || adapter.trial_subscription_identity_is_trusted(catalog, source)
        })
        .collect()
}

pub(super) fn capture(
    adapter: &LocalControlAdapter,
    catalog: &TrustedReleaseCatalog,
    management: &ComputeManagementStoredSnapshotV2,
    batch: Option<&hiroute_cpa_bridge::CpaRoutingBatch<'_>>,
    trusted: &[bool],
) -> CaptureContext {
    CaptureContext(management.sources.iter().zip(trusted).map(|(source, trusted)| {
        let maintenance = adapter.subscription_maintenance_presentation(&source.source_id);
        let (access, runtime) = match &source.provenance {
            ComputeManagementProvenanceV2::UserConfigured { .. } => (Access::Api, None),
            ComputeManagementProvenanceV2::Registered { connection_option_id, .. } => {
                let access = catalog.resolve_connection_option(connection_option_id).ok()
                    .filter(|resolved| super::super::model_connections::registered_source_matches_current_option(source, resolved))
                    .map(|resolved| match resolved.option.origin {
                        ConnectionOrigin::AgentSubscription => Access::Subscription,
                        ConnectionOrigin::NativeApi | ConnectionOrigin::FreeCatalog => Access::Api,
                    }).unwrap_or(Access::Unknown);
                (access, None)
            }
            ComputeManagementProvenanceV2::ConnectorOwned { connector_id, account_ref } => {
                let kind = hiroute_cpa_bridge::CpaAccountKind::from_connector(connector_id);
                let access = kind.and_then(|kind| catalog.resolve_connection_option(kind.connection_option_id()).ok())
                    .filter(|resolved| *trusted && resolved.option.origin == ConnectionOrigin::AgentSubscription)
                    .map_or(Access::Unknown, |_| Access::Subscription);
                let runtime = match maintenance {
                    Some(Maintenance::Updating) => Runtime::SubscriptionUpdating,
                    Some(Maintenance::AuthenticationRequired) => Runtime::AuthenticationRequired,
                    Some(Maintenance::RuntimeUnavailable) => Runtime::RuntimeUnavailable,
                    None => match batch {
                        None => Runtime::RuntimeUnavailable,
                        Some(batch) if super::unique_live_cpa_source(batch.sources(), connector_id, account_ref).is_some() => Runtime::Available,
                        Some(_) => Runtime::AuthenticationRequired,
                    },
                };
                (access, Some(runtime))
            }
        };
        SourceContext { access, runtime, maintenance }
    }).collect())
}

pub(super) fn project<T: ComputeRuntimeStateStoreV1>(
    management: &ComputeManagementStoredSnapshotV2,
    runtime: &T,
    candidates: &[CandidateCompilationFactV1],
    context: &CaptureContext,
) -> Result<TrialManagementSnapshotV1, ControlReadError> {
    let by_binding = candidates
        .iter()
        .map(|candidate| (candidate.binding.binding_id.as_str(), candidate))
        .collect::<std::collections::HashMap<_, _>>();
    let mut sources = management
        .sources
        .iter()
        .zip(&context.0)
        .map(|(source, context)| {
            project_trial_source(
                source,
                runtime,
                &by_binding,
                context.access,
                context.runtime,
            )
            .map_err(|_| ControlReadError::Corrupt)
        })
        .collect::<Result<Vec<_>, _>>()?;
    sources.sort_by(|a, b| {
        a.display_name
            .to_lowercase()
            .cmp(&b.display_name.to_lowercase())
            .then_with(|| a.source_id.cmp(&b.source_id))
    });
    Ok(TrialManagementSnapshotV1 {
        revisions: management.revisions.clone(),
        sources,
    })
}

pub(super) fn maintenance_unchanged(
    adapter: &LocalControlAdapter,
    management: &ComputeManagementStoredSnapshotV2,
    context: &CaptureContext,
) -> bool {
    management
        .sources
        .iter()
        .zip(&context.0)
        .all(|(source, context)| {
            adapter.subscription_maintenance_presentation(&source.source_id) == context.maintenance
        })
}
