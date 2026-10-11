use std::collections::BTreeMap;

use crate::accounts::AccountSnapshotRecord;
use crate::errors::CpaLifecycleError;
use crate::state::save_account_state;

use super::{ManagedCpaRuntime, RuntimeInner};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CpaSourceManagementState {
    Enabled,
    Disabled,
    Removed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SourceManagementProjection {
    revision: u64,
    state: CpaSourceManagementState,
}

impl SourceManagementProjection {
    pub(super) fn enabled(self) -> bool {
        self.state == CpaSourceManagementState::Enabled
    }
    fn denies_admission(self) -> bool {
        matches!(
            self.state,
            CpaSourceManagementState::Disabled | CpaSourceManagementState::Removed
        )
    }
}

impl ManagedCpaRuntime {
    /// Explicit recovery for an exact, durably enabled subscription. Login status,
    /// callback, cancel and credential inspection never call this admission boundary.
    pub fn ensure_saved_runtime_ready(
        &self,
        account_ref: &str,
        source_revision: u64,
    ) -> Result<super::CpaHealth, CpaLifecycleError> {
        if self.managed_kind().is_none() || source_revision == 0 {
            return Err(CpaLifecycleError::InvalidSourceManagement);
        }
        let digest = parse_account_ref(account_ref)?;
        let _scope = self.operation_context().enter();
        let admission = self.admission.snapshot();
        let projection = admission
            .source_management
            .get(digest)
            .ok_or(CpaLifecycleError::InvalidSourceManagement)?;
        if projection.revision != source_revision
            || projection.state != CpaSourceManagementState::Enabled
            || admission.subscription_execution_suspended
        {
            return Err(CpaLifecycleError::StaleSourceManagement);
        }
        let preparation_generation = self.preparation_generation();
        let stamp = self.native_source_stamp();
        let expected = match self.managed_oauth_source().map_or_else(
            || self.inspect_subscription(),
            |source| {
                source
                    .inspect()
                    .map(crate::BorrowedSubscriptionEvidence::Managed)
            },
        ) {
            Ok(expected) => expected,
            Err(error) => {
                if self.native_source_stamp() == stamp {
                    self.reject_preparation(preparation_generation);
                }
                return Err(error);
            }
        };
        let mut inner = self.lock_lifecycle()?;
        let result = (|| {
            // Check the saved provider subject before starting its execution owner.
            // Managed expiration belongs to CPA; native access leases still require
            // their original owner's current credentials and never grant refresh.
            if expected.account_ref() != account_ref {
                return Err(if self.is_managed_oauth() {
                    CpaLifecycleError::ManagedOAuthAccountChanged
                } else if self.managed_kind() == Some(crate::CpaAccountKind::Claude) {
                    CpaLifecycleError::BorrowedClaudeAuthSourceChanged
                } else {
                    CpaLifecycleError::BorrowedCodexAuthSourceChanged
                });
            }
            if inner.live.is_none() {
                self.start_expected_locked(&mut inner, Some(&expected))?;
            } else {
                // Saved-source maintenance establishes current health. Only an
                // ordinary request lease may reuse the bounded health sample.
                if let Some(live) = inner.live.as_mut() {
                    live.last_health = None;
                }
                self.ensure_ready_locked(&mut inner, Some(&expected))?;
            }
            // Retained facts require this process's catalog, even when their saved
            // active flags survived the restart. The same validation also restores
            // snapshots revoked by a transient read/control failure. An empty pending
            // inventory does not establish or require any routing facts.
            let needs_account_recovery = inner.live.as_ref().is_some_and(|live| {
                live.accounts.iter().any(|account| {
                    account.account_digest == digest
                        && (!account.active
                            || !live.accounts_validated_for_process
                            || self.admission.snapshot().needs_account_validation)
                })
            });
            if needs_account_recovery {
                self.discover_materializations_locked(&mut inner, None)?;
                if !inner.live.as_ref().is_some_and(|live| {
                    live.accounts
                        .iter()
                        .any(|account| account.account_digest == digest && account.active)
                }) {
                    return Err(CpaLifecycleError::ControlUnavailable);
                }
            }
            let live = inner.live.as_ref().ok_or(CpaLifecycleError::NotStarted)?;
            Ok(super::ready_health(live, inner.restart_count))
        })();
        if result.is_err() && crate::request_context::check().is_ok() {
            self.invalidate_runtime_accounts(&mut inner, self.managed_kind());
            // A retained cleanup owner is unavailable, not evidence that this
            // saved authorization is invalid. Keep its enabled revision retryable;
            // cleanup_only already prevents health or capability admission.
            if !inner.live.as_ref().is_some_and(|live| live.cleanup_only) {
                self.reject_preparation(preparation_generation);
            }
        }
        result
    }

    /// Called only after exact source/request admission. Authenticated health can
    /// precede CPA's asynchronous model registration; validate retained snapshots
    /// once per actual process before issuing a request capability. Ordinary
    /// discovery preserves its bounded pin wait and never forces remote refresh.
    pub(crate) fn ensure_process_accounts_current_locked(
        &self,
        inner: &mut RuntimeInner,
    ) -> Result<(), CpaLifecycleError> {
        if self.admission.snapshot().needs_account_validation
            || !inner
                .live
                .as_ref()
                .ok_or(CpaLifecycleError::NotStarted)?
                .accounts_validated_for_process
        {
            self.discover_materializations_locked(inner, None)?;
        }
        Ok(())
    }

    /// Revalidates the authority-owned native input before a request-scoped capability is
    /// issued. Same-account rotation advances only CPA's private generation and target epoch;
    /// replacement or unreadable authentication fails closed.
    pub(crate) fn ensure_attempt_account_current_locked(
        &self,
        inner: &mut RuntimeInner,
        expected: &AccountSnapshotRecord,
        prepared: Option<&crate::BorrowedSubscriptionEvidence>,
    ) -> Result<(), CpaLifecycleError> {
        if Some(expected.account_kind) != self.managed_kind() {
            return Ok(());
        }

        let previous_auth_generation = inner
            .live
            .as_ref()
            .and_then(|live| live.published_auth_generation);
        let refreshed = inner
            .live
            .as_mut()
            .ok_or(CpaLifecycleError::NotStarted)?
            .auth_lease
            .as_mut()
            .ok_or(CpaLifecycleError::OwnerState)?
            .refresh_subscription(prepared, Some(&expected.account_digest));
        crate::request_context::check()?;
        let current = refreshed.as_ref().ok().and_then(|identities| {
            identities.iter().find(|identity| {
                identity.account_kind == expected.account_kind
                    && identity.account_digest == expected.account_digest
            })
        });
        if current.is_some_and(|identity| Some(identity.generation) == previous_auth_generation) {
            return Ok(());
        }
        if let Some(current) = current
            && previous_auth_generation.is_none_or(|previous| current.generation > previous)
            && let Some(account) = inner.live.as_mut().and_then(|live| {
                live.accounts.iter_mut().find(|account| {
                    account.account_kind == expected.account_kind
                        && account.account_digest == expected.account_digest
                })
            })
        {
            let mut admission = self.admission.state.lock();
            crate::request_context::check()?;
            admission.auth_generation = admission.auth_generation.wrapping_add(1);
            account.generation = account
                .generation
                .checked_add(1)
                .ok_or(CpaLifecycleError::InvalidAccountState)?
                .max(current.generation);
            self.epochs.advance_target();
            if let Some(live) = inner.live.as_mut() {
                live.published_auth_generation = Some(current.generation);
                live.last_health = None;
            }
            drop(admission);
            if let Ok(layout) = self.prepare_layout()
                && let Some(live) = inner.live.as_ref()
            {
                let _ = save_account_state(&layout.accounts_path, &live.accounts);
            }
            return Ok(());
        }

        // A source revision changed between prepare and ownership. It is not
        // evidence against the newer native token; the next operation prepares it.
        if matches!(
            refreshed,
            Err(CpaLifecycleError::BorrowedClaudeAuthSourceChanged
                | CpaLifecycleError::BorrowedCodexAuthSourceChanged)
        ) {
            return refreshed.map(|_| ());
        }
        self.invalidate_runtime_accounts(inner, self.managed_kind());
        self.reject_preparation(self.preparation_generation());
        match refreshed {
            Err(error) => Err(error),
            Ok(_) if self.is_managed_oauth() => Err(CpaLifecycleError::ManagedOAuthAccountChanged),
            Ok(_) => Err(match self.managed_kind() {
                Some(crate::CpaAccountKind::Claude) => {
                    CpaLifecycleError::BorrowedClaudeAuthSourceChanged
                }
                _ => CpaLifecycleError::BorrowedCodexAuthSourceChanged,
            }),
        }
    }

    /// Immediately closes subscription admission after Local Control observes that the native
    /// authorization evidence no longer matches the committed source. A later exact enabled
    /// projection is the only way to reopen admission.
    pub fn suspend_subscription_execution(&self) {
        let mut admission = self.admission.state.lock();
        if !admission.subscription_execution_suspended {
            admission.subscription_execution_suspended = true;
            let accounts = admission
                .source_management
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            admission.revoked_accounts.extend(accounts);
            self.admission.advance();
            self.epochs.advance_target();
        }
    }

    /// Only the already-persisted source decision changes admission. No owner
    /// lock or disk access is needed to revoke a capability.
    pub fn apply_account_management(
        &self,
        account_ref: &str,
        source_revision: u64,
        state: CpaSourceManagementState,
    ) -> Result<(), CpaLifecycleError> {
        let account_digest = parse_account_ref(account_ref)?;
        if source_revision == 0 {
            return Err(CpaLifecycleError::InvalidSourceManagement);
        }
        let mut admission = self.admission.state.lock();
        let changed = update_management_projection(
            &mut admission.source_management,
            account_digest,
            source_revision,
            state,
        )?;
        let resumed = state == CpaSourceManagementState::Enabled
            && std::mem::replace(&mut admission.subscription_execution_suspended, false);
        if changed || resumed {
            if state == CpaSourceManagementState::Enabled {
                admission.needs_account_validation = true;
            } else {
                admission.revoked_accounts.insert(account_digest.to_owned());
            }
            self.admission.advance();
            self.epochs.advance_target();
        }
        Ok(())
    }

    pub(crate) fn preparation_generation(&self) -> u64 {
        self.admission.state.lock().auth_generation
    }

    pub(crate) fn reject_preparation(&self, generation: u64) {
        let mut admission = self.admission.state.lock();
        if self.reject_preparation_locked(&mut admission, generation) {
            drop(admission);
            // Best-effort cached projection; revocation above never waits for
            // the owner. A newer admitted operation must not be invalidated.
            if let Some(mut inner) = self.inner.try_lock() {
                let admission = self.admission.state.lock();
                if admission.auth_generation == generation
                    && admission.subscription_execution_suspended
                    && let Some(live) = inner.live.as_mut()
                {
                    deactivate_accounts(&mut live.accounts, self.managed_kind());
                    live.last_health = None;
                }
                drop(admission);
                self.publish_observation(&inner);
            }
        }
    }

    pub(super) fn reject_preparation_locked(
        &self,
        admission: &mut super::admission::AdmissionState,
        generation: u64,
    ) -> bool {
        if crate::request_context::check().is_ok()
            && admission.auth_generation == generation
            && !admission.subscription_execution_suspended
        {
            admission.subscription_execution_suspended = true;
            let accounts = admission
                .source_management
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            admission.revoked_accounts.extend(accounts);
            self.admission.advance();
            self.epochs.advance_target();
            return true;
        }
        false
    }
}

fn update_management_projection(
    projections: &mut BTreeMap<String, SourceManagementProjection>,
    account_digest: &str,
    source_revision: u64,
    state: CpaSourceManagementState,
) -> Result<bool, CpaLifecycleError> {
    if let Some(previous) = projections.get(account_digest) {
        if previous.revision > source_revision
            || (previous.revision == source_revision && previous.state != state)
        {
            return Err(CpaLifecycleError::StaleSourceManagement);
        }
        if previous.revision == source_revision {
            return Ok(false);
        }
    }
    projections.insert(
        account_digest.to_owned(),
        SourceManagementProjection {
            revision: source_revision,
            state,
        },
    );
    Ok(true)
}

pub(super) fn account_execution_is_admitted(
    projections: &BTreeMap<String, SourceManagementProjection>,
    subscription_execution_suspended: bool,
    account: &AccountSnapshotRecord,
) -> bool {
    !subscription_execution_suspended
        && projections
            .get(&account.account_digest)
            .is_some_and(|projection| projection.state == CpaSourceManagementState::Enabled)
}

pub(super) fn apply_management_projection(
    accounts: &mut [AccountSnapshotRecord],
    projections: &BTreeMap<String, SourceManagementProjection>,
) {
    for account in accounts {
        if projections
            .get(&account.account_digest)
            .is_some_and(|projection| projection.denies_admission())
        {
            account.active = false;
        }
    }
}

pub(super) fn deactivate_accounts(
    accounts: &mut [AccountSnapshotRecord],
    kind: Option<crate::CpaAccountKind>,
) -> bool {
    let mut changed = false;
    for account in accounts {
        if account.active && kind.is_none_or(|expected| account.account_kind == expected) {
            account.active = false;
            changed = true;
        }
    }
    changed
}

fn parse_account_ref(account_ref: &str) -> Result<&str, CpaLifecycleError> {
    let Some(digest) = account_ref.strip_prefix("account/cpa/") else {
        return Err(CpaLifecycleError::InvalidSourceManagement);
    };
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CpaLifecycleError::InvalidSourceManagement);
    }
    Ok(digest)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use crate::accounts::CpaAccountKind;

    use super::*;

    fn account(digest: char) -> AccountSnapshotRecord {
        AccountSnapshotRecord {
            account_kind: CpaAccountKind::Codex,
            stock_id: format!("stock-{digest}"),
            stock_auth_index: format!("index-{digest}"),
            stock_file_name: format!("codex-{digest}.json"),
            prefix: "hiroute-codex-current".into(),
            account_digest: digest.to_string().repeat(64),
            generation: 1,
            observed_model_ids: BTreeSet::from(["model.fixture".to_owned()]),
            active: true,
        }
    }

    #[test]
    fn disabled_projection_cannot_be_reactivated_by_fresh_discovery() {
        let mut accounts = vec![account('a'), account('b')];
        let projections = BTreeMap::from([(
            "a".repeat(64),
            SourceManagementProjection {
                revision: 2,
                state: CpaSourceManagementState::Disabled,
            },
        )]);

        apply_management_projection(&mut accounts, &projections);

        assert!(!accounts[0].active);
        assert!(accounts[1].active);
    }

    #[test]
    fn authorization_failure_deactivates_only_the_affected_account_kind() {
        let codex = account('a');
        let mut claude = account('b');
        claude.account_kind = CpaAccountKind::Claude;
        let mut accounts = vec![codex.clone(), claude.clone()];

        assert!(deactivate_accounts(
            &mut accounts,
            Some(CpaAccountKind::Codex)
        ));
        assert!(!accounts[0].active);
        assert!(accounts[1].active);
        assert!(!deactivate_accounts(
            &mut accounts,
            Some(CpaAccountKind::Codex)
        ));
    }

    #[test]
    fn source_management_accepts_only_exact_opaque_account_refs() {
        let digest = "a".repeat(64);
        let account_ref = format!("account/cpa/{digest}");
        assert_eq!(parse_account_ref(&account_ref).unwrap(), digest);
        assert!(parse_account_ref("account/cpa/not-a-digest").is_err());
        assert!(parse_account_ref(&format!("other/{}", "a".repeat(64))).is_err());
    }

    #[test]
    fn projection_rejects_stale_or_conflicting_management_results() {
        let digest = "a".repeat(64);
        let mut projections = BTreeMap::new();
        assert!(
            update_management_projection(
                &mut projections,
                &digest,
                4,
                CpaSourceManagementState::Disabled,
            )
            .unwrap()
        );
        assert!(
            !update_management_projection(
                &mut projections,
                &digest,
                4,
                CpaSourceManagementState::Disabled,
            )
            .unwrap()
        );
        assert!(matches!(
            update_management_projection(
                &mut projections,
                &digest,
                4,
                CpaSourceManagementState::Enabled,
            ),
            Err(CpaLifecycleError::StaleSourceManagement)
        ));
        assert!(matches!(
            update_management_projection(
                &mut projections,
                &digest,
                3,
                CpaSourceManagementState::Removed,
            ),
            Err(CpaLifecycleError::StaleSourceManagement)
        ));
    }

    #[test]
    fn codex_execution_uses_the_enabled_source_independent_of_credential_generation() {
        let current = account('a');
        let mut projections = BTreeMap::new();
        assert!(!account_execution_is_admitted(
            &projections,
            false,
            &current
        ));
        update_management_projection(
            &mut projections,
            &current.account_digest,
            1,
            CpaSourceManagementState::Enabled,
        )
        .unwrap();
        assert!(account_execution_is_admitted(&projections, false, &current));
        assert!(!account_execution_is_admitted(&projections, true, &current));

        let mut rotated = current.clone();
        rotated.generation += 1;
        assert!(account_execution_is_admitted(&projections, false, &rotated));
    }
}
