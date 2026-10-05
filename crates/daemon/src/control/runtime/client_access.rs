//! Current product projections joined to the exact immutable publication, never client caches.
use super::{LocalControlAdapter, map_port};
use hiroute_application::client_access::ClientAccessPort;
use hiroute_application::control::ControlReadError;
use hiroute_application_api::*;
use hiroute_domain::{
    ControlRepositoryPort, MaterializedOrderingV1, PlanHeadV1, PlanLifecycleV1, PlanVersionV1,
    PublicationRepositoryPort, WorkspaceId,
};

impl ClientAccessPort for LocalControlAdapter {
    fn operation_status_for_idempotency(
        &self,
        principal: PrincipalKind,
        operation_kind: &str,
        key: &str,
    ) -> Result<Option<hiroute_domain::OperationStatus>, ControlReadError> {
        let principal = match principal {
            PrincipalKind::InteractiveUser => "interactive-user",
            PrincipalKind::Desktop => "desktop",
            PrincipalKind::Skill | PrincipalKind::SealedCollaboration => {
                return Err(ControlReadError::Denied);
            }
        };
        let scope = hiroute_domain::IdempotencyScopeV1::new(principal, operation_kind, key)
            .map_err(|_| ControlReadError::Corrupt)?;
        self.stores_lock()
            .map_err(map_port)?
            .control()
            .operation_status_for_idempotency(&WorkspaceId::default(), &scope)
            .map_err(map_port)
    }

    fn service_status(&self) -> Result<ClientServiceStatusV1, ControlReadError> {
        let workspace = WorkspaceId::default();
        // Keep product/recovery facts stable until the target's nonblocking observation.
        // Retain the composition guard too: a target replacement cannot split this snapshot.
        let target = self
            .publication_target
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?;
        let stores = self.stores_lock().map_err(map_port)?;
        let control = stores.control();
        let revisions = control.current_revisions(&workspace).map_err(map_port)?;
        let active = control.active_publication(&workspace).map_err(map_port)?;
        let pending = control.recoverable_operations().map_err(map_port)?;
        let writer_released = control
            .writer_claim_operation()
            .map_err(map_port)?
            .is_none();
        let recovered = pending.is_empty() && writer_released;
        // A verified service segment can be read while its client file awaits cleanup.
        // Ordinary mutations remain closed, and generic or displaced recovery stays blocked.
        let reads_ready = recovered
            || (writer_released
                && active.as_ref().is_some_and(|publication| {
                    pending.iter().all(|operation| {
                        hiroute_application::settings_service_completion_is_current(
                            operation,
                            publication.publication_revision.get(),
                            &publication.digest,
                        )
                    })
                }));
        let composed = target.is_some();
        let gateway = match target.as_ref() {
            None => ClientGatewayStateV1::NotComposed,
            Some(target) => match target.observes_verified(active.as_ref()) {
                Ok(true) if reads_ready => {
                    if let Some(publication) = active
                        .as_ref()
                        .and_then(|p| p.verify().ok())
                        .filter(|p| p.gateway_snapshot().is_ok())
                    {
                        if publication.aliases.is_empty() {
                            ClientGatewayStateV1::NoNewCalls
                        } else {
                            ClientGatewayStateV1::Ready
                        }
                    } else {
                        ClientGatewayStateV1::Empty
                    }
                }
                _ => ClientGatewayStateV1::Unavailable,
            },
        };
        Ok(ClientServiceStatusV1 {
            schema: "hiroute.client-service-status/v1".into(),
            daemon_role: if composed { "all" } else { "control_only" }.into(),
            recovery_ready: reads_ready,
            mutation_available: recovered
                && composed
                && gateway != ClientGatewayStateV1::Unavailable,
            gateway,
            active_publication: active.map(|p| PublicationIdentityViewV1 {
                revision: p.publication_revision.get(),
                digest: p.digest,
            }),
            revisions,
        })
    }

    fn plan_catalog(&self) -> Result<AgentPlanCatalogViewV2, ControlReadError> {
        let status = self.service_status()?;
        if !status.recovery_ready {
            return Err(ControlReadError::Unavailable);
        }
        let stores = self.stores_lock().map_err(map_port)?;
        let workspace = WorkspaceId::default();
        if stores
            .control()
            .current_revisions(&workspace)
            .map_err(map_port)?
            != status.revisions
        {
            return Err(ControlReadError::Unavailable);
        }
        let Some(record) = stores
            .control()
            .active_publication(&workspace)
            .map_err(map_port)?
        else {
            return Ok(AgentPlanCatalogViewV2 {
                next_cursor: None,
                schema: "hiroute.agent-plan-catalog/v2".into(),
                plans: vec![],
                drafts: stores
                    .control()
                    .plan_drafts(&workspace)
                    .map_err(|_| ControlReadError::Corrupt)?,
            });
        };
        let publication = record.verify().map_err(|_| ControlReadError::Corrupt)?;
        let mut plans = Vec::new();
        for compiled in &publication.plans {
            let stored_head = stores
                .control()
                .plan_head(&workspace, compiled.agent_plan_id())
                .map_err(|_| ControlReadError::Corrupt)?;
            let (head, version) = if let Some(head) = stored_head {
                let version = stores
                    .control()
                    .lookup_exact_plan_version(&head.reference)
                    .map_err(|_| ControlReadError::Corrupt)?;
                if version.compiled != *compiled {
                    return Err(ControlReadError::SnapshotChanged);
                }
                (head, version)
            } else {
                let version = PlanVersionV1::from_unversioned_compiled_recovery(
                    workspace.clone(),
                    compiled.clone(),
                )
                .map_err(|_| ControlReadError::Corrupt)?;
                let head = PlanHeadV1 {
                    reference: version.reference.clone(),
                    head_revision: version.reference.content_revision,
                    model_alias: compiled.model_alias().clone(),
                    status: PlanLifecycleV1::Enabled,
                };
                (head, version)
            };
            let mut editable = compiled.body.materialized.clone();
            for group in &mut editable.attempt_owned.groups {
                group.ordering_evidence = MaterializedOrderingV1::ExplicitOrder;
                group.pinned_ratings.clear();
            }
            let editable_route_digest = editable
                .route_digest()
                .map_err(|_| ControlReadError::Corrupt)?;
            let desired = version.configuration;
            plans.push(AgentPlanStatusV2 {
                protocol_advice: hiroute_domain::agent_protocol_advice(&compiled.body.materialized),
                agent_plan_id: compiled.agent_plan_id().clone(),
                desired,
                head: head.clone(),
                editable_route_digest,
                agent_plan_revision: compiled.body.agent_plan_revision,
                model_alias: compiled.model_alias().clone(),
                materialized_route_digest: compiled.body.materialized_route_digest.clone(),
                publication: PublicationIdentityViewV1 {
                    revision: record.publication_revision.get(),
                    digest: record.digest.clone(),
                },
                execution: if head.status == PlanLifecycleV1::Enabled {
                    status.gateway
                } else {
                    ClientGatewayStateV1::NoNewCalls
                },
                revisions: status.revisions.clone(),
            });
        }
        Ok(AgentPlanCatalogViewV2 {
            next_cursor: None,
            schema: "hiroute.agent-plan-catalog/v2".into(),
            plans,
            drafts: stores
                .control()
                .plan_drafts(&workspace)
                .map_err(|_| ControlReadError::Corrupt)?,
        })
    }
}
