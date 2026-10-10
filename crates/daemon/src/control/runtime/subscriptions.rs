//! Approved native subscription discovery and provider-isolated CPA materialization.
//!
//! Scan is metadata-only. The OAuth document is first opened by the external-effect step of the
//! durably admitted approval Operation, and no SQLite transaction is held across CPA work.

use hiroute_application::compute_management::{
    ComputeApprovedSubscriptionCheckV2, ComputeCandidateFactsV2, ComputeCandidatePort,
    ComputeCandidateProvenanceV2, ComputeCredentialBindingV2,
    ComputeSubscriptionMaterializationPort, ComputeSubscriptionResourceReceiptV2,
    ProtectedInputSourceDescriptorV1,
};
use hiroute_application::subscriptions::{
    ComputeSubscriptionPreparedPreviewV2, verified_subscription_candidate,
};
use hiroute_application_api::{
    ComputeCandidateProducerV2, ComputeCandidateRefV2, ComputeCandidateTargetV2,
    ComputeCandidateViewV2, ComputeCheckCorrelationV2, ComputeSubscriptionCheckResultV2,
    ComputeSubscriptionCheckStatusV2, ComputeValidationRefV2, OperationReferenceV1,
};
use hiroute_cpa_bridge::{
    CpaAccountKind, CpaLifecycleError, CpaSourceManagementState, CpaSubscriptionEffectContext,
    CpaSubscriptionMaterializer,
};
use hiroute_domain::{
    CanonicalDigest, CompensationOutcome, ComputeManagementRepositoryPort, ControlRepositoryPort,
    EffectReconciliation, ExternalEffectIntentV1, GatewayAuthenticationSemanticsV1, OperationId,
    OperationState, OwnedEffectKind, OwnedEffectV1, PortErrorCode, PortResult,
};
use hiroute_local_storage::ComputeSubscriptionValidationStateV1;
use serde::{Deserialize, Serialize};

use self::error::{
    conflict, control_error_to_port, invalid, map_cpa, map_port, map_preparation, not_found,
    unavailable,
};
use self::record::{StoredSubscriptionValidationV1, decode_stored};
use super::{LocalControlAdapter, SubscriptionContextV1};

mod error;
pub(super) mod interaction;
mod lifecycle;
pub(super) mod login;
pub(super) mod maintenance;
pub(in crate::control::runtime) use maintenance::SubscriptionMaintenance;
mod presentation;
mod record;
pub(in crate::control::runtime) mod source;
use source::{SubscriptionSource, pending_candidate, subscription_candidate_ref};
#[cfg(test)]
mod login_admission_tests;
#[cfg(test)]
mod tests;

pub(super) const CONNECTOR_ID: &str = "connector.cpa.codex";
const MARKER_SCHEMA: &str = "hiroute.compute-subscription-effect-marker/v1";

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SubscriptionEffectMarkerV1 {
    schema: String,
    operation_id: String,
    candidate: ComputeCandidateRefV2,
}

impl LocalControlAdapter {
    /// Rebuild the process-local CPA policy from the durable compute-management truth before
    /// publication recovery. A saved Ready source is the user's prior authorization to manage
    /// this runtime, so restart re-establishes it without another approval Operation. Runtime
    /// startup remains best-effort: a missing or unhealthy CPA blocks only connector-owned
    /// candidates and must not prevent Local Control or unrelated sources from starting.
    pub(super) fn reconcile_cpa_runtime_from_management(&self) -> Result<(), String> {
        if self.cpa_runtime.is_none() {
            return Ok(());
        }
        let sources = self
            .stores_lock()
            .map_err(|error| error.to_string())?
            .control()
            .compute_management_snapshot(&hiroute_domain::WorkspaceId::default())
            .map_err(|error| error.to_string())?
            .sources;
        for source in sources {
            // Optional runtime failure cannot take down Control. The shared projection
            // retains the saved choice fail-closed, and maintenance can retry that revision.
            let _ = self.project_saved_subscription_source(&source);
        }
        Ok(())
    }

