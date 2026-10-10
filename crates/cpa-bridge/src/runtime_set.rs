//! Provider-isolated CPA instances sharing the same lifecycle implementation.
use crate::{
    CpaAccountKind, CpaAttemptError, CpaDownstreamCredentialCapability,
    CpaDownstreamCredentialPort, CpaLifecycleError, CpaRegisteredSourcePort, CpaRoutingBatch,
    CpaSourceManagementState, ExactCpaCredentialRequest, ManagedCpaRuntime,
};
use hiroute_integrations::CpaRegisteredSourceV1;
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[path = "managed_sessions.rs"]
mod managed_sessions;
pub use managed_sessions::{CpaLoginSession, CpaLoginState};

pub struct ManagedCpaRuntimeSet {
    runtimes: Vec<Arc<ManagedCpaRuntime>>,
    logins: Option<managed_sessions::ManagedLoginRegistry>,
    /// Rebuilt solely from saved management state; login manifests never set this map.
    selected: BTreeMap<CpaAccountKind, Mutex<ProviderSelection>>,
}

#[derive(Clone, Eq, PartialEq)]
struct SavedSourceProjection {
    source_id: String,
    revision: u64,
    candidate: String,
    account_ref: String,
    state: CpaSourceManagementState,
}

#[derive(Default)]
struct ProviderSelection {
    saved: Option<SavedSourceProjection>,
    runtime: Option<Arc<ManagedCpaRuntime>>,
    revision: Arc<AtomicU64>,
    lifecycle: Arc<Mutex<()>>,
    // A failed shutdown must be retried before any replacement is admitted.
    retiring: Vec<Arc<ManagedCpaRuntime>>,
}

fn retry_retiring(selection: &Mutex<ProviderSelection>) -> Result<(), CpaLifecycleError> {
    loop {
        let runtime = selection.lock().retiring.last().cloned();
        let Some(runtime) = runtime else {
            return Ok(());
        };
        runtime.suspend_subscription_execution();
        match runtime.shutdown() {
            Ok(_) | Err(CpaLifecycleError::NotStarted) => {
                selection
                    .lock()
                    .retiring
                    .retain(|item| !Arc::ptr_eq(item, &runtime));
            }
            Err(error) => return Err(error),
        }
    }
}

fn provider_selections() -> BTreeMap<CpaAccountKind, Mutex<ProviderSelection>> {
    [CpaAccountKind::Codex, CpaAccountKind::Claude]
        .into_iter()
        .map(|kind| (kind, Mutex::new(ProviderSelection::default())))
        .collect()
}
impl ManagedCpaRuntimeSet {
    pub fn new(runtimes: Vec<Arc<ManagedCpaRuntime>>) -> Result<Self, CpaLifecycleError> {
        let mut kinds = std::collections::BTreeSet::new();
        for runtime in &runtimes {
            let kind = runtime
                .managed_kind()
                .ok_or(CpaLifecycleError::InvalidSpec)?;
            if !kinds.insert(kind) {
                return Err(CpaLifecycleError::InvalidSpec);
            }
        }
        // Corrupt or temporarily unavailable optional login state must not take Local Control
        // or native subscriptions down. Saved managed candidates still remain fail-closed.
        let logins = runtimes.first().and_then(|runtime| {
            let root = runtime.spec.state_root.parent()?.join("managed-logins");
            managed_sessions::ManagedLoginRegistry::load(root, &runtimes).ok()
        });
        Ok(Self {
            runtimes,
            logins,
            selected: provider_selections(),
        })
    }
    pub fn for_kind(&self, kind: CpaAccountKind) -> Option<Arc<ManagedCpaRuntime>> {
        self.runtimes
            .iter()
            .find(|runtime| runtime.managed_kind() == Some(kind) && !runtime.is_managed_oauth())
            .cloned()
    }
    pub fn for_connector(&self, connector: &str) -> Option<Arc<ManagedCpaRuntime>> {
        let kind = CpaAccountKind::from_connector(connector)?;
        let selection = self.selected.get(&kind)?.lock();
        if selection.saved.is_some() {
            selection.runtime.clone()
        } else {
            self.for_kind(kind)
        }
    }

