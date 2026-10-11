use super::{Session, query, terminal};
use crate::confirmation::ConfirmationPermit;
use crate::failure::DesktopFailure;
use crate::operation_observation::{IntentEvidence, SubmittedOperation};
use hiroute_application_api::*;
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::{Arc, atomic::AtomicBool};

/// Backend-only approval context bound to the explicit check action and its exact preview.
pub struct SubscriptionCheckConfirmation {
    permit: ConfirmationPermit,
    request: ComputeConnectionApplyRequestV1,
    intent: IntentEvidence,
    language: String,
    message: String,
    close_requested: Arc<AtomicBool>,
}

impl SubscriptionCheckConfirmation {
    pub fn revision(&self) -> u64 {
        self.request.expected_revisions.target
    }
}

/// An approved A request whose protected call can run without monopolizing the backend Session.
/// The only shared state is a close signal; no WebView can construct this value.
#[cfg(any(feature = "desktop-runtime", test))]
pub(crate) struct PendingSubscriptionCheck {
    client: hiroute_client_core::Client,
    request_id: String,
    request: ComputeConnectionApplyRequestV1,
    capability: zeroize::Zeroizing<String>,
    close_requested: Arc<AtomicBool>,
}

#[cfg(any(feature = "desktop-runtime", test))]
pub(crate) struct SubscriptionCheckCompletion {
    idempotency_key: String,
    accepted_digest: CanonicalDigest,
    outcome: SubscriptionCheckSubmission,
    close_requested: Arc<AtomicBool>,
}

#[cfg(any(feature = "desktop-runtime", test))]
enum SubscriptionCheckSubmission {
    ClosedBeforeSend,
    Submitted(Box<Result<MachineEnvelopeV2<ApplyResultV1>, hiroute_client_core::ClientFailure>>),
}

#[cfg(any(feature = "desktop-runtime", test))]
impl PendingSubscriptionCheck {
    /// There is deliberately no await between the close check and entering Client Core. Once the
    /// protected call starts, close follows the normal Operation cancellation/observation policy.
    pub(crate) async fn execute(self) -> SubscriptionCheckCompletion {
        let idempotency_key = self.request.idempotency_key.clone();
        let accepted_digest = self.request.accept_digest.clone();
        let outcome = if self.close_requested.load(Ordering::SeqCst) {
            SubscriptionCheckSubmission::ClosedBeforeSend
        } else {
            SubscriptionCheckSubmission::Submitted(Box::new(
                self.client
                    .apply_subscription_check(
                        &self.request_id,
                        self.request,
                        ProtectedClientGrantV2 {
                            principal_kind: PrincipalKind::Desktop,
                            capability: self.capability.to_string(),
                        },
                    )
                    .await,
            ))
        };
        SubscriptionCheckCompletion {
            idempotency_key,
            accepted_digest,
            outcome,
            close_requested: self.close_requested,
        }
    }
}

impl SubscriptionCheckConfirmation {
    pub fn requires_confirmation(&self) -> bool {
        false
    }

    pub fn message(&self) -> String {
        self.message.clone()
    }

    pub fn english(&self) -> bool {
        self.language == "en"
    }
}

impl Session {
    pub fn prepare_subscription_check_confirmation(
        &mut self,
        preview: ComputeSubscriptionCheckPreviewV2,
        language: String,
        idempotency_key: String,
        close_requested: Arc<AtomicBool>,
    ) -> Result<SubscriptionCheckConfirmation, DesktopFailure> {
        if close_requested.load(Ordering::SeqCst) {
            return Err("SUBSCRIPTION_CHECK_CLOSED".into());
        }
        if !matches!(language.as_str(), "zh" | "en") || idempotency_key.len() != 64 {
            return Err("REQUEST_INVALID".into());
        }
        let mut request = ComputeConnectionApplyRequestV1 {
            spec: preview.spec,
            accept_digest: preview.accept_digest,
            expected_revisions: preview.expected_revisions,
            idempotency_key,
        };
        let intent = IntentEvidence::subscription_check(&request);
        if let Some(existing) = self.subscription_hint.as_ref()
            && existing.operation_kind == APPLY_SUBSCRIPTION_CHECK_OPERATION_V2
            && existing.accepted_digest == request.accept_digest
            && existing.intent.as_ref() == Some(&intent)
        {
            // A retry after an unknown response must use the original identity. The protected
            // capability is still freshly issued for this exact accepted digest below.
            request
                .idempotency_key
                .clone_from(&existing.idempotency_key);
        }
        let message = if language == "en" {
            format!(
                "Check {}\n\nAllow HiRoute to use this subscription sign-in to connect and read the available models? Models are added only after you select and save them.",
                preview.display_scope
            )
        } else {
            format!(
                "检查 {}\n\n允许 HiRoute 使用这项订阅登录连接服务并读取可用模型？选择并保存后才会接入模型。",
                preview.display_scope
            )
        };
        Ok(SubscriptionCheckConfirmation {
            permit: self.confirmation.begin()?,
            request,
            intent,
            language,
            message,
            close_requested,
        })
    }

