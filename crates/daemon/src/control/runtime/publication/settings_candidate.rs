//! Reuse pure compilation between observation and staging of one settings operation.
use std::sync::Arc;

use super::*;

// This owns the immutable plan, not a workspace/epoch validity assertion. Callers
// still observe the actual grant and read the active/last-good base on every use.
// Restart or a different plan allocation intentionally discards the optimization.
pub(in crate::control::runtime) struct SettingsCandidate {
    operation_id: OperationId,
    workspace_id: WorkspaceId,
    accepted_digest: CanonicalDigest,
    plan: Arc<hiroute_domain::TransactionPlanV1>,
    intent: ExternalEffectIntentV1,
    base: PublicationRecordV1,
    grant: OwnedEffectV1,
    record: PublicationRecordV1,
}

impl SettingsCandidate {
    fn matches(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
        base: &PublicationRecordV1,
        grant: &OwnedEffectV1,
    ) -> bool {
        self.operation_id == operation.operation_id
            && self.workspace_id == operation.workspace_id
            && self.accepted_digest == operation.accepted_digest
            && Arc::ptr_eq(&self.plan, &operation.plan)
            && &self.intent == intent
            && &self.base == base
            && &self.grant == grant
    }
}

impl LocalControlAdapter {
    pub(super) fn settings_publication_record(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
        base: &PublicationRecordV1,
        grant: &OwnedEffectV1,
    ) -> PortResult<PublicationRecordV1> {
        let mut cache = self
            .settings_publication_candidate
            .lock()
            .map_err(|_| invalid("publication.settings-candidate.lock"))?;
        if let Some(candidate) = cache.as_ref()
            && candidate.matches(operation, intent, base, grant)
        {
            return Ok(candidate.record.clone());
        }
        #[cfg(test)]
        SETTINGS_RECORD_BUILDS.with(|count| count.set(count.get() + 1));
        let record =
            hiroute_domain::settings_model_publication_record(operation, intent, base, grant)
                .map_err(|_| invalid("publication.settings.decode"))?;
        *cache = Some(SettingsCandidate {
            operation_id: operation.operation_id.clone(),
            workspace_id: operation.workspace_id.clone(),
            accepted_digest: operation.accepted_digest.clone(),
            plan: operation.plan.clone(),
            intent: intent.clone(),
            base: base.clone(),
            grant: grant.clone(),
            record: record.clone(),
        });
        Ok(record)
    }
}
