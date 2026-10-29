//! Product-publication effects for Local Control.
//!
//! `control.db` is the durable product authority. Executable publications are installed into the
//! composition-owned Gateway target before the product active pointer advances. Plan-only
//! publications install an identified `NoNewCalls` state instead of reviving stale routes.

use hiroute_diagnostics::publication::{PublicationStage, measure};
use hiroute_domain::{
    AgentAccessGrantMutationKindV1, AgentAccessGrantRefV1, CanonicalDigest, CompensationOutcome,
    EffectReconciliation, ExternalEffectIntentV1, GatewayPublicationRevision, OperationId,
    OperationStepKind, OperationStepStatus, OperationV1, OwnedEffectKind, OwnedEffectV1, PortError,
    PortErrorCode, PortResult, PublicationRecordV1, PublicationRepositoryPort, SecretStorePort,
    WorkspaceId, agent_connection_publication_record, agent_connection_restore_publication_record,
    is_agent_access_grant_effect, routing_publication_record,
};
use serde::{Deserialize, Serialize};

use super::LocalControlAdapter;

pub(super) const PUBLICATION_TARGET: &str = "publication/current";
const PUBLICATION_MARKER_SCHEMA: &str = "hiroute.product-publication-marker/v2";
const LEGACY_MARKER_SCHEMA: &str = "hiroute.product-publication-marker/v1";