    /// Consumes native approval and retains the in-memory request identity before returning a protected
    /// call that the bridge must execute without holding the Session mutex.
    #[cfg(any(feature = "desktop-runtime", test))]
    pub(crate) async fn begin_subscription_check_confirmation(
        &mut self,
        context: SubscriptionCheckConfirmation,
        accepted: bool,
    ) -> Result<PendingSubscriptionCheck, DesktopFailure> {
        if !self.confirmation.finish(&context.permit, accepted)? {
            return Err("SUBSCRIPTION_CHECK_CANCELLED".into());
        }
        if context.close_requested.load(Ordering::SeqCst) {
            return Err("SUBSCRIPTION_CHECK_CLOSED".into());
        }
        if let Some(existing) = self.subscription_hint.as_ref() {
            let same_intent = existing.operation_kind == APPLY_SUBSCRIPTION_CHECK_OPERATION_V2
                && existing.idempotency_key == context.request.idempotency_key
                && existing.accepted_digest == context.request.accept_digest
                && existing.intent.as_ref() == Some(&context.intent);
            if !same_intent {
                let prior_kind = existing.operation_kind.clone();
                let prior = self.find_subscription().await?;
                if let Some(operation) = prior.as_ref()
                    && !terminal(&operation.state)
                {
                    return Err("SUBSCRIPTION_CHECK_ACTIVE".into());
                }
                if let Some(operation) = prior
                    && prior_kind == APPLY_SUBSCRIPTION_CHECK_OPERATION_V2
                {
                    let result = self
                        .subscription_result_for_operation(&operation.operation_id)
                        .await?;
                    if matches!(
                        result.status,
                        ComputeSubscriptionCheckStatusV2::Verified
                            | ComputeSubscriptionCheckStatusV2::Retained
                    ) {
                        return Err("SUBSCRIPTION_CHECK_ACTIVE".into());
                    }
                }

                self.subscription_hint = None;
            }
        }
        let hint = SubmittedOperation {
            plan_id: context
                .request
                .spec
                .resource_id
                .clone()
                .unwrap_or_else(|| "compute-subscription".into()),
            principal_kind: PrincipalKind::Desktop,
            operation_kind: APPLY_SUBSCRIPTION_CHECK_OPERATION_V2.into(),
            idempotency_key: context.request.idempotency_key.clone(),
            accepted_digest: context.request.accept_digest.clone(),
            operation_id: None,
            after_sequence: 0,
            intent: Some(context.intent),
            latest_edit_not_applied: false,
        };

        self.subscription_hint = Some(hint);
        #[cfg(unix)]
        let capability = self.resident.register_subscription_check(
            &context.request.accept_digest,
            &context.request.expected_revisions,
        )?;
        #[cfg(not(unix))]
        let capability: zeroize::Zeroizing<String> =
            return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
        Ok(PendingSubscriptionCheck {
            client: self.client.clone(),
            request_id: crate::random_id()?,
            request: context.request,
            capability,
            close_requested: context.close_requested,
        })
    }

