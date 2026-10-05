//! Read-only task views and the single-run cancellation command over durable runtime records.

use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hiroute_application_api::{
    DELEGATION_CANCEL_SCHEMA_V1, DELEGATION_GET_SCHEMA_V1, DELEGATION_LIST_SCHEMA_V1,
    DELEGATION_WAIT_SCHEMA_V1, DelegationAcceptedTimeStateV1, DelegationCancelRequestV1,
    DelegationCancelV1, DelegationContentAvailabilityV1, DelegationGetRequestV1, DelegationGetV1,
    DelegationListV1, DelegationRunScopeV1, DelegationRunViewV1, DelegationSubmissionOperationV1,
    DelegationTaskViewV1, DelegationWaitRequestV1, DelegationWaitV1, MAX_DELEGATION_INPUT_BYTES,
    WorkerExecutorPresentationBasisV1, WorkerExecutorPresentationV1, normalize_worker_title,
};
use hiroute_domain::delegation::{
    DelegationErrorV1, DelegationRunV1, DelegationRuntimePort, DelegationTaskV1, RunStateV1,
};
use hiroute_domain::{CanonicalDigest, IdempotencyScopeV1, OperationId, WorkspaceId};
use hiroute_observation::managed_text::{
    CHUNK_BYTES, ManagedTextError, ManagedTextRef, ManagedTextScope, ManagedTextState,
};
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};

use super::{LocalControlAdapter, delegation_worker::DelegationCallerContext};

const STORAGE_PAGE: u16 = 201;
const MAX_SCAN_CANDIDATES: usize = 1_000;
const TASK_BRIEF_CHARS: usize = 512;
const WORKER_LIST_CURSOR_PURPOSE: &str = "WorkerListV1";

impl LocalControlAdapter {
    pub(super) fn list_delegations(
        &self,
        caller: &DelegationCallerContext,
        cursor: Option<&str>,
        limit: u16,
        title: Option<&str>,
    ) -> Result<DelegationListV1, DelegationErrorV1> {
        let normalized_title = title
            .map(normalize_worker_title)
            .transpose()
            .map_err(|_| DelegationErrorV1::InvalidArguments)?;
        let title_lookup_key = normalized_title.as_deref().map(|title| {
            self.delegation_digest_authority
                .worker_title_lookup_digest(caller.workspace_id(), title)
                .as_str()
                .to_owned()
        });
        let filter_digest = title_lookup_key.as_deref().unwrap_or("unfiltered");
        let mut before = cursor
            .map(|cursor| self.parse_list_cursor(caller.workspace_id(), filter_digest, cursor))
            .transpose()?;
        let wanted = usize::from(limit) + 1;
        let mut tasks = Vec::with_capacity(wanted);
        let mut scanned = 0_usize;
        let mut exhausted = false;
        let mut last_scanned = None;
        while tasks.len() < wanted && scanned < MAX_SCAN_CANDIDATES {
            let storage_limit = usize::from(STORAGE_PAGE).min(MAX_SCAN_CANDIDATES - scanned) as u16;
            let runs = DelegationRuntimePort::list_latest_runs(
                self,
                caller.workspace_id(),
                title_lookup_key.as_deref(),
                before,
                storage_limit,
            )?;
            if runs.is_empty() {
                exhausted = true;
                break;
            }
            let storage_exhausted = runs.len() < usize::from(storage_limit);
            for run in &runs {
                scanned += 1;
                before = Some(run.admission_sequence);
                last_scanned = before;
                let task = DelegationRuntimePort::task(self, caller.workspace_id(), &run.task_id)?
                    .ok_or(DelegationErrorV1::StorageUnavailable)?;
                authorize(caller, &task, run)?;
                let mut content = self.task_content_projection(&task, now_ms()?);
                if let Some(expected) = normalized_title.as_deref() {
                    if content.availability != DelegationContentAvailabilityV1::Available
                        || content.title.as_deref() != Some(expected)
                    {
                        continue;
                    }
                    // The second visibility read is the publication point for a filtered title.
                    content = self.task_content_projection(&task, now_ms()?);
                    if content.availability != DelegationContentAvailabilityV1::Available
                        || content.title.as_deref() != Some(expected)
                    {
                        continue;
                    }
                }
                tasks.push(self.task_view_with_content(&task, run, now_ms()?, content));
                if tasks.len() == wanted {
                    break;
                }
            }
            if storage_exhausted {
                exhausted = true;
                break;
            }
        }
        let has_more = tasks.len() == wanted;
        if has_more {
            tasks.pop();
        }
        let resume_before = if has_more {
            tasks.last().map(|task| task.latest_admission_sequence)
        } else if !exhausted && scanned == MAX_SCAN_CANDIDATES {
            last_scanned
        } else {
            None
        };
        let next_cursor = resume_before
            .map(|sequence| self.list_cursor(caller.workspace_id(), filter_digest, sequence))
            .transpose()?;
        Ok(DelegationListV1 {
            schema: DELEGATION_LIST_SCHEMA_V1.into(),
            tasks,
            next_cursor,
        })
    }

