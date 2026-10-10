//! Same-user OAuth lifecycle. Only protected input carries authorization codes.
use hiroute_application::TransactionError;
use hiroute_application::control::{ComputeManagementControlError, ComputeManagementControlPort};
use hiroute_application_api::{
    COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2, ComputeCandidateRefV2, ComputeConnectionApplyRequestV1,
    ComputeManagementChangeV2, ComputeManagementIntentV2, ComputeManagementSubjectV2,
    ComputeSubscriptionLoginRequestV1, ComputeSubscriptionLoginResultV1,
    ComputeSubscriptionLoginSessionV1, SUBSCRIPTION_LOGIN_RESULT_SCHEMA_V1,
    SubscriptionLoginProviderV1, SubscriptionLoginStatusV1,
};
use hiroute_cpa_bridge::{CpaAccountKind, CpaLifecycleError, CpaLoginSession, CpaLoginState};
use hiroute_domain::{
    ComputeManagementMutationV2, ComputeManagementProvenanceV2, ComputeManagementRepositoryPort,
    ControlRepositoryPort, MaterializationState, OperationState, OperationV1, PortError,
    PortErrorCode, PortResult, WorkspaceId,
};
use std::sync::MutexGuard;

use super::super::LocalControlAdapter;

/// Proof that this adapter's lifecycle lock remains held across admission, effects and runtime
/// projection. Only the lock method can construct it; Forget uses the same proof for its normal
/// disable transaction without trying to recursively acquire the mutex.
pub(in crate::control::runtime) struct SubscriptionLifecycleGuard<'a> {
    owner: &'a LocalControlAdapter,
    _lock: MutexGuard<'a, ()>,
}

impl SubscriptionLifecycleGuard<'_> {
    pub(in crate::control::runtime) fn owns(&self, adapter: &LocalControlAdapter) -> bool {
        std::ptr::eq(self.owner, adapter)
    }
}