    pub fn runtime_for_candidate(&self, candidate: &str) -> Option<Arc<ManagedCpaRuntime>> {
        let kind = CpaAccountKind::from_candidate(candidate)?;
        if candidate.starts_with(&format!("candidate/cpa/{}/managed/", kind.stock_provider())) {
            self.logins.as_ref()?.for_candidate(candidate)
        } else {
            self.for_kind(kind)
        }
    }
    /// Project a durably saved source as one provider-serialized lifecycle operation.
    /// Callers release their storage locks first. The revision guard is shared across
    /// native and managed instances, so delayed completion cannot undo a newer save.
    pub fn apply_saved_source(
        &self,
        source_id: &str,
        candidate: &str,
        account_ref: &str,
        revision: u64,
        state: CpaSourceManagementState,
    ) -> Result<(), CpaLifecycleError> {
        let kind = CpaAccountKind::from_candidate(candidate)
            .ok_or(CpaLifecycleError::InvalidSourceManagement)?;
        if source_id.is_empty() || revision == 0 {
            return Err(CpaLifecycleError::InvalidSourceManagement);
        }
        let next = SavedSourceProjection {
            source_id: source_id.to_owned(),
            revision,
            candidate: candidate.to_owned(),
            account_ref: account_ref.to_owned(),
            state,
        };
        let selection = self
            .selected
            .get(&kind)
            .ok_or(CpaLifecycleError::InvalidSourceManagement)?;
        let (context, owner, mut runtime) = {
            let mut selection = selection.lock();
            if let Some(previous) = &selection.saved
                && (previous.source_id != source_id
                    || previous.revision > revision
                    || (previous.revision == revision && previous != &next))
            {
                return Err(CpaLifecycleError::StaleSourceManagement);
            }
            let same_candidate = selection.saved.as_ref().map_or_else(
                || {
                    !candidate
                        .starts_with(&format!("candidate/cpa/{}/managed/", kind.stock_provider()))
                },
                |previous| previous.candidate == candidate,
            );
            let previous = if selection.saved.is_some() {
                selection.runtime.clone()
            } else {
                self.for_kind(kind)
            };
            let changed = selection.saved.as_ref() != Some(&next);
            if changed {
                // Revoke before releasing this short publication lock. Resolving
                // a new login or waiting for stop/recovery never delays revocation.
                if let Some(previous) = &previous {
                    previous.suspend_subscription_execution();
                }
                selection.revision.fetch_add(1, Ordering::AcqRel);
                selection.saved = Some(next.clone());
                if !same_candidate {
                    selection.runtime = None;
                } else if selection.runtime.is_none() {
                    selection.runtime = previous.clone();
                }
            }
            if (!same_candidate || state != CpaSourceManagementState::Enabled)
                && let Some(previous) = previous
                && !selection
                    .retiring
                    .iter()
                    .any(|item| Arc::ptr_eq(item, &previous))
            {
                selection.retiring.push(previous);
            }
            let context = crate::CpaRequestContext::new(Instant::now() + Duration::from_secs(90))
                .observing(
                    Arc::clone(&selection.revision),
                    selection.revision.load(Ordering::Acquire),
                );
            (
                context,
                Arc::clone(&selection.lifecycle),
                selection.runtime.clone(),
            )
        };
        context.run(|| {
            if runtime.is_none() {
                runtime = self.runtime_for_candidate(candidate);
            }
            context.ensure_active()?;
            {
                let mut selection = selection.lock();
                context.ensure_active()?;
                selection.runtime = runtime.clone();
                if state != CpaSourceManagementState::Enabled
                    && let Some(runtime) = &runtime
                {
                    runtime.apply_account_management(account_ref, revision, state)?;
                    if !selection
                        .retiring
                        .iter()
                        .any(|item| Arc::ptr_eq(item, runtime))
                    {
                        selection.retiring.push(runtime.clone());
                    }
                }
            }
            let _owner = crate::request_context::lock(&owner)?;
            retry_retiring(selection)?;
            let runtime = runtime.ok_or(CpaLifecycleError::InvalidSourceManagement)?;
            {
                let _selection = selection.lock();
                context.ensure_active()?;
                runtime.apply_account_management(account_ref, revision, state)?;
            }
            if state == CpaSourceManagementState::Enabled {
                runtime.ensure_saved_runtime_ready(account_ref, revision)?;
            }
            context.ensure_active()
        })
    }

    /// A stale maintenance snapshot must not suspend the newly selected login.
    pub fn suspend_saved_source(&self, source_id: &str, candidate: &str, revision: u64) {
        let Some(kind) = CpaAccountKind::from_candidate(candidate) else {
            return;
        };
        let Some(selection) = self.selected.get(&kind) else {
            return;
        };
        let selection = selection.lock();
        if selection.saved.as_ref().is_some_and(|saved| {
            saved.source_id == source_id
                && saved.candidate == candidate
                && saved.revision == revision
        }) && let Some(runtime) = &selection.runtime
        {
            runtime.suspend_subscription_execution();
        }
    }

    /// Retry failed stops for all saved states without starting a process or reopening
    /// execution. Return only affected source identities for safe maintenance status.
    pub fn retry_saved_shutdowns(&self) -> Vec<String> {
        let mut failures = Vec::new();
        for selection in self.selected.values() {
            let owner = Arc::clone(&selection.lock().lifecycle);
            // Maintenance must not queue behind an active projection/recovery.
            let Some(_owner) = owner.try_lock() else {
                continue;
            };
            let context = crate::CpaRequestContext::new(Instant::now() + Duration::from_secs(30));
            if context.run(|| retry_retiring(selection)).is_err()
                && let Some(saved) = &selection.lock().saved
            {
                failures.push(saved.source_id.clone());
            }
        }

        failures
    }

