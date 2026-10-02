use super::RuntimeStore;
use hiroute_domain::delegation::*;
use hiroute_domain::{OperationId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};

mod acceptance;
mod events;
mod maintenance;
mod native;
#[cfg(test)]
mod tests;

type Result<T> = std::result::Result<T, DelegationErrorV1>;

impl RuntimeStore {
    pub fn upgrade_occupied_tasks(&self) -> std::result::Result<u64, crate::LocalStorageError> {
        Ok(self.connection.borrow().query_row(
            "SELECT count(*) FROM delegation_runs WHERE occupied=1",
            [],
            |row| row.get(0),
        )?)
    }
}

impl DelegationRuntimePort for RuntimeStore {
    fn worker_concurrency_settings(&self) -> Result<WorkerConcurrencySettingsV1> {
        read_worker_concurrency_settings(&self.connection.borrow())
    }
    fn set_worker_concurrency_settings(
        &self,
        settings: WorkerConcurrencySettingsV1,
    ) -> Result<WorkerConcurrencySettingsV1> {
        let settings = settings.validate()?;
        let mut connection = self.connection.borrow_mut();
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        tx.execute(
            "INSERT INTO worker_settings(singleton,max_concurrent) VALUES(1,?1) \
             ON CONFLICT(singleton) DO UPDATE SET max_concurrent=excluded.max_concurrent",
            [settings.max_concurrent],
        )
        .map_err(storage)?;
        let effective = read_worker_concurrency_settings(&tx)?;
        tx.commit().map_err(storage)?;
        Ok(effective)
    }
    fn task(&self, workspace: &WorkspaceId, task: &str) -> Result<Option<DelegationTaskV1>> {
        read_task(&self.connection.borrow(), workspace, task)
    }
    fn run(&self, workspace: &WorkspaceId, run: &str) -> Result<Option<DelegationRunV1>> {
        read_run(&self.connection.borrow(), workspace, run)
    }
    fn find_submission(
        &self,
        workspace: &WorkspaceId,
        continuation: bool,
        key: &str,
    ) -> Result<Option<DelegationRunV1>> {
        find_submission(&self.connection.borrow(), workspace, continuation, key)
    }
    fn list_latest_runs(
        &self,
        workspace: &WorkspaceId,
        title_lookup_key: Option<&str>,
        before_sequence: Option<u64>,
        limit: u16,
    ) -> Result<Vec<DelegationRunV1>> {
        list_latest_runs(
            &self.connection.borrow(),
            workspace,
            title_lookup_key,
            before_sequence,
            limit,
        )
    }
    fn resumable_tasks(
        &self,
        workspace: &WorkspaceId,
        now_ms: u64,
    ) -> Result<Vec<DelegationTaskV1>> {
        resumable_tasks(&self.connection.borrow(), workspace, now_ms)
    }
    fn accept(&self, acceptance: &DelegationAcceptanceV1) -> Result<DelegationRunV1> {
        acceptance::accept(self, acceptance)
    }
    fn checkpoint(
        &self,
        workspace: &WorkspaceId,
        run: &str,
        revision: u64,
        event_id: &str,
        event: &DelegationCheckpointV1,
    ) -> Result<DelegationRunV1> {
        events::checkpoint(self, workspace, run, revision, event_id, event)
    }
    fn request_cancel(
        &self,
        workspace: &WorkspaceId,
        run: &str,
        operation: &OperationId,
        reason: &str,
    ) -> Result<DelegationCancelReceiptV1> {
        events::cancel(self, workspace, run, operation, reason)
    }
    fn unreconciled(&self, workspace: &WorkspaceId) -> Result<Vec<DelegationRunV1>> {
        Ok(occupied(&self.connection.borrow())?
            .into_iter()
            .filter(|run| &run.workspace_id == workspace)
            .collect())
    }
    fn set_resume_materials(
        &self,
        workspace: &WorkspaceId,
        task: &str,
        latest: &str,
        until: u64,
        body_refs: &[DelegationBodyRefV1],
        native_history_paths: &[String],
    ) -> Result<()> {
        events::set_resume(
            self,
            workspace,
            task,
            latest,
            until,
            body_refs,
            native_history_paths,
        )
    }
    fn native_root(
        &self,
        workspace: &WorkspaceId,
        task_id: &str,
    ) -> Result<Option<DelegationNativeRootSnapshotV1>> {
        native::read_snapshot(&self.connection.borrow(), workspace, task_id)
    }
    fn commit_native_root_ready(
        &self,
        ready: &DelegationNativeRootReadyV1,
    ) -> Result<DelegationNativeRootSnapshotV1> {
        native::commit_ready(self, ready)
    }
    fn mark_native_root_unknown(
        &self,
        workspace: &WorkspaceId,
        task_id: &str,
        root_generation: u64,
        creation_nonce: &str,
    ) -> Result<()> {
        native::mark_unknown(self, workspace, task_id, root_generation, creation_nonce)
    }
    fn claim_native_cleanup(
        &self,
        workspace: &WorkspaceId,
        task_id: &str,
        claim: &DelegationNativeCleanupClaimV1,
    ) -> Result<DelegationNativeRootSnapshotV1> {
        native::claim_cleanup(self, workspace, task_id, claim)
    }
    fn record_native_cleanup_batch(
        &self,
        workspace: &WorkspaceId,
        task_id: &str,
        root_generation: u64,
        claim_id: &str,
    ) -> Result<()> {
        native::record_cleanup_batch(self, workspace, task_id, root_generation, claim_id)
    }
    fn record_native_cleanup_failure(
        &self,
        workspace: &WorkspaceId,
        task_id: &str,
        root_generation: u64,
        claim_id: &str,
        kind: DelegationNativeCleanupFailureKindV1,
        attempted_at_ms: i64,
    ) -> Result<()> {
        native::record_cleanup_failure(
            self,
            workspace,
            task_id,
            root_generation,
            claim_id,
            kind,
            attempted_at_ms,
        )
    }
    fn complete_native_cleanup(
        &self,
        workspace: &WorkspaceId,
        task_id: &str,
        root_generation: u64,
        claim_id: &str,
    ) -> Result<DelegationNativeRootSnapshotV1> {
        native::complete_cleanup(self, workspace, task_id, root_generation, claim_id)
    }
    fn maintenance_tasks(
        &self,
        after: Option<&DelegationTaskMaintenanceCursorV1>,
        limit: u16,
    ) -> Result<Vec<DelegationTaskV1>> {
        maintenance::tasks(&self.connection.borrow(), after, limit)
    }
    fn clear_task_title(&self, expected: &DelegationTaskV1) -> Result<bool> {
        maintenance::clear_title(self, expected)
    }
    fn claim_continuation_release(
        &self,
        expected: &DelegationTaskV1,
    ) -> Result<DelegationContinuationReleaseV1> {
        maintenance::claim_continuation_release(self, expected)
    }
    fn pending_continuation_releases(
        &self,
        limit: u16,
    ) -> Result<Vec<DelegationContinuationReleaseV1>> {
        maintenance::pending_continuation_releases(&self.connection.borrow(), limit)
    }
    fn complete_continuation_release(
        &self,
        release: &DelegationContinuationReleaseV1,
    ) -> Result<()> {
        maintenance::complete_continuation_release(self, release)
    }
}