    pub(super) fn validate_subscription_save_change(
        &self,
        change: &hiroute_application_api::ComputeManagementChangeV2,
    ) -> Result<(), hiroute_application::control::ComputeManagementControlError> {
        let hiroute_application_api::ComputeManagementSubjectV2::Candidate { candidate } =
            &change.subject
        else {
            return Ok(());
        };
        let Some(validation) = change.validation.as_ref() else {
            return Ok(());
        };
        let Some(_kind) = CpaAccountKind::from_candidate(&candidate.candidate_ref) else {
            return Ok(());
        };
        let approval_id = OperationId::parse(&validation.approval_operation.operation_id)
            .map_err(|_| hiroute_application::control::ComputeManagementControlError::Invalid)?;
        let record = {
            let stores = self.stores_lock().map_err(|_| {
                hiroute_application::control::ComputeManagementControlError::Unavailable
            })?;
            stores
                .control()
                .compute_subscription_validation(&approval_id)
                .map_err(map_port)?
                .ok_or(hiroute_application::control::ComputeManagementControlError::Conflict)?
        };
        let stored = decode_stored(&record.record_json).map_err(map_port)?;
        let source_evidence = self.subscription_source_evidence(&record, &stored)?;
        if record.state != ComputeSubscriptionValidationStateV1::Verified
            || record.save_operation_id.is_some()
            || record.candidate_ref != candidate.candidate_ref
            || record.candidate_revision != candidate.candidate_revision
            || stored.validation != *validation
            || stored.checked_candidate != *candidate
        {
            return Err(hiroute_application::control::ComputeManagementControlError::Conflict);
        }
        let source = self
            .subscription_source_for_candidate(&candidate.candidate_ref)?
            .ok_or(hiroute_application::control::ComputeManagementControlError::NotFound)?;
        if subscription_candidate_ref(&source)? != candidate.candidate_ref
            || source_evidence != *source.evidence_digest()
        {
            return Err(hiroute_application::control::ComputeManagementControlError::Conflict);
        }
        let snapshot = self
            .stores_lock()
            .map_err(|_| hiroute_application::control::ComputeManagementControlError::Unavailable)?
            .control()
            .compute_management_snapshot(&hiroute_domain::WorkspaceId::default())
            .map_err(map_port)?;
        let mut current = snapshot.sources.iter().filter(|saved| {
            matches!(
                &saved.provenance,
                hiroute_domain::ComputeManagementProvenanceV2::ConnectorOwned { connector_id, .. }
                    if connector_id == source.kind().connector_id()
            )
        });
        let current_source = current.next();
        if current.next().is_some()
            || !self.subscription_check_saved_source_matches(&record, &stored, current_source)?
        {
            return Err(hiroute_application::control::ComputeManagementControlError::Conflict);
        }
        if let Some(expected) = source.expected_account_ref() {
            let runtime = self
                .cpa_runtime
                .as_ref()
                .and_then(|runtimes| runtimes.runtime_for_candidate(&candidate.candidate_ref))
                .ok_or(hiroute_application::control::ComputeManagementControlError::Unavailable)?;
            let observed = runtime
                .inspect_subscription()
                .map_err(map_cpa)
                .map_err(map_port)?;
            if observed.kind() != source.kind() || observed.account_ref() != expected {
                return Err(hiroute_application::control::ComputeManagementControlError::Conflict);
            }
        }
        Ok(())
    }

    pub(super) fn refresh_subscription_candidates(
        &self,
    ) -> Result<
        Vec<ComputeCandidateViewV2>,
        hiroute_application::control::ComputeManagementControlError,
    > {
        let mut views = Vec::new();
        for kind in [CpaAccountKind::Codex, CpaAccountKind::Claude] {
            for source in self.subscription_discovery_sources(kind)? {
                views.extend(self.refresh_subscription_candidate(source)?);
            }
        }
        Ok(views)
    }

