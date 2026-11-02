use super::*;
use rusqlite::TransactionBehavior;

pub(super) fn register_acceptance(
    transaction: &Connection,
    input: &DelegationAcceptanceV1,
    accepted: &DelegationRunV1,
    new_task: bool,
) -> Result<()> {
    let task = &input.task;
    let root = if new_task {
        let relative_root = hiroute_domain::CanonicalDigest::of(&serde_json::json!([
            "delegation-native-root-v1",
            task.workspace_id,
            task.workspace.root_identity,
            task.task_id,
            task.plan.harness,
        ]))
        .map_err(|_| DelegationErrorV1::InvalidArguments)?
        .as_str()
        .trim_start_matches("sha256:")
        .to_owned();
        let root = DelegationNativeRootV1 {
            workspace_id: task.workspace_id.clone(),
            task_id: task.task_id.clone(),
            root_generation: DELEGATION_NATIVE_ROOT_GENERATION_V1,
            harness: task.plan.harness,
            workspace_root_identity: task.workspace.root_identity.clone(),
            relative_root,
            creation_nonce: accepted.launch_nonce.clone(),
            state: DelegationNativeRootStateV1::Creating,
            use_revision: 1,
            managed_base_path: None,
            filesystem_identity: None,
            cleanup_claim: None,
            deletion_batches: 0,
            last_cleanup_failure: None,
        };
        root.validate()?;
        transaction
            .execute(
                "INSERT INTO delegation_native_roots(
                    workspace_id,task_id,root_generation,state,use_revision,record_json
                 ) VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    root.workspace_id.as_str(),
                    root.task_id,
                    root.root_generation,
                    root_state_name(root.state),
                    root.use_revision,
                    encode(&root)?,
                ],
            )
            .map_err(storage)?;
        root
    } else {
        let mut root = read_root(transaction, &task.workspace_id, &task.task_id)?
            .ok_or(DelegationErrorV1::ResumeUnavailable)?;
        if root.state != DelegationNativeRootStateV1::Ready
            || root.harness != task.plan.harness
            || root.workspace_root_identity != task.workspace.root_identity
            || root.cleanup_claim.is_some()
        {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
        root.use_revision = root
            .use_revision
            .checked_add(1)
            .ok_or(DelegationErrorV1::StorageUnavailable)?;
        write_root(transaction, &root)?;
        root
    };
    let accepted_at_ms = accepted
        .accepted_at_ms
        .ok_or(DelegationErrorV1::StorageUnavailable)?;
    let usage = DelegationNativeUseV1 {
        workspace_id: accepted.workspace_id.clone(),
        task_id: accepted.task_id.clone(),
        root_generation: root.root_generation,
        run_id: accepted.run_id.clone(),
        lease_id: accepted.lease_id.clone(),
        daemon_epoch: accepted.daemon_epoch.clone(),
        accepted_at_ms,
        state: DelegationNativeUseStateV1::Accepted,
        scope_run_id: accepted.run_id.clone(),
    };
    usage.validate()?;
    transaction
        .execute(
            "INSERT INTO delegation_native_uses(
                workspace_id,task_id,root_generation,run_id,state,record_json
             ) VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                usage.workspace_id.as_str(),
                usage.task_id,
                usage.root_generation,
                usage.run_id,
                use_state_name(usage.state),
                encode(&usage)?,
            ],
        )
        .map_err(storage)?;
    Ok(())
}

