//! Saved subscription mode is derived from the committed selection, even while offline.
use super::{ComputeManagementPresentationFactsV1, ComputeManagementQueryErrorV2};
use hiroute_application_api::{
    COMPUTE_MANAGEMENT_SNAPSHOT_SCHEMA_V3, ComputeManagementQueryV2, ComputeManagementSnapshotV3,
    ComputeSubscriptionModeV1, ComputeSubscriptionModeViewV1,
};
use hiroute_domain::{
    ComputeManagementProvenanceV2, ComputeManagementRepositoryPort, ComputeRuntimeStateStoreV1,
    WorkspaceId,
};

pub fn query_compute_management_v3<
    R: ComputeManagementRepositoryPort,
    T: ComputeRuntimeStateStoreV1,
>(
    repository: &R,
    runtime: &T,
    workspace: &WorkspaceId,
    query: &ComputeManagementQueryV2,
    presentation: Option<&ComputeManagementPresentationFactsV1>,
) -> Result<ComputeManagementSnapshotV3, ComputeManagementQueryErrorV2> {
    let snapshot = repository.compute_management_snapshot(workspace)?;
    let mut modes = snapshot
        .sources
        .iter()
        .filter(|source| {
            query
                .source_id
                .as_ref()
                .is_none_or(|id| id == &source.source_id)
        })
        .filter_map(|source| {
            let ComputeManagementProvenanceV2::ConnectorOwned { connector_id, .. } =
                &source.provenance
            else {
                return None;
            };
            let provider = match connector_id.as_str() {
                "connector.cpa.codex" => "codex",
                "connector.cpa.claude" => "claude",
                _ => return None,
            };
            let suffix = source
                .last_candidate_ref
                .strip_prefix(&format!("candidate/cpa/{provider}/"))?;
            let mode = if suffix
                .strip_prefix("managed/")
                .is_some_and(|account| !account.is_empty() && !account.contains('/'))
            {
                ComputeSubscriptionModeV1::CpaManaged
            } else if !suffix.is_empty() && !suffix.contains('/') {
                ComputeSubscriptionModeV1::NativeBorrowed
            } else {
                return None;
            };
            Some(ComputeSubscriptionModeViewV1 {
                source_id: source.source_id.clone(),
                source_revision: source.revision,
                mode,
            })
        })
        .collect::<Vec<_>>();
    modes.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    let view = super::query::project_snapshot(snapshot, runtime, query, presentation)?;
    Ok(ComputeManagementSnapshotV3 {
        schema: COMPUTE_MANAGEMENT_SNAPSHOT_SCHEMA_V3.into(),
        revisions: view.revisions,
        runtime_state: view.runtime_state,
        sources: view.sources,
        subscription_modes: modes,
    })
}
