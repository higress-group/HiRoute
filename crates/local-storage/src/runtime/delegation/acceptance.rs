use super::*;
use rusqlite::TransactionBehavior;

/// The application holds 13's shared admission gate and has checked current authority.
/// This transaction owns only runtime acceptance/idempotency/occupancy, not Plan authority.
pub(super) fn accept(
    store: &RuntimeStore,
    input: &DelegationAcceptanceV1,
) -> Result<DelegationRunV1> {
    let run = &input.run;
    let task = &input.task;
    validate(input)?;
    let mut connection = store.connection.borrow_mut();
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    if let Some(existing) = find_submission(
        &tx,
        &run.workspace_id,
        run.continued_from.is_some(),
        &run.idempotency_key,
    )? {
        return if existing.request_digest == run.request_digest {
            Ok(existing)
        } else {
            Err(DelegationErrorV1::Conflict)
        };
    }
    let current = read_task(&tx, &task.workspace_id, &task.task_id)?;
    let new_task = current.is_none();
    let mut task_record = task.clone();
    let mut title_lookup_key = input.title_lookup_key.clone();
    match (&current, &input.expected_latest_run_id) {
        (None, None)
            if run.ordinal == 1
                && run.continued_from.is_none()
                && task.session.is_none()
                && task.resume_until_ms == 0 => {}
        (Some(old), Some(expected)) => {
            if input.title_lookup_key.is_some() {
                return Err(DelegationErrorV1::InvalidArguments);
            }
            title_lookup_key = read_task_title_lookup_key(&tx, &task.workspace_id, &task.task_id)?;
            if old.title.is_some() != title_lookup_key.is_some() {
                return Err(DelegationErrorV1::StorageUnavailable);
            }
            let prior = read_run(&tx, &task.workspace_id, expected)?
                .ok_or(DelegationErrorV1::ResumeUnavailable)?;
            let appended_body = task
                .body_refs
                .starts_with(&old.body_refs)
                .then(|| &task.body_refs[old.body_refs.len()..]);
            if &old.latest_run_id != expected
                || run.continued_from.as_ref() != Some(expected)
                || old.plan != task.plan
                || old.workspace != task.workspace
                || old.session != task.session
                || old.parent_task_ref != task.parent_task_ref
                || old.created_at_ms != task.created_at_ms
                || old.latest_admission_sequence != task.latest_admission_sequence
                || old.title != task.title
                || old.resume_until_ms != task.resume_until_ms
                || old.native_history_paths != task.native_history_paths
                || appended_body
                    .is_none_or(|body| body.len() != 1 || body[0].scope_run_id != run.run_id)
                || old
                    .session
                    .as_ref()
                    .and_then(|s| s.native_session_id.as_ref())
                    .is_none()
                || old.resume_until_ms <= input.admitted_at_ms
                || prior.progress.cleanup != RunCleanupV1::Complete
                || !prior.progress.workspace_releasable()
                || prior.ordinal.checked_add(1) != Some(run.ordinal)
            {
                return Err(DelegationErrorV1::ResumeUnavailable);
            }
            // Only a monotonic, exactly-one input BodyRef append is accepted. A caller copy
            // cannot replace stored creation/session/retention/native-history metadata.
            task_record = task.clone();
        }
        _ => return Err(DelegationErrorV1::Conflict),
    }
    ensure_occupied_integrity(&tx)?;
    let same_task_active: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM delegation_runs \
             WHERE occupied=1 AND workspace_id=?1 AND task_id=?2)",
            params![run.workspace_id.as_str(), run.task_id],
            |row| row.get(0),
        )
        .map_err(storage)?;
    if same_task_active {
        return Err(DelegationErrorV1::Busy);
    }
    let settings = read_worker_concurrency_settings(&tx)?;
    let occupied_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM delegation_runs WHERE occupied=1",
            [],
            |row| row.get(0),
        )
        .map_err(storage)?;
    if occupied_count >= i64::from(settings.max_concurrent) {
        return Err(DelegationErrorV1::CapacityExceeded);
    }
    let mut accepted = run.clone();
    let sequence: u64 = tx
        .query_row(
            "SELECT sequence FROM delegation_sequence WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(storage)?;
    accepted.admission_sequence = sequence
        .checked_add(1)
        .filter(|s| *s <= i64::MAX as u64)
        .ok_or(DelegationErrorV1::StorageUnavailable)?;
    accepted.accepted_at_ms = Some(if run.ordinal == 1 && run.continued_from.is_none() {
        task.created_at_ms
    } else {
        input.admitted_at_ms
    });
    tx.execute(
        "UPDATE delegation_sequence SET sequence=?1 WHERE singleton=1",
        params![accepted.admission_sequence],
    )
    .map_err(storage)?;
    task_record.latest_admission_sequence = accepted.admission_sequence;
    tx.execute("INSERT INTO delegation_runs(run_id,workspace_id,task_id,submission_kind,idempotency_key,occupied,record_json) VALUES(?1,?2,?3,?4,?5,1,?6)",
        params![run.run_id,run.workspace_id.as_str(),run.task_id,
            if run.continued_from.is_some() {"continue"} else {"start"},run.idempotency_key,encode(&accepted)?]).map_err(storage)?;
    write_task_with_title_key(&tx, &task_record, title_lookup_key.as_deref())?;
    native::register_acceptance(&tx, input, &accepted, new_task)?;
    tx.commit().map_err(storage)?;
    Ok(accepted)
}

