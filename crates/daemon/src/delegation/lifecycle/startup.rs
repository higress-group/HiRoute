//! In-memory startup ownership for a borrowed Codex root, never a task concurrency limit.
use super::*;
use crate::delegation::acp::AcpSessionBinding;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Root {
    starting: Arc<AsyncMutex<()>>,
    // Set only while initialization lacks confirmation. A dropped/failed start cannot
    // forget a possibly running native initializer; verified owned-stop clears it.
    process: Mutex<Option<WorkerProcessIdentity>>,
}

#[derive(Default)]
struct StartupRoots(Mutex<BTreeMap<PathBuf, Arc<Root>>>);

fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn roots() -> &'static StartupRoots {
    static ROOTS: OnceLock<StartupRoots> = OnceLock::new();
    ROOTS.get_or_init(StartupRoots::default)
}

impl StartupRoots {
    async fn acquire(
        &self,
        path: &Path,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Arc<StartupPermit>, DelegationErrorV1> {
        let root = {
            let mut roots = lock(&self.0);
            roots.retain(|_, root| Arc::strong_count(root) > 1 || lock(&root.process).is_some());
            roots.entry(path.to_owned()).or_default().clone()
        };
        let held = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(DelegationErrorV1::Cancelled),
            _ = tokio::time::sleep_until(deadline) => return Err(DelegationErrorV1::DeadlineExceeded),
            held = root.starting.clone().lock_owned() => held,
        };
        if lock(&root.process).is_some() {
            // No indefinite wait behind a previous uncertain initializer. The existing
            // cancellation recovery path must stop that exact held process first.
            return Err(DelegationErrorV1::Busy);
        }
        Ok(Arc::new(StartupPermit {
            root,
            held: Mutex::new(Some(held)),
        }))
    }

    fn stopped(&self, identity: &WorkerProcessIdentity, evidence: &WorkerStopEvidence) {
        if !evidence.scope_stopped
            || evidence.residual_unknown
            || !matches!(evidence.observation, WorkerObservation::Exited { .. })
        {
            return;
        }
        for root in lock(&self.0).values() {
            let mut process = lock(&root.process);
            if process.as_ref() == Some(identity) {
                *process = None;
            }
        }
    }
}

pub(super) struct StartupPermit {
    root: Arc<Root>,
    held: Mutex<Option<OwnedMutexGuard<()>>>,
}

impl StartupPermit {
    pub(super) async fn acquire(
        path: Option<&Path>,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Option<Arc<Self>>, DelegationErrorV1> {
        match path {
            Some(path) => roots()
                .acquire(path, deadline, cancellation)
                .await
                .map(Some),
            None => Ok(None),
        }
    }

    pub(super) fn bind_process(&self, identity: &WorkerProcessIdentity) {
        *lock(&self.root.process) = Some(identity.clone());
    }

    fn initialized(&self) {
        let mut held = lock(&self.held);
        if held.is_some() {
            *lock(&self.root.process) = None;
            // Release before session/new, session/load, model selection or any prompt.
            // A repeated receipt from this run cannot release a later initializer.
            held.take();
        }
    }

    pub(super) fn journal(
        self: &Arc<Self>,
        inner: Arc<dyn AcpRunJournal>,
    ) -> Arc<dyn AcpRunJournal> {
        Arc::new(StartupJournal {
            inner,
            permit: self.clone(),
        })
    }
}

pub(super) fn observe_stopped(identity: &WorkerProcessIdentity, evidence: &WorkerStopEvidence) {
    roots().stopped(identity, evidence);
}

struct StartupJournal {
    inner: Arc<dyn AcpRunJournal>,
    permit: Arc<StartupPermit>,
}

impl AcpRunJournal for StartupJournal {
    fn initialized(&self) {
        self.permit.initialized();
        self.inner.initialized();
    }
    fn session_bound(&self, binding: &AcpSessionBinding) -> Result<(), DelegationErrorV1> {
        self.inner.session_bound(binding)
    }
    fn before_prompt(&self) -> Result<(), DelegationErrorV1> {
        self.inner.before_prompt()
    }
    fn text_update(&self, text: &str) -> Result<(), DelegationErrorV1> {
        self.inner.text_update(text)
    }
    fn allow_permission_once(&self, request: &serde_json::Value) -> bool {
        self.inner.allow_permission_once(request)
    }
}

#[cfg(test)]
mod tests;
