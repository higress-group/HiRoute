//! Backend-only, single-use confirmation lifetime. It carries no business authority.
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

pub const CONFIRMATION_TTL: Duration = Duration::from_secs(60);

/// Recovery advances the epoch while holding this same mutex. Check only after
/// acquiring it, before creating a permit or applying an already accepted decision.
pub async fn lock_current_session<'a, T>(
    sessions: &'a tokio::sync::Mutex<T>,
    epoch: &AtomicU64,
    captured: u64,
) -> Result<tokio::sync::MutexGuard<'a, T>, &'static str> {
    let guard = sessions.lock().await;
    if epoch.load(Ordering::SeqCst) != captured {
        return Err("CONFIRMATION_STALE");
    }
    Ok(guard)
}

#[derive(Default)]
pub struct ConfirmationGate {
    identity: Arc<()>,
    generation: u64,
    open: bool,
}
pub struct ConfirmationPermit {
    identity: Arc<()>,
    generation: u64,
    created: Instant,
}
/// Stable for the owning Session, including after its window confirmations expire.
pub struct ConfirmationOwner(Arc<()>);
impl ConfirmationGate {
    pub fn owner(&self) -> ConfirmationOwner {
        ConfirmationOwner(self.identity.clone())
    }
    pub fn check_owner(&self, owner: &ConfirmationOwner) -> Result<(), &'static str> {
        if Arc::ptr_eq(&self.identity, &owner.0) {
            Ok(())
        } else {
            Err("CONFIRMATION_STALE")
        }
    }
    pub fn is_open(&self) -> bool {
        self.open
    }
    pub fn begin(&mut self) -> Result<ConfirmationPermit, &'static str> {
        if self.open {
            return Err("CONFIRMATION_ALREADY_OPEN");
        }
        self.generation = self.generation.wrapping_add(1);
        self.open = true;
        Ok(ConfirmationPermit {
            identity: self.identity.clone(),
            generation: self.generation,
            created: Instant::now(),
        })
    }
    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.open = false;
    }
    pub fn finish(
        &mut self,
        permit: &ConfirmationPermit,
        accepted: bool,
    ) -> Result<bool, &'static str> {
        if !Arc::ptr_eq(&self.identity, &permit.identity)
            || !self.open
            || permit.generation != self.generation
        {
            return Err("CONFIRMATION_STALE");
        }
        self.open = false;
        if accepted && permit.created.elapsed() > CONFIRMATION_TTL {
            return Err("CONFIRMATION_EXPIRED");
        }
        Ok(accepted)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn queued_preview_cannot_open_a_gate_in_the_replacement_session() {
        let epoch = AtomicU64::new(0);
        let sessions = tokio::sync::Mutex::new(ConfirmationGate::default());
        let mut recovery = sessions.lock().await;
        let captured = epoch.load(Ordering::SeqCst);
        let preview = async {
            let mut session = lock_current_session(&sessions, &epoch, captured).await?;
            session.begin()
        };
        tokio::pin!(preview);
        tokio::select! {
            biased;
            _ = &mut preview => panic!("preview passed the held recovery lock"),
            _ = tokio::task::yield_now() => {}
        }
        *recovery = ConfirmationGate::default();
        epoch.fetch_add(1, Ordering::SeqCst);
        drop(recovery);
        assert!(matches!(preview.await, Err("CONFIRMATION_STALE")));
        let mut session = lock_current_session(&sessions, &epoch, 1).await.unwrap();
        let fresh = session.begin().unwrap();
        assert!(session.finish(&fresh, true).unwrap());
    }
    #[tokio::test]
    async fn accepted_decision_waiting_for_the_lock_cannot_apply_after_recovery() {
        let epoch = AtomicU64::new(0);
        let sessions = tokio::sync::Mutex::new(Vec::<&str>::new());
        let recovery = sessions.lock().await;
        let captured = epoch.load(Ordering::SeqCst);
        let apply = async {
            let mut session = lock_current_session(&sessions, &epoch, captured).await?;
            session.push("old accepted decision");
            Ok::<_, &'static str>(())
        };
        tokio::pin!(apply);
        tokio::select! {
            biased;
            _ = &mut apply => panic!("apply passed the held recovery lock"),
            _ = tokio::task::yield_now() => {}
        }
        epoch.fetch_add(1, Ordering::SeqCst);
        drop(recovery);
        assert!(matches!(apply.await, Err("CONFIRMATION_STALE")));
        assert!(sessions.lock().await.is_empty());
        lock_current_session(&sessions, &epoch, 1)
            .await
            .unwrap()
            .push("new accepted decision");
        assert_eq!(*sessions.lock().await, ["new accepted decision"]);
    }
    #[test]
    fn rejection_and_duplicate_callback_never_reopen_a_consumed_confirmation() {
        let mut gate = ConfirmationGate::default();
        let cancelled = gate.begin().unwrap();
        assert!(gate.begin().is_err());
        assert!(!gate.finish(&cancelled, false).unwrap());
        assert_eq!(gate.finish(&cancelled, true), Err("CONFIRMATION_STALE"));
        let accepted = gate.begin().unwrap();
        assert!(gate.finish(&accepted, true).unwrap());
        assert_eq!(gate.finish(&accepted, true), Err("CONFIRMATION_STALE"));
    }
    #[test]
    fn window_close_and_reopen_reject_the_old_callback_without_consuming_the_new_one() {
        let mut gate = ConfirmationGate::default();
        let old = gate.begin().unwrap();
        gate.invalidate();
        let new = gate.begin().unwrap();
        assert_eq!(gate.finish(&old, true), Err("CONFIRMATION_STALE"));
        assert!(gate.finish(&new, true).unwrap());
    }
    #[test]
    fn replacement_session_rejects_old_first_permit_and_keeps_its_own_confirmation() {
        let mut old = ConfirmationGate::default();
        let old_permit = old.begin().unwrap();
        let mut replacement = ConfirmationGate::default();
        let new_permit = replacement.begin().unwrap();
        // Both generations begin at one; the owning gate must still differ.
        assert_eq!(old_permit.generation, new_permit.generation);
        assert_eq!(
            replacement.finish(&old_permit, true),
            Err("CONFIRMATION_STALE")
        );
        assert_eq!(
            replacement.finish(&old_permit, false),
            Err("CONFIRMATION_STALE")
        );
        assert!(replacement.finish(&new_permit, true).unwrap());
    }
    #[test]
    fn window_close_preserves_submitted_completion_owner_but_recovery_rejects_it() {
        let mut session = ConfirmationGate::default();
        let permit = session.begin().unwrap();
        assert!(session.finish(&permit, true).unwrap());
        let completion_owner = session.owner();
        // Close may find no operation yet. Its later completion must still run cleanup.
        session.invalidate();
        assert_eq!(session.check_owner(&completion_owner), Ok(()));
        let replacement = ConfirmationGate::default();
        assert_eq!(
            replacement.check_owner(&completion_owner),
            Err("CONFIRMATION_STALE")
        );
    }
    #[test]
    fn expired_confirmation_is_consumed_and_requires_a_fresh_preview() {
        let mut gate = ConfirmationGate::default();
        let mut expired = gate.begin().unwrap();
        expired.created -= Duration::from_secs(61);
        assert_eq!(gate.finish(&expired, true), Err("CONFIRMATION_EXPIRED"));
        assert_eq!(gate.finish(&expired, true), Err("CONFIRMATION_STALE"));
        let fresh = gate.begin().unwrap();
        assert!(gate.finish(&fresh, true).unwrap());
    }
}
