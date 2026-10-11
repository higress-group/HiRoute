//! Facts are local to one candidate construction, and checked again before publication use.
use super::*;
use crate::attempt::{
    CpaAttemptError, ExactCpaAttemptRequest, PreparedCpaTarget, prepare_from_accounts,
};

#[derive(Clone, PartialEq, Eq)]
struct BatchIdentity {
    accounts: Vec<AccountSnapshotRecord>,
    address: SocketAddr,
    epochs: (u64, u64),
    pid: u32,
    restart_count: u64,
}

#[derive(Clone, PartialEq, Eq)]
enum BatchState {
    Ready(BatchIdentity),
    Disabled { epochs: (u64, u64) },
}

/// Non-secret, one-construction facts. The owner must finish before accepting its candidates.
/// No runtime lock is held while the caller reads its database.
pub struct CpaRoutingBatch<'a> {
    runtime: BatchRuntime<'a>,
    identity: BatchState,
    sources: Vec<CpaRegisteredSourceV1>,
    additional: Vec<CpaRoutingBatch<'a>>,
}

enum BatchRuntime<'a> {
    Borrowed(&'a ManagedCpaRuntime),
    Owned(Arc<ManagedCpaRuntime>),
}

impl std::ops::Deref for BatchRuntime<'_> {
    type Target = ManagedCpaRuntime;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(runtime) => runtime,
            Self::Owned(runtime) => runtime,
        }
    }
}

impl<'a> CpaRoutingBatch<'a> {
    pub(super) fn begin(runtime: &'a ManagedCpaRuntime) -> Result<Self, CpaLifecycleError> {
        let _scope = runtime.operation_context().enter();
        let (identity, materials) = read_batch_facts(runtime)?;
        let sources = materials
            .iter()
            .map(|material| {
                register_cpa_account(&runtime.catalog, material)
                    .map_err(|_| CpaLifecycleError::InvalidMaterialization)
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            runtime: BatchRuntime::Borrowed(runtime),
            identity,
            sources,
            additional: Vec::new(),
        })
    }

    pub(crate) fn begin_owned(runtime: Arc<ManagedCpaRuntime>) -> Result<Self, CpaLifecycleError> {
        let _scope = runtime.operation_context().enter();
        let (identity, materials) = read_batch_facts(&runtime)?;
        let sources = materials
            .iter()
            .map(|material| {
                register_cpa_account(&runtime.catalog, material)
                    .map_err(|_| CpaLifecycleError::InvalidMaterialization)
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            runtime: BatchRuntime::Owned(runtime),
            identity,
            sources,
            additional: Vec::new(),
        })
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.sources.extend(other.sources.iter().cloned());
        self.additional.push(other);
    }

    pub fn sources(&self) -> &[CpaRegisteredSourceV1] {
        &self.sources
    }

    /// Refresh external authorization and readiness, including a restart that preserves epochs.
    pub fn finish(self) -> Result<bool, CpaLifecycleError> {
        let _scope = self.runtime.operation_context().enter();
        let (identity, _) = read_batch_facts(&self.runtime)?;
        let current = self.identity == identity;
        for other in self.additional {
            if !other.finish()? {
                return Ok(false);
            }
        }
        Ok(current)
    }
}

impl CpaRoutingBatch<'_> {
    pub fn prepare_target(
        &self,
        request: ExactCpaAttemptRequest<'_>,
    ) -> Result<PreparedCpaTarget, CpaAttemptError> {
        for batch in std::iter::once(self).chain(self.additional.iter()) {
            if !batch.runtime.spec.bindings.iter().any(|binding| {
                request.credential_ref.subject() == format!("connector/{}", binding.connector_id)
            }) {
                continue;
            }
            let BatchState::Ready(identity) = &batch.identity else {
                return Err(CpaAttemptError::RevokedCredential);
            };
            match prepare_from_accounts(
                &batch.runtime,
                &identity.accounts,
                identity.address,
                identity.epochs,
                ExactCpaAttemptRequest {
                    credential_ref: request.credential_ref,
                    upstream_model_id: request.upstream_model_id,
                    protocol: request.protocol,
                },
            ) {
                Ok(target) => return Ok(target),
                Err(CpaAttemptError::UnregisteredTarget) => {}
                Err(error) => return Err(error),
            }
        }
        Err(CpaAttemptError::UnregisteredTarget)
    }
}