    pub(super) fn get_delegation(
        &self,
        caller: &DelegationCallerContext,
        request: &DelegationGetRequestV1,
    ) -> Result<DelegationGetV1, DelegationErrorV1> {
        let (task, run) = if let Some(task_id) = request.task_id.as_deref() {
            let task = DelegationRuntimePort::task(self, caller.workspace_id(), task_id)?
                .ok_or(DelegationErrorV1::NotFound)?;
            let run_id = request.run_id.as_deref().unwrap_or(&task.latest_run_id);
            let run = DelegationRuntimePort::run(self, caller.workspace_id(), run_id)?
                .ok_or(DelegationErrorV1::NotFound)?;
            (task, run)
        } else if let Some(run_id) = request.run_id.as_deref() {
            let run = DelegationRuntimePort::run(self, caller.workspace_id(), run_id)?
                .ok_or(DelegationErrorV1::NotFound)?;
            let task = DelegationRuntimePort::task(self, caller.workspace_id(), &run.task_id)?
                .ok_or(DelegationErrorV1::StorageUnavailable)?;
            (task, run)
        } else {
            let key = request
                .submission_key
                .as_deref()
                .ok_or(DelegationErrorV1::InvalidArguments)?;
            let continuation =
                request.submission_operation == Some(DelegationSubmissionOperationV1::Continue);
            let run = DelegationRuntimePort::find_submission(
                self,
                caller.workspace_id(),
                continuation,
                key,
            )?
            .ok_or(DelegationErrorV1::NotFound)?;
            let task = DelegationRuntimePort::task(self, caller.workspace_id(), &run.task_id)?
                .ok_or(DelegationErrorV1::StorageUnavailable)?;
            (task, run)
        };
        authorize(caller, &task, &run)?;
        Ok(DelegationGetV1 {
            schema: DELEGATION_GET_SCHEMA_V1.into(),
            task: self.task_view(&task, &run, now_ms()?),
        })
    }