pub(super) fn transition_checkpoint(
    transaction: &Connection,
    run: &DelegationRunV1,
    event: &DelegationCheckpointV1,
) -> Result<()> {
    let Some(mut usage) = read_use(transaction, &run.workspace_id, &run.run_id)? else {
        return Err(DelegationErrorV1::StorageUnavailable);
    };
    let next = match event {
        DelegationCheckpointV1::Progress {
            event: RunEventV1::Preparing,
        } if usage.state == DelegationNativeUseStateV1::Accepted => {
            Some(DelegationNativeUseStateV1::MayHaveSpawned)
        }
        DelegationCheckpointV1::Progress {
            event: RunEventV1::LaunchFailedBeforeSpawn,
        }
        | DelegationCheckpointV1::ExecutionFailed {
            event: RunEventV1::LaunchFailedBeforeSpawn,
            ..
        } if matches!(
            usage.state,
            DelegationNativeUseStateV1::Accepted | DelegationNativeUseStateV1::MayHaveSpawned
        ) =>
        {
            Some(DelegationNativeUseStateV1::NeverSpawned)
        }
        DelegationCheckpointV1::ProcessStopped { evidence }
            if evidence.scope_stopped && !evidence.residual_unknown =>
        {
            Some(DelegationNativeUseStateV1::Stopped)
        }
        _ => None,
    };
    let Some(next) = next else { return Ok(()) };
    if usage.state == next {
        return Ok(());
    }
    if usage.state.ended() {
        return Err(DelegationErrorV1::Conflict);
    }
    usage.state = next;
    write_use(transaction, &usage)?;
    let mut root = read_root(transaction, &usage.workspace_id, &usage.task_id)?
        .ok_or(DelegationErrorV1::StorageUnavailable)?;
    if root.root_generation != usage.root_generation
        || matches!(
            root.state,
            DelegationNativeRootStateV1::Deleting | DelegationNativeRootStateV1::Removed
        )
    {
        return Err(DelegationErrorV1::Conflict);
    }
    root.use_revision = root
        .use_revision
        .checked_add(1)
        .ok_or(DelegationErrorV1::StorageUnavailable)?;
    if next == DelegationNativeUseStateV1::NeverSpawned
        && root.state == DelegationNativeRootStateV1::Creating
        && read_uses(
            transaction,
            &root.workspace_id,
            &root.task_id,
            root.root_generation,
        )?
        .iter()
        .all(|usage| usage.state == DelegationNativeUseStateV1::NeverSpawned)
    {
        root.state = DelegationNativeRootStateV1::NoNative;
    }
    write_root(transaction, &root)
}

