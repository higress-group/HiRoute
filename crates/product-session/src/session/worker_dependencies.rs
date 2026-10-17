//! Confirmed, recoverable selection of one instance-level Worker installation.

use super::*;

impl Session {
    pub async fn select_worker_dependencies(
        &mut self,
        request: WorkerDependenciesSelectRequestV1,
    ) -> Result<MachineEnvelopeV2<WorkerDependenciesViewV1>, DesktopFailure> {
        let plan = plan_worker_dependency_selection(&request).ok_or("REQUEST_INVALID")?;
        let intent = IntentEvidence::worker_dependency_selection(&request);

        if self.hint.as_ref().is_some_and(|hint| {
            hint.operation_kind == WORKER_DEPENDENCIES_SELECT_OPERATION_V1
                && hint.idempotency_key == plan.idempotency_key
                && hint.accepted_digest == plan.accept_digest
                && hint.intent.as_ref() == Some(&intent)
        }) && let Some(operation) = self.find_pending().await?
        {
            return self
                .recovered_worker_dependency_selection(&request, operation)
                .await;
        }

        let hint = SubmittedOperation {
            plan_id: plan
                .spec
                .resource_id
                .clone()
                .unwrap_or_else(|| "worker-dependency-selection".into()),
            principal_kind: PrincipalKind::InteractiveUser,
            operation_kind: WORKER_DEPENDENCIES_SELECT_OPERATION_V1.into(),
            idempotency_key: plan.idempotency_key.clone(),
            accepted_digest: plan.accept_digest.clone(),
            operation_id: None,
            after_sequence: 0,
            intent: Some(intent),
            latest_edit_not_applied: false,
        };

        self.hint = Some(hint);

        let response = self
            .client
            .select_worker_dependencies(&crate::random_id()?, &request)
            .await;
        if let Ok(envelope) = &response
            && let Some(operation) = &envelope.operation
            && let Some(hint) = &mut self.hint
        {
            hint.operation_id = Some(operation.operation_id.clone());
            hint.after_sequence = operation.sequence;
        }

        match response {
            Ok(envelope) => {
                if envelope.error.as_ref().is_some_and(|error| {
                    matches!(
                        error.code,
                        ErrorCode::RevisionConflict
                            | ErrorCode::ChangePreviewStale
                            | ErrorCode::CapabilityDenied
                            | ErrorCode::InvalidArguments
                            | ErrorCode::IdempotencyKeyReused
                    )
                }) {
                    self.hint = None;
                } else if envelope.error.is_none()
                    && (envelope.operation.is_none()
                        || envelope.data.as_ref().is_none_or(|view| !view.valid()))
                {
                    return Err("RESPONSE_DATA_INVALID".into());
                }
                self.resume_observing();
                Ok(envelope)
            }
            Err(failure) => match self.find_pending().await? {
                Some(operation) => {
                    self.recovered_worker_dependency_selection(&request, operation)
                        .await
                }
                None => Err(failure.into()),
            },
        }
    }

    async fn recovered_worker_dependency_selection(
        &self,
        request: &WorkerDependenciesSelectRequestV1,
        operation: ClientOperationViewV1,
    ) -> Result<MachineEnvelopeV2<WorkerDependenciesViewV1>, DesktopFailure> {
        let mut envelope = self
            .client
            .worker_dependencies_discover(
                &crate::random_id()?,
                &WorkerDependenciesDiscoverRequestV1 {
                    harness: Some(request.harness),
                },
            )
            .await?;
        if envelope.error.is_none()
            && envelope
                .data
                .as_ref()
                .is_some_and(WorkerDependenciesViewV1::valid)
        {
            envelope.status = MachineStatus::Accepted;
            envelope.operation = Some(OperationReferenceV1 {
                operation_id: operation.operation_id,
                state: operation.state,
                sequence: operation.sequence,
                cancellable: operation.cancellable,
            });
            return Ok(envelope);
        }
        if envelope.error.is_some() {
            return Ok(envelope);
        }
        Err("RESPONSE_DATA_INVALID".into())
    }
}
