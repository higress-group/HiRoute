use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwapOption;
use hiroute_application::publication::{PublicationTargetError, PublicationTargetPort};
use hiroute_domain::PublicationRecordV1;
use hiroute_gateway::server::composition::RuntimePublicationFeed;
use hiroute_gateway::server::publication::{
    GatewayPrepareOutcome, GatewayPublicationInstaller, PublishedGatewayPublication,
};

/// Installs one Product-sealed aggregate and exposes an immutable request pin
/// decorated with observation-only facts from that verified Product record.
pub struct GatewayPublicationAdapter {
    installer: Arc<GatewayPublicationInstaller>,
    active: ArcSwapOption<PublishedGatewayPublication>,
    suspended: AtomicBool,
    // Status uses try_lock: it never waits for an install's durable I/O.
    // Request pinning remains lock-free; this lock is only for publication mutations.
    mutation: Mutex<()>,
    // Only the pure expected identity of one exact authenticated Product record.
    // Live installation/suspension/durability are observed on every status read.
    expected: Mutex<Option<(PublicationRecordV1, ExpectedPublication)>>,
    #[cfg(test)]
    projection_builds: std::sync::atomic::AtomicUsize,
}

#[derive(Clone)]
struct ExpectedPublication {
    workspace: String,
    revision: u64,
    digest: String,
}

impl GatewayPublicationAdapter {
    pub fn new(installer: Arc<GatewayPublicationInstaller>) -> Self {
        let active = installer.active();
        Self {
            installer,
            active: ArcSwapOption::new(active),
            suspended: AtomicBool::new(true),
            mutation: Mutex::new(()),
            expected: Mutex::new(None),
            #[cfg(test)]
            projection_builds: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    #[cfg(test)]
    pub(crate) fn installer(&self) -> &Arc<GatewayPublicationInstaller> {
        &self.installer
    }

    fn expected_identity(
        &self,
        record: &PublicationRecordV1,
    ) -> Result<ExpectedPublication, PublicationTargetError> {
        let mut cache = self
            .expected
            .try_lock()
            .map_err(|_| PublicationTargetError::Unavailable)?;
        if let Some((previous, expected)) = cache.as_ref()
            && previous == record
        {
            return Ok(expected.clone());
        }
        *cache = None;
        let projection = record
            .gateway_snapshot()
            .map_err(|_| PublicationTargetError::VerificationFailed)?;
        let projected = hiroute_integrations::gateway::project_publication(&projection)
            .map_err(|_| PublicationTargetError::VerificationFailed)?;
        #[cfg(test)]
        self.projection_builds.fetch_add(1, Ordering::Relaxed);
        let expected = ExpectedPublication {
            workspace: projected.workspace_id,
            revision: projected.publication_revision,
            digest: projected.payload_digest,
        };
        if record
            .bytes
            .len()
            .saturating_mul(8)
            .saturating_add(64 * 1024)
            <= 64 * 1024 * 1024
        {
            *cache = Some((record.clone(), expected.clone()));
        }
        Ok(expected)
    }
}

impl PublicationTargetPort for GatewayPublicationAdapter {
    fn observes_verified(
        &self,
        record: Option<&PublicationRecordV1>,
    ) -> Result<bool, PublicationTargetError> {
        let _observation = self
            .mutation
            .try_lock()
            .map_err(|_| PublicationTargetError::Unavailable)?;
        if self.installer.durability_uncertain() {
            return Err(PublicationTargetError::Unavailable);
        }
        if self.suspended.load(Ordering::Acquire) {
            return Ok(false);
        }
        let Some(record) = record else {
            return self.is_empty();
        };
        // verify_installed authenticates the complete record and its Gateway projection.
        self.verify_installed(record)
    }
    fn verify_installed(
        &self,
        record: &PublicationRecordV1,
    ) -> Result<bool, PublicationTargetError> {
        if self.installer.durability_uncertain() {
            return Err(PublicationTargetError::Unavailable);
        }
        let expected = self.expected_identity(record)?;
        Ok(self.active.load_full().is_some_and(|active| {
            active.workspace_id() == expected.workspace
                && active.publication_revision() == expected.revision
                && active.payload_digest() == expected.digest
        }))
    }
    fn is_empty(&self) -> Result<bool, PublicationTargetError> {
        if self.installer.durability_uncertain() {
            return Err(PublicationTargetError::Unavailable);
        }
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
        if self.installer.durability_uncertain() {
            return Err(PublicationTargetError::Unavailable);
        }
        self.suspended.store(false, Ordering::Release);
        Ok(())
    }
    fn activate_verified(
        &self,
        record: &PublicationRecordV1,
    ) -> Result<(), PublicationTargetError> {
        let projection = record
            .gateway_snapshot()
            .map_err(|_| PublicationTargetError::VerificationFailed)?;
        let snapshot = hiroute_integrations::gateway::project_publication(&projection)
            .map_err(|_| PublicationTargetError::VerificationFailed)?;
        let pricing_snapshot = snapshot.clone();
        let _mutation = self
            .mutation
            .lock()
            .map_err(|_| PublicationTargetError::Unavailable)?;
        let active = match self
            .installer
            .prepare(snapshot)
            .map_err(|_| PublicationTargetError::Unavailable)?
        {
            GatewayPrepareOutcome::Prepared(prepared) => {
                if crate::publication_failpoint::requested("after_gateway_durable") {
                    let _ = self.installer.publish_with_failpoint(
                        prepared,
                        hiroute_gateway::server::publication::PublicationFailpoint::AfterDurableLkg,
                    );
                    crate::publication_failpoint::crash("after_gateway_durable");
                    return Err(PublicationTargetError::Unavailable);
                }
                self.installer
                    .publish(prepared)
                    .map_err(|_| PublicationTargetError::Unavailable)?
            }
            GatewayPrepareOutcome::Duplicate(active) => active
                .with_verified_request_pricing(&pricing_snapshot)
                .map_err(|_| PublicationTargetError::VerificationFailed)?,
        };
        self.active.store(Some(active));
        Ok(())
    }
}

impl RuntimePublicationFeed for GatewayPublicationAdapter {
    fn pin(&self) -> Option<Arc<PublishedGatewayPublication>> {
        let pinned = self.active.load_full();
        if self.suspended.load(Ordering::Acquire) || self.installer.durability_uncertain() {
            None
        } else {
            pinned
        }
    }
}

#[cfg(test)]
#[path = "publication_tests.rs"]
mod tests;
