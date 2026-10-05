//! Operation-owned publication checkpoints and serving availability.
use hiroute_diagnostics::publication::{PublicationStage, measure};
use std::sync::Arc;

use hiroute_application::publication::{PublicationTargetError, PublicationTargetPort};
use hiroute_domain::{OperationState, OperationV1};

use super::*;

pub(super) fn target_error(_: PublicationTargetError) -> PortError {
    PortError::new(PortErrorCode::Unavailable, "publication.target.unavailable")
}

impl LocalControlAdapter {
    pub(in crate::control::runtime) fn validate_publication_admission(
        &self,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<()> {
        if intent.kind() != OwnedEffectKind::Publication {
            return Ok(());
        }
        self.guard_codex_pending_change(None)?;
        self.guard_additional_pending_model_change(None)?;
        if intent.desired()["transaction"] == "settings" {
            hiroute_domain::validate_settings_model_publication_intent(intent)
                .map_err(|_| invalid("publication.settings.admission"))?;
        }
        if let Some(record) = routing_publication_record(intent)
            .map_err(|_| invalid("publication.admission.record"))?
        {
            self.validate_additional_installed_model_budgets(&record)?;
            record
                .verify()
                .map_err(|_| invalid("publication.admission.verify"))?
                .gateway_snapshot()
                .map_err(|_| invalid("publication.admission.projection"))?;
        }
        self.required_publication_target()?;
        Ok(())
    }
    pub(in crate::control::runtime) fn publication_target(
        &self,
    ) -> PortResult<Option<Arc<dyn PublicationTargetPort + Send + Sync>>> {
        self.publication_target
            .lock()
            .map(|target| target.clone())
            .map_err(|_| PortError::new(PortErrorCode::Unavailable, "publication.target.lock"))
    }

    pub(super) fn required_publication_target(
        &self,
    ) -> PortResult<Arc<dyn PublicationTargetPort + Send + Sync>> {
        self.publication_target()?
            .ok_or_else(|| target_error(PublicationTargetError::Unavailable))
    }

    pub(in crate::control::runtime) fn publication_is_installed(
        &self,
        record: &PublicationRecordV1,
    ) -> PortResult<bool> {
        let publication = record
            .verify()
            .map_err(|_| invalid("publication.target.record"))?;
        publication
            .gateway_snapshot()
            .map_err(|_| invalid("publication.target.projection"))?;
        self.required_publication_target()?
            .verify_installed(record)
            .map_err(target_error)
    }

    pub(in crate::control::runtime) fn begin_product_activation(
        &self,
        operation: &OperationV1,
    ) -> PortResult<()> {
        self.require_current_operation(operation)?;
        self.guard_codex_pending_change(Some(&operation.operation_id))?;
        self.guard_additional_pending_model_change(Some(&operation.operation_id))?;
        self.begin_plan_content_activation(operation)?;
        // Settings model publications are hot-swappable: the aggregate ArcSwap cutover keeps
        // every in-flight pin on its old root, so normal B updates never suspend admission.
        if operation.plan.external().iter().any(|intent| {
            intent.kind() == OwnedEffectKind::Publication
                && !hiroute_domain::is_settings_publication(intent)
        }) && let Some(target) = self.publication_target()?
        {
            target.suspend_requests().map_err(target_error)?;
        }
        Ok(())
    }

    pub(in crate::control::runtime) fn publication_install_checkpoint(
        &self,
        operation: &OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<OwnedEffectV1> {
        let diagnostics = self
            .publication_diagnostics
            .lock()
            .map(|p| p.clone())
            .unwrap_or_default();
        measure(
            &diagnostics,
            PublicationStage::PublicationCheckpoint,
            Some(operation.operation_id.as_str()),
            None,
            || {
                if effect.kind != OwnedEffectKind::Publication {
                    return Ok(effect.clone());
                }
                let (_, mut marker) = self.publication_marker(operation, effect)?;
                if marker.decision == InstallationDecision::Abort {
                    return Err(invalid("publication.install.aborted"));
                }
                if marker.decision == InstallationDecision::Install {
                    return Ok(effect.clone());
                }
                let record = marker
                    .record
                    .as_ref()
                    .expect("publication_marker always returns the current shape");
                self.validate_additional_installed_model_budgets(record)?;
                record
                    .verify()
                    .map_err(|_| invalid("publication.install.record"))?
                    .gateway_snapshot()
                    .map_err(|_| invalid("publication.install.projection"))?;
                self.required_publication_target()?;
                marker.schema = PUBLICATION_MARKER_SCHEMA.to_owned();
                marker.decision = InstallationDecision::Install;
                let mut checkpoint = effect.clone();
                checkpoint.compensation = serde_json::to_value(marker)
                    .map_err(|_| invalid("publication.checkpoint.encode"))?
                    .into();
                Ok(checkpoint)
            },
        )
    }

    pub(in crate::control::runtime) fn publication_abort_checkpoint(
        &self,
        operation: &OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<OwnedEffectV1> {
        let diagnostics = self
            .publication_diagnostics
            .lock()
            .map(|p| p.clone())
            .unwrap_or_default();
        measure(
            &diagnostics,
            PublicationStage::PublicationCheckpoint,
            Some(operation.operation_id.as_str()),
            None,
            || {
                if effect.kind != OwnedEffectKind::Publication {
                    return Ok(effect.clone());
                }
                let (_, mut marker) = self.publication_marker(operation, effect)?;
                if marker.decision == InstallationDecision::Abort {
                    return Ok(effect.clone());
                }
                let (active, lkg) = {
                    let stores = self.stores_lock()?;
                    (
                        stores.control().active_publication(&marker.workspace_id)?,
                        stores
                            .control()
                            .last_known_good_publication(&marker.workspace_id)?,
                    )
                };
                if !marker.no_op
                    && active
                        .as_ref()
                        .is_some_and(|record| record.digest == marker.after_digest)
                {
                    return Err(target_error(PublicationTargetError::Unavailable));
                }
                let before = active
                    .into_iter()
                    .chain(lkg)
                    .find(|record| Some(&record.digest) == marker.before_digest.as_ref());
                let still_before = match before {
                    Some(record) => self.publication_is_installed(&record)?,
                    None if marker.before_digest.is_none() => match self.publication_target()? {
                        Some(target) => target.is_empty().map_err(target_error)?,
                        None => true,
                    },
                    None => false,
                };
                if !still_before {
                    // Preserve dependent secrets and the durable writer claim. A fresh installer on
                    // restart can complete the exact prepared record after a durability-uncertain error.
                    return Err(target_error(PublicationTargetError::Unavailable));
                }
                marker.decision = InstallationDecision::Abort;
                marker.schema = PUBLICATION_MARKER_SCHEMA.to_owned();
                let mut checkpoint = effect.clone();
                checkpoint.compensation = serde_json::to_value(marker)
                    .map_err(|_| invalid("publication.abort.encode"))?
                    .into();
                Ok(checkpoint)
            },
        )
    }

    pub(in crate::control::runtime) fn finish_product_activation(
        &self,
        operation: &OperationV1,
    ) -> PortResult<()> {
        self.require_current_operation(operation)?;
        let diagnostics = self
            .publication_diagnostics
            .lock()
            .map(|p| p.clone())
            .unwrap_or_default();
        measure(
            &diagnostics,
            PublicationStage::ProductActivationFinish,
            Some(operation.operation_id.as_str()),
            None,
            || {
                self.finish_plan_content_activation(operation)?;
                if matches!(
                    operation.state,
                    OperationState::Succeeded | OperationState::RolledBack
                ) && operation
                    .plan
                    .external()
                    .iter()
                    .any(|intent| intent.kind() == OwnedEffectKind::Publication)
                {
                    self.reconcile_active_publication()?;
                }
                Ok(())
            },
        )
    }

    pub(in crate::control::runtime) fn reconcile_active_publication(&self) -> PortResult<()> {
        self.restore_active_publication()?;
        if !self
            .startup_recovery_complete
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Ok(());
        }
        self.resume_active_publication()
    }

    pub(in crate::control::runtime) fn finish_startup_publication_recovery(
        &self,
    ) -> PortResult<()> {
        self.stores_lock()?
            .validate_recovered_grant_publications()
            .map_err(|_| invalid("publication.startup.grant-integrity"))?;
        self.restore_active_publication()?;
        self.resume_active_publication()?;
        self.startup_recovery_complete
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }

    fn resume_active_publication(&self) -> PortResult<()> {
        if let Some(target) = self.publication_target()? {
            if self
                .stores_lock()?
                .control()
                .active_publication(&WorkspaceId::default())?
                .is_none()
                && !target.is_empty().map_err(target_error)?
            {
                return Err(invalid("publication.startup.orphan-target"));
            }
            target.resume_requests().map_err(target_error)?;
        }
        Ok(())
    }

    /// Reconstruct committed authority before journal recovery observes service receipts.
    /// Admission remains suspended until the complete startup recovery succeeds.
    pub(in crate::control::runtime) fn restore_active_publication(&self) -> PortResult<()> {
        let record = self
            .stores_lock()?
            .control()
            .active_publication(&WorkspaceId::default())?;
        let Some(target) = self.publication_target()? else {
            if let Some(record) = &record {
                record
                    .verify()
                    .map_err(|_| invalid("publication.startup.record"))?
                    .gateway_snapshot()
                    .map_err(|_| invalid("publication.startup.projection"))?;
                return Err(target_error(PublicationTargetError::Unavailable));
            }
            return Ok(());
        };
        if let Some(record) = record
            && !self.publication_is_installed(&record)?
        {
            target.activate_verified(&record).map_err(target_error)?;
            if !self.publication_is_installed(&record)? {
                return Err(invalid("publication.startup.identity"));
            }
        }
        Ok(())
    }
}
