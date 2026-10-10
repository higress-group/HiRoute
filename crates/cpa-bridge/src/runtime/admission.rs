//! The only source of execution admission. Lifecycle I/O never owns this lock.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use parking_lot::{Mutex, MutexGuard};

use super::{CpaHealth, ManagedCpaRuntime, RuntimeInner, subscriptions};
use crate::{CpaLifecycleError, CpaRequestContext, request_context};

#[derive(Default)]
pub(super) struct Admission {
    pub(super) state: Mutex<AdmissionState>,
    revision: Arc<AtomicU64>,
}

#[derive(Default, Clone)]
pub(super) struct AdmissionState {
    pub(super) source_management: BTreeMap<String, subscriptions::SourceManagementProjection>,
    pub(super) subscription_execution_suspended: bool,
    pub(super) auth_generation: u64,
    pub(super) needs_account_validation: bool,
    // Revocation is immediate here; the lifecycle owner consumes it on its next
    // catalog merge so re-enabling cannot reuse a pre-revocation generation.
    pub(super) revoked_accounts: BTreeSet<String>,
    pub(super) managed_status_sequence: u64,
    pub(super) managed_status_success: Option<ManagedStatusSuccess>,
}

#[derive(Clone)]
pub(super) struct ManagedStatusSuccess {
    pub(super) ticket: u64,
    pub(super) binding: hiroute_domain::CanonicalDigest,
    pub(super) epochs: (u64, u64),
}

impl AdmissionState {
    pub(super) fn passive_source_is_admitted(&self, requires_saved: bool) -> bool {
        !self.subscription_execution_suspended
            && ((!requires_saved && self.source_management.is_empty())
                || self
                    .source_management
                    .values()
                    .any(|projection| projection.enabled()))
    }
}

impl Admission {
    pub(super) fn snapshot(&self) -> AdmissionState {
        self.state.lock().clone()
    }
    pub(super) fn advance(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
    }
    pub(super) fn context(&self, deadline: Instant) -> CpaRequestContext {
        let _state = self.state.lock();
        CpaRequestContext::new(deadline).observing(
            Arc::clone(&self.revision),
            self.revision.load(Ordering::Acquire),
        )
    }
}

#[derive(Clone)]
pub(super) struct Observation {
    pub(super) health: CpaHealth,
    pub(super) health_sample: Option<Instant>,
    pub(super) last_exit: Option<crate::CpaExit>,
    pub(super) live: Option<LiveObservation>,
}

#[derive(Clone)]
pub(super) struct LiveObservation {
    pub(super) address: std::net::SocketAddr,
    pub(super) management: Arc<crate::config::SecretText>,
    pub(super) version: String,
    pub(super) epochs: (u64, u64),
    pub(super) accounts: Vec<crate::accounts::AccountSnapshotRecord>,
}

impl Default for Observation {
    fn default() -> Self {
        Self {
            health: CpaHealth::Stopped { last_exit: None },
            health_sample: None,
            last_exit: None,
            live: None,
        }
    }
}

pub(crate) struct LifecycleOperation<'a> {
    runtime: &'a ManagedCpaRuntime,
    inner: MutexGuard<'a, RuntimeInner>,
    _scope: request_context::ContextGuard,
    _profile_policy: crate::claude_profile::NoProfileIo,
}

impl std::ops::Deref for LifecycleOperation<'_> {
    type Target = RuntimeInner;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl std::ops::DerefMut for LifecycleOperation<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl Drop for LifecycleOperation<'_> {
    fn drop(&mut self) {
        self.runtime.publish_observation(&self.inner);
    }
}

impl ManagedCpaRuntime {
    pub(crate) fn operation_context(&self) -> CpaRequestContext {
        self.admission.context(
            Instant::now()
                + self.spec.startup_timeout
                + self.spec.control_timeout
                + self.spec.shutdown_timeout
                + self.spec.restart_policy.max_backoff,
        )
    }