fn read_batch_facts(
    runtime: &ManagedCpaRuntime,
) -> Result<(BatchState, Vec<CpaAccountMaterializationV1>), CpaLifecycleError> {
    let admission = runtime.admission.snapshot();
    if admission.subscription_execution_suspended {
        return Err(CpaLifecycleError::StaleSourceManagement);
    }
    if !admission.passive_source_is_admitted(false) {
        // Disabled is a valid empty inventory, not a transient runtime failure.
        // This passive observation never waits for the owner or reads credentials.
        if runtime.observation.lock().live.is_none() {
            return Err(CpaLifecycleError::NotStarted);
        }
        let identity = BatchState::Disabled {
            epochs: runtime.epochs.current(),
        };
        crate::request_context::check()?;
        return Ok((identity, Vec::new()));
    }
    runtime.prepare_native_subscription()?;
    let mut inner = runtime.lock_lifecycle()?;
    let materials = runtime.discover_materializations_locked(&mut inner, None)?;
    let identity = BatchState::Ready(batch_identity(runtime, &inner)?);
    crate::request_context::check()?;
    Ok((identity, materials))
}

fn batch_identity(
    runtime: &ManagedCpaRuntime,
    inner: &RuntimeInner,
) -> Result<BatchIdentity, CpaLifecycleError> {
    let live = inner.live.as_ref().ok_or(CpaLifecycleError::NotStarted)?;
    let process = live.process.as_ref().ok_or(CpaLifecycleError::NotStarted)?;
    Ok(BatchIdentity {
        accounts: live.accounts.clone(),
        address: live.address,
        epochs: runtime.epochs.current(),
        pid: process.pid(),
        restart_count: inner.restart_count,
    })
}

impl ManagedCpaRuntime {
    pub(crate) fn discover_materializations(
        &self,
        expected: Option<&BorrowedSubscriptionEvidence>,
    ) -> Result<Vec<CpaAccountMaterializationV1>, CpaLifecycleError> {
        let _scope = self.operation_context().enter();
        let admission = self.admission.snapshot();
        if expected.is_none() && !admission.passive_source_is_admitted(false) {
            return if admission.subscription_execution_suspended {
                Err(CpaLifecycleError::StaleSourceManagement)
            } else {
                Ok(Vec::new())
            };
        }
        self.prepare_native_subscription()?;
        let mut inner = self.lock_lifecycle()?;
        self.discover_materializations_locked(&mut inner, expected)
    }