    pub(super) fn wait_delegation(
        &self,
        caller: &DelegationCallerContext,
        request: &DelegationWaitRequestV1,
    ) -> Result<DelegationWaitV1, DelegationErrorV1> {
        let deadline =
            Instant::now() + Duration::from_millis(u64::from(request.effective_wait_ms()));
        loop {
            let run = DelegationRuntimePort::run(self, caller.workspace_id(), &request.run_id)?
                .ok_or(DelegationErrorV1::NotFound)?;
            let task = DelegationRuntimePort::task(self, caller.workspace_id(), &run.task_id)?
                .ok_or(DelegationErrorV1::StorageUnavailable)?;
            authorize(caller, &task, &run)?;
            let changed = request
                .after_revision
                .is_none_or(|revision| run.progress.revision > revision);
            let terminal = terminal(run.progress.state);
            if changed || terminal || Instant::now() >= deadline {
                return Ok(DelegationWaitV1 {
                    schema: DELEGATION_WAIT_SCHEMA_V1.into(),
                    run: run_view(&task, &run),
                    changed,
                    timed_out: !changed && !terminal,
                });
            }
            thread::sleep(
                Duration::from_secs(1).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }

    pub(super) fn cancel_delegation(
        &self,
        caller: &DelegationCallerContext,
        request: &DelegationCancelRequestV1,
    ) -> Result<DelegationCancelV1, DelegationErrorV1> {
        let workspace = caller.workspace_id();
        let run = DelegationRuntimePort::run(self, workspace, &request.run_id)?
            .ok_or(DelegationErrorV1::NotFound)?;
        let task = DelegationRuntimePort::task(self, workspace, &run.task_id)?
            .ok_or(DelegationErrorV1::StorageUnavailable)?;
        authorize(caller, &task, &run)?;
        let scope = IdempotencyScopeV1::new(
            "worker-instance",
            "CancelDelegation",
            &request.idempotency_key,
        )
        .map_err(|_| DelegationErrorV1::InvalidArguments)?;
        let digest =
            CanonicalDigest::of(&("delegation-cancel-idempotency/v1", &scope, &request.run_id))
                .map_err(|_| DelegationErrorV1::InvalidArguments)?;
        let operation = OperationId::derive(workspace, &scope, &digest);
        let reason = if request.reason.is_empty() {
            "user-requested"
        } else {
            &request.reason
        };
        let receipt = DelegationRuntimePort::request_cancel(
            self,
            workspace,
            &request.run_id,
            &operation,
            reason,
        )?;
        // Persist the exact idempotent intent before revoking an in-memory verifier. A
        // conflicting replay must never deny a run whose cancellation was rejected.
        // A poisoned acceleration index already denies Gateway authentication. Do not turn a
        // committed cancellation into a lost wake or an apparently uncommitted response.
        let _ = self.delegation_run_authority.deny_run(&run);
        let updated = DelegationRuntimePort::run(self, workspace, &request.run_id)?
            .ok_or(DelegationErrorV1::StorageUnavailable)?;
        Ok(DelegationCancelV1 {
            schema: DELEGATION_CANCEL_SCHEMA_V1.into(),
            operation_id: receipt.operation_id.to_string(),
            run: run_view(&task, &updated),
        })
    }
}

pub(super) fn run_view(task: &DelegationTaskV1, run: &DelegationRunV1) -> DelegationRunViewV1 {
    let (accepted_at_ms, accepted_time_state) = match run.accepted_at_ms {
        Some(accepted) => (Some(accepted), DelegationAcceptedTimeStateV1::Recorded),
        None if run.ordinal == 1 && run.continued_from.is_none() => (
            Some(task.created_at_ms),
            DelegationAcceptedTimeStateV1::Recorded,
        ),
        None => (None, DelegationAcceptedTimeStateV1::LegacyUnavailable),
    };
    let display_name = match task.plan.harness {
        hiroute_domain::delegation::WorkerHarnessV1::CodexCli => "Codex",
        hiroute_domain::delegation::WorkerHarnessV1::ClaudeCode => "Claude Code",
        hiroute_domain::delegation::WorkerHarnessV1::QoderCli => "Qoder",
        hiroute_domain::delegation::WorkerHarnessV1::Pi => "Pi",
    };
    DelegationRunViewV1 {
        task_id: run.task_id.clone(),
        run_id: run.run_id.clone(),
        ordinal: run.ordinal,
        continued_from: run.continued_from.clone(),
        admission_sequence: run.admission_sequence,
        accepted_at_ms,
        accepted_time_state,
        executor: WorkerExecutorPresentationV1 {
            harness: Some(task.plan.harness),
            display_name: Some(display_name.to_owned()),
            basis: WorkerExecutorPresentationBasisV1::FrozenPlan,
        },
        state: run.progress.state,
        state_revision: run.progress.revision,
        cleanup: run.progress.cleanup,
        result_available: run.result_body.is_some(),
        scope: DelegationRunScopeV1 {
            canonical_cwd: run.configuration.canonical_workspace_path.clone(),
            permission_policy: run.configuration.permission_policy,
            deadline_ms: run.deadline_ms,
        },
    }
}

impl LocalControlAdapter {
    fn task_view(
        &self,
        task: &DelegationTaskV1,
        run: &DelegationRunV1,
        now: u64,
    ) -> DelegationTaskViewV1 {
        let content = self.task_content_projection(task, now);
        self.task_view_with_content(task, run, now, content)
    }

    fn task_view_with_content(
        &self,
        task: &DelegationTaskV1,
        run: &DelegationRunV1,
        now: u64,
        content: TaskContent,
    ) -> DelegationTaskViewV1 {
        let (session_ids, session_links_complete) =
            self.observation_sessions(&task.workspace_id, &run.run_id);
        DelegationTaskViewV1 {
            task_id: task.task_id.clone(),
            created_at_ms: task.created_at_ms,
            latest_admission_sequence: task.latest_admission_sequence,
            title: content.title,
            brief: content.brief,
            content_availability: content.availability,
            plan_id: task.plan.plan_id.clone(),
            plan_revision: task.plan.plan_revision,
            latest_run_id: task.latest_run_id.clone(),
            run: run_view(task, run),
            resumable_until_ms: (run.progress.state == RunStateV1::Succeeded
                && run.progress.cleanup == hiroute_domain::delegation::RunCleanupV1::Complete
                && run.result_body.is_some()
                && !run.result_incomplete
                && task.resume_until_ms > now
                && task
                    .session
                    .as_ref()
                    .and_then(|session| session.native_session_id.as_ref())
                    .is_some()
                && !task.native_history_paths.is_empty())
            .then_some(task.resume_until_ms),
            session_ids,
            session_links_complete,
        }
    }

    /// Re-read the initial task Goal through managed-text visibility on every query. The
    /// projection is response-only: it is never cached or copied into runtime storage, and a
    /// continuation body cannot replace a hidden initial objective.
    pub(super) fn task_content_projection(
        &self,
        task: &DelegationTaskV1,
        now_ms: u64,
    ) -> TaskContent {
        let Some(body) = task.body_refs.first() else {
            return TaskContent::unavailable();
        };
        let Ok(now_ms) = i64::try_from(now_ms) else {
            return TaskContent::indeterminate();
        };
        let scope = ManagedTextScope {
            workspace_id: task.workspace_id.clone(),
            task_id: task.task_id.clone(),
            run_id: body.scope_run_id.clone(),
        };
        let reference = ManagedTextRef {
            opaque_id: body.opaque_id.clone(),
            scope: scope.clone(),
            visibility_generation: body.visibility_generation,
            original_retention_deadline_ms: body.original_retention_deadline_ms,
            state: ManagedTextState::Complete,
        };
        let page = match self.delegation_observation.managed_text_read(
            &scope,
            &reference,
            0,
            MAX_DELEGATION_INPUT_BYTES / CHUNK_BYTES,
            now_ms,
        ) {
            Ok(page) => page,
            Err(ManagedTextError::Unavailable) => return TaskContent::unavailable(),
            Err(_) => return TaskContent::indeterminate(),
        };
        if page.reference.state != ManagedTextState::Complete || page.next_chunk.is_some() {
            return TaskContent::indeterminate();
        }
        let Ok(prompt) = std::str::from_utf8(&page.bytes) else {
            return TaskContent::indeterminate();
        };
        let Some(brief) = project_initial_goal(prompt) else {
            return TaskContent::indeterminate();
        };
        TaskContent {
            title: task.title.as_ref().map(|title| title.value.clone()),
            brief: Some(brief),
            availability: DelegationContentAvailabilityV1::Available,
        }
    }

    /// Reads the same confirmed run/request relation used by the Sessions queries. Observation
    /// unavailability does not hide task state; it is represented as an incomplete link set.
    fn observation_sessions(&self, workspace: &WorkspaceId, run_id: &str) -> (Vec<String>, bool) {
        let read = || -> rusqlite::Result<Vec<String>> {
            let connection = Connection::open_with_flags(
                &self.observation_activity_path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            let mut statement = connection.prepare(
                "SELECT r.session_id
                 FROM logical_requests r
                 JOIN observation_run_links l
                   ON l.workspace_id=r.workspace_id AND l.request_id=r.request_id
                 JOIN sessions s
                   ON s.workspace_id=r.workspace_id AND s.session_id=r.session_id
                 WHERE r.workspace_id=?1 AND l.run_id=?2 AND l.conflicted=0
                   AND (s.tombstone_reason IS NULL OR EXISTS(
                     SELECT 1 FROM observation_tombstones t
                     WHERE t.workspace_id=r.workspace_id AND t.session_id=r.session_id
                       AND t.delete_scope='content_only'))
                 GROUP BY r.session_id
                 ORDER BY MAX(r.started_at_ms) DESC, r.session_id
                 LIMIT 17",
            )?;
            statement
                .query_map(params![workspace.as_str(), run_id], |row| row.get(0))?
                .collect()
        };
        match read() {
            Ok(mut sessions) => {
                let complete = sessions.len() <= 16;
                sessions.truncate(16);
                (sessions, complete)
            }
            Err(_) => (Vec::new(), false),
        }
    }
}

pub(super) struct TaskContent {
    pub(super) title: Option<String>,
    pub(super) brief: Option<String>,
    pub(super) availability: DelegationContentAvailabilityV1,
}

impl TaskContent {
    fn unavailable() -> Self {
        Self {
            title: None,
            brief: None,
            availability: DelegationContentAvailabilityV1::Unavailable,
        }
    }

    fn indeterminate() -> Self {
        Self {
            title: None,
            brief: None,
            availability: DelegationContentAvailabilityV1::Indeterminate,
        }
    }
}

fn project_initial_goal(prompt: &str) -> Option<String> {
    let rendered_goal = prompt.strip_prefix("Goal:\n")?;
    let goal = rendered_goal
        .split_once("\n\nContext:\n")
        .map_or(rendered_goal, |(goal, _)| goal);
    let brief = collapse_whitespace(goal);
    if brief.is_empty() {
        return None;
    }
    Some(bounded_chars(&brief, TASK_BRIEF_CHARS))
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn bounded_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

pub(super) fn authorize(
    caller: &DelegationCallerContext,
    task: &DelegationTaskV1,
    run: &DelegationRunV1,
) -> Result<(), DelegationErrorV1> {
    if task.workspace_id != *caller.workspace_id()
        || run.workspace_id != *caller.workspace_id()
        || task.task_id != run.task_id
        || task.latest_admission_sequence < run.admission_sequence
    {
        return Err(DelegationErrorV1::PermissionDenied);
    }
    Ok(())
}

fn terminal(state: RunStateV1) -> bool {
    matches!(
        state,
        RunStateV1::Succeeded | RunStateV1::Failed | RunStateV1::Cancelled | RunStateV1::Unknown
    )
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WorkerListCursorPayloadV1 {
    workspace_id: String,
    before_sequence: u64,
    filter_digest: String,
}

impl LocalControlAdapter {
    fn list_cursor(
        &self,
        workspace: &WorkspaceId,
        filter_digest: &str,
        before_sequence: u64,
    ) -> Result<String, DelegationErrorV1> {
        self.encode_worker_cursor(
            WORKER_LIST_CURSOR_PURPOSE,
            WorkerListCursorPayloadV1 {
                workspace_id: workspace.as_str().to_owned(),
                before_sequence,
                filter_digest: filter_digest.to_owned(),
            },
        )
    }

    fn parse_list_cursor(
        &self,
        workspace: &WorkspaceId,
        filter_digest: &str,
        cursor: &str,
    ) -> Result<u64, DelegationErrorV1> {
        let parsed: WorkerListCursorPayloadV1 =
            self.decode_worker_cursor(WORKER_LIST_CURSOR_PURPOSE, cursor)?;
        if parsed.workspace_id != workspace.as_str()
            || parsed.before_sequence == 0
            || parsed.filter_digest != filter_digest
        {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        Ok(parsed.before_sequence)
    }
}

fn now_ms() -> Result<u64, DelegationErrorV1> {
    let value = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DelegationErrorV1::StorageUnavailable)?
        .as_millis();
    u64::try_from(value).map_err(|_| DelegationErrorV1::StorageUnavailable)
}
