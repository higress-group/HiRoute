use std::collections::BTreeSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use hiroute_application::publication::admission::{AdmissionAction, AdmissionSubject};
use hiroute_domain::delegation::{DelegationErrorV1, DelegationRunV1, RunCleanupV1, RunStateV1};
use hiroute_domain::{
    PlanExecutionRef, PlanLifecycleV1, VersionOwnerKindV1, VersionOwnerPurposeV1,
    VersionOwnerRefV1, VersionReservationV1,
};
use hiroute_observation::managed_text::RETENTION_MS;

use super::DelegationRunExecutor;
use crate::delegation::profile::native_history;

impl DelegationRunExecutor {
    pub(super) fn retain_continuation(
        &self,
        run: &DelegationRunV1,
        session_root: &Path,
    ) -> Result<(), DelegationErrorV1> {
        if run.progress.state != RunStateV1::Succeeded
            || run.progress.cleanup != RunCleanupV1::Complete
            || run.result_incomplete
        {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
        let mut task = self
            .runtime
            .task(&run.workspace_id, &run.task_id)?
            .ok_or(DelegationErrorV1::StorageUnavailable)?;
        if task.latest_run_id != run.run_id
            || task
                .session
                .as_ref()
                .and_then(|session| session.native_session_id.as_ref())
                .is_none()
        {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
        let result = run
            .result_body
            .clone()
            .ok_or(DelegationErrorV1::ResumeUnavailable)?;
        if !task
            .body_refs
            .iter()
            .any(|reference| reference.opaque_id == result.opaque_id)
        {
            if task.body_refs.len() >= 16 {
                return Err(DelegationErrorV1::ResumeUnavailable);
            }
            task.body_refs.push(result);
        }
        let native_session_id = task
            .session
            .as_ref()
            .and_then(|session| session.native_session_id.as_deref())
            .ok_or(DelegationErrorV1::ResumeUnavailable)?;
        // These are owned continuation materials. A borrowed Codex/Qoder binding is an exact-ID
        // witness, not a rollout path or proof that the native client can still load its history.
        // Body visibility/retention remains authoritative even while native history survives.
        let continuation_materials =
            native_history(session_root, task.plan.harness, native_session_id)?;
        let now = now_ms()?;
        let retention =
            u64::try_from(RETENTION_MS).map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        let body_deadline = task
            .body_refs
            .iter()
            .map(|reference| reference.original_retention_deadline_ms)
            .min()
            .and_then(|value| u64::try_from(value).ok())
            .ok_or(DelegationErrorV1::ResumeUnavailable)?;
        let until = now.saturating_add(retention).min(body_deadline);
        if until <= now {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
        let scope = BTreeSet::from([AdmissionSubject::Plan(task.plan.plan_id.clone())]);
        let action = format!("resume/{}", run.run_id);
        let guard = self
            .gate
            .enter(
                &run.workspace_id,
                &scope,
                AdmissionAction::Continue,
                &action,
            )
            .map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
        let current = self
            .versions
            .current_plan(&guard, &task.plan.plan_id)
            .map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
        if current.status != PlanLifecycleV1::Enabled {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
        let owner = task_owner(&task.task_id);
        let reservation = VersionReservationV1 {
            owner: owner.clone(),
            reference: PlanExecutionRef {
                workspace_id: task.workspace_id.clone(),
                plan_id: task.plan.plan_id.clone(),
                content_revision: task.plan.plan_revision,
                content_digest: task.plan.plan_digest.clone(),
            },
            expires_at_unix: deadline_seconds(until)?,
        };
        let version = self
            .versions
            .acquire_exact(&guard, &reservation)
            .map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
        if version.reference != reservation.reference {
            let _ = self.versions.release(&run.workspace_id, &owner);
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
        task.required_body_ids = task
            .body_refs
            .iter()
            .map(|reference| reference.opaque_id.clone())
            .collect();
        let stored = self.runtime.set_resume_materials(
            &run.workspace_id,
            &run.task_id,
            &run.run_id,
            until,
            &task.body_refs,
            &continuation_materials,
        );
        if stored.is_err() {
            let _ = self.versions.release(&run.workspace_id, &owner);
        }
        stored
    }
}

fn task_owner(task_id: &str) -> VersionOwnerRefV1 {
    VersionOwnerRefV1 {
        kind: VersionOwnerKindV1::Task,
        owner_id: format!("delegation-task/{task_id}"),
        purpose: VersionOwnerPurposeV1::Continuation,
    }
}

fn deadline_seconds(deadline_ms: u64) -> Result<i64, DelegationErrorV1> {
    let rounded = deadline_ms
        .checked_add(999)
        .ok_or(DelegationErrorV1::DeadlineExceeded)?
        / 1_000;
    i64::try_from(rounded).map_err(|_| DelegationErrorV1::DeadlineExceeded)
}

fn now_ms() -> Result<u64, DelegationErrorV1> {
    let value = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DelegationErrorV1::StorageUnavailable)?
        .as_millis();
    u64::try_from(value).map_err(|_| DelegationErrorV1::StorageUnavailable)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use hiroute_domain::delegation::WorkerHarnessV1;
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn private_root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        root
    }

    #[test]
    fn codex_inventory_selects_exact_rollout_and_ignores_transient_wrapper_links() {
        let root = private_root();
        let history = root
            .path()
            .join("sessions/2026/09/07/rollout-2026-09-07T00-00-00-native-a.jsonl");
        fs::create_dir_all(history.parent().unwrap()).unwrap();
        fs::write(&history, "history").unwrap();
        let transient = root.path().join(".tmp/run");
        fs::create_dir_all(&transient).unwrap();
        symlink(&history, transient.join("codex-execve-wrapper")).unwrap();

        assert_eq!(
            native_history(root.path(), WorkerHarnessV1::CodexCli, "native-a").unwrap(),
            vec!["sessions/2026/09/07/rollout-2026-09-07T00-00-00-native-a.jsonl"]
        );
    }

    #[test]
    fn exact_native_history_still_rejects_links_and_ambiguous_sessions() {
        let root = private_root();
        let first = root
            .path()
            .join("sessions/2026/09/07/rollout-2026-09-07T00-00-00-native-a.jsonl");
        fs::create_dir_all(first.parent().unwrap()).unwrap();
        fs::write(&first, "history").unwrap();
        let second = root
            .path()
            .join("sessions/2026/09/08/rollout-2026-09-08T00-00-00-native-a.jsonl");
        fs::create_dir_all(second.parent().unwrap()).unwrap();
        fs::write(&second, "history").unwrap();
        assert_eq!(
            native_history(root.path(), WorkerHarnessV1::CodexCli, "native-a"),
            Err(DelegationErrorV1::ResumeUnavailable)
        );

        fs::remove_file(&second).unwrap();
        fs::remove_file(&first).unwrap();
        symlink(root.path().join("outside"), &first).unwrap();
        assert_eq!(
            native_history(root.path(), WorkerHarnessV1::CodexCli, "native-a"),
            Err(DelegationErrorV1::ResumeUnavailable)
        );
    }

    #[test]
    fn claude_inventory_requires_the_exact_project_session_name() {
        let root = private_root();
        let history = root.path().join("projects/workspace/native-a.jsonl");
        fs::create_dir_all(history.parent().unwrap()).unwrap();
        fs::write(&history, "history").unwrap();
        fs::write(history.parent().unwrap().join("native-b.jsonl"), "other").unwrap();

        assert_eq!(
            native_history(root.path(), WorkerHarnessV1::ClaudeCode, "native-a").unwrap(),
            vec!["projects/workspace/native-a.jsonl"]
        );
    }
}
