//! Bounded retention maintenance for task metadata, continuation holds, and owned native roots.
//!
//! Observation jobs are discovery signals only.  Runtime ownership/use records and a short
//! finalization + Plan-gate CAS are the deletion authority.

use std::collections::{BTreeSet, HashSet};

use hiroute_application::publication::admission::{AdmissionAction, AdmissionSubject};
use hiroute_application::publication::versions::ExactPlanVersionPort;
use hiroute_domain::CanonicalDigest;
use hiroute_domain::delegation::{
    DelegationBodyRefV1, DelegationContinuationReleaseV1, DelegationErrorV1,
    DelegationNativeCleanupClaimV1, DelegationNativeCleanupJobV1, DelegationNativeRootSnapshotV1,
    DelegationNativeRootStateV1, DelegationNativeUseStateV1, DelegationRuntimePort,
    DelegationTaskMaintenanceCursorV1, DelegationTaskV1,
};
use hiroute_observation::maintenance::{
    ObservationMaintenanceHook, ObservationMaintenanceHookError,
};
use hiroute_observation::managed_text::{
    ManagedTextError, ManagedTextNativeCleanup, ManagedTextNativeCleanupCursor, ManagedTextRef,
    ManagedTextScope, ManagedTextState,
};

use crate::delegation::native_cleanup::{NativeDeletionOutcome, delete_native_root_batch};

use super::{LocalControlAdapter, delegation_continue::task_owner};

const PAGE_SIZE: u16 = 16;
const DELETE_BUDGET: usize = 64;

impl ObservationMaintenanceHook for LocalControlAdapter {
    fn cycle(&self, now_ms: i64) -> Result<(), ObservationMaintenanceHookError> {
        let mut failed = false;
        failed |= self.retry_continuation_releases().is_err();
        failed |= self.scan_task_maintenance(now_ms).is_err();
        failed |= self.scan_native_cleanup(now_ms).is_err();
        failed |= self.scan_subscription_maintenance(now_ms).is_err();
        failed |= self.upgrade_cleanup.cycle().is_err();
        if failed {
            Err(ObservationMaintenanceHookError)
        } else {
            Ok(())
        }
    }
}

impl LocalControlAdapter {
    fn retry_continuation_releases(&self) -> Result<(), DelegationErrorV1> {
        let mut failed = false;
        for release in DelegationRuntimePort::pending_continuation_releases(self, PAGE_SIZE)? {
            failed |= self.finish_continuation_release(&release).is_err();
        }
        if failed {
            Err(DelegationErrorV1::StorageUnavailable)
        } else {
            Ok(())
        }
    }

    fn scan_task_maintenance(&self, now_ms: i64) -> Result<(), DelegationErrorV1> {
        if now_ms < 0 {
            return Err(DelegationErrorV1::StorageUnavailable);
        }
        let after = self
            .delegation_task_maintenance_cursor
            .lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?
            .clone();
        let tasks = DelegationRuntimePort::maintenance_tasks(self, after.as_ref(), PAGE_SIZE)?;
        let next = tasks.last().map(|task| DelegationTaskMaintenanceCursorV1 {
            workspace_id: task.workspace_id.clone(),
            task_id: task.task_id.clone(),
        });
        *self
            .delegation_task_maintenance_cursor
            .lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)? = next;

