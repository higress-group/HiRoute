use std::sync::Arc;
use std::time::Duration;

use hiroute_domain::WorkspaceId;
use hiroute_domain::delegation::DelegationErrorV1;
use tokio::runtime::Builder;

use super::DelegationRunExecutor;

impl DelegationRunExecutor {
    /// Close the same lock used by wake before signalling all current executions. This
    /// token is process-local; it never creates a user cancellation or replays old work.
    pub(crate) fn shutdown(&self) {
        let _active = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.shutdown.cancel();
    }

    pub(crate) fn join(&self, timeout: Duration) -> Result<(), DelegationErrorV1> {
        let active = self
            .active
            .lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        let (active, _) = self
            .idle
            .wait_timeout_while(active, timeout, |active| !active.is_empty())
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        if active.is_empty() {
            Ok(())
        } else {
            Err(DelegationErrorV1::DeadlineExceeded)
        }
    }

    /// Wake the existing durable cancellation dispatcher after intent has committed. Replays may
    /// create another bounded wake, but never another prompt or cancellation receipt.
    pub(crate) fn wake_cancellation(
        self: &Arc<Self>,
        workspace: &WorkspaceId,
    ) -> Result<(), DelegationErrorV1> {
        let executor = Arc::clone(self);
        let workspace = workspace.clone();
        std::thread::Builder::new()
            .name("hiroute-delegation-cancel".into())
            .spawn(move || {
                let Ok(runtime) = Builder::new_current_thread().enable_all().build() else {
                    return;
                };
                let _ = runtime.block_on(executor.cancellation.dispatch_pending(
                    executor.runtime.as_ref(),
                    &workspace,
                    executor.platform.as_ref(),
                ));
            })
            .map(|_| ())
            .map_err(|_| DelegationErrorV1::StorageUnavailable)
    }
}