    fn refresh_subscription_candidate(
        &self,
        source: SubscriptionSource,
    ) -> Result<
        Vec<ComputeCandidateViewV2>,
        hiroute_application::control::ComputeManagementControlError,
    > {
        let candidate_ref = subscription_candidate_ref(&source)?;
        let existing = self
            .subscription_sources
            .lock()
            .map_err(|_| hiroute_application::control::ComputeManagementControlError::Unavailable)?
            .get(&candidate_ref)
            .cloned();
        let (existing_source, latest_validation) = {
            let stores = self.stores_lock().map_err(|_| {
                hiroute_application::control::ComputeManagementControlError::Unavailable
            })?;
            let snapshot = stores
                .control()
                .compute_management_snapshot(&hiroute_domain::WorkspaceId::default())
                .map_err(map_port)?;
            let mut matching = snapshot
                .sources
                .into_iter()
                .filter(|saved| matches!(
                    &saved.provenance,
                    hiroute_domain::ComputeManagementProvenanceV2::ConnectorOwned { connector_id, .. }
                        if connector_id == source.kind().connector_id()
                ));
            let saved = matching.next();
            if matching.next().is_some() {
                return Err(hiroute_application::control::ComputeManagementControlError::Conflict);
            }
            let validation = stores
                .control()
                .latest_compute_subscription_validation(&candidate_ref)
                .map_err(map_port)?;
            (saved, validation)
        };
        let existing_source_id = existing_source
            .as_ref()
            .map(|saved| saved.source_id.clone());
        // Never advance the candidate beneath a check whose effects are still staged.
        if latest_validation
            .as_ref()
            .is_some_and(|record| record.state == ComputeSubscriptionValidationStateV1::Staged)
        {
            return Err(hiroute_application::control::ComputeManagementControlError::Unavailable);
        }
        // Metadata-only pending candidates can remain stable: Preview binds the current
        // saved revision. Checked candidates must instead pass the durable receipt checks
        // below; source identity can stay unchanged after their validation was consumed.
        if let Some(existing) = &existing
            && existing.source.evidence_digest() == source.evidence_digest()
            && latest_validation
                .as_ref()
                .is_none_or(|record| record.candidate_revision < existing.candidate_revision)
        {
            let candidate = ComputeCandidateRefV2 {
                candidate_ref: candidate_ref.clone(),
                candidate_revision: existing.candidate_revision,
            };
            if let Ok(facts) = self
                .model_connections
                .candidate_port()
                .resolve_compute_candidate(&candidate)
                && matches!(
                    facts.credential_binding,
                    ComputeCredentialBindingV2::CpaPendingApproval { .. }
                )
                && facts.existing_source_id == existing_source_id
                && facts.trusted_lineage_digest.as_ref()
                    == existing_source
                        .as_ref()
                        .map(|source| &source.lineage_digest)
                && let Ok(view) = self
                    .model_connections
                    .candidate_port()
                    .get_compute_candidate(&candidate)
            {
                return Ok(vec![view]);
            }
        }
        if let Some(record) = latest_validation.as_ref()
            && record.state == ComputeSubscriptionValidationStateV1::Verified
            && record.save_operation_id.is_none()
        {
            let stored = decode_stored(&record.record_json).map_err(map_port)?;
            if stored.checked_candidate.candidate_ref != record.candidate_ref
                || stored.checked_candidate.candidate_revision != record.candidate_revision
            {
                return Err(hiroute_application::control::ComputeManagementControlError::Corrupt);
            }
            if self.subscription_check_saved_source_matches(
                record,
                &stored,
                existing_source.as_ref(),
            )? && self.subscription_source_evidence(record, &stored)?
                == *source.evidence_digest()
            {
                let checked = stored.checked_facts().map_err(map_port)?;
                let view = self
                    .model_connections
                    .candidate_port()
                    .register_compute_candidate(checked)
                    .map_err(map_port)?;
                self.subscription_sources
                    .lock()
                    .map_err(|_| {
                        hiroute_application::control::ComputeManagementControlError::Unavailable
                    })?
                    .insert(
                        candidate_ref,
                        SubscriptionContextV1 {
                            source,
                            evidence: None,
                            candidate_revision: record.candidate_revision,
                        },
                    );
                return Ok(vec![view]);
            }
        }
        let previous_revision = existing
            .as_ref()
            .map(|context| context.candidate_revision)
            .into_iter()
            .chain(
                latest_validation
                    .as_ref()
                    .map(|record| record.candidate_revision),
            )
            .max();
        let candidate_revision = previous_revision.map_or(Ok(1), |revision| {
            revision
                .checked_add(1)
                .ok_or(hiroute_application::control::ComputeManagementControlError::Corrupt)
        })?;
        let pending = pending_candidate(
            &source,
            candidate_revision,
            existing_source
                .as_ref()
                .map(|saved| (saved.source_id.clone(), saved.lineage_digest.clone())),
        )?;
        let view = self
            .model_connections
            .candidate_port()
            .register_compute_candidate(pending.clone())
            .map_err(map_port)?;
        self.subscription_sources
            .lock()
            .map_err(|_| hiroute_application::control::ComputeManagementControlError::Unavailable)?
            .insert(
                pending.candidate.candidate_ref.clone(),
                SubscriptionContextV1 {
                    source,
                    evidence: None,
                    candidate_revision: pending.candidate.candidate_revision,
                },
            );
        Ok(vec![view])
    }

    pub(super) fn remember_subscription_preview(
        &self,
        preview: &ComputeSubscriptionPreparedPreviewV2,
    ) -> Result<(), hiroute_application::control::ComputeManagementControlError> {
        let intent = preview
            .plan
            .external()
            .first()
            .ok_or(hiroute_application::control::ComputeManagementControlError::Corrupt)?;
        let decoded = hiroute_domain::decode_subscription_check_intent(intent.desired())
            .map_err(|_| hiroute_application::control::ComputeManagementControlError::Corrupt)?;
        let context = self
            .subscription_sources
            .lock()
            .map_err(|_| hiroute_application::control::ComputeManagementControlError::Unavailable)?
            .get(decoded.candidate_ref())
            .cloned()
            .ok_or(hiroute_application::control::ComputeManagementControlError::NotFound)?;
        self.subscription_targets
            .lock()
            .map_err(|_| hiroute_application::control::ComputeManagementControlError::Unavailable)?
            .insert(intent.target().to_owned(), context);
        Ok(())
    }

