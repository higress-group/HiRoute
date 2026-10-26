//! Retry empty transient directories only from exact saved-source maintenance.
use super::*;

const FIRST_RETRY_DELAY: Duration = Duration::from_secs(5);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct CatalogRecovery {
    process_id: Option<u32>,
    next_retry: Option<Instant>,
    delay: Option<Duration>,
    current_failure: bool,
}

impl CatalogRecovery {
    pub(super) fn clear_observation(&mut self) {
        self.current_failure = false;
    }

    pub(super) fn observe(&mut self, error: &AccountDiscoveryError, now: Instant) {
        let AccountDiscoveryError::TransientModelCatalog { process_id, .. } = error else {
            self.current_failure = false;
            return;
        };
        if self.process_id != Some(*process_id) {
            *self = Self {
                process_id: Some(*process_id),
                next_retry: Some(now),
                delay: Some(FIRST_RETRY_DELAY),
                current_failure: true,
            };
        } else {
            // Re-reading an unchanged failure must neither postpone recovery nor
            // reset the backoff. A new failed attempt retains that same bound.
            self.current_failure = true;
        }
    }

    fn begin_retry(&mut self, now: Instant) -> bool {
        if !self.current_failure || self.next_retry.is_none_or(|next| now < next) {
            return false;
        }
        let delay = self.delay.unwrap_or(FIRST_RETRY_DELAY);
        self.next_retry = Some(now + delay);
        self.delay = Some(delay.saturating_mul(2).min(MAX_RETRY_DELAY));
        self.current_failure = false;
        true
    }
}

impl ManagedCpaRuntime {
    pub(super) fn recover_saved_catalog_locked(
        &self,
        inner: &mut RuntimeInner,
    ) -> Result<Vec<CpaAccountMaterializationV1>, CpaLifecycleError> {
        let deadline = Instant::now() + self.spec.control_timeout;
        let result = self.discover_materializations_locked(inner, None);
        if result.is_ok() || !inner.catalog_recovery.begin_retry(Instant::now()) {
            return result;
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return result;
        };
        // One actual refresh, sharing the original control budget. No token
        // refresh, native probe or synthetic inventory is introduced. All source,
        // account, prefix and final publication guards run again in this path.
        self.discover_materializations_locked_with_refresh(inner, None, true, remaining)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_observations_cannot_postpone_or_reset_bounded_recovery() {
        let failure = AccountDiscoveryError::TransientModelCatalog { process_id: 123 };
        let now = Instant::now();
        let mut recovery = CatalogRecovery::default();
        recovery.observe(&failure, now);
        assert!(recovery.begin_retry(now));
        for offset in 1..5 {
            recovery.observe(&failure, now + Duration::from_secs(offset));
            assert!(!recovery.begin_retry(now + Duration::from_secs(offset)));
        }
        let mut elapsed = 5;
        for next_delay in [10, 20, 40, 60, 60] {
            let current = now + Duration::from_secs(elapsed);
            recovery.observe(&failure, current);
            assert!(recovery.begin_retry(current));
            recovery.observe(&failure, current + Duration::from_secs(1));
            assert!(!recovery.begin_retry(current + Duration::from_secs(next_delay - 1)));
            elapsed += next_delay;
        }
        recovery.clear_observation();
        assert!(!recovery.begin_retry(now + Duration::from_secs(1000)));
        recovery.observe(&AccountDiscoveryError::PinNotApplied, now);
        assert!(!recovery.begin_retry(now + Duration::from_secs(1000)));
        recovery.observe(
            &AccountDiscoveryError::TransientModelCatalog { process_id: 456 },
            now,
        );
        assert!(recovery.begin_retry(now));
    }
}
