//! Bounded automatic maintenance for an already confirmed native subscription.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use hiroute_application::compute_management::{
    ComputeManagementPlanner, ComputeSubscriptionMaintenanceScopeV1,
};
use hiroute_application::subscriptions::ComputeSubscriptionPlanner;
use hiroute_application_api::{
    APPLY_COMPUTE_SAVE_OPERATION_V2, APPLY_SUBSCRIPTION_CHECK_OPERATION_V2,
    COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2, ComputeCandidateFactStateV2,
    ComputeConnectionApplyRequestV1, ComputeManagementChangeV2, ComputeManagementIntentV2,
    ComputeManagementSubjectV2, ComputeSubscriptionCheckStatusV2, OperationReferenceV1,
};
use hiroute_domain::{
    CanonicalDigest, ComputeManagementProvenanceV2, ComputeManagementRepositoryPort,
    ComputeManagementSourceV2, MaterializationState, OperationState, RevisionSetV1, WorkspaceId,
};
use hiroute_local_storage::ApplyCapabilityRegistrationV1;

use super::{LocalControlAdapter, subscription_candidate_ref};
use hiroute_cpa_bridge::CpaAccountKind;

const SUBSCRIPTION_EVIDENCE_INTERVAL_MS: i64 = 5_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::control::runtime) enum SubscriptionMaintenancePresentation {
    Updating,
    AuthenticationRequired,
    RuntimeUnavailable,
}

pub(in crate::control::runtime) struct SubscriptionMaintenance {
    enabled: bool,
    next_scan_ms: i64,
    process_run: CanonicalDigest,
    entries: BTreeMap<String, SubscriptionMaintenanceEntry>,
}

#[derive(Clone)]
struct SubscriptionMaintenanceEntry {
    presentation: SubscriptionMaintenancePresentation,
    failed_evidence: Option<CanonicalDigest>,
    failed_source_revision: Option<u64>,
}

#[derive(Clone, Copy)]
enum MaintenanceFailure {
    AuthenticationRequired,
    RuntimeUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommittedEvidenceAction {
    Restore,
    Retry,
    HoldFailure,
}

impl MaintenanceFailure {
    const fn presentation(self) -> SubscriptionMaintenancePresentation {
        match self {
            Self::AuthenticationRequired => {
                SubscriptionMaintenancePresentation::AuthenticationRequired
            }
            Self::RuntimeUnavailable => SubscriptionMaintenancePresentation::RuntimeUnavailable,
        }
    }
}

impl LocalControlAdapter {
    pub(in crate::control::runtime) fn enable_subscription_maintenance(&self) {
        if let Ok(mut maintenance) = self.subscription_maintenance.lock() {
            maintenance.enabled = true;
            maintenance.next_scan_ms = 0;
        }
    }

    pub(in crate::control::runtime) fn subscription_maintenance_presentation(
        &self,
        source_id: &str,
    ) -> Option<SubscriptionMaintenancePresentation> {
        self.subscription_maintenance
            .lock()
            .ok()
            .and_then(|maintenance| {
                maintenance
                    .entries
                    .get(source_id)
                    .map(|entry| entry.presentation)
            })
    }