impl LocalControlAdapter {
    pub(in crate::control::runtime) fn lock_subscription_lifecycle(
        &self,
    ) -> Result<SubscriptionLifecycleGuard<'_>, TransactionError> {
        Ok(SubscriptionLifecycleGuard {
            owner: self,
            _lock: self
                .subscription_lifecycle
                .lock()
                .map_err(|_| TransactionError::RecoveryRequired)?,
        })
    }

    /// Called by both journal admission ports while the coordinator holds its writer. Inspect
    /// the sealed final source, so Candidate, saved-source re-enable, retained models, generic
    /// Apply and maintenance all enforce the same credential lifetime. This is memory-only:
    /// no CPA start, refresh or provider request may execute inside the writer.
    pub(in crate::control::runtime) fn guard_managed_subscription_admission(
        &self,
        operation: &OperationV1,
    ) -> PortResult<()> {
        let declares_management = operation
            .plan
            .spec()
            .desired_state
            .get("schema")
            .and_then(serde_json::Value::as_str)
            == Some(COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2);
        if operation
            .plan
            .control()
            .get("compute_management_mutation")
            .is_none()
        {
            return if declares_management {
                Err(PortError::new(
                    PortErrorCode::InvalidData,
                    "subscription.admission.invalid_plan",
                ))
            } else {
                Ok(())
            };
        }
        if !declares_management || operation.plan.spec().command_id != "compute.connection.apply" {
            return Err(PortError::new(
                PortErrorCode::InvalidData,
                "subscription.admission.invalid_plan",
            ));
        }
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Envelope {
            compute_management_mutation: ComputeManagementMutationV2,
        }
        let envelope: Envelope =
            serde_json::from_value(operation.plan.control().clone()).map_err(|_| {
                PortError::new(
                    PortErrorCode::InvalidData,
                    "subscription.admission.invalid_plan",
                )
            })?;
        envelope
            .compute_management_mutation
            .validate_shape(operation.plan.spec())
            .map_err(|_| {
                PortError::new(
                    PortErrorCode::InvalidData,
                    "subscription.admission.invalid_plan",
                )
            })?;
        let Some(source) = envelope.compute_management_mutation.desired() else {
            // Forgetting a saved connection does not require or revoke a connector login.
            return Ok(());
        };
        if operation
            .plan
            .spec()
            .desired_state
            .pointer("/subject/kind")
            .and_then(serde_json::Value::as_str)
            == Some("saved_source")
            && matches!(
                operation
                    .plan
                    .spec()
                    .desired_state
                    .pointer("/edit/action")
                    .and_then(serde_json::Value::as_str),
                Some("rename" | "remove_models")
            )
        {
            return Ok(());
        }
        if source.state != MaterializationState::Ready {
            return Ok(());
        }
        let managed = [CpaAccountKind::Codex, CpaAccountKind::Claude]
            .into_iter()
            .find_map(|kind| {
                let prefix = format!("candidate/cpa/{}/managed/", kind.stock_provider());
                source
                    .last_candidate_ref
                    .strip_prefix(&prefix)
                    .map(|id| (kind, id))
            });
        let Some((kind, login_ref)) = managed else {
            return Ok(());
        };
        let ComputeManagementProvenanceV2::ConnectorOwned {
            connector_id,
            account_ref,
        } = &source.provenance
        else {
            return Err(PortError::new(
                PortErrorCode::Conflict,
                "subscription.admission.login_changed",
            ));
        };
        let session = self
            .cpa_runtime
            .as_ref()
            .and_then(|runtimes| runtimes.login_session(login_ref));
        if connector_id != kind.connector_id()
            || session.as_ref().is_none_or(|session| {
                session.kind != kind
                    || session.state != CpaLoginState::Authorized
                    || session.account_ref.as_ref() != Some(account_ref)
                    || session.candidate_ref() != source.last_candidate_ref
            })
        {
            return Err(PortError::new(
                PortErrorCode::Conflict,
                "subscription.admission.login_changed",
            ));
        }
        Ok(())
    }

    pub(in crate::control::runtime) fn subscription_login(
        &self,
        request: ComputeSubscriptionLoginRequestV1,
    ) -> Result<ComputeSubscriptionLoginResultV1, ComputeManagementControlError> {
        if !request.valid() {
            return Err(ComputeManagementControlError::Invalid);
        }
        let runtimes = self
            .cpa_runtime
            .as_ref()
            .ok_or(ComputeManagementControlError::Unavailable)?;
        let mut authorization_url = None;
        let sessions = match request {
            ComputeSubscriptionLoginRequestV1::List { provider } => {
                runtimes.login_sessions(kind(provider))
            }
            ComputeSubscriptionLoginRequestV1::Start { provider } => {
                let (session, url) = runtimes.start_login(kind(provider)).map_err(login_error)?;
                authorization_url = Some(url);
                vec![session]
            }
            ComputeSubscriptionLoginRequestV1::Status { login_ref } => {
                vec![runtimes.login_status(&login_ref).map_err(login_error)?]
            }
            ComputeSubscriptionLoginRequestV1::Callback {
                login_ref,
                input_candidate,
            } => {
                let session = runtimes
                    .login_session(&login_ref)
                    .ok_or(ComputeManagementControlError::NotFound)?;
                if session.state != CpaLoginState::Pending
                    || input_candidate != callback_candidate(&login_ref)
                {
                    return Err(ComputeManagementControlError::Conflict);
                }
                let secret = self
                    .manual_protected_inputs
                    .lock()
                    .map_err(|_| ComputeManagementControlError::Unavailable)?
                    .remove(&input_candidate.candidate_ref)
                    .ok_or(ComputeManagementControlError::Invalid)?;
                let value = std::str::from_utf8(secret.expose())
                    .map_err(|_| ComputeManagementControlError::Invalid)?;
                if value.len() > 16_384 {
                    return Err(ComputeManagementControlError::Invalid);
                }
                runtimes
                    .submit_login_callback(&login_ref, value.trim())
                    .map_err(login_error)?;
                vec![runtimes.login_status(&login_ref).map_err(login_error)?]
            }
            ComputeSubscriptionLoginRequestV1::Cancel { login_ref } => {
                self.clear_login_input(&login_ref)?;
                vec![
                    runtimes
                        .cancel_login(&login_ref, false)
                        .map_err(login_error)?,
                ]
            }
            ComputeSubscriptionLoginRequestV1::Forget { login_ref } => {
                // Serializes the snapshot, normal disable transaction and withdrawal against
                // every control admission/execution. A save already in progress finishes first;
                // an older Prepared arriving later sees Forgotten at journal admission.
                let lifecycle = self
                    .lock_subscription_lifecycle()
                    .map_err(|_| ComputeManagementControlError::Unavailable)?;
                // A stalled admitted operation may still publish its sealed source during
                // recovery even though no Apply call is currently holding the lifecycle lock.
                // Keep its credential until the durable writer claim has been reconciled.
                if self
                    .stores_lock()
                    .map_err(|_| ComputeManagementControlError::Unavailable)?
                    .control()
                    .writer_recovery_required()
                    .map_err(super::error::map_port)?
                {
                    return Err(ComputeManagementControlError::Conflict);
                }
                let session = runtimes
                    .login_session(&login_ref)
                    .ok_or(ComputeManagementControlError::NotFound)?;
                // This is an explicit user command, so reuse normal save admission before
                // deleting the credential. Failure leaves the credential available for retry.
                self.disable_login_sources(&session, &lifecycle)?;
                self.clear_login_input(&login_ref)?;
                vec![
                    runtimes
                        .cancel_login(&login_ref, true)
                        .map_err(login_error)?,
                ]
            }
        };
        let views = if sessions
            .iter()
            .any(|session| session.state == CpaLoginState::Authorized)
        {
            self.refresh_subscription_candidates()?
        } else {
            Vec::new()
        };
        let sessions = sessions
            .into_iter()
            .map(|session| {
                let candidate = views
                    .iter()
                    .find(|view| view.candidate.candidate_ref == session.candidate_ref())
                    .map(|view| view.candidate.clone());
                let callback_input_candidate = (session.state == CpaLoginState::Pending)
                    .then(|| callback_candidate(&session.login_ref));
                ComputeSubscriptionLoginSessionV1 {
                    provider: provider(session.kind),
                    login_ref: session.login_ref,
                    status: match session.state {
                        CpaLoginState::Pending => SubscriptionLoginStatusV1::Pending,
                        CpaLoginState::Authorized => SubscriptionLoginStatusV1::Authorized,
                        CpaLoginState::Cancelled => SubscriptionLoginStatusV1::Cancelled,
                        CpaLoginState::Failed => SubscriptionLoginStatusV1::Failed,
                        CpaLoginState::Expired => SubscriptionLoginStatusV1::Expired,
                        CpaLoginState::Forgotten => SubscriptionLoginStatusV1::Forgotten,
                    },
                    authorization_url: authorization_url.take(),
                    callback_input_candidate,
                    account_ref: session.account_ref,
                    candidate,
                    reason_code: None,
                }
            })
            .collect();
        Ok(ComputeSubscriptionLoginResultV1 {
            schema: SUBSCRIPTION_LOGIN_RESULT_SCHEMA_V1.into(),
            sessions,
        })
    }

    fn clear_login_input(&self, id: &str) -> Result<(), ComputeManagementControlError> {
        self.manual_protected_inputs
            .lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?
            .remove(&callback_candidate(id).candidate_ref);
        Ok(())
    }

    fn disable_login_sources(
        &self,
        session: &CpaLoginSession,
        lifecycle: &SubscriptionLifecycleGuard<'_>,
    ) -> Result<(), ComputeManagementControlError> {
        let snapshot = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .map_err(super::error::map_port)?;
        for source in snapshot
            .sources
            .into_iter()
            .filter(|source| source.last_candidate_ref == session.candidate_ref())
        {
            if source.state == MaterializationState::Disabled {
                continue;
            }
            let validation = if let Some(saved) = &source.validation {
                let operation_id = hiroute_domain::OperationId::parse(&saved.approval_operation_id)
                    .map_err(|_| ComputeManagementControlError::Corrupt)?;
                let operation = self
                    .stores_lock()
                    .map_err(|_| ComputeManagementControlError::Unavailable)?
                    .control()
                    .load_operation(&operation_id)
                    .map_err(super::error::map_port)?
                    .ok_or(ComputeManagementControlError::Corrupt)?;
                self.subscription_check_result(&super::operation_reference(&operation))?
                    .validation
            } else {
                None
            };
            let revisions = self
                .stores_lock()
                .map_err(|_| ComputeManagementControlError::Unavailable)?
                .control()
                .current_revisions(&WorkspaceId::default())
                .map_err(super::error::map_port)?;
            let preview = self.preview_compute_save(ComputeManagementChangeV2 {
                edit: None,
                schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
                subject: ComputeManagementSubjectV2::SavedSource {
                    source_id: source.source_id.clone(),
                },
                expected_revisions: revisions,
                selected_model_refs: source
                    .models
                    .iter()
                    .map(|model| model.model_ref.clone())
                    .collect(),
                intent: ComputeManagementIntentV2::SaveDisabled,
                key_edits: Vec::new(),
                validation,
            })?;
            let prepared = self.prepare_compute_save(ComputeConnectionApplyRequestV1 {
                spec: preview.spec,
                accept_digest: preview.accept_digest,
                expected_revisions: preview.expected_revisions,
                idempotency_key: format!("forget-{}-{}", session.login_ref, source.revision),
            })?;
            let operation = self
                .apply_local_prepared_with_subscription_guard(prepared, lifecycle)
                .map_err(|_| ComputeManagementControlError::Conflict)?;
            if operation.state != OperationState::Succeeded {
                return Err(ComputeManagementControlError::Conflict);
            }
        }
        Ok(())
    }
}

pub(in crate::control::runtime) fn callback_candidate(id: &str) -> ComputeCandidateRefV2 {
    ComputeCandidateRefV2 {
        candidate_ref: format!("candidate/subscription-login/{id}"),
        candidate_revision: 1,
    }
}

fn kind(value: SubscriptionLoginProviderV1) -> CpaAccountKind {
    match value {
        SubscriptionLoginProviderV1::Codex => CpaAccountKind::Codex,
        SubscriptionLoginProviderV1::Claude => CpaAccountKind::Claude,
    }
}
fn provider(value: CpaAccountKind) -> SubscriptionLoginProviderV1 {
    match value {
        CpaAccountKind::Codex => SubscriptionLoginProviderV1::Codex,
        CpaAccountKind::Claude => SubscriptionLoginProviderV1::Claude,
    }
}
fn login_error(value: CpaLifecycleError) -> ComputeManagementControlError {
    match value {
        CpaLifecycleError::AlreadyOwned | CpaLifecycleError::StaleSourceManagement => {
            ComputeManagementControlError::Conflict
        }
        CpaLifecycleError::InvalidSourceManagement => ComputeManagementControlError::NotFound,
        _ => ComputeManagementControlError::Unavailable,
    }
}