#[cfg(test)]
thread_local! { pub(super) static MARKER_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
thread_local! { pub(super) static SETTINGS_RECORD_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

mod lifecycle;
mod settings_candidate;
use lifecycle::target_error;
pub(super) use settings_candidate::SettingsCandidate;

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum InstallationDecision {
    #[default]
    Prepared,
    Install,
    Abort,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PublicationEffectMarkerV1 {
    schema: String,
    operation_id: String,
    workspace_id: WorkspaceId,
    publication_revision: u64,
    before_digest: Option<CanonicalDigest>,
    after_digest: CanonicalDigest,
    no_op: bool,
    #[serde(default)]
    record: Option<PublicationRecordV1>,
    #[serde(default)]
    decision: InstallationDecision,
}

// One immutable parsed marker. Holding the Arc means a caller's Arc::make_mut
// produces a different allocation; pointer equality never accepts changed content.
// The operation, current journal and publication bindings are rechecked by callers.
pub(super) struct DecodedMarker {
    effect: OwnedEffectV1,
    operation_id: OperationId,
    marker: PublicationEffectMarkerV1,
}
impl DecodedMarker {
    fn matches(&self, effect: &OwnedEffectV1) -> bool {
        std::sync::Arc::ptr_eq(&self.effect.compensation, &effect.compensation)
            && self.effect.effect_id == effect.effect_id
            && self.effect.kind == effect.kind
            && self.effect.target == effect.target
            && self.effect.before_fingerprint == effect.before_fingerprint
            && self.effect.after_fingerprint == effect.after_fingerprint
    }
}

impl LocalControlAdapter {
    fn succeeded_connection_grant_reference(
        &self,
        connection_id: &str,
    ) -> PortResult<AgentAccessGrantRefV1> {
        let stores = self.stores_lock()?;
        for operation in stores.control().succeeded_agent_operations_for_kind(
            &WorkspaceId::default(),
            "ApplyAgentConnectionChange",
        )? {
            let projection = operation
                .plan
                .agent_connection_projection()
                .map_err(|_| invalid("publication.agent.projection"))?;
            if !projection
                .as_ref()
                .is_some_and(|projection| projection.connection_id == connection_id)
            {
                continue;
            }
            let [mutation] = operation.plan.agent_access_grants() else {
                return Err(invalid("publication.agent.source-grant-count"));
            };
            if mutation.kind() != AgentAccessGrantMutationKindV1::Ensure {
                return Err(invalid("publication.agent.source-grant-kind"));
            }
            let effect = operation
                .grant_effects()
                .iter()
                .find(|effect| is_agent_access_grant_effect(effect))
                .ok_or_else(|| invalid("publication.agent.source-grant-effect"))?;
            return AgentAccessGrantRefV1::from_ensure_effect(effect, mutation)
                .map_err(|_| invalid("publication.agent.source-grant-reference"));
        }
        Err(PortError::new(
            PortErrorCode::NotFound,
            "publication.agent.source-connection",
        ))
    }

    pub(super) fn publication_record_from_operation(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<PublicationRecordV1> {
        if intent.kind() != OwnedEffectKind::Publication
            || intent.target() != PUBLICATION_TARGET
            || intent.sensitive()
        {
            return Err(invalid("publication.intent.shape"));
        }
        if !operation
            .plan
            .external()
            .iter()
            .any(|candidate| candidate == intent)
        {
            return Err(invalid("publication.intent.unregistered"));
        }
        if let Some(record) =
            routing_publication_record(intent).map_err(|_| invalid("publication.routing.decode"))?
        {
            if record.workspace_id != operation.workspace_id {
                return Err(invalid("publication.routing.workspace"));
            }
            return Ok(record);
        }

        if let Some(effect) = operation
            .step(OperationStepKind::CompilePublication)
            .effects
            .iter()
            .find(|effect| effect.effect_id == intent.effect_id())
        {
            let (_, marker) = self.decode_publication_marker(effect)?;
            if let Some(record) = marker.record {
                return Ok(record);
            }
        }
        let [mutation] = operation.plan.agent_access_grants() else {
            return Err(invalid("publication.agent.grant-count"));
        };
        let grant_effect = match self
            .stores_lock()?
            .secrets()
            .observe_agent_access_grant(&operation.operation_id, mutation)?
        {
            EffectReconciliation::Staged(effect) | EffectReconciliation::Applied(effect) => effect,
            EffectReconciliation::Missing => {
                return Err(PortError::new(
                    PortErrorCode::Conflict,
                    "publication.agent.grant-missing",
                ));
            }
            EffectReconciliation::OwnershipLost(_) => {
                return Err(PortError::new(
                    PortErrorCode::Conflict,
                    "publication.agent.grant-ownership",
                ));
            }
        };
        let settings = operation.plan.spec().command_id == "agents.settings.apply";
        let restore_reference =
            if !settings && mutation.kind() == AgentAccessGrantMutationKindV1::Revoke {
                Some(self.succeeded_connection_grant_reference(mutation.connection_id())?)
            } else {
                None
            };
        let stores = self.stores_lock()?;
        let active = stores
            .control()
            .active_publication(&operation.workspace_id)?;
        let last_known_good = stores
            .control()
            .last_known_good_publication(&operation.workspace_id)?;
        let base = active
            .as_ref()
            .into_iter()
            .chain(last_known_good.as_ref())
            .find(|record| Some(&record.digest) == intent.before_fingerprint())
            .ok_or_else(|| PortError::new(PortErrorCode::Conflict, "publication.agent.base"))?;
        if settings {
            return self.settings_publication_record(operation, intent, base, &grant_effect);
        }
        let record = match mutation.kind() {
            AgentAccessGrantMutationKindV1::Ensure => {
                agent_connection_publication_record(operation, intent, base, &grant_effect)
            }
            AgentAccessGrantMutationKindV1::Revoke => agent_connection_restore_publication_record(
                operation,
                intent,
                base,
                restore_reference
                    .as_ref()
                    .ok_or_else(|| invalid("publication.agent.restore-reference"))?,
            ),
        }
        .map_err(|_| invalid("publication.agent.decode"))?;
        record.ok_or_else(|| invalid("publication.intent.unknown"))
    }

    fn publication_effect(
        &self,
        operation_id: &OperationId,
        intent: &ExternalEffectIntentV1,
        record: &PublicationRecordV1,
    ) -> PortResult<OwnedEffectV1> {
        record
            .verify()
            .map_err(|_| invalid("publication.record.verify"))?;
        let no_op = intent.before_fingerprint() == Some(&record.digest);
        let marker = PublicationEffectMarkerV1 {
            schema: PUBLICATION_MARKER_SCHEMA.to_owned(),
            operation_id: operation_id.to_string(),
            workspace_id: record.workspace_id.clone(),
            publication_revision: record.publication_revision.get(),
            before_digest: intent.before_fingerprint().cloned(),
            after_digest: record.digest.clone(),
            no_op,
            record: Some(record.clone()),
            decision: InstallationDecision::Prepared,
        };
        let effect = OwnedEffectV1 {
            effect_id: intent.effect_id().to_owned(),
            kind: OwnedEffectKind::Publication,
            target: PUBLICATION_TARGET.to_owned(),
            before_fingerprint: intent.before_fingerprint().cloned(),
            after_fingerprint: Some(record.digest.clone()),
            compensation: serde_json::Value::Null.into(),
        };
        self.encode_publication_marker(effect, marker)
    }

    // Preserve the authenticated typed record while encoding a local marker. The
    // existing single-slot exact-Arc decoder cache still rechecks journal binding
    // on use; changed or reopened bytes take the full cold authentication path.
    fn encode_publication_marker(
        &self,
        mut effect: OwnedEffectV1,
        marker: PublicationEffectMarkerV1,
    ) -> PortResult<OwnedEffectV1> {
        let operation_id = OperationId::parse(marker.operation_id.clone())
            .map_err(|_| invalid("publication.effect.operation"))?;
        effect.compensation = serde_json::to_value(&marker)
            .map_err(|_| invalid("publication.effect.encode"))?
            .into();
        *self
            .publication_marker_cache
            .lock()
            .map_err(|_| invalid("publication.marker-cache.lock"))? = Some(DecodedMarker {
            effect: effect.clone(),
            operation_id,
            marker,
        });
        Ok(effect)
    }

    fn publication_marker(
        &self,
        operation: &OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<(OperationId, PublicationEffectMarkerV1)> {
        self.require_current_operation(operation)?;
        self.publication_marker_from_operation(operation, effect)
    }

    fn publication_marker_from_operation(
        &self,
        operation: &OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<(OperationId, PublicationEffectMarkerV1)> {
        let (operation_id, mut marker) = self.decode_publication_marker(effect)?;
        if operation_id != operation.operation_id {
            return Err(invalid("publication.effect.operation-binding"));
        }
        let intent = operation
            .plan
            .external()
            .iter()
            .find(|intent| {
                intent.effect_id() == effect.effect_id
                    && intent.kind() == OwnedEffectKind::Publication
                    && intent.target() == effect.target
                    && intent.before_fingerprint() == effect.before_fingerprint.as_ref()
            })
            .cloned()
            .ok_or_else(|| invalid("publication.effect.unregistered"))?;
        let record = self.publication_record_from_operation(operation, &intent)?;
        let recorded = operation
            .step(OperationStepKind::CompilePublication)
            .effects
            .iter()
            .find(|stored| stored.effect_id == effect.effect_id);
        if operation.workspace_id != marker.workspace_id
            || record.digest != marker.after_digest
            || record.publication_revision.get() != marker.publication_revision
            || marker.no_op != (marker.before_digest.as_ref() == Some(&marker.after_digest))
            || recorded.is_some_and(|stored| stored != effect)
            || marker
                .record
                .as_ref()
                .is_some_and(|embedded| embedded != &record)
        {
            return Err(invalid("publication.effect.shape"));
        }
        // V1 journals did not embed the authenticated Product record or an installation decision.
        // Reconstruct the exact record from the owning Operation/intention above and immediately
        // return the sole current V2 marker in memory. Callers persist that normalized marker on
        // their next recovery checkpoint; no live path observes a raw V1 marker.
        marker.schema = PUBLICATION_MARKER_SCHEMA.to_owned();
        marker.record = Some(record);
        Ok((operation_id, marker))
    }

    fn decode_publication_marker(
        &self,
        effect: &OwnedEffectV1,
    ) -> PortResult<(OperationId, PublicationEffectMarkerV1)> {
        let mut cache = self
            .publication_marker_cache
            .lock()
            .map_err(|_| invalid("publication.marker-cache.lock"))?;
        if let Some(cached) = cache.as_ref()
            && cached.matches(effect)
        {
            return Ok((cached.operation_id.clone(), cached.marker.clone()));
        }
        let (operation_id, marker) = Self::decode_publication_marker_uncached(effect)?;
        *cache = Some(DecodedMarker {
            effect: effect.clone(),
            operation_id: operation_id.clone(),
            marker: marker.clone(),
        });
        Ok((operation_id, marker))
    }

    fn decode_publication_marker_uncached(
        effect: &OwnedEffectV1,
    ) -> PortResult<(OperationId, PublicationEffectMarkerV1)> {
        #[cfg(test)]
        MARKER_DECODES.with(|count| count.set(count.get() + 1));
        if effect.kind != OwnedEffectKind::Publication || effect.target != PUBLICATION_TARGET {
            return Err(invalid("publication.effect.kind"));
        }
        let marker: PublicationEffectMarkerV1 =
            serde::Deserialize::deserialize(effect.compensation.as_ref())
                .map_err(|_| invalid("publication.effect.decode"))?;
        let operation_id = OperationId::parse(marker.operation_id.clone())
            .map_err(|_| invalid("publication.effect.operation"))?;
        GatewayPublicationRevision::new(marker.publication_revision)
            .map_err(|_| invalid("publication.effect.revision"))?;
        if !matches!(
            marker.schema.as_str(),
            PUBLICATION_MARKER_SCHEMA | LEGACY_MARKER_SCHEMA
        ) || marker.before_digest.as_ref() != effect.before_fingerprint.as_ref()
            || effect.after_fingerprint.as_ref() != Some(&marker.after_digest)
        {
            return Err(invalid("publication.effect.shape"));
        }
        if marker.schema == PUBLICATION_MARKER_SCHEMA {
            let record = marker
                .record
                .as_ref()
                .ok_or_else(|| invalid("publication.effect.record-missing"))?;
            record
                .verify()
                .map_err(|_| invalid("publication.effect.record-invalid"))?;
            if record.workspace_id != marker.workspace_id
                || record.digest != marker.after_digest
                || record.publication_revision.get() != marker.publication_revision
            {
                return Err(invalid("publication.effect.record-identity"));
            }
        } else if marker.record.is_some() || marker.decision != InstallationDecision::Prepared {
            return Err(invalid("publication.effect.legacy-shape"));
        }
        Ok((operation_id, marker))
    }

    pub(super) fn observe_publication(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<EffectReconciliation> {
        let operation_id = &operation.operation_id;
        let diagnostics = self
            .publication_diagnostics
            .lock()
            .map(|p| p.clone())
            .unwrap_or_default();
        measure(
            &diagnostics,
            PublicationStage::PublicationObserve,
            Some(operation_id.as_str()),
            None,
            || {
                if operation.step(OperationStepKind::CompilePublication).status
                    == OperationStepStatus::Compensated
                {
                    return Ok(EffectReconciliation::Missing);
                }
                let record = self.publication_record_from_operation(operation, intent)?;
                let (active, prepared) = {
                    let stores = self.stores_lock()?;
                    (
                        stores.control().active_publication(&record.workspace_id)?,
                        stores
                            .control()
                            .prepared_publication(&record.workspace_id)?,
                    )
                };
                let recorded = operation
                    .step(OperationStepKind::CompilePublication)
                    .effects
                    .iter()
                    .find(|effect| effect.effect_id == intent.effect_id())
                    .cloned();
                let effect = match recorded {
                    Some(effect) => effect,
                    None => self.publication_effect(operation_id, intent, &record)?,
                };
                let (_, marker) = self.publication_marker_from_operation(operation, &effect)?;
                let effect = if effect.compensation["schema"] == PUBLICATION_MARKER_SCHEMA {
                    effect
                } else {
                    self.encode_publication_marker(effect, marker)?
                };
                if active.as_ref() == Some(&record) {
                    return if self.publication_is_installed(&record)? {
                        Ok(EffectReconciliation::Applied(effect))
                    } else {
                        Ok(EffectReconciliation::Staged(effect))
                    };
                }
                if prepared.as_ref() == Some(&record) {
                    return Ok(EffectReconciliation::Staged(effect));
                }
                let active_digest = active.as_ref().map(|value| &value.digest);
                if active_digest == intent.before_fingerprint() {
                    Ok(EffectReconciliation::Missing)
                } else {
                    Ok(EffectReconciliation::OwnershipLost(effect))
                }
            },
        )
    }

    pub(super) fn apply_publication(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<OwnedEffectV1> {
        let operation_id = &operation.operation_id;
        let diagnostics = self
            .publication_diagnostics
            .lock()
            .map(|p| p.clone())
            .unwrap_or_default();
        measure(
            &diagnostics,
            PublicationStage::PublicationApply,
            Some(operation_id.as_str()),
            None,
            || {
                let record = self.publication_record_from_operation(operation, intent)?;
                let stores = self.stores_lock()?;
                let active = stores.control().active_publication(&record.workspace_id)?;
                if active.as_ref() != Some(&record) {
                    let active_digest = active.as_ref().map(|value| &value.digest);
                    if active_digest != intent.before_fingerprint() {
                        return Err(PortError::new(
                            PortErrorCode::Conflict,
                            "publication.apply.cas",
                        ));
                    }
                    stores.control().prepare_publication(
                        &record,
                        active.as_ref().map(|value| value.publication_revision),
                    )?;
                }
                crate::publication_failpoint::crash("after_prepare");
                self.publication_effect(operation_id, intent, &record)
            },
        )
    }

    pub(super) fn activate_publication(
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
            PublicationStage::PublicationActivate,
            None,
            None,
            || {
                let (_, marker) = self.publication_marker_from_operation(operation, effect)?;
                if marker.decision != InstallationDecision::Install || marker.record.is_none() {
                    return Err(invalid("publication.activate.decision-missing"));
                }
                let record = marker.record.as_ref().expect("record checked");
                crate::publication_failpoint::crash("before_install");
                let (active, prepared) = {
                    let stores = self.stores_lock()?;
                    (
                        stores.control().active_publication(&marker.workspace_id)?,
                        stores
                            .control()
                            .prepared_publication(&marker.workspace_id)?,
                    )
                };
                if active.as_ref() != Some(record) && prepared.as_ref() != Some(record) {
                    return Err(invalid("publication.activate.record-missing"));
                }
                let publication = record
                    .verify()
                    .map_err(|_| invalid("publication.activate.verify"))?;
                record
                    .gateway_snapshot()
                    .map_err(|_| invalid("publication.activate.projection"))?;
                self.register_operation_agent_grants(
                    &record.workspace_id,
                    operation,
                    &operation.step(OperationStepKind::ApplySecrets).effects,
                    &publication,
                )?;
                measure(
                    &diagnostics,
                    PublicationStage::PublicationInstall,
                    Some(operation.operation_id.as_str()),
                    None,
                    || {
                        let target = self.required_publication_target()?;
                        if !target.verify_installed(record).map_err(target_error)? {
                            crate::publication_failpoint::during_operation_install(|| {
                                target.activate_verified(record)
                            })
                            .map_err(target_error)?;
                        }
                        if !target.verify_installed(record).map_err(target_error)? {
                            return Err(invalid("publication.activate.target-mismatch"));
                        }
                        Ok(())
                    },
                )?;
                crate::publication_failpoint::crash("after_target");
                self.stores_lock()?.control().mark_publication_active(
                    &marker.workspace_id,
                    record.publication_revision,
                    &record.digest,
                )?;
                crate::publication_failpoint::crash("after_active");
                Ok(effect.clone())
            },
        )
    }

    pub(super) fn compensate_publication(
        &self,
        operation: &OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<CompensationOutcome> {
        if operation.step(OperationStepKind::CompilePublication).status
            == OperationStepStatus::Compensated
        {
            return Ok(CompensationOutcome::AlreadyCompensated);
        }
        let (_, marker) = self.publication_marker_from_operation(operation, effect)?;
        if marker.decision != InstallationDecision::Abort {
            return Ok(CompensationOutcome::OwnershipLost);
        }
        if marker.no_op {
            return Ok(CompensationOutcome::Compensated);
        }
        let stores = self.stores_lock()?;
        if stores
            .control()
            .prepared_publication(&marker.workspace_id)?
            .as_ref()
            .is_some_and(|record| {
                record.publication_revision.get() == marker.publication_revision
                    && record.digest == marker.after_digest
            })
        {
            stores.control().discard_prepared_publication(
                &marker.workspace_id,
                GatewayPublicationRevision::new(marker.publication_revision)
                    .map_err(|_| invalid("publication.compensate.revision"))?,
                &marker.after_digest,
            )?;
            return Ok(CompensationOutcome::Compensated);
        }
        // Publication is activated last. If the active pointer did change, no adapter can claim
        // a safe rollback without atomically restoring both Product and Gateway authorities.
        Ok(CompensationOutcome::OwnershipLost)
    }

    pub(super) fn current_publication_fingerprint(&self) -> PortResult<Option<CanonicalDigest>> {
        Ok(self
            .stores_lock()?
            .control()
            .active_publication(&WorkspaceId::default())?
            .map(|record| record.digest))
    }
}

fn invalid(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::InvalidData, context)
}