    pub(in crate::control::runtime) fn scan_subscription_maintenance(
        &self,
        now_ms: i64,
    ) -> Result<(), String> {
        if now_ms < 0 {
            return Err("subscription maintenance clock is invalid".to_owned());
        }
        {
            let mut maintenance = self
                .subscription_maintenance
                .lock()
                .map_err(|_| "subscription maintenance is unavailable".to_owned())?;
            if !maintenance.enabled || now_ms < maintenance.next_scan_ms {
                return Ok(());
            }
            maintenance.next_scan_ms = now_ms.saturating_add(SUBSCRIPTION_EVIDENCE_INTERVAL_MS);
        }

        let sources = match self.subscription_maintenance_sources() {
            Ok(sources) => sources,
            Err(error) => {
                self.suspend_subscription_execution();
                return Err(error);
            }
        };
        let active_ids = sources
            .iter()
            .map(|source| source.source_id.clone())
            .collect::<BTreeSet<_>>();
        self.subscription_maintenance
            .lock()
            .map_err(|_| "subscription maintenance is unavailable".to_owned())?
            .entries
            .retain(|source_id, _| active_ids.contains(source_id));
        if sources.is_empty() {
            return Ok(());
        }

        for source in sources {
            let Some(kind) = CpaAccountKind::from_candidate(&source.last_candidate_ref) else {
                continue;
            };
            let native_source = match self.scanner.subscription_source(kind) {
                Ok(Some(native)) => native,
                _ => {
                    self.suspend_subscription_execution_for(kind);
                    self.set_subscription_maintenance_status(
                        &source.source_id,
                        SubscriptionMaintenancePresentation::AuthenticationRequired,
                        None,
                    )?;
                    continue;
                }
            };
            let candidate_ref = subscription_candidate_ref(&native_source)
                .map_err(|_| "subscription candidate identity is invalid".to_owned())?;
            let evidence = native_source.evidence_digest().clone();
            if source.last_candidate_ref != candidate_ref {
                self.suspend_subscription_execution_for(kind);
                self.set_subscription_maintenance_status(
                    &source.source_id,
                    SubscriptionMaintenancePresentation::AuthenticationRequired,
                    None,
                )?;
                continue;
            }
            let committed = match self.committed_subscription_evidence(&source) {
                Ok(committed) => committed,
                Err(_) => {
                    self.suspend_subscription_execution_for(kind);
                    self.set_subscription_maintenance_status(
                        &source.source_id,
                        SubscriptionMaintenancePresentation::RuntimeUnavailable,
                        None,
                    )?;
                    continue;
                }
            };
            if committed.as_ref() == Some(&evidence) {
                let failed = self
                    .subscription_maintenance
                    .lock()
                    .map_err(|_| "subscription maintenance is unavailable".to_owned())?
                    .entries
                    .get(&source.source_id)
                    .and_then(|entry| {
                        entry
                            .failed_evidence
                            .as_ref()
                            .zip(entry.failed_source_revision)
                    })
                    .map(|(failed_evidence, revision)| (failed_evidence.clone(), revision));
                match committed_evidence_action(failed.as_ref(), &evidence, source.revision) {
                    CommittedEvidenceAction::Retry => {}
                    CommittedEvidenceAction::HoldFailure => continue,
                    CommittedEvidenceAction::Restore => {
                        if self
                            .restore_committed_subscription_execution(&source)
                            .is_ok()
                        {
                            self.clear_subscription_maintenance_status(&source.source_id)?;
                        } else {
                            self.suspend_subscription_execution_for(kind);
                            self.set_subscription_maintenance_status(
                                &source.source_id,
                                SubscriptionMaintenancePresentation::RuntimeUnavailable,
                                None,
                            )?;
                        }
                        continue;
                    }
                }
            }
            let repeated_failure = self
                .subscription_maintenance
                .lock()
                .map_err(|_| "subscription maintenance is unavailable".to_owned())?
                .entries
                .get(&source.source_id)
                .is_some_and(|entry| {
                    entry.failed_evidence.as_ref() == Some(&evidence)
                        && entry.failed_source_revision == Some(source.revision)
                });
            if repeated_failure {
                continue;
            }
            self.suspend_subscription_execution_for(kind);
            self.set_subscription_maintenance_status(
                &source.source_id,
                SubscriptionMaintenancePresentation::Updating,
                None,
            )?;
            let process_run = self
                .subscription_maintenance
                .lock()
                .map_err(|_| "subscription maintenance is unavailable".to_owned())?
                .process_run
                .clone();
            match self.run_subscription_maintenance(&source, &evidence, &process_run) {
                Ok(()) => self.clear_subscription_maintenance_status(&source.source_id)?,
                Err(failure) => self.set_subscription_maintenance_status(
                    &source.source_id,
                    failure.presentation(),
                    Some((evidence.clone(), source.revision)),
                )?,
            }
        }
        Ok(())
    }

    fn suspend_subscription_execution(&self) {
        for kind in [CpaAccountKind::Codex, CpaAccountKind::Claude] {
            self.suspend_subscription_execution_for(kind);
        }
    }
    fn suspend_subscription_execution_for(&self, kind: CpaAccountKind) {
        if let Some(runtime) = self
            .cpa_runtime
            .as_ref()
            .and_then(|runtimes| runtimes.for_kind(kind))
        {
            runtime.suspend_subscription_execution();
        }
    }