    /// Reconciles the exact A identity after the protected request completes. If the page closed
    /// while A was running, apply the close policy before returning control to the WebView.
    #[cfg(any(feature = "desktop-runtime", test))]
    pub(crate) async fn finish_subscription_check_confirmation(
        &mut self,
        completion: SubscriptionCheckCompletion,
    ) -> Result<ComputeSubscriptionCheckResultV2, DesktopFailure> {
        let owns_hint = self.subscription_hint.as_ref().is_some_and(|hint| {
            hint.operation_kind == APPLY_SUBSCRIPTION_CHECK_OPERATION_V2
                && hint.idempotency_key == completion.idempotency_key
                && hint.accepted_digest == completion.accepted_digest
        });
        let response = match completion.outcome {
            SubscriptionCheckSubmission::ClosedBeforeSend => {
                if owns_hint {
                    self.subscription_hint = None;
                }
                return Err("SUBSCRIPTION_CHECK_CLOSED".into());
            }
            SubscriptionCheckSubmission::Submitted(response) => *response,
        };
        if let Ok(envelope) = &response
            && let Some(operation) = &envelope.operation
            && owns_hint
        {
            let hint = self
                .subscription_hint
                .as_mut()
                .expect("matched subscription hint");
            hint.operation_id = Some(operation.operation_id.clone());
            hint.after_sequence = operation.sequence;
        }
        let recovered = if owns_hint {
            self.find_subscription().await?
        } else {
            None
        };
        let operation = match response {
            Ok(envelope) if envelope.error.is_none() => envelope
                .operation
                .or_else(|| recovered.as_ref().map(operation_reference))
                .ok_or("RESPONSE_OPERATION_MISSING")?,
            Ok(envelope) => {
                if let Some(operation) = recovered.as_ref() {
                    operation_reference(operation)
                } else {
                    if owns_hint
                        && envelope.error.as_ref().is_some_and(|error| {
                            matches!(
                                error.code,
                                ErrorCode::RevisionConflict
                                    | ErrorCode::ChangePreviewStale
                                    | ErrorCode::CapabilityDenied
                                    | ErrorCode::InvalidArguments
                                    | ErrorCode::ActionRequired
                            )
                        })
                    {
                        self.subscription_hint = None;
                    }
                    return Err(DesktopFailure::backend(erase_envelope(envelope)?));
                }
            }
            Err(failure) => {
                let Some(operation) = recovered.as_ref() else {
                    return Err(failure.into());
                };
                operation_reference(operation)
            }
        };
        let result = self
            .subscription_result_for_operation(&operation.operation_id)
            .await?;
        self.finish_subscription_hint(&result)?;
        self.resume_observing();
        if completion.close_requested.load(Ordering::SeqCst) {
            self.apply_subscription_close_policy(&result).await?;
        }
        Ok(result)
    }

    /// Observes an accepted check from the current interaction without another confirmation or
    /// another Apply. It is read-only and returns `None` for unrelated pending operations.
    pub async fn recover_subscription_check(
        &mut self,
    ) -> Result<Option<ComputeSubscriptionCheckResultV2>, DesktopFailure> {
        self.resume_subscription_interaction();
        self.observe_subscription_check().await
    }

    pub fn resume_subscription_interaction(&mut self) -> Arc<AtomicBool> {
        if self.subscription_close_requested.load(Ordering::SeqCst) {
            self.subscription_close_requested = Arc::new(AtomicBool::new(false));
        }
        self.subscription_close_requested.clone()
    }

    async fn observe_subscription_check(
        &mut self,
    ) -> Result<Option<ComputeSubscriptionCheckResultV2>, DesktopFailure> {
        if !self
            .subscription_hint
            .as_ref()
            .is_some_and(|hint| hint.operation_kind == APPLY_SUBSCRIPTION_CHECK_OPERATION_V2)
        {
            return Ok(None);
        }
        let Some(operation) = self.find_subscription().await? else {
            return Ok(None);
        };
        let result = self
            .subscription_result_for_operation(&operation.operation_id)
            .await?;
        self.finish_subscription_hint(&result)?;
        Ok(Some(result))
    }

    pub async fn release_subscription_validation(
        &mut self,
        validation: ComputeValidationRefV2,
    ) -> Result<ComputeSubscriptionCheckResultV2, DesktopFailure> {
        validation.validate_shape().map_err(|_| "REQUEST_INVALID")?;
        if self
            .subscription_hint
            .as_ref()
            .is_some_and(|hint| hint.operation_kind == APPLY_SUBSCRIPTION_CHECK_OPERATION_V2)
        {
            let _ = self.find_subscription().await?;
        }
        let status: ClientServiceStatusV1 = query(
            &self.client,
            "GetClientServiceStatus",
            &ClientEmptyRequestV1 {},
        )
        .await?;
        let digest = CanonicalDigest::of(&validation).map_err(|_| "REQUEST_INVALID")?;
        #[cfg(unix)]
        let capability = self
            .resident
            .register_subscription_release(&digest, &status.revisions)?;
        #[cfg(not(unix))]
        let capability: zeroize::Zeroizing<String> =
            return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
        let result = envelope_data(
            self.client
                .release_subscription_check(
                    &crate::random_id()?,
                    validation,
                    ProtectedClientGrantV2 {
                        principal_kind: PrincipalKind::Desktop,
                        capability: capability.to_string(),
                    },
                )
                .await?,
        )?;
        self.finish_subscription_hint(&result)?;
        Ok(result)
    }

