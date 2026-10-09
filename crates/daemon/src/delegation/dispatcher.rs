//! Cancellation is replayed from existing run records. No queue database, prompt retry or
//! dispatch acknowledgement is added to the security-revocation completion condition.
use super::{lifecycle::stop_owned, platform::*};
use hiroute_domain::delegation::*;
use hiroute_domain::{CanonicalDigest, WorkspaceId};
use std::collections::BTreeMap;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

type Result<T> = std::result::Result<T, DelegationErrorV1>;
struct ActiveRun {
    lease: String,
    epoch: String,
    nonce: String,
    cancellation: CancellationToken,
}
#[derive(Default)]
pub struct DelegationCancellationDispatcher {
    active: Mutex<BTreeMap<(WorkspaceId, String), ActiveRun>>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancellationDispatchState {
    StopRequested,
    PendingIdentity,
    Stopped,
    ResidualUnknown,
}
pub struct CancellationDispatchResult {
    pub run_id: String,
    pub state: CancellationDispatchState,
}
impl DelegationCancellationDispatcher {
    /// Bind the exact accepted run to the token passed to lifecycle::execute. Attachment reads
    /// durable intent as well, so revoke-before-attach cannot miss cancellation.
    pub fn attach(
        &self,
        runtime: &dyn DelegationRuntimePort,
        workspace: &WorkspaceId,
        run_id: &str,
        cancellation: CancellationToken,
    ) -> Result<()> {
        let run = runtime
            .run(workspace, run_id)?
            .ok_or(DelegationErrorV1::Conflict)?;
        if run.progress.workspace_releasable() {
            return Err(DelegationErrorV1::Conflict);
        }
        let mut active = self
            .active
            .lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        let key = (workspace.clone(), run_id.to_owned());
        if active.contains_key(&key) {
            return Err(DelegationErrorV1::Conflict);
        }
        if run.lease_revoked {
            cancellation.cancel();
        }
        active.insert(
            key,
            ActiveRun {
                lease: run.lease_id,
                epoch: run.daemon_epoch,
                nonce: run.launch_nonce,
                cancellation,
            },
        );
        Ok(())
    }
    /// Lifecycle owner detaches only its own lease after recording the result/stop evidence.
    pub fn detach(&self, workspace: &WorkspaceId, run_id: &str, lease: &str) -> Result<()> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        let key = (workspace.clone(), run_id.to_owned());
        if let Some(entry) = active.get(&key) {
            if entry.lease != lease {
                return Err(DelegationErrorV1::Conflict);
            }
            active.remove(&key);
        }
        Ok(())
    }
    /// Invoke after a durable intent and at startup/periodic recovery. All ACP/stop work is
    /// outside the admission guard. Lost wakes are harmless: unfinished runs remain readable.
    pub async fn dispatch_pending(
        &self,
        runtime: &dyn DelegationRuntimePort,
        workspace: &WorkspaceId,
        platform: &dyn WorkerPlatformPort,
    ) -> Result<Vec<CancellationDispatchResult>> {
        let mut results = vec![];
        for run in runtime.unreconciled(workspace)? {
            if !run.lease_revoked {
                continue;
            }
            let signalled = {
                let active = self
                    .active
                    .lock()
                    .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
                if let Some(entry) = active.get(&(workspace.clone(), run.run_id.clone())) {
                    if entry.lease != run.lease_id
                        || entry.epoch != run.daemon_epoch
                        || entry.nonce != run.launch_nonce
                    {
                        return Err(DelegationErrorV1::Conflict);
                    }
                    entry.cancellation.cancel();
                    true
                } else {
                    false
                }
            };
            let state = if signalled {
                CancellationDispatchState::StopRequested
            } else if let Some(identity) = &run.process {
                // The platform alone verifies ownership of a persisted locator; a stale PID
                // cannot make this a held object. Missing ownership returns unknown/error.
                let evidence = stop_owned(platform, identity).await;
                let event = match evidence {
                    Some(evidence) => DelegationCheckpointV1::ProcessStopped { evidence },
                    None => DelegationCheckpointV1::ProcessObserved {
                        observation: RunProcessObservationV1::Unknown,
                    },
                };
                let id = CanonicalDigest::of(&(
                    "delegation-cancel-recovery-v1",
                    &run.run_id,
                    identity,
                    &event,
                ))
                .map_err(|_| DelegationErrorV1::InvalidArguments)?;
                // CAS rejects a concurrent lifecycle/recovery result. Retry by rereading next
                // pass, never retry a prompt or reinterpret a conflicting process identity.
                let updated = runtime.checkpoint(
                    workspace,
                    &run.run_id,
                    run.progress.revision,
                    id.as_str(),
                    &event,
                )?;
                if updated.progress.cleanup == RunCleanupV1::Complete {
                    CancellationDispatchState::Stopped
                } else {
                    CancellationDispatchState::ResidualUnknown
                }
            } else {
                // Spawn may have happened before its binding was committed. No identity is
                // neither failure nor proof of stopping. Leave durable cancellation pending
                // for the execution owner (or startup reconciliation) to settle; publishing
                // ProcessUnknown here races an executor that has not reached spawn at all.
                CancellationDispatchState::PendingIdentity
            };
            results.push(CancellationDispatchResult {
                run_id: run.run_id,
                state,
            });
        }
        Ok(results)
    }
}
