//! Resolve an existing subscription's original credential owner, not a scan's first match.

use super::*;
use hiroute_application::control::ComputeManagementControlError as Error;
use hiroute_application_api::{
    ComputeSavedSubscriptionSourceV1, ComputeSubscriptionCandidatesV2,
    ComputeSubscriptionDiscoveryStateV2,
};

impl LocalControlAdapter {
    pub(in crate::control::runtime) fn saved_subscription_candidates(
        &self,
        query: ComputeSavedSubscriptionSourceV1,
    ) -> Result<ComputeSubscriptionCandidatesV2, Error> {
        let saved = {
            let stores = self.stores_lock().map_err(|_| Error::Unavailable)?;
            stores
                .control()
                .compute_management_snapshot(&hiroute_domain::WorkspaceId::default())
                .map_err(map_port)?
                .sources
                .into_iter()
                .find(|source| source.source_id == query.source_id)
                .ok_or(Error::NotFound)?
        };
        if saved.revision != query.expected_source_revision
            || !saved.provenance.is_connector_owned()
        {
            return Err(Error::Conflict);
        }
        // This only verifies the committed save; its consumed validation is never
        // returned as fresh approval. Check/Preview/Apply still run normally.
        let (record, stored) = self
            .committed_subscription_validation(&saved)?
            .ok_or(Error::Conflict)?;
        if !matches!(
            &saved.provenance,
            hiroute_domain::ComputeManagementProvenanceV2::ConnectorOwned {
                connector_id, account_ref
            } if connector_id == &stored.connector_id && account_ref == &stored.account_ref
        ) || stored.checked_candidate.candidate_ref != saved.last_candidate_ref
            || stored.checked_candidate.candidate_revision != saved.last_candidate_revision
        {
            return Err(Error::Conflict);
        }
        let owner = self
            .subscription_source_for_candidate(&saved.last_candidate_ref)?
            .ok_or(Error::Unavailable)?;
        if subscription_candidate_ref(&owner)? != saved.last_candidate_ref
            || &self.subscription_source_evidence(&record, &stored)? != owner.evidence_digest()
        {
            return Err(Error::Conflict);
        }
        let candidates = self.refresh_subscription_candidate(owner)?;
        Ok(ComputeSubscriptionCandidatesV2 {
            candidates,
            discovery_state: ComputeSubscriptionDiscoveryStateV2::Complete,
            reason_code: None,
        })
    }
}
