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

/// Non-secret, one-construction facts. The owner must finish before accepting its candidates.
/// No runtime lock is held while the caller reads its database.
pub struct CpaRoutingBatch<'a> {
    runtime: &'a ManagedCpaRuntime,
    identity: BatchIdentity,
    sources: Vec<CpaRegisteredSourceV1>,
}

impl<'a> CpaRoutingBatch<'a> {
    pub(super) fn begin(runtime: &'a ManagedCpaRuntime) -> Result<Self, CpaLifecycleError> {
        let mut inner = runtime.inner.lock();
        let materials = runtime.discover_materializations_locked(&mut inner, None)?;
        let identity = batch_identity(runtime, &inner)?;
        let sources = materials
            .iter()
            .map(|material| {
                register_cpa_account(&runtime.catalog, material)
                    .map_err(|_| CpaLifecycleError::InvalidMaterialization)
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            runtime,
            identity,
            sources,
        })
    }

    pub fn sources(&self) -> &[CpaRegisteredSourceV1] {
        &self.sources
    }

    /// Refresh external authorization and readiness, including a restart that preserves epochs.
    pub fn finish(self) -> Result<bool, CpaLifecycleError> {
        let mut inner = self.runtime.inner.lock();
        self.runtime
            .discover_materializations_locked(&mut inner, None)?;
        Ok(self.identity == batch_identity(self.runtime, &inner)?)
    }
}

impl CpaRoutingBatch<'_> {
    pub fn prepare_target(
        &self,
        request: ExactCpaAttemptRequest<'_>,
    ) -> Result<PreparedCpaTarget, CpaAttemptError> {
        prepare_from_accounts(
            self.runtime,
            &self.identity.accounts,
            self.identity.address,
            self.identity.epochs,
            request,
        )
    }
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
        expected: Option<&BorrowedCodexEvidence>,
    ) -> Result<Vec<CpaAccountMaterializationV1>, CpaLifecycleError> {
        let mut inner = self.inner.lock();
        self.discover_materializations_locked(&mut inner, expected)
    }

    fn discover_materializations_locked(
        &self,
        inner: &mut RuntimeInner,
        expected: Option<&BorrowedCodexEvidence>,
    ) -> Result<Vec<CpaAccountMaterializationV1>, CpaLifecycleError> {
        if let Err(error) = self.ensure_ready_locked(inner, expected) {
            self.invalidate_runtime_accounts(inner, None);
            return Err(error);
        }
        let layout = self.prepare_layout()?;
        let source_management = inner.source_management.clone();
        let live = inner.live.as_mut().ok_or(CpaLifecycleError::NotStarted)?;
        let previous_auth_generation = live
            .auth_lease
            .as_ref()
            .and_then(|lease| lease.codex_generation());
        let managed_identities = match live
            .auth_lease
            .as_mut()
            .ok_or(CpaLifecycleError::OwnerState)?
            .refresh_expected(expected, None)
        {
            Ok(identities) => identities,
            Err(error) => {
                self.invalidate_live_accounts(live, Some(crate::CpaAccountKind::Codex));
                let _ = save_account_state(&layout.accounts_path, &live.accounts);
                return Err(error);
            }
        };
        let current_auth_generation = managed_identities
            .iter()
            .find(|identity| identity.account_kind == crate::CpaAccountKind::Codex)
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
            self.invalidate_live_accounts(live, Some(crate::CpaAccountKind::Codex));
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
                    Some(crate::CpaAccountKind::Codex)
                } else {
                    None
                };
                self.invalidate_live_accounts(live, affected_kind);
                let _ = save_account_state(&layout.accounts_path, &live.accounts);
                return Err(map_control_error(error));
            }
        };
        // Disabled/removed discoveries never reactivate an account during the merge.
        // Otherwise every read would rotate its generation before projecting it inactive again.
        subscriptions::apply_management_projection(&mut discovered, &source_management);
        let before = account_epoch_facts(&live.accounts);
        live.accounts = match merge_accounts(&live.accounts, discovered) {
            Ok(accounts) => accounts,
            Err(error) => {
                self.invalidate_live_accounts(live, None);
                let _ = save_account_state(&layout.accounts_path, &live.accounts);
                return Err(error);
            }
        };
        subscriptions::apply_management_projection(&mut live.accounts, &source_management);
        if account_epoch_facts(&live.accounts) != before
            || current_auth_generation != previous_auth_generation
        {
            self.epochs.advance_target();
        }
        save_account_state(&layout.accounts_path, &live.accounts)?;
        let mut result = Vec::new();
        for account in live.accounts.iter().filter(|account| account.active) {
            let binding = self
                .spec
                .bindings
                .iter()
                .find(|binding| binding.account_kind == account.account_kind)
                .ok_or(CpaLifecycleError::InvalidBinding)?;
            result.push(
                account
                    .materialize(binding)
                    .map_err(|_| CpaLifecycleError::InvalidMaterialization)?,
            );
        }
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