    pub(crate) fn lock_lifecycle(&self) -> Result<LifecycleOperation<'_>, CpaLifecycleError> {
        let scope = self.operation_context().enter();
        let inner = request_context::lock(&self.inner)?;
        Ok(LifecycleOperation {
            runtime: self,
            inner,
            _scope: scope,
            _profile_policy: crate::claude_profile::NoProfileIo::enter(),
        })
    }

    pub(crate) fn account_execution_is_admitted(
        &self,
        account: &crate::accounts::AccountSnapshotRecord,
    ) -> bool {
        let admission = self.admission.state.lock();
        subscriptions::account_execution_is_admitted(
            &admission.source_management,
            admission.subscription_execution_suspended,
            account,
        )
    }

    pub(crate) fn commit_admitted<T>(
        &self,
        account: &crate::accounts::AccountSnapshotRecord,
        commit: impl FnOnce() -> T,
    ) -> Result<T, crate::CpaAttemptError> {
        let admission = self.admission.state.lock();
        request_context::check().map_err(|_| crate::CpaAttemptError::RevokedCredential)?;
        if !subscriptions::account_execution_is_admitted(
            &admission.source_management,
            admission.subscription_execution_suspended,
            account,
        ) {
            return Err(crate::CpaAttemptError::RevokedCredential);
        }
        Ok(commit())
    }

    pub(super) fn commit_lifecycle<T>(
        &self,
        commit: impl FnOnce() -> T,
    ) -> Result<T, CpaLifecycleError> {
        let _admission = self.admission.state.lock();
        request_context::check()?;
        Ok(commit())
    }

    pub(super) fn commit_discovery<T>(
        &self,
        authorization_changed: bool,
        commit: impl FnOnce() -> T,
    ) -> Result<T, CpaLifecycleError> {
        let mut admission = self.admission.state.lock();
        request_context::check()?;
        let result = commit();
        admission.needs_account_validation = false;
        admission.revoked_accounts.clear();
        if authorization_changed {
            admission.auth_generation = admission.auth_generation.wrapping_add(1);
        }
        Ok(result)
    }

    pub(super) fn publish_observation(&self, inner: &RuntimeInner) {
        let health = match &inner.live {
            None => CpaHealth::Stopped {
                last_exit: inner.last_exit,
            },
            Some(live) if live.process.is_none() => CpaHealth::Crashed {
                exit: inner.last_exit.unwrap_or(crate::CpaExit { code: None }),
                restart_count: inner.restart_count,
            },
            Some(live)
                if live.cleanup_only
                    || live.last_health.is_none_or(|sample| {
                        sample.elapsed() >= std::time::Duration::from_secs(5)
                    })
                    || self.admission.snapshot().subscription_execution_suspended
                    || self.health_invalidated.load(Ordering::Acquire) =>
            {
                CpaHealth::Unhealthy {
                    pid: live.process.as_ref().map_or(0, |process| process.pid()),
                    restart_count: inner.restart_count,
                }
            }
            Some(live) => super::ready_health(live, inner.restart_count),
        };
        let live = inner.live.as_ref().map(|live| LiveObservation {
            address: live.address,
            management: Arc::clone(&live.secrets.management),
            version: live.artifact.version().to_string(),
            epochs: self.epochs.current(),
            accounts: live.accounts.clone(),
        });
        *self.observation.lock() = Observation {
            health,
            health_sample: inner.live.as_ref().and_then(|live| live.last_health),
            last_exit: inner.last_exit,
            live,
        };
    }

    pub(crate) fn observed_accounts(&self) -> Option<Vec<crate::accounts::AccountSnapshotRecord>> {
        self.observation
            .lock()
            .live
            .as_ref()
            .map(|live| live.accounts.clone())
    }

    pub(crate) fn prepare_native_subscription(
        &self,
    ) -> Result<Option<crate::BorrowedSubscriptionEvidence>, CpaLifecycleError> {
        if let Some(spec) = &self.spec.borrowed_claude_auth {
            return spec.inspect().map(|value| Some(value.into()));
        }
        if let Some(spec) = &self.spec.borrowed_codex_auth {
            return spec.inspect().map(|value| Some(value.into()));
        }
        Ok(None)
    }

    pub(crate) fn native_source_stamp(&self) -> Option<hiroute_domain::CanonicalDigest> {
        if let Some(spec) = &self.spec.borrowed_claude_auth {
            return spec.source_stamp().ok();
        }
        self.spec
            .borrowed_codex_auth
            .as_ref()
            .and_then(|spec| spec.inspect().ok())
            .map(|evidence| evidence.evidence_digest().clone())
    }
}
