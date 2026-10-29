use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwapOption;
use hiroute_domain::{GatewayPublicationV1, PublicationRecordV1};
use thiserror::Error;

pub trait PublicationTargetPort {
    /// A coherent proof of installation AND serving availability, for client status.
    /// Recovery must use `verify_installed` instead: a suspended target can be installed.
    fn observes_verified(
        &self,
        _record: Option<&PublicationRecordV1>,
    ) -> Result<bool, PublicationTargetError> {
        Err(PublicationTargetError::Unavailable)
    }
    /// Proves installation independently of request suspension, for recovery reconciliation.
    fn verify_installed(
        &self,
        _record: &PublicationRecordV1,
    ) -> Result<bool, PublicationTargetError> {
        Err(PublicationTargetError::Unavailable)
    }
    fn is_empty(&self) -> Result<bool, PublicationTargetError> {
        Err(PublicationTargetError::Unavailable)
    }
    fn suspend_requests(&self) -> Result<(), PublicationTargetError> {
        Ok(())
    }
    fn resume_requests(&self) -> Result<(), PublicationTargetError> {
        Ok(())
    }
    /// Installs one already-prepared closed aggregate with one atomic pointer swap.
    fn activate_verified(&self, record: &PublicationRecordV1)
    -> Result<(), PublicationTargetError>;
}

#[derive(Default)]
pub struct AtomicPublicationTarget {
    active: ArcSwapOption<GatewayPublicationV1>,
    suspended: AtomicBool,
    mutation: Mutex<()>,
}

impl AtomicPublicationTarget {
    /// Pins both GatewayPublicationRevision and every AgentPlanRevision for one logical request.
    /// A later activation cannot change this Arc.
    pub fn pin_request(&self) -> Option<Arc<GatewayPublicationV1>> {
        let pinned = self.active.load_full();
        if self.suspended.load(Ordering::Acquire) {
            None
        } else {
            pinned
        }
    }
}

impl PublicationTargetPort for AtomicPublicationTarget {
    fn observes_verified(
        &self,
        record: Option<&PublicationRecordV1>,
    ) -> Result<bool, PublicationTargetError> {
        let _observation = self
            .mutation
            .try_lock()
            .map_err(|_| PublicationTargetError::Unavailable)?;
        if self.suspended.load(Ordering::Acquire) {
            return Ok(false);
        }
        match record {
            Some(record) => {
                record
                    .gateway_snapshot()
                    .map_err(|_| PublicationTargetError::VerificationFailed)?;
                self.verify_installed(record)
            }
            None => self.is_empty(),
        }
    }
    fn verify_installed(
        &self,
        record: &PublicationRecordV1,
    ) -> Result<bool, PublicationTargetError> {
        let expected = record
            .verify()
            .map_err(|_| PublicationTargetError::VerificationFailed)?;
        Ok(self
            .active
            .load_full()
            .is_some_and(|active| *active == expected))
    }
    fn is_empty(&self) -> Result<bool, PublicationTargetError> {
        Ok(self.active.load_full().is_none())
    }
    fn suspend_requests(&self) -> Result<(), PublicationTargetError> {
        let _mutation = self
            .mutation
            .lock()
            .map_err(|_| PublicationTargetError::Unavailable)?;
        self.suspended.store(true, Ordering::Release);
        Ok(())
    }
    fn resume_requests(&self) -> Result<(), PublicationTargetError> {
        let _mutation = self
            .mutation
            .lock()
            .map_err(|_| PublicationTargetError::Unavailable)?;
        self.suspended.store(false, Ordering::Release);
        Ok(())
    }
    fn activate_verified(
        &self,
        record: &PublicationRecordV1,
    ) -> Result<(), PublicationTargetError> {
        let publication = record
            .verify()
            .map_err(|_| PublicationTargetError::VerificationFailed)?;
        let _mutation = self
            .mutation
            .lock()
            .map_err(|_| PublicationTargetError::Unavailable)?;
        self.active.store(Some(Arc::new(publication)));
        Ok(())
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PublicationTargetError {
    #[error("prepared publication verification failed")]
    VerificationFailed,
    #[error("publication target is unavailable")]
    Unavailable,
}