    fn restore_committed_subscription_execution(
        &self,
        source: &ComputeManagementSourceV2,
    ) -> Result<(), String> {
        let Some(runtimes) = &self.cpa_runtime else {
            return Ok(());
        };
        let ComputeManagementProvenanceV2::ConnectorOwned {
            connector_id,
            account_ref,
        } = &source.provenance
        else {
            return Err("subscription maintenance source is not connector-owned".to_owned());
        };
        let runtime = runtimes
            .for_connector(connector_id)
            .ok_or_else(|| "subscription maintenance connector is invalid".to_owned())?;
        runtime
            .apply_account_management(
                account_ref,
                source.revision,
                hiroute_cpa_bridge::CpaSourceManagementState::Enabled,
            )
            .map_err(|error| error.to_string())
    }

    fn subscription_maintenance_sources(&self) -> Result<Vec<ComputeManagementSourceV2>, String> {
        self.stores_lock()
            .map_err(|error| error.to_string())?
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .map_err(|error| error.to_string())
            .map(|snapshot| {
                snapshot
                    .sources
                    .into_iter()
                    .filter(|source| {
                        source.state == MaterializationState::Ready
                            && matches!(
                                &source.provenance,
                                ComputeManagementProvenanceV2::ConnectorOwned {
                                    connector_id,
                                    ..
                                } if CpaAccountKind::from_connector(connector_id).is_some()
                            )
                    })
                    .collect()
            })
    }

