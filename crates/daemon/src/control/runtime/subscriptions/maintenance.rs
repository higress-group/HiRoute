//! Bounded maintenance of saved subscriptions without transferring refresh ownership.

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

        // Pending-login expiry/cancellation cleanup is independent of saved execution.
        // Keep maintaining healthy saved sources even when one optional cleanup fails.
        let login_maintenance = self.cpa_runtime.as_ref().map_or(Ok(()), |runtimes| {
            runtimes
                .maintain_login_sessions()
                .map_err(|_| "subscription login maintenance is unavailable".to_owned())
        });

        let shutdown_failures = self
            .cpa_runtime
            .as_ref()
            .map(|runtimes| runtimes.retry_saved_shutdowns())
            .unwrap_or_default();
        let housekeeping = if shutdown_failures.is_empty() {
            login_maintenance
        } else {
            Err("subscription runtime shutdown remains unavailable".to_owned())
        };

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
            return housekeeping;
        }

        for source in sources {
            if shutdown_failures.contains(&source.source_id) {
                self.set_subscription_maintenance_status(
                    &source.source_id,
                    SubscriptionMaintenancePresentation::RuntimeUnavailable,
                    None,
                )?;
                continue;
            }
            if source.state != MaterializationState::Ready {
                self.clear_subscription_maintenance_status(&source.source_id)?;
                continue;
            }
            let Some(kind) = CpaAccountKind::from_candidate(&source.last_candidate_ref) else {
                continue;
            };
            let native_source =
                match self.subscription_source_for_candidate(&source.last_candidate_ref) {
                    Ok(Some(native)) => native,
                    _ => {
                        self.suspend_saved_subscription_execution(&source);
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
                self.suspend_saved_subscription_execution(&source);
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
                    self.suspend_saved_subscription_execution(&source);
                    self.set_subscription_maintenance_status(
                        &source.source_id,
                        SubscriptionMaintenancePresentation::RuntimeUnavailable,
                        None,
                    )?;
                    continue;
                }
            };
            // Managed credential rotation never changes discovery evidence or starts a
            // save/check chain. Validate identity every cycle so a transient CPA file write
            // or control outage can recover without an evidence-generation change.
            if native_source.is_managed() {
                let account = match &source.provenance {
                    ComputeManagementProvenanceV2::ConnectorOwned { account_ref, .. } => {
                        account_ref
                    }
                    _ => continue,
                };
                if committed.as_ref() != Some(&evidence)
                    || native_source.expected_account_ref() != Some(account.as_str())
                {
                    self.suspend_saved_subscription_execution(&source);
                    self.set_subscription_maintenance_status(
                        &source.source_id,
                        SubscriptionMaintenancePresentation::AuthenticationRequired,
                        None,
                    )?;
                    continue;
                }
                // Only this confirmed Ready source may restart a crashed refresh writer.
                // Authentication/status reads stay side-effect free, so recovery must
                // precede them and share the saved-source revision/selection gate.
                match self.restore_committed_subscription_execution(&source) {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(error) => {
                        self.suspend_saved_subscription_execution(&source);
                        let status = lifecycle_failure_presentation(&error);
                        self.set_subscription_maintenance_status(&source.source_id, status, None)?;
                        continue;
                    }
                }
                let observed = self
                    .cpa_runtime
                    .as_ref()
                    .and_then(|runtimes| runtimes.runtime_for_candidate(&source.last_candidate_ref))
                    .ok_or(hiroute_cpa_bridge::CpaLifecycleError::ManagedOAuthCredentialsMissing)
                    .and_then(|runtime| runtime.inspect_subscription());
                let status = match observed {
                    Ok(observed)
                        if observed.kind() == kind
                            && observed.account_ref() == *account
                            && native_source.expected_account_ref() == Some(account.as_str()) =>
                    {
                        None
                    }
                    Ok(_) => Some(SubscriptionMaintenancePresentation::AuthenticationRequired),
                    Err(error) => Some(lifecycle_failure_presentation(&error)),
                };
                if let Some(status) = status {
                    self.suspend_saved_subscription_execution(&source);
                    self.set_subscription_maintenance_status(&source.source_id, status, None)?;
                } else {
                    self.clear_subscription_maintenance_status(&source.source_id)?;
                }
                continue;
            }
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
                        match self.restore_committed_subscription_execution(&source) {
                            Ok(true) => {
                                self.clear_subscription_maintenance_status(&source.source_id)?
                            }
                            // A newer save already won; this old scan has no lifecycle effect.
                            Ok(false) => {}
                            Err(error) => {
                                self.suspend_saved_subscription_execution(&source);
                                self.set_subscription_maintenance_status(
                                    &source.source_id,
                                    lifecycle_failure_presentation(&error),
                                    None,
                                )?;
                            }
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
            self.suspend_saved_subscription_execution(&source);
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
        housekeeping
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
            .and_then(|runtimes| runtimes.for_connector(kind.connector_id()))
        {
            runtime.suspend_subscription_execution();
        }
    }

    fn suspend_saved_subscription_execution(&self, source: &ComputeManagementSourceV2) {
        if let Some(runtimes) = &self.cpa_runtime {
            runtimes.suspend_saved_source(
                &source.source_id,
                &source.last_candidate_ref,
                source.revision,
            );
        }
    }

    fn restore_committed_subscription_execution(
        &self,
        source: &ComputeManagementSourceV2,
    ) -> Result<bool, hiroute_cpa_bridge::CpaLifecycleError> {
        match self.project_saved_subscription_source(source) {
            Ok(()) => Ok(true),
            Err(hiroute_cpa_bridge::CpaLifecycleError::StaleSourceManagement) => Ok(false),
            Err(error) => Err(error),
        }
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
                        matches!(
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
            return Err(check_failure(checked.status));
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
            let runtimes = self.cpa_runtime.clone();
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
                    let observed = super::source::source_for_candidate(
                        &scanner,
                        runtimes.as_deref(),
                        &expected_candidate_ref,
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

    pub(super) fn set_subscription_maintenance_status(
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

    pub(super) fn clear_subscription_maintenance_status(
        &self,
        source_id: &str,
    ) -> Result<(), String> {
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
    Err(check_failure(super::subscription_operation_status(
        operation,
    )))
}

fn check_failure(status: ComputeSubscriptionCheckStatusV2) -> MaintenanceFailure {
    if status == ComputeSubscriptionCheckStatusV2::NeedsAuth {
        MaintenanceFailure::AuthenticationRequired
    } else {
        MaintenanceFailure::RuntimeUnavailable
    }
}

fn lifecycle_failure_presentation(
    error: &hiroute_cpa_bridge::CpaLifecycleError,
) -> SubscriptionMaintenancePresentation {
    use hiroute_cpa_bridge::{CpaSubscriptionAvailability, cpa_subscription_availability};
    if cpa_subscription_availability(Err(error)) == CpaSubscriptionAvailability::NeedsAuthentication
    {
        SubscriptionMaintenancePresentation::AuthenticationRequired
    } else {
        SubscriptionMaintenancePresentation::RuntimeUnavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_credential_causes_keep_authentication_presentation_through_maintenance() {
        use hiroute_cpa_bridge::CpaLifecycleError as E;
        for error in [
            E::BorrowedCodexStoreUnsupported,
            E::BorrowedCodexLoginUnsupported,
            E::BorrowedCodexAccountMissing,
            E::ManagedOAuthAuthenticationRequired,
        ] {
            assert_eq!(
                lifecycle_failure_presentation(&error),
                SubscriptionMaintenancePresentation::AuthenticationRequired
            );
        }
        for code in [
            "SUBSCRIPTION_NEEDS_AUTH",
            "SUBSCRIPTION_NATIVE_STORE_UNSUPPORTED",
            "SUBSCRIPTION_NATIVE_LOGIN_UNSUPPORTED",
            "SUBSCRIPTION_NATIVE_ACCOUNT_MISSING",
            "SUBSCRIPTION_MANAGED_LOGIN_REQUIRED",
        ] {
            assert_eq!(
                check_failure(super::super::subscription_error_status(Some(code))).presentation(),
                SubscriptionMaintenancePresentation::AuthenticationRequired
            );
        }
        assert_eq!(
            check_failure(super::super::subscription_error_status(Some(
                "SUBSCRIPTION_NATIVE_READ_FAILED"
            )))
            .presentation(),
            SubscriptionMaintenancePresentation::RuntimeUnavailable
        );
    }

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
