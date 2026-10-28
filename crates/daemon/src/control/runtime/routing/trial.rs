//! Trial projection from the routing owner's already captured catalog and CPA batch.
use super::*;
use crate::control::runtime::subscriptions::maintenance::SubscriptionMaintenancePresentation as Maintenance;
use hiroute_application::compute_management::{
    ComputeManagementModelRuntimeAvailabilityFactV1 as Runtime, project_trial_source,
};
use hiroute_application_api::{
    ComputeConnectionAccessKindV1 as Access, TrialManagementSnapshotV1,
    TrialRetainedApiConnectionV1,
};
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

pub(super) struct ManagedOwnership<'a> {
    sources: std::collections::HashSet<&'a str>,
    bindings: std::collections::HashSet<&'a str>,
}

impl<'a> ManagedOwnership<'a> {
    pub(super) fn new(management: &'a ComputeManagementStoredSnapshotV2) -> Self {
        Self {
            sources: management
                .sources
                .iter()
                .map(|source| source.source_id.as_str())
                .collect(),
            bindings: management
                .sources
                .iter()
                .flat_map(|source| source.models.iter().map(|model| model.binding_id.as_str()))
                .collect(),
        }
    }

    pub(super) fn owns(&self, source: &str, binding: &str) -> bool {
        self.sources.contains(source) || self.bindings.contains(binding)
    }
}

/// This only describes the exact legacy API fact admitted by the routing owner.
/// An existing V2 owner always remains responsible for its own admission.
pub(super) fn retained_api(
    source: &hiroute_domain::ComputeSourceV1,
    candidate: &CandidateCompilationFactV1,
    option: &hiroute_domain::ResolvedConnectionOptionV1,
    ownership: &ManagedOwnership<'_>,
) -> Option<TrialRetainedApiConnectionV1> {
    let binding = &candidate.binding;
    if !matches!(
        source.origin,
        SourceOrigin::NativeApi | SourceOrigin::ReleaseFree
    ) || !matches!(
        option.option.origin,
        ConnectionOrigin::NativeApi | ConnectionOrigin::FreeCatalog
    ) || option.connector.runtime_kind != hiroute_domain::ConnectorRuntimeKind::BuiltinNative
        || candidate.connector_runtime != hiroute_domain::ConnectorRuntimeKind::BuiltinNative
        || candidate.authority
            != hiroute_application::compiler::CandidateFactAuthorityV1::RegisteredCatalog
        || ownership.owns(&source.source_id, &binding.binding_id)
        || binding.source_id != source.source_id
        || binding.source_revision != source.revision
        || candidate.model.model_configuration_id != binding.model_configuration_id
        || !candidate.is_routable()
    {
        return None;
    }
    Some(TrialRetainedApiConnectionV1 {
        source_id: source.source_id.clone(),
        source_revision: source.revision,
        binding_id: binding.binding_id.clone(),
        binding_revision: binding.revision,
        model_configuration_id: binding.model_configuration_id.clone(),
        display_name: option.option.display_name.clone(),
    })
}

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
    retained_api_connections: &[TrialRetainedApiConnectionV1],
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
    let mut retained_api_connections = retained_api_connections.to_vec();
    retained_api_connections.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.source_id.cmp(&right.source_id))
            .then_with(|| left.binding_id.cmp(&right.binding_id))
    });
    Ok(TrialManagementSnapshotV1 {
        revisions: management.revisions.clone(),
        sources,
        retained_api_connections,
    })
}

pub(super) fn capture_projected<T: ComputeRuntimeStateStoreV1>(
    adapter: &LocalControlAdapter,
    catalog: &TrustedReleaseCatalog,
    management: &ComputeManagementStoredSnapshotV2,
    batch: Option<&hiroute_cpa_bridge::CpaRoutingBatch<'_>>,
    trusted: &[bool],
    runtime: &T,
    facts: (
        &[CandidateCompilationFactV1],
        &[TrialRetainedApiConnectionV1],
    ),
) -> Result<(CaptureContext, TrialManagementSnapshotV1), ControlReadError> {
    let context = capture(adapter, catalog, management, batch, trusted);
    let projected = project(management, runtime, facts.0, &context, facts.1)?;
    Ok((context, projected))
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