    fn run_subscription_maintenance(
        &self,
        source: &ComputeManagementSourceV2,
        evidence: &CanonicalDigest,
        process_run: &CanonicalDigest,
    ) -> Result<(), MaintenanceFailure> {
        let candidate = self
            .refresh_subscription_candidates()
            .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?
            .into_iter()
            .find(|candidate| candidate.existing_source_id.as_deref() == Some(&source.source_id))
            .ok_or(MaintenanceFailure::RuntimeUnavailable)?;

        let checked = if candidate.fact_state == ComputeCandidateFactStateV2::PendingApproval {
            let preview = {
                let stores = self
                    .stores_lock()
                    .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
                ComputeSubscriptionPlanner::new(
                    self.model_connections.candidate_port(),
                    stores.control(),
                )
                .preview(candidate.candidate.clone())
                .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?
            };
            self.remember_subscription_preview(&preview)
                .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
            let capability = self.issue_subscription_maintenance_capability(
                APPLY_SUBSCRIPTION_CHECK_OPERATION_V2,
                &preview.result.accept_digest,
                &preview.result.expected_revisions,
            )?;
            let request = ComputeConnectionApplyRequestV1 {
                spec: preview.result.spec,
                accept_digest: preview.result.accept_digest,
                expected_revisions: preview.result.expected_revisions,
                idempotency_key: maintenance_idempotency_key(
                    "check",
                    source,
                    evidence,
                    process_run,
                )?,
            };
            let prepared = {
                let stores = self
                    .stores_lock()
                    .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
                ComputeSubscriptionPlanner::new(
                    self.model_connections.candidate_port(),
                    stores.control(),
                )
                .prepare_apply(request, Some(capability))
                .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?
            };
            let operation = self
                .apply_subscription_maintenance_prepared(prepared)
                .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
            ensure_operation_succeeded(&operation)?;
            self.subscription_check_result(&operation_reference(&operation))
                .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?
        } else {
            let validation = candidate
                .validation
                .as_ref()
                .ok_or(MaintenanceFailure::RuntimeUnavailable)?;
            self.subscription_check_result(&validation.approval_operation)
                .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?
        };
        if checked.status != ComputeSubscriptionCheckStatusV2::Verified {
            return Err(match checked.reason.as_deref() {
                Some("SUBSCRIPTION_NEEDS_AUTH") => MaintenanceFailure::AuthenticationRequired,
                _ => MaintenanceFailure::RuntimeUnavailable,
            });
        }
        let checked_candidate = checked
            .checked_candidate
            .ok_or(MaintenanceFailure::RuntimeUnavailable)?;
        let validation = checked
            .validation
            .ok_or(MaintenanceFailure::RuntimeUnavailable)?;

        let save_result: Result<(), MaintenanceFailure> = (|| {
            let (current, revisions) = {
                let stores = self
                    .stores_lock()
                    .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
                let snapshot = stores
                    .control()
                    .compute_management_snapshot(&WorkspaceId::default())
                    .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
                let current = snapshot
                    .sources
                    .iter()
                    .find(|current| current.source_id == source.source_id)
                    .cloned()
                    .ok_or(MaintenanceFailure::RuntimeUnavailable)?;
                (current, snapshot.revisions)
            };
            if current.revision != source.revision || current.state != MaterializationState::Ready {
                return Err(MaintenanceFailure::RuntimeUnavailable);
            }
            let change = ComputeManagementChangeV2 {
                schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.to_owned(),
                subject: ComputeManagementSubjectV2::Candidate {
                    candidate: checked_candidate.candidate.clone(),
                },
                expected_revisions: revisions,
                selected_model_refs: current
                    .models
                    .iter()
                    .map(|model| model.model_ref.clone())
                    .collect(),
                intent: ComputeManagementIntentV2::SaveReady,
                key_edits: Vec::new(),
                validation: Some(validation.clone()),
            };
            change
                .validate_shape()
                .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
            self.validate_subscription_save_change(&change)
                .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
            let scope = ComputeSubscriptionMaintenanceScopeV1 {
                source_id: source.source_id.clone(),
                expected_source_revision: source.revision,
            };
            let preview = {
                let stores = self
                    .stores_lock()
                    .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
                ComputeManagementPlanner::new(
                    self.model_connections.candidate_port(),
                    stores.control(),
                    stores.secrets(),
                    self,
                )
                .preview_subscription_maintenance(change, &scope)
                .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?
            };
            let capability = self.issue_subscription_maintenance_capability(
                APPLY_COMPUTE_SAVE_OPERATION_V2,
                &preview.result.accept_digest,
                &preview.result.expected_revisions,
            )?;
            let request = ComputeConnectionApplyRequestV1 {
                spec: preview.result.spec,
                accept_digest: preview.result.accept_digest,
                expected_revisions: preview.result.expected_revisions,
                idempotency_key: maintenance_idempotency_key(
                    "save",
                    source,
                    evidence,
                    process_run,
                )?,
            };
            let scanner = self.scanner.clone();
            let expected_candidate_ref = source.last_candidate_ref.clone();
            let expected_evidence = evidence.clone();
            let prepared = {
                let stores = self
                    .stores_lock()
                    .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
                ComputeManagementPlanner::new(
                    self.model_connections.candidate_port(),
                    stores.control(),
                    stores.secrets(),
                    self,
                )
                .prepare_subscription_maintenance_apply(request, capability, &scope, move || {
                    let observed = scanner
                        .subscription_source(
                            CpaAccountKind::from_candidate(&expected_candidate_ref)
                                .ok_or(hiroute_application::TransactionError::ChangePreviewStale)?,
                        )
                        .map_err(|_| hiroute_application::TransactionError::ChangePreviewStale)?
                        .ok_or(hiroute_application::TransactionError::ChangePreviewStale)?;
                    if observed.evidence_digest() != &expected_evidence
                        || subscription_candidate_ref(&observed).map_err(|_| {
                            hiroute_application::TransactionError::ChangePreviewStale
                        })? != expected_candidate_ref
                    {
                        return Err(hiroute_application::TransactionError::ChangePreviewStale);
                    }
                    Ok(())
                })
                .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?
            };
            let operation = self
                .apply_subscription_maintenance_prepared(prepared)
                .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
            ensure_operation_succeeded(&operation)
        })();
        if save_result.is_err() {
            let _ = self.release_subscription_validation(&validation);
        }
        save_result
    }

