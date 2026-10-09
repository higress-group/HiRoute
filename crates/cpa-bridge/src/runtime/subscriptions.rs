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
    fn denies_admission(self) -> bool {
        matches!(
            self.state,
            CpaSourceManagementState::Disabled | CpaSourceManagementState::Removed
        )
    }
}

impl ManagedCpaRuntime {
    /// Revalidates the authority-owned native input before a request-scoped capability is
    /// issued. Same-account rotation advances only CPA's private generation and target epoch;
    /// replacement or unreadable authentication fails closed.
    pub(crate) fn ensure_attempt_account_current_locked(
        &self,
        inner: &mut RuntimeInner,
        expected: &AccountSnapshotRecord,
    ) -> Result<(), CpaLifecycleError> {
        if Some(expected.account_kind) != self.managed_kind() {
            return Ok(());
        }

        let previous_auth_generation = inner
            .live
            .as_ref()
            .and_then(|live| live.auth_lease.as_ref())
            .and_then(|lease| lease.generation());
        let refreshed = inner
            .live
            .as_mut()
            .ok_or(CpaLifecycleError::NotStarted)?
            .auth_lease
            .as_mut()
            .ok_or(CpaLifecycleError::OwnerState)?
            .refresh_subscription(None, Some(&expected.account_digest));
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
            && previous_auth_generation.is_some_and(|previous| current.generation > previous)
            && let Some(account) = inner.live.as_mut().and_then(|live| {
                live.accounts.iter_mut().find(|account| {
                    account.account_kind == expected.account_kind
                        && account.account_digest == expected.account_digest
                })
            })
        {
            account.generation = account
                .generation
                .checked_add(1)
                .ok_or(CpaLifecycleError::InvalidAccountState)?
                .max(current.generation);
            self.epochs.advance_target();
            if let Ok(layout) = self.prepare_layout()
                && let Some(live) = inner.live.as_ref()
            {
                let _ = save_account_state(&layout.accounts_path, &live.accounts);
            }
            return Ok(());
        }

        let was_suspended = std::mem::replace(&mut inner.subscription_execution_suspended, true);
        let accounts_changed = inner
            .live
            .as_mut()
            .is_some_and(|live| deactivate_accounts(&mut live.accounts, self.managed_kind()));
        if !was_suspended || accounts_changed {
            self.epochs.advance_target();
        }
        if accounts_changed
            && let Ok(layout) = self.prepare_layout()
            && let Some(live) = inner.live.as_ref()
        {
            let _ = save_account_state(&layout.accounts_path, &live.accounts);
        }
        match refreshed {
            Err(error) => Err(error),
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
        let mut inner = self.inner.lock();
        if !inner.subscription_execution_suspended {
            inner.subscription_execution_suspended = true;
            self.epochs.advance_target();
        }
    }

    /// Applies Application's already-persisted source decision to this runtime projection.
    /// Re-enabling only permits a future verified discovery; it never makes cached facts active.
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
        let mut inner = self.inner.lock();
        let projection_changed = update_management_projection(
            &mut inner.source_management,
            account_digest,
            source_revision,
            state,
        )?;
        let resumed = state == CpaSourceManagementState::Enabled
            && std::mem::replace(&mut inner.subscription_execution_suspended, false);
        if !projection_changed && !resumed {
            return Ok(());
        }

        let Some(live) = inner.live.as_mut() else {
            return Ok(());
        };
        let mut changed = false;
        if state != CpaSourceManagementState::Enabled {
            for account in &mut live.accounts {
                if account.account_digest == account_digest && account.active {
                    account.active = false;
                    changed = true;
                }
            }
        }
        if changed {
            self.epochs.advance_target();
            let layout = self.prepare_layout()?;
            save_account_state(&layout.accounts_path, &live.accounts)?;
        }
        Ok(())
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