    pub(super) fn validate_subscription_effect(
        &self,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<bool> {
        if !hiroute_domain::is_subscription_check_effect(intent) {
            return Ok(false);
        }
        hiroute_domain::decode_subscription_check_intent(intent.desired())
            .map_err(|_| invalid("subscription.intent.decode"))?;
        if self.cpa_runtime.is_none() {
            return Err(unavailable("subscription.runtime.unavailable"));
        }
        Ok(true)
    }

    pub(super) fn current_subscription_fingerprint(
        &self,
        target: &str,
    ) -> PortResult<Option<CanonicalDigest>> {
        if !target.starts_with("compute-subscription/") {
            return Ok(None);
        }
        if let Some(context) = self
            .subscription_targets
            .lock()
            .map_err(|_| unavailable("subscription.target.lock"))?
            .get(target)
            .cloned()
        {
            return Ok(Some(context.source.evidence_digest().clone()));
        }
        for kind in [CpaAccountKind::Codex, CpaAccountKind::Claude] {
            for source in self
                .subscription_discovery_sources(kind)
                .map_err(control_error_to_port)?
            {
                let candidate =
                    subscription_candidate_ref(&source).map_err(control_error_to_port)?;
                if subscription_target(&candidate)? == target {
                    return Ok(Some(source.evidence_digest().clone()));
                }
            }
        }
        Ok(None)
    }

    pub(super) fn apply_subscription_effect(
        &self,
        operation_id: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<Option<OwnedEffectV1>> {
        if !self.validate_subscription_effect(intent)? {
            return Ok(None);
        }
        let decoded = hiroute_domain::decode_subscription_check_intent(intent.desired())
            .map_err(|_| invalid("subscription.intent.decode"))?;
        let runtime = self
            .cpa_runtime
            .as_ref()
            .and_then(|runtimes| runtimes.runtime_for_candidate(decoded.candidate_ref()))
            .ok_or_else(|| unavailable("subscription.runtime.unavailable"))?;
        let operation = {
            let stores = self.stores_lock()?;
            stores
                .control()
                .load_operation(operation_id)?
                .ok_or_else(|| invalid("subscription.operation.missing"))?
        };
        if !operation
            .plan
            .external()
            .iter()
            .any(|original| original == intent)
        {
            return Err(invalid("subscription.operation.binding"));
        }
        let mut context = self.resolve_subscription_context(&decoded)?;
        let evidence = if interaction::is_explicit_check(operation_id) {
            runtime.inspect_subscription_for_check()
        } else {
            runtime.inspect_subscription()
        }
        .map_err(map_cpa)?;
        if context
            .source
            .expected_account_ref()
            .is_some_and(|expected| expected != evidence.account_ref())
        {
            return Err(conflict("subscription.source.account-changed"));
        }
        context.evidence = Some(evidence.clone());
        let approval = operation_reference(&operation);
        let pending_ref = ComputeCandidateRefV2 {
            candidate_ref: decoded.candidate_ref().to_owned(),
            candidate_revision: decoded.candidate_revision(),
        };
        let pending = self
            .model_connections
            .candidate_port()
            .resolve_compute_candidate(&pending_ref)?;
        if pending.evidence_digest != *decoded.expected_evidence_digest() {
            return Err(conflict("subscription.candidate.evidence"));
        }
        let receipt =
            ComputeSubscriptionResourceReceiptV2::new(format!("subscription/{operation_id}"), 1)?;
        let materializer = CpaSubscriptionMaterializer::new(
            runtime,
            CpaSubscriptionEffectContext::new(
                approval.clone(),
                pending_ref.clone(),
                context.source.descriptor().clone(),
                evidence.clone(),
                decoded
                    .existing_source()
                    .map(|(source_id, expected_revision)| {
                        hiroute_application_api::ComputeSavedSourceExpectationV2 {
                            source_id: source_id.to_owned(),
                            expected_revision,
                        }
                    }),
                context.source.kind().connector_id(),
                receipt,
            )?,
        );
        let validation = materializer.materialize_selected(ComputeApprovedSubscriptionCheckV2 {
            approval_operation: approval.clone(),
            candidate: pending_ref,
            expected_evidence_digest: evidence.evidence_digest().clone(),
            existing_source: decoded
                .existing_source()
                .map(|(source_id, expected_revision)| {
                    hiroute_application_api::ComputeSavedSourceExpectationV2 {
                        source_id: source_id.to_owned(),
                        expected_revision,
                    }
                }),
            protected_source: context.source.descriptor().clone(),
        })?;
        let target = self.subscription_logical_target(context.source.kind())?;
        let checked = verified_subscription_candidate(
            &pending,
            validation.clone(),
            target,
            GatewayAuthenticationSemanticsV1::Bearer,
        )
        .map_err(map_preparation)?;
        let stored =
            StoredSubscriptionValidationV1::new(approval, &pending, &checked, &validation)?;
        let record_json =
            serde_json::to_string(&stored).map_err(|_| invalid("subscription.record.encode"))?;
        {
            let stores = self.stores_lock()?;
            stores.control().stage_compute_subscription_validation(
                operation_id,
                &checked.candidate.candidate_ref,
                checked.candidate.candidate_revision,
                &record_json,
            )?;
        }
        self.model_connections
            .candidate_port()
            .register_compute_candidate(checked.clone())?;
        context.candidate_revision = checked.candidate.candidate_revision;
        self.subscription_sources
            .lock()
            .map_err(|_| unavailable("subscription.source.lock"))?
            .insert(checked.candidate.candidate_ref.clone(), context.clone());
        self.subscription_targets
            .lock()
            .map_err(|_| unavailable("subscription.target.lock"))?
            .insert(intent.target().to_owned(), context);
        Ok(Some(subscription_effect(
            operation_id,
            intent,
            &checked.candidate,
        )?))
    }

    pub(super) fn observe_subscription_effect(
        &self,
        operation_id: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<Option<EffectReconciliation>> {
        if !self.validate_subscription_effect(intent)? {
            return Ok(None);
        }
        let record = {
            let stores = self.stores_lock()?;
            stores
                .control()
                .compute_subscription_validation(operation_id)?
        };
        let Some(record) = record else {
            return Ok(Some(EffectReconciliation::Missing));
        };
        let candidate = ComputeCandidateRefV2 {
            candidate_ref: record.candidate_ref,
            candidate_revision: record.candidate_revision,
        };
        let effect = subscription_effect(operation_id, intent, &candidate)?;
        Ok(Some(match record.state {
            ComputeSubscriptionValidationStateV1::Staged => EffectReconciliation::Staged(effect),
            ComputeSubscriptionValidationStateV1::Verified
            | ComputeSubscriptionValidationStateV1::Retained => {
                EffectReconciliation::Applied(effect)
            }
            ComputeSubscriptionValidationStateV1::Released => EffectReconciliation::Missing,
        }))
    }

    pub(super) fn activate_subscription_effect(
        &self,
        effect: &OwnedEffectV1,
    ) -> PortResult<Option<OwnedEffectV1>> {
        let Some((operation_id, _)) = subscription_marker(effect)? else {
            return Ok(None);
        };
        {
            let stores = self.stores_lock()?;
            stores
                .control()
                .activate_compute_subscription_validation(&operation_id)?;
        }
        Ok(Some(effect.clone()))
    }

    pub(super) fn compensate_subscription_effect(
        &self,
        effect: &OwnedEffectV1,
    ) -> PortResult<Option<CompensationOutcome>> {
        let Some((operation_id, _)) = subscription_marker(effect)? else {
            return Ok(None);
        };
        let record = {
            let stores = self.stores_lock()?;
            stores
                .control()
                .compute_subscription_validation(&operation_id)?
        };
        let Some(record) = record else {
            return Ok(Some(CompensationOutcome::AlreadyCompensated));
        };
        if record.state == ComputeSubscriptionValidationStateV1::Released {
            return Ok(Some(CompensationOutcome::AlreadyCompensated));
        }
        let stored = decode_stored(&record.record_json)?;
        self.shutdown_unretained_subscription_runtime(&stored.original_candidate.candidate_ref)?;
        {
            let stores = self.stores_lock()?;
            stores
                .control()
                .release_compute_subscription_validation(&operation_id)?;
        }
        Ok(Some(CompensationOutcome::Compensated))
    }

    pub(super) fn subscription_check_result(
        &self,
        requested: &OperationReferenceV1,
    ) -> Result<
        ComputeSubscriptionCheckResultV2,
        hiroute_application::control::ComputeManagementControlError,
    > {
        let operation_id = OperationId::parse(&requested.operation_id)
            .map_err(|_| hiroute_application::control::ComputeManagementControlError::Invalid)?;
        let (operation, record) = {
            let stores = self.stores_lock().map_err(|_| {
                hiroute_application::control::ComputeManagementControlError::Unavailable
            })?;
            let operation = stores
                .control()
                .load_operation(&operation_id)
                .map_err(map_port)?
                .ok_or(hiroute_application::control::ComputeManagementControlError::NotFound)?;
            let record = stores
                .control()
                .compute_subscription_validation(&operation_id)
                .map_err(map_port)?;
            (operation, record)
        };
        if requested.sequence > operation.generation {
            return Err(hiroute_application::control::ComputeManagementControlError::Conflict);
        }
        let intent = operation
            .plan
            .external()
            .first()
            .and_then(|value| {
                hiroute_domain::decode_subscription_check_intent(value.desired()).ok()
            })
            .ok_or(hiroute_application::control::ComputeManagementControlError::Invalid)?;
        let original_candidate = ComputeCandidateRefV2 {
            candidate_ref: intent.candidate_ref().to_owned(),
            candidate_revision: intent.candidate_revision(),
        };
        let approval = operation_reference(&operation);
        let mut result = ComputeSubscriptionCheckResultV2 {
            candidate: original_candidate,
            approval_operation: approval,
            status: subscription_operation_status(&operation),
            save_operation: None,
            validation: None,
            checked_candidate: None,
            reason: operation.safe_error_code.clone(),
        };
        if let Some(record) = record {
            match record.state {
                ComputeSubscriptionValidationStateV1::Staged => {
                    result.status = ComputeSubscriptionCheckStatusV2::Checking;
                }
                ComputeSubscriptionValidationStateV1::Verified
                | ComputeSubscriptionValidationStateV1::Retained => {
                    let stored = decode_stored(&record.record_json).map_err(map_port)?;
                    let source_status = if record.state
                        == ComputeSubscriptionValidationStateV1::Verified
                        && record.save_operation_id.is_none()
                    {
                        self.subscription_source_status(&record, &stored)?
                    } else {
                        None
                    };
                    let checked = stored.checked_facts().map_err(map_port)?;
                    let checked_view = if source_status.is_some() {
                        None
                    } else {
                        match self
                            .model_connections
                            .candidate_port()
                            .register_compute_candidate(checked)
                        {
                            Ok(view) => Some(view),
                            Err(error) if error.code == PortErrorCode::Conflict => {
                                match self
                                    .model_connections
                                    .candidate_port()
                                    .get_compute_candidate(&stored.checked_candidate)
                                {
                                    // A different fact set at the same immutable revision is corrupt.
                                    Ok(_) => {
                                        return Err(hiroute_application::control::ComputeManagementControlError::Corrupt);
                                    }
                                    // A newer trusted revision means the discovered source changed
                                    // after A. Keep the durable validation releasable, but never expose
                                    // the superseded checked candidate to B.
                                    Err(current) if current.code == PortErrorCode::Conflict => None,
                                    Err(current) => return Err(map_port(current)),
                                }
                            }
                            Err(error) => return Err(map_port(error)),
                        }
                    };
                    result.validation = Some(stored.validation.clone());
                    result.checked_candidate = checked_view;
                    result.save_operation = record
                        .save_operation_id
                        .as_deref()
                        .map(|id| self.operation_reference_by_id(id))
                        .transpose()?;
                    result.status =
                        if record.state == ComputeSubscriptionValidationStateV1::Retained {
                            ComputeSubscriptionCheckStatusV2::Retained
                        } else if let Some((status, reason)) = source_status {
                            result.reason = Some(reason.into());
                            status
                        } else if result.checked_candidate.is_some() {
                            ComputeSubscriptionCheckStatusV2::Verified
                        } else {
                            result.reason = Some("SUBSCRIPTION_SOURCE_CHANGED".into());
                            ComputeSubscriptionCheckStatusV2::SourceChanged
                        };
                }
                ComputeSubscriptionValidationStateV1::Released => {
                    result.status = ComputeSubscriptionCheckStatusV2::Released;
                }
            }
        }
        result
            .validate_shape()
            .map_err(|_| hiroute_application::control::ComputeManagementControlError::Corrupt)?;
        Ok(result)
    }

    pub(super) fn release_subscription_validation(
        &self,
        validation: &ComputeValidationRefV2,
    ) -> Result<
        ComputeSubscriptionCheckResultV2,
        hiroute_application::control::ComputeManagementControlError,
    > {
        let requested = &validation.approval_operation;
        let before = self.subscription_check_result(requested)?;
        if before.validation.as_ref() != Some(validation) {
            return Err(hiroute_application::control::ComputeManagementControlError::Conflict);
        }
        if before.status == ComputeSubscriptionCheckStatusV2::Retained {
            return Ok(before);
        }
        if before.save_operation.is_some() {
            return Ok(before);
        }
        let operation_id = OperationId::parse(&requested.operation_id)
            .map_err(|_| hiroute_application::control::ComputeManagementControlError::Invalid)?;
        let stored = {
            let stores = self.stores_lock().map_err(|_| {
                hiroute_application::control::ComputeManagementControlError::Unavailable
            })?;
            let record = stores
                .control()
                .compute_subscription_validation(&operation_id)
                .map_err(map_port)?
                .ok_or(hiroute_application::control::ComputeManagementControlError::NotFound)?;
            decode_stored(&record.record_json).map_err(map_port)?
        };
        self.shutdown_unretained_subscription_runtime(&stored.original_candidate.candidate_ref)
            .map_err(map_port)?;
        {
            let stores = self.stores_lock().map_err(|_| {
                hiroute_application::control::ComputeManagementControlError::Unavailable
            })?;
            stores
                .control()
                .release_compute_subscription_validation(&operation_id)
                .map_err(map_port)?;
        }
        if let Some(context) = self
            .subscription_sources
            .lock()
            .map_err(|_| hiroute_application::control::ComputeManagementControlError::Unavailable)?
            .get_mut(&stored.original_candidate.candidate_ref)
        {
            let next = stored
                .checked_candidate
                .candidate_revision
                .checked_add(1)
                .ok_or(hiroute_application::control::ComputeManagementControlError::Corrupt)?;
            let saved = stored.existing_source_id.as_deref().and_then(|source_id| {
                self.stores_lock()
                    .ok()
                    .and_then(|stores| {
                        stores
                            .control()
                            .compute_management_source(source_id)
                            .ok()
                            .flatten()
                    })
                    .map(|source| (source.source_id, source.lineage_digest))
            });
            let pending = pending_candidate(&context.source, next, saved)?;
            self.model_connections
                .candidate_port()
                .register_compute_candidate(pending)
                .map_err(map_port)?;
            context.evidence = None;
            context.candidate_revision = next;
        }
        self.subscription_check_result(requested)
    }

    pub(super) fn finish_compute_subscription_save(&self, operation: &hiroute_domain::OperationV1) {
        if operation.state != OperationState::Succeeded {
            return;
        }
        let Ok(change) = serde_json::from_value::<hiroute_application_api::ComputeManagementChangeV2>(
            operation.plan.spec().desired_state.clone(),
        ) else {
            return;
        };
        let Some(source_id) = operation.plan.spec().resource_id.as_deref() else {
            return;
        };
        let Ok(stores) = self.stores_lock() else {
            return;
        };
        let Ok(Some(source)) = stores.control().compute_management_source(source_id) else {
            return;
        };
        match &change.subject {
            hiroute_application_api::ComputeManagementSubjectV2::Candidate { candidate } => {
                let Some(validation) = change.validation.as_ref() else {
                    return;
                };
                if source.last_candidate_ref != candidate.candidate_ref
                    || source.validation.as_ref().is_none_or(|saved| {
                        saved.validation_ref != validation.validation_ref
                            || saved.validation_revision != validation.validation_revision
                            || saved.approval_operation_id
                                != validation.approval_operation.operation_id
                    })
                {
                    return;
                }
            }
            hiroute_application_api::ComputeManagementSubjectV2::SavedSource {
                source_id: expected,
            } if expected != source_id => return,
            _ => {}
        }
        drop(stores);
        // Selection and lifecycle changes are serialized and revision-guarded together.
        // Never keep the storage lock while the CPA projection performs external work.
        match self.project_saved_subscription_source(&source) {
            Ok(()) => {
                let _ = self.clear_subscription_maintenance_status(&source.source_id);
            }
            Err(CpaLifecycleError::StaleSourceManagement) => {}
            Err(_) => {
                let _ = self.set_subscription_maintenance_status(
                    &source.source_id,
                    maintenance::SubscriptionMaintenancePresentation::RuntimeUnavailable,
                    None,
                );
            }
        }
    }

    fn project_saved_subscription_source(
        &self,
        source: &hiroute_domain::ComputeManagementSourceV2,
    ) -> Result<(), CpaLifecycleError> {
        let Some(runtimes) = &self.cpa_runtime else {
            return Ok(());
        };
        let hiroute_domain::ComputeManagementProvenanceV2::ConnectorOwned {
            account_ref,
            connector_id,
        } = &source.provenance
        else {
            return Ok(());
        };
        let Some(kind) = CpaAccountKind::from_connector(connector_id) else {
            return Ok(());
        };
        if CpaAccountKind::from_candidate(&source.last_candidate_ref) != Some(kind) {
            return Err(CpaLifecycleError::InvalidSourceManagement);
        }
        let state = if source.state == hiroute_domain::MaterializationState::Ready {
            CpaSourceManagementState::Enabled
        } else {
            CpaSourceManagementState::Disabled
        };
        runtimes.apply_saved_source(
            &source.source_id,
            &source.last_candidate_ref,
            account_ref,
            source.revision,
            state,
        )
    }

    /// Releasing a check must never stop another login or the saved runtime of this provider.
    fn shutdown_unretained_subscription_runtime(&self, candidate: &str) -> PortResult<()> {
        let saved = self
            .stores_lock()?
            .control()
            .compute_management_snapshot(&hiroute_domain::WorkspaceId::default())?
            .sources
            .into_iter()
            .any(|source| source.last_candidate_ref == candidate);
        if saved {
            return Ok(());
        }
        if let Some(runtime) = self
            .cpa_runtime
            .as_ref()
            .and_then(|runtimes| runtimes.runtime_for_candidate(candidate))
        {
            match runtime.shutdown() {
                Ok(_) | Err(CpaLifecycleError::NotStarted) => {}
                Err(error) => return Err(map_cpa(error)),
            }
        }
        Ok(())
    }

    fn resolve_subscription_context(
        &self,
        intent: &hiroute_domain::SubscriptionCheckIntentV2,
    ) -> PortResult<SubscriptionContextV1> {
        let source = self
            .subscription_source_for_candidate(intent.candidate_ref())
            .map_err(control_error_to_port)?
            .ok_or_else(|| not_found("subscription.source.missing"))?;
        let candidate_ref = subscription_candidate_ref(&source).map_err(control_error_to_port)?;
        if candidate_ref != intent.candidate_ref()
            || source.evidence_digest() != intent.expected_evidence_digest()
        {
            return Err(conflict("subscription.source.changed"));
        }
        Ok(SubscriptionContextV1 {
            source,
            evidence: None,
            candidate_revision: intent.candidate_revision(),
        })
    }

    pub(super) fn subscription_logical_target(
        &self,
        kind: CpaAccountKind,
    ) -> PortResult<ComputeCandidateTargetV2> {
        let catalog = self
            .release_catalog
            .as_ref()
            .ok_or_else(|| unavailable("subscription.catalog.unavailable"))?;
        let resolved = catalog
            .resolve_connection_option(kind.connection_option_id())
            .map_err(|_| invalid("subscription.option.resolve"))?;
        if resolved.connector.connector_id != kind.connector_id() {
            return Err(invalid("subscription.option.connector"));
        }
        let endpoint = resolved
            .endpoint_profile
            .protocol_endpoints
            .iter()
            .find(|endpoint| endpoint.protocol == kind.required_protocol())
            .ok_or_else(|| invalid("subscription.option.endpoint"))?;
        let authority = endpoint
            .base_url
            .strip_prefix("https://")
            .filter(|value| !value.is_empty() && !value.contains('/'))
            .ok_or_else(|| invalid("subscription.option.base"))?;
        Ok(ComputeCandidateTargetV2 {
            scheme: "https".into(),
            authority: authority.into(),
            port: 443,
            request_path: endpoint.request_path.clone(),
            upstream_protocol: endpoint.protocol,
            protocol_profile_id: resolved.endpoint_profile.endpoint_profile_id.clone(),
            protocol_profile_revision: resolved.endpoint_profile.revision,
        })
    }

    fn operation_reference_by_id(
        &self,
        operation_id: &str,
    ) -> Result<OperationReferenceV1, hiroute_application::control::ComputeManagementControlError>
    {
        let operation_id = OperationId::parse(operation_id)
            .map_err(|_| hiroute_application::control::ComputeManagementControlError::Corrupt)?;
        let stores = self.stores_lock().map_err(|_| {
            hiroute_application::control::ComputeManagementControlError::Unavailable
        })?;
        let operation = stores
            .control()
            .load_operation(&operation_id)
            .map_err(map_port)?
            .ok_or(hiroute_application::control::ComputeManagementControlError::Corrupt)?;
        Ok(operation_reference(&operation))
    }
}

fn subscription_operation_status(
    operation: &hiroute_domain::OperationV1,
) -> ComputeSubscriptionCheckStatusV2 {
    if !operation.state.is_terminal() {
        return ComputeSubscriptionCheckStatusV2::Checking;
    }
    subscription_error_status(operation.safe_error_code.as_deref())
}

fn subscription_error_status(code: Option<&str>) -> ComputeSubscriptionCheckStatusV2 {
    match code {
        Some("SUBSCRIPTION_SOURCE_CHANGED" | "CHANGE_PREVIEW_STALE") => {
            ComputeSubscriptionCheckStatusV2::SourceChanged
        }
        Some(
            "SUBSCRIPTION_NEEDS_AUTH"
            | "SUBSCRIPTION_NATIVE_LOGIN_MISSING"
            | "SUBSCRIPTION_NATIVE_LOGIN_INVALID"
            | "SUBSCRIPTION_NATIVE_ACCOUNT_MISSING"
            | "SUBSCRIPTION_NATIVE_STORE_UNSUPPORTED"
            | "SUBSCRIPTION_NATIVE_LOGIN_UNSUPPORTED"
            | "SUBSCRIPTION_MANAGED_LOGIN_REQUIRED",
        ) => ComputeSubscriptionCheckStatusV2::NeedsAuth,
        Some("SUBSCRIPTION_NATIVE_READ_FAILED" | "SUBSCRIPTION_NATIVE_CLIENT_UNAVAILABLE") => {
            ComputeSubscriptionCheckStatusV2::Unavailable
        }
        Some("SUBSCRIPTION_RUNTIME_UNAVAILABLE") => ComputeSubscriptionCheckStatusV2::Unavailable,
        _ => ComputeSubscriptionCheckStatusV2::Failed,
    }
}

fn subscription_target(candidate_ref: &str) -> PortResult<String> {
    let digest = CanonicalDigest::of(&("hiroute.compute-subscription-target/v2", candidate_ref))
        .map_err(|_| invalid("subscription.target.digest"))?;
    Ok(format!(
        "compute-subscription/{}",
        digest.as_str().trim_start_matches("sha256:")
    ))
}

fn subscription_effect(
    operation_id: &OperationId,
    intent: &ExternalEffectIntentV1,
    candidate: &ComputeCandidateRefV2,
) -> PortResult<OwnedEffectV1> {
    Ok(OwnedEffectV1 {
        effect_id: hiroute_domain::COMPUTE_SUBSCRIPTION_EFFECT_ID_V2.into(),
        kind: OwnedEffectKind::AgentArtifact,
        target: intent.target().to_owned(),
        before_fingerprint: intent.before_fingerprint().cloned(),
        after_fingerprint: intent.before_fingerprint().cloned(),
        compensation: serde_json::to_value(SubscriptionEffectMarkerV1 {
            schema: MARKER_SCHEMA.into(),
            operation_id: operation_id.to_string(),
            candidate: candidate.clone(),
        })
        .map_err(|_| invalid("subscription.effect.encode"))?
        .into(),
    })
}

fn subscription_marker(
    effect: &OwnedEffectV1,
) -> PortResult<Option<(OperationId, ComputeCandidateRefV2)>> {
    if effect.effect_id != hiroute_domain::COMPUTE_SUBSCRIPTION_EFFECT_ID_V2 {
        return Ok(None);
    }
    let marker: SubscriptionEffectMarkerV1 =
        serde::Deserialize::deserialize(effect.compensation.as_ref())
            .map_err(|_| invalid("subscription.effect.decode"))?;
    let operation_id = OperationId::parse(marker.operation_id)
        .map_err(|_| invalid("subscription.effect.operation"))?;
    if marker.schema != MARKER_SCHEMA
        || effect.kind != OwnedEffectKind::AgentArtifact
        || !effect.target.starts_with("compute-subscription/")
        || effect.before_fingerprint != effect.after_fingerprint
    {
        return Err(invalid("subscription.effect.shape"));
    }
    marker
        .candidate
        .validate_shape()
        .map_err(|_| invalid("subscription.effect.candidate"))?;
    Ok(Some((operation_id, marker.candidate)))
}

fn operation_reference(operation: &hiroute_domain::OperationV1) -> OperationReferenceV1 {
    OperationReferenceV1 {
        operation_id: operation.operation_id.to_string(),
        state: operation.state.as_str().into(),
        sequence: operation.generation,
        cancellable: !operation.state.is_terminal(),
    }
}