    pub fn maintain_login_sessions(&self) -> Result<(), CpaLifecycleError> {
        self.logins
            .as_ref()
            .map_or(Ok(()), |logins| logins.maintain())
    }

    pub fn start_login(
        &self,
        kind: CpaAccountKind,
    ) -> Result<(CpaLoginSession, String), CpaLifecycleError> {
        self.logins
            .as_ref()
            .ok_or(CpaLifecycleError::InvalidSpec)?
            .start(&self.runtimes, kind)
    }
    pub fn login_sessions(&self, kind: CpaAccountKind) -> Vec<CpaLoginSession> {
        self.logins
            .as_ref()
            .map(|logins| logins.list(kind))
            .unwrap_or_default()
    }
    pub fn login_session(&self, id: &str) -> Option<CpaLoginSession> {
        self.logins.as_ref()?.session(id)
    }
    pub fn login_status(&self, id: &str) -> Result<CpaLoginSession, CpaLifecycleError> {
        self.logins
            .as_ref()
            .ok_or(CpaLifecycleError::InvalidSpec)?
            .status(id)
    }
    pub fn submit_login_callback(&self, id: &str, value: &str) -> Result<(), CpaLifecycleError> {
        self.logins
            .as_ref()
            .ok_or(CpaLifecycleError::InvalidSpec)?
            .callback(id, value)
    }
    pub fn cancel_login(
        &self,
        id: &str,
        forget: bool,
    ) -> Result<CpaLoginSession, CpaLifecycleError> {
        self.logins
            .as_ref()
            .ok_or(CpaLifecycleError::InvalidSpec)?
            .cancel(id, forget)
    }
    fn selected_runtimes(&self) -> Vec<Arc<ManagedCpaRuntime>> {
        [CpaAccountKind::Codex, CpaAccountKind::Claude]
            .into_iter()
            .filter_map(|kind| self.for_connector(kind.connector_id()))
            .collect()
    }
    pub fn shutdown(&self) -> Result<(), CpaLifecycleError> {
        let mut error = None;
        let mut all = self.runtimes.clone();
        if let Some(logins) = &self.logins {
            all.extend(logins.all_runtimes());
        }
        for runtime in all {
            if let Err(e) = runtime.shutdown()
                && !matches!(e, CpaLifecycleError::NotStarted)
            {
                error = Some(e);
            }
        }
        error.map_or(Ok(()), Err)
    }
}
impl From<Arc<ManagedCpaRuntime>> for ManagedCpaRuntimeSet {
    fn from(runtime: Arc<ManagedCpaRuntime>) -> Self {
        Self {
            runtimes: vec![runtime],
            logins: None,
            selected: provider_selections(),
        }
    }
}
impl CpaRegisteredSourcePort for ManagedCpaRuntimeSet {
    fn discover_registered_sources_for(
        &self,
        kind: CpaAccountKind,
    ) -> Result<Vec<CpaRegisteredSourceV1>, CpaLifecycleError> {
        self.for_connector(kind.connector_id())
            .ok_or(CpaLifecycleError::NotStarted)?
            .discover_registered_sources_for(kind)
    }

    fn discover_registered_sources(&self) -> Result<Vec<CpaRegisteredSourceV1>, CpaLifecycleError> {
        let mut sources = Vec::new();
        for runtime in self.selected_runtimes() {
            // A missing/failed optional subscription must not remove healthy sibling facts.
            if let Ok(found) = runtime.discover_registered_sources() {
                sources.extend(found);
            }
        }
        Ok(sources)
    }
    fn begin_routing_batch(&self) -> Result<CpaRoutingBatch<'_>, CpaLifecycleError> {
        let mut batch: Option<CpaRoutingBatch<'_>> = None;
        for runtime in self.selected_runtimes() {
            if let Ok(next) = super::runtime::CpaRoutingBatch::begin_owned(runtime) {
                if let Some(batch) = &mut batch {
                    batch.extend(next);
                } else {
                    batch = Some(next);
                }
            }
        }
        batch.ok_or(CpaLifecycleError::NotStarted)
    }
}
impl CpaDownstreamCredentialPort for ManagedCpaRuntimeSet {
    fn lease_downstream_capability(
        &self,
        request: ExactCpaCredentialRequest<'_>,
    ) -> Result<Option<CpaDownstreamCredentialCapability>, CpaAttemptError> {
        self.for_connector(request.connector_id)
            .ok_or(CpaAttemptError::UnregisteredTarget)?
            .lease_downstream_capability(request)
    }
}