    pub(super) fn discover_materializations_locked(
        &self,
        inner: &mut RuntimeInner,
        expected: Option<&BorrowedSubscriptionEvidence>,
    ) -> Result<Vec<CpaAccountMaterializationV1>, CpaLifecycleError> {
        let admission = self.admission.snapshot();
        if expected.is_none() {
            // A stale batch or passive inventory read cannot reopen a retiring
            // writer. Explicit Check supplies evidence and has separate authority.
            if admission.subscription_execution_suspended {
                return Err(CpaLifecycleError::StaleSourceManagement);
            }
            if !admission.passive_source_is_admitted(false) {
                return Ok(Vec::new());
            }
        }
        // Candidate publication and explicit discovery each establish a fresh
        // readiness boundary; the short health cache is only for request leases.
        if let Some(live) = inner.live.as_mut() {
            live.last_health = None;
        }
        if let Err(error) = self.ensure_ready_locked(inner, expected) {
            self.invalidate_runtime_accounts(inner, None);
            return Err(error);
        }
        let layout = self.prepare_layout()?;
        let source_management = admission.source_management;
        let live = inner.live.as_mut().ok_or(CpaLifecycleError::NotStarted)?;
        let previous_auth_generation = live
            .auth_lease
            .as_ref()
            .and_then(|lease| lease.generation());
        let managed_identities = match live
            .auth_lease
            .as_mut()
            .ok_or(CpaLifecycleError::OwnerState)?
            .refresh_subscription(expected, None)
        {
            Ok(identities) => identities,
            Err(error) => {
                self.invalidate_live_accounts(live, self.managed_kind());
                let _ = save_account_state(&layout.accounts_path, &live.accounts);
                return Err(error);
            }
        };
        let current_auth_generation = managed_identities
            .iter()
            .find(|identity| Some(identity.account_kind) == self.managed_kind())
            .map(|identity| identity.generation);
        let control_started = Instant::now();
        self.emit_stage(
            CpaStageKind::ControlCall,
            CpaStageOutcome::Entered,
            0,
            inner.restart_count,
        );
        if expected.is_some()
            && !live
                .auth_lease
                .as_ref()
                .is_some_and(|lease| lease.has_client_version())
        {
            self.emit_stage(
                CpaStageKind::ControlCall,
                CpaStageOutcome::Failed {
                    code: CpaFailureCode::NativeClientVersionUnavailable,
                },
                control_started.elapsed().as_millis() as u64,
                inner.restart_count,
            );
            self.invalidate_live_accounts(live, self.managed_kind());
            let _ = save_account_state(&layout.accounts_path, &live.accounts);
            return Err(CpaLifecycleError::BorrowedCodexClientVersionUnavailable);
        }
        let mut discovered = match self.control.discover_and_pin(
            live.address,
            &layout.auth_dir,
            &managed_identities,
            &live.secrets,
            &live.artifact.version().to_string(),
            self.spec.control_timeout,
            expected.is_some(),
        ) {
            Ok(discovered) => {
                self.emit_stage(
                    CpaStageKind::ControlCall,
                    CpaStageOutcome::Completed,
                    control_started.elapsed().as_millis() as u64,
                    inner.restart_count,
                );
                discovered
            }
            Err(error) => {
                self.emit_stage(
                    CpaStageKind::ControlCall,
                    CpaStageOutcome::Failed {
                        code: control_failure_code(&error),
                    },
                    control_started.elapsed().as_millis() as u64,
                    inner.restart_count,
                );
                let affected_kind = if matches!(error, AccountDiscoveryError::AccountDisappeared) {
                    self.managed_kind()
                } else {
                    None
                };
                self.invalidate_live_accounts(live, affected_kind);
                let _ = save_account_state(&layout.accounts_path, &live.accounts);
                return Err(map_control_error(error));
            }
        };
        crate::request_context::check()?;
        // An approved recheck needs current inventory even while the saved source is disabled.
        // Keep those observed identities separately; they must not reopen runtime admission.
        let checked_accounts = expected.map(|_| {
            discovered
                .iter()
                .filter(|account| account.active)
                .map(|account| account.account_digest.clone())
                .collect::<std::collections::BTreeSet<_>>()
        });
        // Disabled/removed discoveries never reactivate an account during the merge.
        // Otherwise every read would rotate its generation before projecting it inactive again.
        subscriptions::apply_management_projection(&mut discovered, &source_management);
        let before = account_epoch_facts(&live.accounts);
        let mut previous = live.accounts.clone();
        for account in &mut previous {
            if admission.revoked_accounts.contains(&account.account_digest) {
                account.active = false;
            }
        }
        let mut merged_accounts = match merge_accounts(&previous, discovered) {
            Ok(accounts) => accounts,
            Err(error) => {
                self.invalidate_live_accounts(live, None);
                let _ = save_account_state(&layout.accounts_path, &live.accounts);
                return Err(error);
            }
        };
        subscriptions::apply_management_projection(&mut merged_accounts, &source_management);
        self.commit_discovery(current_auth_generation != previous_auth_generation, || {
            if account_epoch_facts(&merged_accounts) != before
                || current_auth_generation != previous_auth_generation
            {
                self.epochs.advance_target();
            }
            live.accounts = merged_accounts;
            live.published_auth_generation = current_auth_generation;
            live.accounts_validated_for_process = true;
        })?;
        save_account_state(&layout.accounts_path, &live.accounts)?;
        let mut result = Vec::new();
        for account in live.accounts.iter().filter(|account| {
            account.active
                || checked_accounts
                    .as_ref()
                    .is_some_and(|checked| checked.contains(&account.account_digest))
        }) {
            let binding = self
                .spec
                .bindings
                .iter()
                .find(|binding| binding.account_kind == account.account_kind)
                .ok_or(CpaLifecycleError::InvalidBinding)?;
            // This copy supplies check facts only. Persisted state and request-scoped
            // capability issuance retain Disabled until the separate SaveReady succeeds.
            let mut checked = account.clone();
            checked.active = true;
            result.push(
                checked
                    .materialize(binding)
                    .map_err(|_| CpaLifecycleError::InvalidMaterialization)?,
            );
        }
        crate::request_context::check()?;
        Ok(result)
    }
}

fn control_failure_code(error: &AccountDiscoveryError) -> CpaFailureCode {
    use crate::http::LoopbackHttpError;
    match error {
        AccountDiscoveryError::Http(LoopbackHttpError::Io(error)) => {
            if matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) {
                CpaFailureCode::ControlTimeout
            } else {
                CpaFailureCode::ControlTransport
            }
        }
        AccountDiscoveryError::ManagementAuthentication
        | AccountDiscoveryError::DownstreamAuthentication => CpaFailureCode::ControlAuthentication,
        AccountDiscoveryError::PinNotApplied => CpaFailureCode::ControlPinNotApplied,
        _ => CpaFailureCode::ControlRejected,
    }
}