fn decode<T: DeserializeOwned>(value: String) -> Result<T> {
    serde_json::from_str(&value).map_err(|_| DelegationErrorV1::InvalidArguments)
}
fn encode(value: &impl Serialize) -> Result<String> {
    serde_json::to_string(value).map_err(|_| DelegationErrorV1::InvalidArguments)
}
fn storage(_: rusqlite::Error) -> DelegationErrorV1 {
    DelegationErrorV1::StorageUnavailable
}

fn read_worker_concurrency_settings(
    connection: &Connection,
) -> Result<WorkerConcurrencySettingsV1> {
    let value: Option<i64> = connection
        .query_row(
            "SELECT max_concurrent FROM worker_settings WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)?;
    let Some(value) = value else {
        return Ok(WorkerConcurrencySettingsV1::default());
    };
    let max_concurrent = u16::try_from(value).map_err(|_| DelegationErrorV1::StorageUnavailable)?;
    let settings = WorkerConcurrencySettingsV1 { max_concurrent };
    settings
        .validate()
        .map_err(|_| DelegationErrorV1::StorageUnavailable)
}

fn read_run(
    connection: &Connection,
    workspace: &WorkspaceId,
    run: &str,
) -> Result<Option<DelegationRunV1>> {
    let json: Option<String> = connection
        .query_row(
            "SELECT record_json FROM delegation_runs WHERE workspace_id=?1 AND run_id=?2",
            params![workspace.as_str(), run],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)?;
    json.map(decode).transpose()
}
fn read_task(
    connection: &Connection,
    workspace: &WorkspaceId,
    task: &str,
) -> Result<Option<DelegationTaskV1>> {
    let json: Option<String> = connection
        .query_row(
            "SELECT record_json FROM delegation_tasks WHERE workspace_id=?1 AND task_id=?2",
            params![workspace.as_str(), task],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)?;
    json.map(decode).transpose()
}
fn find_submission(
    connection: &Connection,
    workspace: &WorkspaceId,
    continuation: bool,
    key: &str,
) -> Result<Option<DelegationRunV1>> {
    let json: Option<String> = connection.query_row(
        "SELECT record_json FROM delegation_runs WHERE workspace_id=?1 AND submission_kind=?2 AND idempotency_key=?3",
        params![workspace.as_str(), if continuation {"continue"} else {"start"},key], |row| row.get(0)).optional().map_err(storage)?;
    json.map(decode).transpose()
}

fn list_latest_runs(
    connection: &Connection,
    workspace: &WorkspaceId,
    title_lookup_key: Option<&str>,
    before_sequence: Option<u64>,
    limit: u16,
) -> Result<Vec<DelegationRunV1>> {
    if title_lookup_key.is_some_and(str::is_empty) || limit == 0 || limit > 201 {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let mut statement = connection
        .prepare(
            "SELECT r.record_json,t.record_json FROM delegation_runs r \
             JOIN delegation_tasks t ON t.workspace_id=r.workspace_id AND t.task_id=r.task_id \
             WHERE r.workspace_id=?1 \
               AND (?2 IS NULL OR t.title_lookup_key=?2) \
               AND r.run_id=json_extract(t.record_json,'$.latest_run_id') \
               AND (?3 IS NULL OR t.latest_admission_sequence < ?3) \
             ORDER BY t.latest_admission_sequence DESC",
        )
        .map_err(storage)?;
    let rows = statement
        .query_map(
            params![workspace.as_str(), title_lookup_key, before_sequence],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(storage)?;
    let mut result = Vec::new();
    for row in rows {
        let (run, task) = row.map_err(storage)?;
        if let Some(run) = decode_run_with_task(run, task) {
            result.push(run);
            if result.len() == usize::from(limit) {
                break;
            }
        }
    }
    Ok(result)
}

fn resumable_tasks(
    connection: &Connection,
    workspace: &WorkspaceId,
    now_ms: u64,
) -> Result<Vec<DelegationTaskV1>> {
    let mut statement = connection
        .prepare(
            "SELECT record_json FROM delegation_tasks \
             WHERE workspace_id=?1 AND json_extract(record_json,'$.resume_until_ms') > ?2 \
             ORDER BY json_extract(record_json,'$.created_at_ms')",
        )
        .map_err(storage)?;
    let rows = statement
        .query_map(params![workspace.as_str(), now_ms], |row| {
            row.get::<_, String>(0)
        })
        .map_err(storage)?;
    let mut tasks = Vec::new();
    for row in rows {
        let json = row.map_err(storage)?;
        let Ok(task) = decode::<DelegationTaskV1>(json) else {
            continue;
        };
        let run = match read_run(connection, workspace, &task.latest_run_id) {
            Ok(Some(run)) => run,
            Ok(None) => return Err(DelegationErrorV1::StorageUnavailable),
            Err(DelegationErrorV1::InvalidArguments) => continue,
            Err(error) => return Err(error),
        };
        if run.task_id != task.task_id || run.workspace_id != task.workspace_id {
            return Err(DelegationErrorV1::StorageUnavailable);
        }
        if run.progress.state == RunStateV1::Succeeded
            && run.progress.cleanup == RunCleanupV1::Complete
            && run.result_body.is_some()
            && !run.result_incomplete
            && task
                .session
                .as_ref()
                .and_then(|session| session.native_session_id.as_ref())
                .is_some()
            && !task.native_history_paths.is_empty()
        {
            tasks.push(task);
            if tasks.len() > 1024 {
                return Err(DelegationErrorV1::StorageUnavailable);
            }
        }
    }
    Ok(tasks)
}
fn decode_run_with_task(run: String, task: String) -> Option<DelegationRunV1> {
    // Unsupported records stay untouched and fail on direct lookup, not unrelated scans.
    let task: DelegationTaskV1 = decode(task).ok()?;
    let run: DelegationRunV1 = decode(run).ok()?;
    (task.task_id == run.task_id && task.workspace_id == run.workspace_id).then_some(run)
}

fn occupied(connection: &Connection) -> Result<Vec<DelegationRunV1>> {
    let mut statement = connection
        .prepare(
            "SELECT r.record_json,t.record_json FROM delegation_runs r \
         LEFT JOIN delegation_tasks t ON t.workspace_id=r.workspace_id AND t.task_id=r.task_id \
         WHERE r.occupied=1",
        )
        .map_err(storage)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(storage)?;
    let mut records = Vec::new();
    for row in rows {
        let (run, task) = row.map_err(storage)?;
        let Ok(run) = decode::<DelegationRunV1>(run) else {
            continue;
        };
        let task = task.ok_or(DelegationErrorV1::StorageUnavailable)?;
        let Ok(task) = decode::<DelegationTaskV1>(task) else {
            continue;
        };
        if task.task_id != run.task_id || task.workspace_id != run.workspace_id {
            return Err(DelegationErrorV1::StorageUnavailable);
        }
        records.push(run);
        if records.len() > 1024 {
            return Err(DelegationErrorV1::StorageUnavailable);
        }
    }
    Ok(records)
}
fn write_run(connection: &Connection, run: &DelegationRunV1) -> Result<()> {
    connection.execute("UPDATE delegation_runs SET record_json=?1,occupied=?2 WHERE run_id=?3 AND workspace_id=?4",
        params![encode(run)?, i32::from(!run.progress.workspace_releasable()),run.run_id,run.workspace_id.as_str()]).map_err(storage)?;
    Ok(())
}
fn write_task(connection: &Connection, task: &DelegationTaskV1) -> Result<()> {
    let title_lookup_key: Option<String> = connection
        .query_row(
            "SELECT title_lookup_key FROM delegation_tasks WHERE workspace_id=?1 AND task_id=?2",
            params![task.workspace_id.as_str(), task.task_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)?
        .flatten();
    write_task_with_title_key(connection, task, title_lookup_key.as_deref())
}
fn write_task_with_title_key(
    connection: &Connection,
    task: &DelegationTaskV1,
    title_lookup_key: Option<&str>,
) -> Result<()> {
    connection.execute("INSERT INTO delegation_tasks(workspace_id,task_id,latest_admission_sequence,title_lookup_key,record_json) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace_id,task_id) DO UPDATE SET latest_admission_sequence=excluded.latest_admission_sequence,title_lookup_key=excluded.title_lookup_key,record_json=excluded.record_json",
        params![task.workspace_id.as_str(), task.task_id, task.latest_admission_sequence, title_lookup_key, encode(task)?]).map_err(storage)?;
    Ok(())
}

impl DelegationScopeCancellationPort for RuntimeStore {
    fn cancel_scope(
        &self,
        workspace: &WorkspaceId,
        operation: &OperationId,
        scope: &DelegationAuthorizationScopeV1,
    ) -> Result<String> {
        scope.validate()?;
        if OperationId::parse(operation.as_str()).is_err() {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        let digest = hiroute_domain::CanonicalDigest::of(&(
            "delegation-scope-cancel-v1",
            workspace,
            operation,
            scope,
        ))
        .map_err(|_| DelegationErrorV1::InvalidArguments)?;
        let reference = digest.as_str().to_owned();
        let mut conn = self.connection.borrow_mut();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        // The shared admission guard excludes insertion with the revoked authority until
        // the current generation/deny is visible. Whole expansion commits or rolls back.
        for run in occupied(&tx)? {
            if &run.workspace_id == workspace && scope.matches(&run) {
                events::cancel_in_transaction(
                    &tx,
                    workspace,
                    &run.run_id,
                    operation,
                    "authority-revoked",
                )?;
            }
        }
        tx.commit().map_err(storage)?;
        Ok(reference)
    }
}