fn read_task_title_lookup_key(
    connection: &Connection,
    workspace: &hiroute_domain::WorkspaceId,
    task_id: &str,
) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT title_lookup_key FROM delegation_tasks WHERE workspace_id=?1 AND task_id=?2",
            params![workspace.as_str(), task_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)
        .map(Option::flatten)
}

fn validate(input: &DelegationAcceptanceV1) -> Result<()> {
    let r = &input.run;
    let t = &input.task;
    t.workspace.validate()?;
    r.configuration.validate_for(r)?;
    if r.workspace_id != t.workspace_id
        || r.task_id != t.task_id
        || t.latest_run_id != r.run_id
        || r.execution.root_identity != t.workspace.root_identity
        || t.parent_task_ref
            .as_ref()
            .is_some_and(|value| !valid_delegation_id(value))
        || r.progress != RunProgressV1::default()
        || r.stop_evidence.is_some()
        || r.result_body.is_some()
        || r.result_incomplete
        || r.execution_failure.is_some()
        || r.accepted_at_ms.is_some()
        || r.process.is_some()
        || r.session.is_some()
        || r.lease_revoked
        || r.deadline_ms <= input.admitted_at_ms
        || input.admitted_at_ms.checked_add(r.execution.duration_ms) != Some(r.deadline_ms)
        || r.permit_generation == 0
        || r.execution.delegation_depth != 1
        || r.execution.duration_ms == 0
        || r.execution.duration_ms > MAX_RUN_DURATION_MS
        || [
            &r.task_id,
            &r.run_id,
            &r.idempotency_key,
            &r.execution_owner_ref,
            &r.lease_id,
            &r.daemon_epoch,
            &r.permit_id,
            &r.launch_nonce,
        ]
        .iter()
        .any(|v| !valid_delegation_id(v))
        || (r.ordinal == 1
            && (t.latest_admission_sequence != 0
                || t.title.is_none()
                || input.title_lookup_key.as_deref().is_none_or(str::is_empty)))
        || t.title.as_ref().is_some_and(|title| {
            title.value.is_empty()
                || title.value.len() > 1024
                || title.value.chars().any(char::is_control)
                || title.initial_body_ref.validate().is_err()
                || (r.ordinal == 1 && t.body_refs.first() != Some(&title.initial_body_ref))
        })
        || t.required_body_ids.len() > 16
        || t.required_body_ids.len() != t.body_refs.len()
        || t.body_refs
            .iter()
            .zip(&t.required_body_ids)
            .any(|(reference, id)| reference.validate().is_err() || &reference.opaque_id != id)
        || t.native_history_paths.len() > 256
        || t.native_history_paths
            .iter()
            .any(|path| !valid_native_history_path(path))
        || t.native_history_paths
            .iter()
            .enumerate()
            .any(|(index, path)| t.native_history_paths[..index].contains(path))
    {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    Ok(())
}

fn ensure_occupied_integrity(connection: &Connection) -> Result<()> {
    let orphaned: bool = connection
        .query_row(
            "SELECT EXISTS(\
                 SELECT 1 FROM delegation_runs r \
                 LEFT JOIN delegation_tasks t \
                   ON t.workspace_id=r.workspace_id AND t.task_id=r.task_id \
                 WHERE r.occupied=1 AND t.task_id IS NULL\
             )",
            [],
            |row| row.get(0),
        )
        .map_err(storage)?;
    if orphaned {
        return Err(DelegationErrorV1::StorageUnavailable);
    }
    Ok(())
}
