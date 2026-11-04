use super::*;
use rusqlite::TransactionBehavior;

pub(super) fn checkpoint(
    store: &RuntimeStore,
    workspace: &WorkspaceId,
    run_id: &str,
    revision: u64,
    event_id: &str,
    event: &DelegationCheckpointV1,
) -> Result<DelegationRunV1> {
    if !valid_delegation_id(event_id) {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let mut connection = store.connection.borrow_mut();
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let mut run = read_run(&tx, workspace, run_id)?.ok_or(DelegationErrorV1::Conflict)?;
    let encoded = encode(event)?;
    let old: Option<String> = tx
        .query_row(
            "SELECT event_json FROM delegation_run_events WHERE run_id=?1 AND event_id=?2",
            params![run_id, event_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    if let Some(old) = old {
        return if old == encoded {
            Ok(run)
        } else {
            Err(DelegationErrorV1::Conflict)
        };
    }
    if run.progress.revision != revision {
        return Err(DelegationErrorV1::Conflict);
    }
    // A run has one immutable process binding. Once its owned scope is proven stopped,
    // a later lookup of a released/lost handle cannot revoke that durable proof.
    if run
        .stop_evidence
        .is_some_and(|evidence| evidence.scope_stopped && !evidence.residual_unknown)
        && matches!(
            event,
            DelegationCheckpointV1::ProcessObserved { .. }
                | DelegationCheckpointV1::ProcessStopped { .. }
                | DelegationCheckpointV1::Progress {
                    event: RunEventV1::ProcessRunning
                        | RunEventV1::ProcessUnknown
                        | RunEventV1::RootProcessExited { .. }
                }
        )
    {
        return Err(DelegationErrorV1::Conflict);
    }
    if let DelegationCheckpointV1::ExecutionFailed { event, failure } = event {
        if !matches!(
            event,
            RunEventV1::LaunchFailedBeforeSpawn
                | RunEventV1::ConnectionLost
                | RunEventV1::PromptFailed
        ) || run.execution_failure.is_some()
            || !matches!(
                run.progress.state,
                RunStateV1::Accepted
                    | RunStateV1::Preparing
                    | RunStateV1::Running
                    | RunStateV1::Cancelling
            )
            || (run.progress.cancel_requested && failure.error != DelegationErrorV1::Cancelled)
        {
            return Err(DelegationErrorV1::Conflict);
        }
        run.execution_failure = Some(*failure);
    }
    match event {
        DelegationCheckpointV1::ExecutionFailed { event, .. }
        | DelegationCheckpointV1::Progress { event } => {
            if *event == RunEventV1::ResidualAcknowledged {
                return Err(DelegationErrorV1::PermissionDenied);
            }
            if *event == RunEventV1::PromptSendIntent && run.session.is_none() {
                return Err(DelegationErrorV1::Conflict);
            }
            run.progress.advance(*event)?;
            if run.progress.cancel_requested
                || !matches!(
                    run.progress.state,
                    RunStateV1::Accepted | RunStateV1::Preparing | RunStateV1::Running
                )
            {
                run.lease_revoked = true;
            }
        }
        DelegationCheckpointV1::ProcessObserved { observation } => {
            run.progress.advance(match observation {
                RunProcessObservationV1::Running => RunEventV1::ProcessRunning,
                RunProcessObservationV1::Exited { code } => RunEventV1::RootProcessExited {
                    success: *code == Some(0),
                },
                RunProcessObservationV1::Unknown => RunEventV1::ProcessUnknown,
            })?;
        }
        DelegationCheckpointV1::ProcessStopped { evidence } => {
            if !evidence.valid() {
                return Err(DelegationErrorV1::InvalidArguments);
            }
            run.stop_evidence = Some(*evidence);
            run.lease_revoked = true;
            run.progress
                .advance(if evidence.scope_stopped && !evidence.residual_unknown {
                    RunEventV1::ManagedScopeStopped {
                        success: matches!(
                            evidence.observation,
                            RunProcessObservationV1::Exited { code: Some(0) }
                        ),
                    }
                } else if evidence.observation == RunProcessObservationV1::Running {
                    RunEventV1::ProcessRunning
                } else {
                    RunEventV1::ProcessUnknown
                })?;
        }
        DelegationCheckpointV1::ResidualConfirmed {
            operation_id,
            actor_id,
        } => {
            if !valid_delegation_id(actor_id) || OperationId::parse(operation_id.as_str()).is_err()
            {
                return Err(DelegationErrorV1::InvalidArguments);
            }
            run.progress.advance(RunEventV1::ResidualAcknowledged)?;
            run.lease_revoked = true;
        }
        DelegationCheckpointV1::ProcessSpawned { binding } => {
            if run.progress.state != RunStateV1::Preparing
                || run.process.is_some()
                || binding.launch_nonce != run.launch_nonce
                || !valid_delegation_id(&binding.handle_id)
                || !valid_delegation_id(&binding.creation_identity)
            {
                return Err(DelegationErrorV1::Conflict);
            }
            run.process = Some(binding.clone());
            run.progress.advance(RunEventV1::ProcessRunning)?;
        }
        DelegationCheckpointV1::SessionBound { binding } => {
            if run.progress.state != RunStateV1::Preparing
                || run.process.is_none()
                || run.session.is_some()
                || !valid_delegation_id(&binding.acp_session_id)
                || binding
                    .native_session_id
                    .as_ref()
                    .is_some_and(|s| !valid_delegation_id(s))
            {
                return Err(DelegationErrorV1::Conflict);
            }
            let mut task =
                read_task(&tx, workspace, &run.task_id)?.ok_or(DelegationErrorV1::Conflict)?;
            if task.latest_run_id != run_id
                || (run.continued_from.is_some() && task.session.as_ref() != Some(binding))
            {
                return Err(DelegationErrorV1::ResumeUnavailable);
            }
            task.session = Some(binding.clone());
            run.session = Some(binding.clone());
            run.progress.revision = revision.checked_add(1).ok_or(DelegationErrorV1::Conflict)?;
            write_task(&tx, &task)?;
        }
        DelegationCheckpointV1::ResultRecorded { body, incomplete } => {
            if !matches!(
                run.progress.state,
                RunStateV1::Running | RunStateV1::Cancelling
            ) || run.result_body.is_some()
                || run.result_incomplete
                || body.as_ref().is_some_and(|reference| {
                    reference.validate().is_err() || reference.scope_run_id != run.run_id
                })
                || (body.is_none() && !*incomplete)
            {
                return Err(DelegationErrorV1::Conflict);
            }
            run.result_body = body.clone();
            run.result_incomplete = *incomplete;
        }
    }
    // Even an idempotent state transition consumes this checkpoint's CAS revision.
    if run.progress.revision == revision {
        run.progress.revision = revision.checked_add(1).ok_or(DelegationErrorV1::Conflict)?;
    }
    native::transition_checkpoint(&tx, &run, event)?;
    write_run(&tx, &run)?;
    tx.execute(
        "INSERT INTO delegation_run_events(run_id,event_id,event_json) VALUES(?1,?2,?3)",
        params![run_id, event_id, encoded],
    )
    .map_err(storage)?;
    tx.commit().map_err(storage)?;
    Ok(run)
}

pub(super) fn cancel(
    store: &RuntimeStore,
    workspace: &WorkspaceId,
    run_id: &str,
    operation: &OperationId,
    reason: &str,
) -> Result<DelegationCancelReceiptV1> {
    if !valid_delegation_id(reason) {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let mut connection = store.connection.borrow_mut();
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let receipt = cancel_in_transaction(&tx, workspace, run_id, operation, reason)?;
    tx.commit().map_err(storage)?;
    Ok(receipt)
}

pub(super) fn cancel_in_transaction(
    tx: &Connection,
    workspace: &WorkspaceId,
    run_id: &str,
    operation: &OperationId,
    reason: &str,
) -> Result<DelegationCancelReceiptV1> {
    let mut run = read_run(tx, workspace, run_id)?.ok_or(DelegationErrorV1::Conflict)?;
    let prior: Option<String> = tx.query_row("SELECT receipt_json FROM delegation_cancel_receipts WHERE run_id=?1 AND operation_id=?2",params![run_id,operation.as_str()],|r|r.get(0)).optional().map_err(storage)?;
    if let Some(prior) = prior {
        let receipt: DelegationCancelReceiptV1 = decode(prior)?;
        return if receipt.reason == reason {
            Ok(receipt)
        } else {
            Err(DelegationErrorV1::Conflict)
        };
    }
    run.lease_revoked = true;
    run.progress.advance(RunEventV1::CancelRequested)?;
    let receipt = DelegationCancelReceiptV1 {
        operation_id: operation.clone(),
        run_id: run_id.into(),
        reason: reason.into(),
        state_revision: run.progress.revision,
    };
    write_run(tx, &run)?;
    tx.execute(
        "INSERT INTO delegation_cancel_receipts(run_id,operation_id,receipt_json) VALUES(?1,?2,?3)",
        params![run_id, operation.as_str(), encode(&receipt)?],
    )
    .map_err(storage)?;
    Ok(receipt)
}

pub(super) fn set_resume(
    store: &RuntimeStore,
    workspace: &WorkspaceId,
    task_id: &str,
    latest: &str,
    until: u64,
    body_refs: &[DelegationBodyRefV1],
    native_history_paths: &[String],
) -> Result<()> {
    if body_refs.len() > 16
        || native_history_paths.is_empty()
        || native_history_paths.len() > 256
        || native_history_paths
            .iter()
            .any(|path| !valid_native_history_path(path))
        || native_history_paths
            .iter()
            .enumerate()
            .any(|(index, path)| native_history_paths[..index].contains(path))
        || body_refs
            .iter()
            .any(|reference| reference.validate().is_err())
        || body_refs.iter().enumerate().any(|(index, reference)| {
            body_refs[..index]
                .iter()
                .any(|prior| prior.opaque_id == reference.opaque_id)
        })
    {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let mut connection = store.connection.borrow_mut();
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let mut task = read_task(&tx, workspace, task_id)?.ok_or(DelegationErrorV1::Conflict)?;
    let run = read_run(&tx, workspace, latest)?.ok_or(DelegationErrorV1::Conflict)?;
    let continuation_released: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM delegation_continuation_releases
             WHERE workspace_id=?1 AND task_id=?2 AND latest_run_id=?3)",
            params![workspace.as_str(), task_id, latest],
            |row| row.get(0),
        )
        .map_err(storage)?;
    if task.latest_run_id != latest || run.task_id != task_id || continuation_released {
        return Err(DelegationErrorV1::Conflict);
    }
    if until != 0
        && (!run.progress.workspace_releasable() || run.progress.cleanup != RunCleanupV1::Complete)
    {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    task.resume_until_ms = until;
    task.required_body_ids = body_refs
        .iter()
        .map(|reference| reference.opaque_id.clone())
        .collect();
    task.body_refs = body_refs.to_vec();
    task.native_history_paths = native_history_paths.to_vec();
    native::register_resume_materials(&tx, workspace, task_id, latest)?;
    write_task(&tx, &task)?;
    tx.commit().map_err(storage)?;
    Ok(())
}