        let now = u64::try_from(now_ms).map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        for task in tasks {
            if task.resume_until_ms > 0 {
                let expired = task.resume_until_ms <= now;
                let bodies_visible = if expired {
                    true
                } else {
                    self.required_bodies_visible(&task, now_ms)?
                };
                if expired || !bodies_visible {
                    self.claim_and_finish_continuation_release(&task)?;
                }
            }

            let current = DelegationRuntimePort::task(self, &task.workspace_id, &task.task_id)?;
            if let Some(current) = current
                && self.title_body_hidden(&current, now_ms)?
            {
                let _finalization = self.delegation_finalization.acquire();
                let _ = DelegationRuntimePort::clear_task_title(self, &current);
            }
        }
        Ok(())
    }

    fn required_bodies_visible(
        &self,
        task: &DelegationTaskV1,
        now_ms: i64,
    ) -> Result<bool, DelegationErrorV1> {
        if task.body_refs.is_empty()
            || task.required_body_ids.len() != task.body_refs.len()
            || task
                .body_refs
                .iter()
                .zip(&task.required_body_ids)
                .any(|(body, id)| &body.opaque_id != id)
        {
            return Ok(false);
        }
        for body in &task.body_refs {
            if !self.body_visible(task, body, now_ms)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn body_visible(
        &self,
        task: &DelegationTaskV1,
        body: &DelegationBodyRefV1,
        now_ms: i64,
    ) -> Result<bool, DelegationErrorV1> {
        let scope = body_scope(task, body);
        let reference = managed_reference(&scope, body);
        match self
            .delegation_observation
            .managed_text_resolve(&scope, &reference, now_ms)
        {
            Ok(current) => Ok(matches!(
                current.state,
                ManagedTextState::Pending | ManagedTextState::Complete
            )),
            Err(ManagedTextError::Unavailable) => Ok(false),
            Err(_) => Err(DelegationErrorV1::StorageUnavailable),
        }
    }

    fn title_body_hidden(
        &self,
        task: &DelegationTaskV1,
        now_ms: i64,
    ) -> Result<bool, DelegationErrorV1> {
        let Some(title) = task.title.as_ref() else {
            return Ok(false);
        };
        self.body_visible(task, &title.initial_body_ref, now_ms)
            .map(|visible| !visible)
    }

    fn claim_and_finish_continuation_release(
        &self,
        expected: &DelegationTaskV1,
    ) -> Result<(), DelegationErrorV1> {
        let _finalization = self.delegation_finalization.acquire();
        let scope = BTreeSet::from([AdmissionSubject::Plan(expected.plan.plan_id.clone())]);
        let action = stable_action_id("hold-release", &expected.task_id, &expected.latest_run_id);
        let _guard = self
            .plan_admission
            .enter(
                &expected.workspace_id,
                &scope,
                AdmissionAction::Continue,
                &action,
            )
            .map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
        let current = DelegationRuntimePort::task(self, &expected.workspace_id, &expected.task_id)?
            .ok_or(DelegationErrorV1::Conflict)?;
        if &current != expected {
            return Ok(());
        }
        let release = DelegationRuntimePort::claim_continuation_release(self, expected)?;
        self.release_continuation_hold(&release)
    }

    fn finish_continuation_release(
        &self,
        release: &DelegationContinuationReleaseV1,
    ) -> Result<(), DelegationErrorV1> {
        let _finalization = self.delegation_finalization.acquire();
        let scope = BTreeSet::from([AdmissionSubject::Plan(release.task.plan.plan_id.clone())]);
        let action = stable_action_id("hold-release", &release.task_id, &release.latest_run_id);
        let _guard = self
            .plan_admission
            .enter(
                &release.workspace_id,
                &scope,
                AdmissionAction::Continue,
                &action,
            )
            .map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
        let current = DelegationRuntimePort::task(self, &release.workspace_id, &release.task_id)?
            .ok_or(DelegationErrorV1::Conflict)?;
        if current.latest_run_id != release.latest_run_id
            || current.plan != release.task.plan
            || current.resume_until_ms != 0
        {
            return Err(DelegationErrorV1::Conflict);
        }
        self.release_continuation_hold(release)
    }

    fn release_continuation_hold(
        &self,
        release: &DelegationContinuationReleaseV1,
    ) -> Result<(), DelegationErrorV1> {
        ExactPlanVersionPort::release(self, &release.workspace_id, &task_owner(&release.task))
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        DelegationRuntimePort::complete_continuation_release(self, release)
    }

    fn scan_native_cleanup(&self, now_ms: i64) -> Result<(), DelegationErrorV1> {
        if now_ms < 0 {
            return Err(DelegationErrorV1::StorageUnavailable);
        }
        let after = self
            .delegation_native_cleanup_cursor
            .lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?
            .clone();
        let jobs = self
            .delegation_observation
            .managed_text_pending_native_cleanup_page(after.as_ref(), usize::from(PAGE_SIZE))
            .map_err(map_observation)?;
        let next = jobs.last().map(|job| ManagedTextNativeCleanupCursor {
            scope: job.scope.clone(),
            visibility_generation: job.visibility_generation,
        });
        *self
            .delegation_native_cleanup_cursor
            .lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)? = next;

        let mut handled = HashSet::new();
        let mut remaining = DELETE_BUDGET;
        for job in jobs {
            let key = (job.scope.workspace_id.clone(), job.scope.task_id.clone());
            if handled.insert(key) {
                self.process_native_root_for_job(&job, now_ms, &mut remaining)?;
            }
        }
        Ok(())
    }

    fn process_native_root_for_job(
        &self,
        job: &ManagedTextNativeCleanup,
        now_ms: i64,
        remaining: &mut usize,
    ) -> Result<(), DelegationErrorV1> {
        let Some(snapshot) =
            DelegationRuntimePort::native_root(self, &job.scope.workspace_id, &job.scope.task_id)?
        else {
            return Ok(());
        };
        if !snapshot.uses.iter().any(|usage| {
            usage.workspace_id == job.scope.workspace_id
                && usage.task_id == job.scope.task_id
                && usage.scope_run_id == job.scope.run_id
                && usage.root_generation == snapshot.root.root_generation
        }) {
            return Ok(());
        }
        match snapshot.root.state {
            DelegationNativeRootStateV1::Creating | DelegationNativeRootStateV1::Unknown => Ok(()),
            DelegationNativeRootStateV1::Deleting => {
                self.drive_native_deletion(&snapshot, now_ms, remaining)
            }
            DelegationNativeRootStateV1::Removed => {
                self.ack_claim_jobs(&snapshot)?;
                if self.native_scope_is_fully_hidden(&snapshot, job, now_ms)? {
                    self.delegation_observation
                        .managed_text_native_gc_ack_exact(job)
                        .map_err(map_observation)?;
                }
                Ok(())
            }
            DelegationNativeRootStateV1::Ready | DelegationNativeRootStateV1::NoNative => {
                let Some((task, jobs)) = self.native_cleanup_coverage(&snapshot, now_ms)? else {
                    return Ok(());
                };
                let claim_id = stable_action_id(
                    "native-cleanup",
                    &snapshot.root.task_id,
                    &snapshot.root.use_revision.to_string(),
                );
                let claim = DelegationNativeCleanupClaimV1 {
                    claim_id: claim_id.clone(),
                    root_generation: snapshot.root.root_generation,
                    expected_use_revision: snapshot.root.use_revision,
                    checked_at_ms: now_ms,
                    jobs,
                };
                let claimed = {
                    let _finalization = self.delegation_finalization.acquire();
                    let scope = BTreeSet::from([AdmissionSubject::Plan(task.plan.plan_id.clone())]);
                    let _guard = self
                        .plan_admission
                        .enter(
                            &task.workspace_id,
                            &scope,
                            AdmissionAction::Continue,
                            &claim_id,
                        )
                        .map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
                    let current =
                        DelegationRuntimePort::task(self, &task.workspace_id, &task.task_id)?;
                    if current.as_ref() != Some(&task) {
                        return Ok(());
                    }
                    DelegationRuntimePort::claim_native_cleanup(
                        self,
                        &task.workspace_id,
                        &task.task_id,
                        &claim,
                    )?
                };
                self.drive_native_deletion(&claimed, now_ms, remaining)
            }
        }
    }

    fn native_cleanup_coverage(
        &self,
        snapshot: &DelegationNativeRootSnapshotV1,
        now_ms: i64,
    ) -> Result<Option<(DelegationTaskV1, Vec<DelegationNativeCleanupJobV1>)>, DelegationErrorV1>
    {
        // Acceptance caps task body refs at 16; Start has at least its titled input ref and every
        // Continue appends exactly one new run-scoped input ref. Consequently a task has at most
        // 16 uses/scopes; this pass cannot reach the claim's wider defensive 200-job limit.
        if snapshot.uses.is_empty() || snapshot.uses.iter().any(|usage| !usage.state.ended()) {
            return Ok(None);
        }
        let Some(task) =
            DelegationRuntimePort::task(self, &snapshot.root.workspace_id, &snapshot.root.task_id)?
        else {
            return Ok(None);
        };
        let mut claim_jobs = Vec::with_capacity(snapshot.uses.len());
        for usage in &snapshot.uses {
            let Some(run) = DelegationRuntimePort::run(self, &usage.workspace_id, &usage.run_id)?
            else {
                return Ok(None);
            };
            if run.task_id != usage.task_id
                || run.lease_id != usage.lease_id
                || run.daemon_epoch != usage.daemon_epoch
                || run.accepted_at_ms != Some(usage.accepted_at_ms)
                || match usage.state {
                    DelegationNativeUseStateV1::Stopped => {
                        run.stop_evidence.is_none_or(|evidence| {
                            !evidence.scope_stopped || evidence.residual_unknown
                        })
                    }
                    DelegationNativeUseStateV1::NeverSpawned => run.process.is_some(),
                    DelegationNativeUseStateV1::Accepted
                    | DelegationNativeUseStateV1::MayHaveSpawned => true,
                }
            {
                return Ok(None);
            }
            let scope = ManagedTextScope {
                workspace_id: usage.workspace_id.clone(),
                task_id: usage.task_id.clone(),
                run_id: usage.scope_run_id.clone(),
            };
            let mut ids = task
                .body_refs
                .iter()
                .filter(|body| body.scope_run_id == usage.scope_run_id)
                .map(|body| body.opaque_id.clone())
                .collect::<Vec<_>>();
            if let Some(result) = run.result_body.as_ref()
                && !ids.contains(&result.opaque_id)
            {
                ids.push(result.opaque_id.clone());
            }
            if ids.is_empty()
                || !self
                    .delegation_observation
                    .managed_text_scope_contains_refs(&scope, &ids)
                    .map_err(map_observation)?
            {
                return Ok(None);
            }
            let Some(state) = self
                .delegation_observation
                .managed_text_scope_cleanup_state(&scope, now_ms)
                .map_err(map_observation)?
            else {
                return Ok(None);
            };
            let accepted = i64::try_from(usage.accepted_at_ms)
                .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
            let cutoff = state.latest_created_ms.unwrap_or(accepted).max(accepted);
            if state.visible_reference_count != 0 || state.deleted_through_ms < cutoff {
                return Ok(None);
            }
            let jobs = self
                .delegation_observation
                .managed_text_pending_native_cleanup(&scope, 0, 200)
                .map_err(map_observation)?;
            let Some(supporting) = jobs
                .into_iter()
                .filter(|job| job.through_ms >= cutoff)
                .max_by_key(|job| job.visibility_generation)
            else {
                return Ok(None);
            };
            claim_jobs.push(native_job(&supporting));
        }
        Ok(Some((task, claim_jobs)))
    }

    fn native_scope_is_fully_hidden(
        &self,
        snapshot: &DelegationNativeRootSnapshotV1,
        job: &ManagedTextNativeCleanup,
        now_ms: i64,
    ) -> Result<bool, DelegationErrorV1> {
        let Some(usage) = snapshot
            .uses
            .iter()
            .find(|usage| usage.scope_run_id == job.scope.run_id)
        else {
            return Ok(false);
        };
        let Some(state) = self
            .delegation_observation
            .managed_text_scope_cleanup_state(&job.scope, now_ms)
            .map_err(map_observation)?
        else {
            return Ok(false);
        };
        let accepted = i64::try_from(usage.accepted_at_ms)
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        let cutoff = state.latest_created_ms.unwrap_or(accepted).max(accepted);
        Ok(state.visible_reference_count == 0
            && state.deleted_through_ms >= cutoff
            && job.through_ms >= cutoff)
    }

    fn drive_native_deletion(
        &self,
        snapshot: &DelegationNativeRootSnapshotV1,
        now_ms: i64,
        remaining: &mut usize,
    ) -> Result<(), DelegationErrorV1> {
        let claim = snapshot
            .root
            .cleanup_claim
            .as_ref()
            .ok_or(DelegationErrorV1::StorageUnavailable)?;
        if snapshot.root.managed_base_path.is_none() && snapshot.root.filesystem_identity.is_none()
        {
            let removed = DelegationRuntimePort::complete_native_cleanup(
                self,
                &snapshot.root.workspace_id,
                &snapshot.root.task_id,
                snapshot.root.root_generation,
                &claim.claim_id,
            )?;
            return self.ack_claim_jobs(&removed);
        }
        if *remaining == 0 {
            return Ok(());
        }
        let outcome = delete_native_root_batch(&snapshot.root, *remaining);
        let processed_entries = match outcome {
            NativeDeletionOutcome::Deferred { processed_entries }
            | NativeDeletionOutcome::Removed { processed_entries }
            | NativeDeletionOutcome::Blocked {
                processed_entries, ..
            } => processed_entries,
        };
        *remaining = remaining.saturating_sub(processed_entries);
        if let NativeDeletionOutcome::Blocked { kind, .. } = outcome {
            DelegationRuntimePort::record_native_cleanup_failure(
                self,
                &snapshot.root.workspace_id,
                &snapshot.root.task_id,
                snapshot.root.root_generation,
                &claim.claim_id,
                kind,
                now_ms,
            )?;
            return Ok(());
        }
        if processed_entries > 0 {
            DelegationRuntimePort::record_native_cleanup_batch(
                self,
                &snapshot.root.workspace_id,
                &snapshot.root.task_id,
                snapshot.root.root_generation,
                &claim.claim_id,
            )?;
        }
        if matches!(outcome, NativeDeletionOutcome::Removed { .. }) {
            let removed = DelegationRuntimePort::complete_native_cleanup(
                self,
                &snapshot.root.workspace_id,
                &snapshot.root.task_id,
                snapshot.root.root_generation,
                &claim.claim_id,
            )?;
            self.ack_claim_jobs(&removed)?;
        }
        Ok(())
    }

    fn ack_claim_jobs(
        &self,
        snapshot: &DelegationNativeRootSnapshotV1,
    ) -> Result<(), DelegationErrorV1> {
        if snapshot.root.state != DelegationNativeRootStateV1::Removed {
            return Err(DelegationErrorV1::Conflict);
        }
        let claim = snapshot
            .root
            .cleanup_claim
            .as_ref()
            .ok_or(DelegationErrorV1::StorageUnavailable)?;
        for job in &claim.jobs {
            self.delegation_observation
                .managed_text_native_gc_ack_exact(&ManagedTextNativeCleanup {
                    scope: ManagedTextScope {
                        workspace_id: job.workspace_id.clone(),
                        task_id: job.task_id.clone(),
                        run_id: job.run_id.clone(),
                    },
                    visibility_generation: job.visibility_generation,
                    through_ms: job.through_ms,
                })
                .map_err(map_observation)?;
        }
        Ok(())
    }
}