    /// Applies the domain close policy from native state. A verified resource is released, a
    /// running A receives a cancellation request and remains observable, and a handed-off B is
    /// only observed.
    pub async fn close_subscription_check(&mut self) -> Result<(), DesktopFailure> {
        self.subscription_close_requested
            .store(true, Ordering::SeqCst);
        if let Some(result) = self.observe_subscription_check().await? {
            self.apply_subscription_close_policy(&result).await?;
            return Ok(());
        }

        // A terminal verified check may outlive a different, already-terminal Desktop hint. The
        // daemon only rehydrates validations that remain owned by A (never B-owned/retained ones),
        // so this metadata-only list is also a safe close-time ownership sweep.
        let candidates = envelope_data(
            self.client
                .compute_subscriptions(&crate::random_id()?)
                .await?,
        )?;
        for candidate in candidates.candidates {
            if let Some(validation) = candidate.validation {
                self.release_subscription_validation(validation).await?;
            }
        }
        Ok(())
    }

    async fn apply_subscription_close_policy(
        &mut self,
        result: &ComputeSubscriptionCheckResultV2,
    ) -> Result<(), DesktopFailure> {
        if result.save_operation.is_some()
            || result.status == ComputeSubscriptionCheckStatusV2::Retained
        {
            return Ok(());
        }
        if result.status == ComputeSubscriptionCheckStatusV2::Checking {
            let operation_id = &result.approval_operation.operation_id;
            let request = match self.subscription_cancel_request.as_ref() {
                Some(request) if &request.operation_id == operation_id => request.clone(),
                _ => {
                    let request = OperationCancelRequestV1 {
                        operation_id: operation_id.clone(),
                        idempotency_key: crate::random_id()?,
                    };
                    self.subscription_cancel_request = Some(request.clone());
                    request
                }
            };
            self.request_subscription_cancel(request).await?;
        } else if let Some(validation) = result.validation.clone() {
            // A superseded checked candidate is reported as source-changed, but A still owns its
            // durable validation until B is admitted. Closing must release that exact resource.
            self.release_subscription_validation(validation).await?;
        }
        Ok(())
    }

    async fn request_subscription_cancel(
        &mut self,
        request: OperationCancelRequestV1,
    ) -> Result<(), DesktopFailure> {
        let status: ClientServiceStatusV1 = query(
            &self.client,
            "GetClientServiceStatus",
            &ClientEmptyRequestV1 {},
        )
        .await?;
        let digest = CanonicalDigest::of(&request).map_err(|_| "REQUEST_INVALID")?;
        #[cfg(unix)]
        let capability = self
            .resident
            .register_operation_cancel(&digest, &status.revisions)?;
        #[cfg(not(unix))]
        let capability: zeroize::Zeroizing<String> =
            return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
        let _ = envelope_data(
            self.client
                .cancel_subscription_check(
                    &crate::random_id()?,
                    request,
                    ProtectedClientGrantV2 {
                        principal_kind: PrincipalKind::Desktop,
                        capability: capability.to_string(),
                    },
                )
                .await?,
        )?;
        Ok(())
    }

    async fn subscription_result_for_operation(
        &self,
        operation_id: &str,
    ) -> Result<ComputeSubscriptionCheckResultV2, DesktopFailure> {
        envelope_data(
            self.client
                .subscription_check_result(&crate::random_id()?, operation_id.to_owned())
                .await?,
        )
    }

    fn finish_subscription_hint(
        &mut self,
        result: &ComputeSubscriptionCheckResultV2,
    ) -> Result<(), DesktopFailure> {
        if !matches!(
            result.status,
            ComputeSubscriptionCheckStatusV2::SourceChanged
                | ComputeSubscriptionCheckStatusV2::NeedsAuth
                | ComputeSubscriptionCheckStatusV2::Unavailable
                | ComputeSubscriptionCheckStatusV2::Failed
                | ComputeSubscriptionCheckStatusV2::Released
        ) {
            return Ok(());
        }
        let owns_hint = self.subscription_hint.as_ref().is_some_and(|hint| {
            hint.operation_kind == APPLY_SUBSCRIPTION_CHECK_OPERATION_V2
                && hint.operation_id.as_deref()
                    == Some(result.approval_operation.operation_id.as_str())
        });
        if owns_hint {
            self.subscription_hint = None;
            self.subscription_cancel_request = None;
        }
        Ok(())
    }
}

