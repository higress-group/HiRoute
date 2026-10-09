//! Explicit Desktop recovery through the authenticated owner-only local channel.
use hiroute_application::delegation::control::{
    DelegationControl, DelegationControlAccess, DelegationControlAction, DelegationRunDenial,
};
use hiroute_application_api::{DelegationRunViewV1, WorkerResidualConfirmRequestV1};
use hiroute_domain::delegation::{
    DelegationErrorV1, DelegationRunV1, DelegationRuntimePort, DelegationTaskV1, RunCleanupV1,
    RunEventV1,
};
use hiroute_domain::{CanonicalDigest, IdempotencyScopeV1, OperationId};

use super::{
    LocalControlAdapter,
    delegation_task_queries::{authorize, run_view},
    delegation_worker::DelegationCallerContext,
};

struct LocalConfirmation<'a> {
    adapter: &'a LocalControlAdapter,
    caller: &'a DelegationCallerContext,
    task: &'a DelegationTaskV1,
    expected_revision: u64,
}

impl DelegationControlAccess for LocalConfirmation<'_> {
    fn authorize(
        &self,
        run: &DelegationRunV1,
        action: DelegationControlAction,
    ) -> Result<String, DelegationErrorV1> {
        if action != DelegationControlAction::ConfirmResidual {
            return Err(DelegationErrorV1::PermissionDenied);
        }
        authorize(self.caller, self.task, run)?;
        // Reject a stale or known-live action before denying execution. An already
        // acknowledged run still reaches the store's exact event replay check.
        if run.progress.cleanup != RunCleanupV1::ResidualAcknowledged {
            if run.progress.revision != self.expected_revision {
                return Err(DelegationErrorV1::Conflict);
            }
            let mut progress = run.progress.clone();
            progress.advance(RunEventV1::ResidualAcknowledged)?;
        }
        Ok("local-user".into())
    }
}

impl DelegationRunDenial for LocalConfirmation<'_> {
    fn deny(&self, run: &DelegationRunV1) -> Result<(), DelegationErrorV1> {
        self.adapter.delegation_run_authority.deny_run(run)
    }
}