pub(super) fn register_resume_materials(
    transaction: &Connection,
    workspace: &WorkspaceId,
    task_id: &str,
    latest_run_id: &str,
) -> Result<()> {
    let mut root =
        read_root(transaction, workspace, task_id)?.ok_or(DelegationErrorV1::ResumeUnavailable)?;
    let usage = read_use(transaction, workspace, latest_run_id)?
        .ok_or(DelegationErrorV1::ResumeUnavailable)?;
    if root.state != DelegationNativeRootStateV1::Ready
        || root.cleanup_claim.is_some()
        || usage.task_id != task_id
        || usage.root_generation != root.root_generation
        || usage.state != DelegationNativeUseStateV1::Stopped
    {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    root.use_revision = root
        .use_revision
        .checked_add(1)
        .ok_or(DelegationErrorV1::StorageUnavailable)?;
    write_root(transaction, &root)
}

pub(super) fn read_snapshot(
    connection: &Connection,
    workspace: &WorkspaceId,
    task_id: &str,
) -> Result<Option<DelegationNativeRootSnapshotV1>> {
    let Some(root) = read_root(connection, workspace, task_id)? else {
        return Ok(None);
    };
    let uses = read_uses(connection, workspace, task_id, root.root_generation)?;
    Ok(Some(DelegationNativeRootSnapshotV1 { root, uses }))
}

pub(super) fn commit_ready(
    store: &RuntimeStore,
    ready: &DelegationNativeRootReadyV1,
) -> Result<DelegationNativeRootSnapshotV1> {
    if ready.root_generation == 0
        || !valid_delegation_id(&ready.task_id)
        || !valid_delegation_id(&ready.creation_nonce)
        || ready.managed_base_path.is_empty()
        || ready.managed_base_path.len() > 4096
        || ready.managed_base_path.contains('\0')
        || !matches!(
            ready.filesystem_identity.scheme.as_str(),
            "unix-dev-inode-v1" | "windows-volume-file-index-v1"
        )
    {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let mut root = read_root(&transaction, &ready.workspace_id, &ready.task_id)?
        .ok_or(DelegationErrorV1::Conflict)?;
    if root.root_generation != ready.root_generation
        || root.creation_nonce != ready.creation_nonce
        || root.state != DelegationNativeRootStateV1::Creating
    {
        return Err(DelegationErrorV1::Conflict);
    }
    root.state = DelegationNativeRootStateV1::Ready;
    root.managed_base_path = Some(ready.managed_base_path.clone());
    root.filesystem_identity = Some(ready.filesystem_identity.clone());
    write_root(&transaction, &root)?;
    let uses = read_uses(
        &transaction,
        &root.workspace_id,
        &root.task_id,
        root.root_generation,
    )?;
    transaction.commit().map_err(storage)?;
    Ok(DelegationNativeRootSnapshotV1 { root, uses })
}

pub(super) fn mark_unknown(
    store: &RuntimeStore,
    workspace: &WorkspaceId,
    task_id: &str,
    root_generation: u64,
    creation_nonce: &str,
) -> Result<()> {
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let mut root =
        read_root(&transaction, workspace, task_id)?.ok_or(DelegationErrorV1::Conflict)?;
    if root.root_generation != root_generation || root.creation_nonce != creation_nonce {
        return Err(DelegationErrorV1::Conflict);
    }
    match root.state {
        DelegationNativeRootStateV1::Creating => {
            root.state = DelegationNativeRootStateV1::Unknown;
            write_root(&transaction, &root)?;
        }
        DelegationNativeRootStateV1::Unknown => {}
        _ => return Err(DelegationErrorV1::Conflict),
    }
    transaction.commit().map_err(storage)
}

pub(super) fn claim_cleanup(
    store: &RuntimeStore,
    workspace: &WorkspaceId,
    task_id: &str,
    claim: &DelegationNativeCleanupClaimV1,
) -> Result<DelegationNativeRootSnapshotV1> {
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let mut root =
        read_root(&transaction, workspace, task_id)?.ok_or(DelegationErrorV1::Conflict)?;
    if root.root_generation != claim.root_generation
        || root.use_revision != claim.expected_use_revision
        || !matches!(
            root.state,
            DelegationNativeRootStateV1::Ready | DelegationNativeRootStateV1::NoNative
        )
        || root.cleanup_claim.is_some()
    {
        return Err(DelegationErrorV1::Conflict);
    }
    claim.validate_for(&root)?;
    let uses = read_uses(
        &transaction,
        &root.workspace_id,
        &root.task_id,
        root.root_generation,
    )?;
    if uses.is_empty()
        || uses.iter().any(|usage| !usage.state.ended())
        || uses.iter().any(|usage| {
            !claim
                .jobs
                .iter()
                .any(|job| job.run_id == usage.scope_run_id)
        })
        || claim
            .jobs
            .iter()
            .any(|job| !uses.iter().any(|usage| usage.scope_run_id == job.run_id))
    {
        return Err(DelegationErrorV1::Conflict);
    }
    root.state = DelegationNativeRootStateV1::Deleting;
    root.cleanup_claim = Some(claim.clone());
    write_root(&transaction, &root)?;
    transaction.commit().map_err(storage)?;
    Ok(DelegationNativeRootSnapshotV1 { root, uses })
}

pub(super) fn record_cleanup_batch(
    store: &RuntimeStore,
    workspace: &WorkspaceId,
    task_id: &str,
    root_generation: u64,
    claim_id: &str,
) -> Result<()> {
    update_claimed_root(
        store,
        workspace,
        task_id,
        root_generation,
        claim_id,
        |root| {
            root.deletion_batches = root
                .deletion_batches
                .checked_add(1)
                .ok_or(DelegationErrorV1::StorageUnavailable)?;
            root.last_cleanup_failure = None;
            Ok(())
        },
    )
    .map(|_| ())
}

pub(super) fn record_cleanup_failure(
    store: &RuntimeStore,
    workspace: &WorkspaceId,
    task_id: &str,
    root_generation: u64,
    claim_id: &str,
    kind: DelegationNativeCleanupFailureKindV1,
    attempted_at_ms: i64,
) -> Result<()> {
    if attempted_at_ms < 0 {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    update_claimed_root(
        store,
        workspace,
        task_id,
        root_generation,
        claim_id,
        |root| {
            let attempts = root
                .last_cleanup_failure
                .as_ref()
                .map_or(1, |failure| failure.attempts.saturating_add(1));
            root.last_cleanup_failure = Some(DelegationNativeCleanupFailureV1 {
                kind,
                attempted_at_ms,
                attempts,
            });
            Ok(())
        },
    )
    .map(|_| ())
}

pub(super) fn complete_cleanup(
    store: &RuntimeStore,
    workspace: &WorkspaceId,
    task_id: &str,
    root_generation: u64,
    claim_id: &str,
) -> Result<DelegationNativeRootSnapshotV1> {
    update_claimed_root(
        store,
        workspace,
        task_id,
        root_generation,
        claim_id,
        |root| {
            root.state = DelegationNativeRootStateV1::Removed;
            root.last_cleanup_failure = None;
            Ok(())
        },
    )
}

fn update_claimed_root(
    store: &RuntimeStore,
    workspace: &WorkspaceId,
    task_id: &str,
    root_generation: u64,
    claim_id: &str,
    update: impl FnOnce(&mut DelegationNativeRootV1) -> Result<()>,
) -> Result<DelegationNativeRootSnapshotV1> {
    if !valid_delegation_id(claim_id) {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let mut root =
        read_root(&transaction, workspace, task_id)?.ok_or(DelegationErrorV1::Conflict)?;
    if root.root_generation != root_generation
        || root.state != DelegationNativeRootStateV1::Deleting
        || root
            .cleanup_claim
            .as_ref()
            .map(|claim| claim.claim_id.as_str())
            != Some(claim_id)
    {
        return Err(DelegationErrorV1::Conflict);
    }
    update(&mut root)?;
    write_root(&transaction, &root)?;
    let uses = read_uses(
        &transaction,
        &root.workspace_id,
        &root.task_id,
        root.root_generation,
    )?;
    transaction.commit().map_err(storage)?;
    Ok(DelegationNativeRootSnapshotV1 { root, uses })
}

fn read_root(
    connection: &Connection,
    workspace: &WorkspaceId,
    task_id: &str,
) -> Result<Option<DelegationNativeRootV1>> {
    let encoded: Option<String> = connection
        .query_row(
            "SELECT record_json FROM delegation_native_roots
             WHERE workspace_id=?1 AND task_id=?2",
            params![workspace.as_str(), task_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)?;
    let root: Option<DelegationNativeRootV1> = encoded.map(decode).transpose()?;
    if let Some(root) = &root {
        root.validate()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
    }
    Ok(root)
}

fn read_use(
    connection: &Connection,
    workspace: &WorkspaceId,
    run_id: &str,
) -> Result<Option<DelegationNativeUseV1>> {
    let encoded: Option<String> = connection
        .query_row(
            "SELECT record_json FROM delegation_native_uses
             WHERE workspace_id=?1 AND run_id=?2",
            params![workspace.as_str(), run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)?;
    let usage: Option<DelegationNativeUseV1> = encoded.map(decode).transpose()?;
    if let Some(usage) = &usage {
        usage
            .validate()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
    }
    Ok(usage)
}

fn read_uses(
    connection: &Connection,
    workspace: &WorkspaceId,
    task_id: &str,
    root_generation: u64,
) -> Result<Vec<DelegationNativeUseV1>> {
    let mut statement = connection
        .prepare(
            "SELECT record_json FROM delegation_native_uses
             WHERE workspace_id=?1 AND task_id=?2 AND root_generation=?3 ORDER BY run_id",
        )
        .map_err(storage)?;
    let rows = statement
        .query_map(
            params![workspace.as_str(), task_id, root_generation],
            |row| row.get::<_, String>(0),
        )
        .map_err(storage)?;
    let mut uses = Vec::new();
    for row in rows {
        let usage: DelegationNativeUseV1 = decode(row.map_err(storage)?)?;
        usage
            .validate()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        uses.push(usage);
        if uses.len() > 1024 {
            return Err(DelegationErrorV1::StorageUnavailable);
        }
    }
    Ok(uses)
}

fn write_root(connection: &Connection, root: &DelegationNativeRootV1) -> Result<()> {
    root.validate()?;
    let changed = connection
        .execute(
            "UPDATE delegation_native_roots
             SET state=?1,use_revision=?2,record_json=?3
             WHERE workspace_id=?4 AND task_id=?5 AND root_generation=?6",
            params![
                root_state_name(root.state),
                root.use_revision,
                encode(root)?,
                root.workspace_id.as_str(),
                root.task_id,
                root.root_generation,
            ],
        )
        .map_err(storage)?;
    if changed != 1 {
        return Err(DelegationErrorV1::Conflict);
    }
    Ok(())
}

fn write_use(connection: &Connection, usage: &DelegationNativeUseV1) -> Result<()> {
    usage.validate()?;
    let changed = connection
        .execute(
            "UPDATE delegation_native_uses SET state=?1,record_json=?2
             WHERE workspace_id=?3 AND run_id=?4 AND root_generation=?5",
            params![
                use_state_name(usage.state),
                encode(usage)?,
                usage.workspace_id.as_str(),
                usage.run_id,
                usage.root_generation,
            ],
        )
        .map_err(storage)?;
    if changed != 1 {
        return Err(DelegationErrorV1::Conflict);
    }
    Ok(())
}

fn root_state_name(state: DelegationNativeRootStateV1) -> &'static str {
    match state {
        DelegationNativeRootStateV1::Creating => "creating",
        DelegationNativeRootStateV1::Ready => "ready",
        DelegationNativeRootStateV1::NoNative => "no_native",
        DelegationNativeRootStateV1::Unknown => "unknown",
        DelegationNativeRootStateV1::Deleting => "deleting",
        DelegationNativeRootStateV1::Removed => "removed",
    }
}

fn use_state_name(state: DelegationNativeUseStateV1) -> &'static str {
    match state {
        DelegationNativeUseStateV1::Accepted => "accepted",
        DelegationNativeUseStateV1::MayHaveSpawned => "may_have_spawned",
        DelegationNativeUseStateV1::Stopped => "stopped",
        DelegationNativeUseStateV1::NeverSpawned => "never_spawned",
    }
}