fn operation_reference(operation: &ClientOperationViewV1) -> OperationReferenceV1 {
    OperationReferenceV1 {
        operation_id: operation.operation_id.clone(),
        state: operation.state.clone(),
        sequence: operation.sequence,
        cancellable: operation.cancellable,
    }
}

fn erase_envelope<T: Serialize>(
    envelope: MachineEnvelopeV2<T>,
) -> Result<MachineEnvelopeV2<serde_json::Value>, DesktopFailure> {
    Ok(MachineEnvelopeV2 {
        schema_version: envelope.schema_version,
        request_id: envelope.request_id,
        status: envelope.status,
        data: envelope
            .data
            .map(serde_json::to_value)
            .transpose()
            .map_err(|_| "RESPONSE_DATA_INVALID")?,
        operation: envelope.operation,
        warnings: envelope.warnings,
        next_actions: envelope.next_actions,
        error: envelope.error,
    })
}

fn envelope_data<T: Serialize>(envelope: MachineEnvelopeV2<T>) -> Result<T, DesktopFailure> {
    if envelope.error.is_some() {
        return Err(DesktopFailure::backend(erase_envelope(envelope)?));
    }
    envelope.data.ok_or_else(|| "RESPONSE_DATA_MISSING".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::confirmation::ConfirmationGate;
    use hiroute_client_core::{Client, LocalEndpoint};

    fn confirmation(gate: &mut ConfirmationGate) -> SubscriptionCheckConfirmation {
        let request = ComputeConnectionApplyRequestV1 {
            spec: ChangeSpecV1 {
                schema_version: SchemaVersion::new(1, 0),
                command_id: "compute.subscription.check.apply".into(),
                resource_id: Some("candidate/one".into()),
                desired_state: serde_json::json!({"candidate":"candidate/one"}),
            },
            accept_digest: CanonicalDigest::of_bytes(b"accepted"),
            expected_revisions: RevisionSetV1 {
                target: 1,
                dependencies: Default::default(),
            },
            idempotency_key: "a".repeat(64),
        };
        SubscriptionCheckConfirmation {
            permit: gate.begin().unwrap(),
            intent: IntentEvidence::subscription_check(&request),
            request,
            language: "zh".into(),
            message: "check".into(),
            close_requested: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn explicit_check_needs_no_second_prompt_but_keeps_single_use_gate() {
        let mut gate = ConfirmationGate::default();
        let context = confirmation(&mut gate);
        assert!(!context.requires_confirmation());
        assert!(gate.finish(&context.permit, true).unwrap());
        assert_eq!(
            gate.finish(&context.permit, true),
            Err("CONFIRMATION_STALE")
        );
        assert_eq!(context.request.expected_revisions.target, 1);
        assert_eq!(
            context.request.accept_digest,
            CanonicalDigest::of_bytes(b"accepted")
        );
    }

    #[test]
    fn closing_the_window_still_invalidates_a_prompt_free_check() {
        let mut gate = ConfirmationGate::default();
        let old = confirmation(&mut gate);
        gate.invalidate();
        let current = confirmation(&mut gate);
        assert_eq!(gate.finish(&old.permit, true), Err("CONFIRMATION_STALE"));
        assert!(gate.finish(&current.permit, true).unwrap());
    }

    #[tokio::test]
    async fn close_signal_before_submission_prevents_the_protected_call() {
        let root = tempfile::tempdir().unwrap();
        let close_requested = Arc::new(AtomicBool::new(false));
        let pending = PendingSubscriptionCheck {
            client: Client::new(
                "desktop-test",
                LocalEndpoint::from_runtime_root(root.path()),
            ),
            request_id: "request-a".into(),
            request: ComputeConnectionApplyRequestV1 {
                spec: ChangeSpecV1 {
                    schema_version: SchemaVersion::new(1, 0),
                    command_id: "compute.subscription.check.apply".into(),
                    resource_id: Some("candidate/one".into()),
                    desired_state: serde_json::json!({"candidate":"candidate/one"}),
                },
                accept_digest: CanonicalDigest::of_bytes(b"accepted"),
                expected_revisions: RevisionSetV1 {
                    target: 1,
                    dependencies: Default::default(),
                },
                idempotency_key: "a".repeat(64),
            },
            capability: zeroize::Zeroizing::new("not-sent".into()),
            close_requested: close_requested.clone(),
        };

        close_requested.store(true, Ordering::SeqCst);
        let completion = pending.execute().await;

        assert!(matches!(
            completion.outcome,
            SubscriptionCheckSubmission::ClosedBeforeSend
        ));
    }
}