    fn issue_subscription_maintenance_capability(
        &self,
        operation_kind: &str,
        accepted_digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<String, MaintenanceFailure> {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
        let mut capability = String::with_capacity(64);
        for byte in bytes {
            use std::fmt::Write as _;
            write!(&mut capability, "{byte:02x}")
                .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
        }
        let expires_at: i64 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?
            .as_secs()
            .try_into()
            .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
        let registration = ApplyCapabilityRegistrationV1::from_subscription_maintenance(
            capability.clone(),
            WorkspaceId::default(),
            operation_kind,
            accepted_digest.clone(),
            revisions.clone(),
            expires_at + 120,
        )
        .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
        self.stores_lock()
            .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?
            .apply_capability_registrar()
            .register(registration)
            .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
        Ok(capability)
    }

    fn set_subscription_maintenance_status(
        &self,
        source_id: &str,
        presentation: SubscriptionMaintenancePresentation,
        failure: Option<(CanonicalDigest, u64)>,
    ) -> Result<(), String> {
        let (failed_evidence, failed_source_revision) = failure
            .map(|(evidence, revision)| (Some(evidence), Some(revision)))
            .unwrap_or((None, None));
        self.subscription_maintenance
            .lock()
            .map_err(|_| "subscription maintenance is unavailable".to_owned())?
            .entries
            .insert(
                source_id.to_owned(),
                SubscriptionMaintenanceEntry {
                    presentation,
                    failed_evidence,
                    failed_source_revision,
                },
            );
        Ok(())
    }

    fn clear_subscription_maintenance_status(&self, source_id: &str) -> Result<(), String> {
        self.subscription_maintenance
            .lock()
            .map_err(|_| "subscription maintenance is unavailable".to_owned())?
            .entries
            .remove(source_id);
        Ok(())
    }
}

fn committed_evidence_action(
    failure: Option<&(CanonicalDigest, u64)>,
    evidence: &CanonicalDigest,
    source_revision: u64,
) -> CommittedEvidenceAction {
    match failure {
        Some((failed_evidence, revision)) if *revision == source_revision => {
            if failed_evidence == evidence {
                CommittedEvidenceAction::HoldFailure
            } else {
                CommittedEvidenceAction::Retry
            }
        }
        _ => CommittedEvidenceAction::Restore,
    }
}

impl SubscriptionMaintenance {
    pub(in crate::control::runtime) fn new() -> Result<Self, String> {
        let mut random = [0_u8; 32];
        getrandom::fill(&mut random)
            .map_err(|_| "subscription maintenance identity is unavailable".to_owned())?;
        Ok(Self {
            enabled: false,
            next_scan_ms: 0,
            process_run: CanonicalDigest::of_bytes(&random),
            entries: BTreeMap::new(),
        })
    }
}

fn maintenance_idempotency_key(
    phase: &str,
    source: &ComputeManagementSourceV2,
    evidence: &CanonicalDigest,
    process_run: &CanonicalDigest,
) -> Result<String, MaintenanceFailure> {
    let digest = CanonicalDigest::of(&(
        "hiroute.subscription-maintenance/v2",
        process_run,
        phase,
        &source.source_id,
        source.revision,
        evidence,
    ))
    .map_err(|_| MaintenanceFailure::RuntimeUnavailable)?;
    Ok(format!(
        "subscription-maintenance-{phase}-{}",
        hiroute_domain::digest_suffix(&digest, 32)
    ))
}

fn operation_reference(operation: &hiroute_domain::OperationV1) -> OperationReferenceV1 {
    OperationReferenceV1 {
        operation_id: operation.operation_id.to_string(),
        state: operation.state.as_str().to_owned(),
        sequence: operation.generation,
        cancellable: !operation.state.is_terminal(),
    }
}

fn ensure_operation_succeeded(
    operation: &hiroute_domain::OperationV1,
) -> Result<(), MaintenanceFailure> {
    if operation.state == OperationState::Succeeded {
        return Ok(());
    }
    Err(match operation.safe_error_code.as_deref() {
        Some("SUBSCRIPTION_NEEDS_AUTH") => MaintenanceFailure::AuthenticationRequired,
        _ => MaintenanceFailure::RuntimeUnavailable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_evidence_restores_or_retries_without_looping_the_same_failure() {
        let committed = CanonicalDigest::of_bytes(b"committed");
        let changed = CanonicalDigest::of_bytes(b"changed");

        assert_eq!(
            committed_evidence_action(None, &committed, 7),
            CommittedEvidenceAction::Restore
        );
        assert_eq!(
            committed_evidence_action(Some(&(changed, 7)), &committed, 7),
            CommittedEvidenceAction::Retry
        );
        assert_eq!(
            committed_evidence_action(Some(&(committed.clone(), 7)), &committed, 7),
            CommittedEvidenceAction::HoldFailure
        );
        assert_eq!(
            committed_evidence_action(Some(&(committed.clone(), 7)), &committed, 8),
            CommittedEvidenceAction::Restore
        );
    }
}
