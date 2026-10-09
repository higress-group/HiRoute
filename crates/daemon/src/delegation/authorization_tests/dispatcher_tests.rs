use super::*;
use crate::delegation::{dispatcher::*, platform::*};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_util::sync::CancellationToken;

struct StopFixture {
    stops: AtomicUsize,
    unknown: bool,
}
#[async_trait]
impl WorkerPlatformPort for StopFixture {
    fn capabilities(
        &self,
        _: &crate::delegation::profile::CandidateWorkerProfile,
    ) -> Result<WorkerPlatformCapabilities, DelegationErrorV1> {
        Ok(WorkerPlatformCapabilities {
            can_start: true,
            can_stop: true,
        })
    }
    async fn launch(&self, _: WorkerLaunchRequest) -> Result<ReadyWorker, DelegationErrorV1> {
        panic!("cancel recovery must never spawn or prompt")
    }
    async fn observe(
        &self,
        _: &WorkerProcessIdentity,
    ) -> Result<WorkerObservation, DelegationErrorV1> {
        Ok(WorkerObservation::Unknown)
    }
    async fn terminate(
        &self,
        identity: &WorkerProcessIdentity,
        _: u64,
    ) -> Result<WorkerStopEvidence, DelegationErrorV1> {
        assert_eq!(identity.creation_identity, "owned-creation");
        self.stops.fetch_add(1, Ordering::SeqCst);
        if self.unknown {
            return Err(DelegationErrorV1::CapabilityUnavailable);
        }
        Ok(WorkerStopEvidence {
            scope: WorkerStopScope::ProcessGroup,
            observation: WorkerObservation::Exited { code: Some(0) },
            scope_stopped: true,
            residual_unknown: false,
        })
    }
}
pub(super) fn bind(stores: &LocalStorageSet, run: &DelegationRunV1) {
    let run = stores
        .runtime()
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "preparing",
            &DelegationCheckpointV1::Progress {
                event: RunEventV1::Preparing,
            },
        )
        .unwrap();
    stores
        .runtime()
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "bound",
            &DelegationCheckpointV1::ProcessSpawned {
                binding: DelegationProcessBindingV1 {
                    launch_nonce: run.launch_nonce.clone(),
                    handle_id: "owned-handle".into(),
                    creation_identity: "owned-creation".into(),
                },
            },
        )
        .unwrap();
}
#[tokio::test]
async fn durable_cancel_drives_exact_active_token_then_restart_stops_without_respawn() {
    let (dir, stores, _gate, _safety) = setup();
    let run = stores
        .runtime()
        .accept(&sample("task-a", "run-a", "root-a"))
        .unwrap();
    bind(&stores, &run);
    let dispatcher = DelegationCancellationDispatcher::default();
    let cancellation = CancellationToken::new();
    dispatcher
        .attach(
            stores.runtime(),
            &run.workspace_id,
            &run.run_id,
            cancellation.clone(),
        )
        .unwrap();
    let other = CancellationToken::new();
    let b = stores
        .runtime()
        .accept(&sample("task-b", "run-b", "root-b"))
        .unwrap();
    dispatcher
        .attach(stores.runtime(), &b.workspace_id, &b.run_id, other.clone())
        .unwrap();
    let operation = OperationId::parse("op_00112233445566778899aabbccddeeff").unwrap();
    stores
        .runtime()
        .request_cancel(&run.workspace_id, &run.run_id, &operation, "user")
        .unwrap();
    let platform = StopFixture {
        stops: AtomicUsize::new(0),
        unknown: false,
    };
    let statuses = dispatcher
        .dispatch_pending(stores.runtime(), &run.workspace_id, &platform)
        .await
        .unwrap();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].state, CancellationDispatchState::StopRequested);
    assert!(cancellation.is_cancelled());
    assert!(!other.is_cancelled());
    assert_eq!(platform.stops.load(Ordering::SeqCst), 0);
    assert!(
        dispatcher
            .detach(&run.workspace_id, &run.run_id, "wrong-lease")
            .is_err()
    );
    drop(dispatcher);
    drop(stores);
    let stores = LocalStorageSet::open_for_daemon_startup(dir.path().join("store")).unwrap();
    let recovered = DelegationCancellationDispatcher::default();
    let result = recovered
        .dispatch_pending(stores.runtime(), &run.workspace_id, &platform)
        .await
        .unwrap();
    assert_eq!(result[0].state, CancellationDispatchState::Stopped);
    let saved = stores
        .runtime()
        .run(&run.workspace_id, &run.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.progress.state, RunStateV1::Cancelled);
    assert_eq!(saved.progress.cleanup, RunCleanupV1::Complete);
    assert!(
        recovered
            .dispatch_pending(stores.runtime(), &run.workspace_id, &platform)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(platform.stops.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn absent_or_unverifiable_identity_never_claims_stop_and_late_attach_sees_intent() {
    let (_dir, stores, _gate, _safety) = setup();
    let a = stores
        .runtime()
        .accept(&sample("task-a", "run-a", "root-a"))
        .unwrap();
    let a = stores
        .runtime()
        .checkpoint(
            &a.workspace_id,
            &a.run_id,
            a.progress.revision,
            "preparing",
            &DelegationCheckpointV1::Progress {
                event: RunEventV1::Preparing,
            },
        )
        .unwrap();
    let b = stores
        .runtime()
        .accept(&sample("task-b", "run-b", "root-b"))
        .unwrap();
    bind(&stores, &b);
    let operation = OperationId::parse("op_00112233445566778899aabbccddeeff").unwrap();
    stores
        .runtime()
        .cancel_scope(
            &a.workspace_id,
            &operation,
            &DelegationAuthorizationScopeV1::Permit {
                id: a.permit_id.clone(),
                through_generation: 1,
            },
        )
        .unwrap();
    let dispatcher = DelegationCancellationDispatcher::default();
    let platform = StopFixture {
        stops: AtomicUsize::new(0),
        unknown: true,
    };
    let result = dispatcher
        .dispatch_pending(stores.runtime(), &a.workspace_id, &platform)
        .await
        .unwrap();
    assert!(
        result
            .iter()
            .any(|r| r.run_id == a.run_id && r.state == CancellationDispatchState::PendingIdentity)
    );
    assert!(result.iter().all(|r| r.run_id != b.run_id));
    let pending = stores
        .runtime()
        .run(&a.workspace_id, &a.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(pending.progress.state, RunStateV1::Cancelling);
    assert_eq!(pending.progress.cleanup, RunCleanupV1::Pending);
    assert_ne!(
        stores
            .runtime()
            .run(&b.workspace_id, &b.run_id)
            .unwrap()
            .unwrap()
            .progress
            .cleanup,
        RunCleanupV1::Complete
    );
    let token = CancellationToken::new();
    dispatcher
        .attach(stores.runtime(), &a.workspace_id, &a.run_id, token.clone())
        .unwrap();
    assert!(token.is_cancelled());
}