impl LocalControlAdapter {
    pub(super) fn confirm_delegation_residual(
        &self,
        request: &WorkerResidualConfirmRequestV1,
    ) -> Result<DelegationRunViewV1, DelegationErrorV1> {
        if !request.valid() {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        let caller = self.worker_instance();
        let workspace = caller.workspace_id();
        let run = DelegationRuntimePort::run(self, workspace, &request.run_id)?
            .ok_or(DelegationErrorV1::NotFound)?;
        let task = DelegationRuntimePort::task(self, workspace, &run.task_id)?
            .ok_or(DelegationErrorV1::StorageUnavailable)?;
        let scope = IdempotencyScopeV1::new(
            "worker-instance",
            "WorkerConfirmResidual",
            &request.idempotency_key,
        )
        .map_err(|_| DelegationErrorV1::InvalidArguments)?;
        let digest = CanonicalDigest::of(&("worker-residual-confirm/v1", &scope, request))
            .map_err(|_| DelegationErrorV1::InvalidArguments)?;
        let operation = OperationId::derive(workspace, &scope, &digest);
        let confirmation = LocalConfirmation {
            adapter: self,
            caller: &caller,
            task: &task,
            expected_revision: request.expected_revision,
        };
        let updated = DelegationControl {
            runtime: self,
            access: &confirmation,
            denial: &confirmation,
        }
        .confirm_residual(
            workspace,
            &request.run_id,
            request.expected_revision,
            &operation,
        )?;
        Ok(run_view(&task, &updated))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{LocalControlDaemon, runtime::ProductionControlRuntime};
    use hiroute_application::ApplicationService;
    use hiroute_application_api::{LOCAL_CONTROL_SCHEMA_V2, LocalControlWireRequestV2};
    use hiroute_domain::{WorkspaceId, delegation::DelegationCheckpointV1};

    #[test]
    fn local_residual_confirmation_is_explicit_cas_and_replay_safe() {
        if crate::test_support::isolated_agent_home(
            "control::runtime::delegation_residual::tests::local_residual_confirmation_is_explicit_cas_and_replay_safe",
        ) {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let runtime = ProductionControlRuntime::open_with_release_catalog(
            root.path(),
            crate::release_catalog::fixture_catalog(),
        )
        .unwrap();
        let workspace = WorkspaceId::default();
        // This runtime starts maintenance with the real clock; synthetic epoch
        // timestamps would race expiry while the fixture's body is being written.
        let now_ms = super::super::delegation_tasks::current_time_ms().unwrap();
        let body = runtime
            .adapter
            .persist_task_input(
                &workspace,
                "task-residual",
                "run-residual",
                "fixture",
                now_ms,
            )
            .unwrap();
        assert!(
            body.original_retention_deadline_ms > now_ms as i64,
            "residual fixture body is already expired under the runtime maintenance clock"
        );
        let acceptance = super::super::tests::worker_list_acceptance(
            "task-residual",
            "run-residual",
            body,
            now_ms,
        );
        let mut run = DelegationRuntimePort::accept(runtime.adapter.as_ref(), &acceptance).unwrap();
        let daemon = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()))
            .with_released_commands_only();
        let send = |request: &WorkerResidualConfirmRequestV1| {
            daemon.dispatch_wire(LocalControlWireRequestV2 {
                schema_version: LOCAL_CONTROL_SCHEMA_V2,
                request_id: "residual-test".into(),
                operation_id: "WorkerConfirmResidual".into(),
                payload: serde_json::to_value(request).unwrap(),
                protected_grant: None,
            })
        };
        let mut request = WorkerResidualConfirmRequestV1 {
            run_id: run.run_id.clone(),
            expected_revision: run.progress.revision,
            idempotency_key: "residual-test".into(),
            user_confirmed: true,
        };
        assert!(send(&request).error.is_some());
        assert!(
            !DelegationRuntimePort::run(runtime.adapter.as_ref(), &workspace, &run.run_id)
                .unwrap()
                .unwrap()
                .lease_revoked
        );
        for (id, event) in [
            ("live", RunEventV1::ProcessRunning),
            ("unknown", RunEventV1::ProcessUnknown),
        ] {
            run = DelegationRuntimePort::checkpoint(
                runtime.adapter.as_ref(),
                &workspace,
                &run.run_id,
                run.progress.revision,
                id,
                &DelegationCheckpointV1::Progress { event },
            )
            .unwrap();
            if id == "live" {
                request.expected_revision = run.progress.revision;
                assert!(send(&request).error.is_some());
                assert!(
                    !DelegationRuntimePort::run(runtime.adapter.as_ref(), &workspace, &run.run_id)
                        .unwrap()
                        .unwrap()
                        .lease_revoked
                );
            }
        }
        assert!(send(&request).error.is_some(), "stale revision must fail");
        request.expected_revision = run.progress.revision;
        request.user_confirmed = false;
        assert!(send(&request).error.is_some());
        request.user_confirmed = true;
        let first = send(&request);
        assert!(first.error.is_none(), "{first:?}");
        assert_eq!(
            first.data.as_ref().unwrap()["cleanup"],
            "residual_acknowledged"
        );
        assert_eq!(first.data.as_ref().unwrap()["state"], "failed");
        let replay = send(&request);
        assert!(replay.error.is_none(), "{replay:?}");
        assert_eq!(first.data, replay.data);
        let final_run =
            DelegationRuntimePort::run(runtime.adapter.as_ref(), &workspace, &run.run_id)
                .unwrap()
                .unwrap();
        assert!(final_run.progress.workspace_releasable());
        assert!(final_run.lease_revoked);
        assert_eq!(final_run.progress.revision, run.progress.revision + 1);
    }
}