fn body_scope(task: &DelegationTaskV1, body: &DelegationBodyRefV1) -> ManagedTextScope {
    ManagedTextScope {
        workspace_id: task.workspace_id.clone(),
        task_id: task.task_id.clone(),
        run_id: body.scope_run_id.clone(),
    }
}

fn managed_reference(scope: &ManagedTextScope, body: &DelegationBodyRefV1) -> ManagedTextRef {
    ManagedTextRef {
        opaque_id: body.opaque_id.clone(),
        scope: scope.clone(),
        visibility_generation: body.visibility_generation,
        original_retention_deadline_ms: body.original_retention_deadline_ms,
        state: ManagedTextState::Complete,
    }
}

fn native_job(job: &ManagedTextNativeCleanup) -> DelegationNativeCleanupJobV1 {
    DelegationNativeCleanupJobV1 {
        workspace_id: job.scope.workspace_id.clone(),
        task_id: job.scope.task_id.clone(),
        run_id: job.scope.run_id.clone(),
        visibility_generation: job.visibility_generation,
        through_ms: job.through_ms,
    }
}

fn stable_action_id(prefix: &str, first: &str, second: &str) -> String {
    let digest = CanonicalDigest::of_bytes(format!("{prefix}\0{first}\0{second}").as_bytes());
    format!("{prefix}/{}", digest.as_str().trim_start_matches("sha256:"))
}

fn map_observation(_: ManagedTextError) -> DelegationErrorV1 {
    DelegationErrorV1::StorageUnavailable
}

#[cfg(all(test, unix))]
#[path = "delegation_maintenance_tests.rs"]
mod tests;
