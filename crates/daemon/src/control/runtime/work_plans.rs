//! The existing control store is the sole collaboration grant authority and the
//! current published-plan projection used by collaboration discovery.
use super::{LocalControlAdapter, ProductionControlRuntime};
use hiroute_application::control::ApplicationPorts;
use hiroute_application::delegation::work_plans::{
    WorkPlanDirectory, WorkPlanMetadataPort, WorkPlanMetadataV1,
};
use hiroute_application_api::{ErrorCode, WorkPlanAvailabilityV1};
use hiroute_domain::delegation::{DelegationErrorV1, DelegationGrantAuthorityPort};
use hiroute_domain::{
    AgentCollaborationGrant, AgentPlanId, PlanLifecycleV1, PublicationRepositoryPort, WorkspaceId,
};
use std::collections::BTreeSet;
use std::sync::Arc;

impl DelegationGrantAuthorityPort for LocalControlAdapter {
    fn principal_for_credential(
        &self,
        material: &hiroute_domain::AgentCollaborationCredential,
    ) -> Result<hiroute_domain::VerifiedCollaborationPrincipal, DelegationErrorV1> {
        self.stores_lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?
            .control()
            .collaboration_principal_for_credential(material)
            .map_err(|error| match error.code {
                hiroute_domain::PortErrorCode::PermissionDenied => {
                    DelegationErrorV1::PermissionDenied
                }
                _ => DelegationErrorV1::StorageUnavailable,
            })
    }

    fn open_sealed_bootstrap(
        &self,
        encoded: &str,
    ) -> Result<
        (
            AgentCollaborationGrant,
            hiroute_domain::AgentCollaborationCredential,
        ),
        DelegationErrorV1,
    > {
        self.stores_lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?
            .secrets()
            .open_collaboration_bootstrap(encoded)
            .map_err(|_| DelegationErrorV1::PermissionDenied)
    }
    fn current_grant(
        &self,
        workspace: &WorkspaceId,
        grant: &str,
    ) -> Result<Option<AgentCollaborationGrant>, hiroute_domain::delegation::DelegationErrorV1>
    {
        self.stores
            .lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?
            .control()
            .current_grant(workspace, grant)
    }
}

impl LocalControlAdapter {
    /// Project only the current, published Worker portion of a Plan.  The editor draft and
    /// task-owned historical bindings are deliberately not inputs here: a directory query is
    /// an authorization-time view of the current product publication.
    pub(super) fn current_work_plan_metadata(
        &self,
        workspace: &WorkspaceId,
        allowed: &BTreeSet<AgentPlanId>,
    ) -> Result<Vec<WorkPlanMetadataV1>, ErrorCode> {
        if allowed.len() > 256 {
            return Err(ErrorCode::CapabilityUnavailable);
        }
        self.current_work_plan_metadata_filtered(workspace, Some(allowed))
    }

    pub(super) fn current_worker_plan_metadata(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<Vec<WorkPlanMetadataV1>, ErrorCode> {
        self.current_work_plan_metadata_filtered(workspace, None)
    }

    fn current_work_plan_metadata_filtered(
        &self,
        workspace: &WorkspaceId,
        allowed: Option<&BTreeSet<AgentPlanId>>,
    ) -> Result<Vec<WorkPlanMetadataV1>, ErrorCode> {
        let worker_availability_snapshot = self
            .managed_agent_runtime
            .lock()
            .map_err(|_| ErrorCode::CapabilityUnavailable)?
            .as_ref()
            .map(|runtime| runtime.worker_executor_availability.snapshot());
        let stores = self
            .stores_lock()
            .map_err(|_| ErrorCode::CapabilityUnavailable)?;
        let control = stores.control();
        let Some(record) = control
            .active_publication(workspace)
            .map_err(|_| ErrorCode::CapabilityUnavailable)?
        else {
            return Ok(Vec::new());
        };
        let publication = record
            .verify()
            .map_err(|_| ErrorCode::CapabilityUnavailable)?;
        let published = publication
            .plans
            .iter()
            .map(|plan| plan.agent_plan_id().clone())
            .collect::<BTreeSet<_>>();
        let mut result = Vec::new();
        for head in control
            .plan_heads(workspace)
            .map_err(|_| ErrorCode::CapabilityUnavailable)?
        {
            if allowed.is_some_and(|allowed| !allowed.contains(&head.reference.plan_id))
                || head.status != PlanLifecycleV1::Enabled
                || !published.contains(&head.reference.plan_id)
            {
                continue;
            }
            let version = control
                .lookup_exact_plan_version(&head.reference)
                .map_err(|_| ErrorCode::CapabilityUnavailable)?;
            if !version.configuration.delegation_enabled {
                continue;
            }
            let Some(work) = version.configuration.work else {
                continue;
            };
            let harness = work.harness;
            let (availability, reason) =
                worker_availability(worker_availability_snapshot.as_ref(), harness);
            result.push(WorkPlanMetadataV1 {
                agent_plan_id: head.reference.plan_id,
                alias: head.model_alias.as_str().to_owned(),
                display_name: version.configuration.display_name.as_str().to_owned(),
                purpose: version.configuration.purpose.as_str().to_owned(),
                published: true,
                work: Some((harness, work.protocol)),
                availability,
                reason,
            });
        }
        result.sort_by(|left, right| left.agent_plan_id.cmp(&right.agent_plan_id));
        Ok(result)
    }
}

fn worker_availability(
    availability: Option<&hiroute_application_api::WorkerExecutorAvailabilityListV1>,
    harness: hiroute_domain::delegation::WorkerHarnessV1,
) -> (WorkPlanAvailabilityV1, Option<String>) {
    let Some(executor) = availability.and_then(|availability| {
        availability
            .executors
            .iter()
            .find(|item| item.harness == harness)
    }) else {
        return (
            WorkPlanAvailabilityV1::Unknown,
            Some("Worker installation has not been checked by this runtime".to_owned()),
        );
    };
    match executor.state {
        hiroute_application_api::WorkerExecutorAvailabilityStateV1::Ready => {
            (WorkPlanAvailabilityV1::Ready, None)
        }
        hiroute_application_api::WorkerExecutorAvailabilityStateV1::Unavailable => (
            WorkPlanAvailabilityV1::Unavailable,
            Some("Worker entry or required runtime is unavailable".to_owned()),
        ),
        hiroute_application_api::WorkerExecutorAvailabilityStateV1::Unknown => (
            WorkPlanAvailabilityV1::Unknown,
            Some("Worker installation has not been checked by this runtime".to_owned()),
        ),
    }
}

impl WorkPlanMetadataPort for LocalControlAdapter {
    fn current_plans(
        &self,
        workspace: &WorkspaceId,
        allowed: &BTreeSet<AgentPlanId>,
    ) -> Result<Vec<WorkPlanMetadataV1>, ErrorCode> {
        self.current_work_plan_metadata(workspace, allowed)
    }
}

impl ProductionControlRuntime {
    /// Final composition point for MVP-13's published current metadata. Authentication and
    /// filtering remain in Application, using the same production control store as settings.
    pub fn application_ports_with_work_plan_metadata(
        &self,
        metadata: Arc<dyn WorkPlanMetadataPort>,
    ) -> ApplicationPorts {
        self.application_ports()
            .with_work_plans(Arc::new(WorkPlanDirectory::new(
                self.adapter.clone(),
                metadata,
            )))
    }
}

#[cfg(all(test, unix))]
#[path = "work_plans/current_authority_tests.rs"]
mod current_authority_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delegation::installation::WorkerExecutorAvailabilityRegistry;
    use hiroute_domain::delegation::WorkerHarnessV1;

    #[test]
    fn directory_availability_requires_behavior_proven_harness() {
        assert_eq!(
            worker_availability(None, WorkerHarnessV1::CodexCli).0,
            WorkPlanAvailabilityV1::Unknown
        );
        assert_eq!(
            worker_availability(
                Some(&WorkerExecutorAvailabilityRegistry::unconfigured().snapshot()),
                WorkerHarnessV1::CodexCli,
            )
            .0,
            WorkPlanAvailabilityV1::Unavailable
        );
        let configured = hiroute_application_api::WorkerExecutorAvailabilityListV1 {
            schema: hiroute_application_api::WORKER_EXECUTOR_AVAILABILITY_SCHEMA_V1.to_owned(),
            executors: vec![
                hiroute_application_api::WorkerExecutorAvailabilityV1 {
                    harness: WorkerHarnessV1::CodexCli,
                    state: hiroute_application_api::WorkerExecutorAvailabilityStateV1::Ready,
                    reason: None,
                    start_approve_all: hiroute_application_api::WorkerExecutorCapabilityAvailabilityV1 {
                        state: hiroute_application_api::WorkerExecutorAvailabilityStateV1::Ready,
                        reason: None,
                    },
                    cancel: hiroute_application_api::WorkerExecutorCapabilityAvailabilityV1 {
                        state: hiroute_application_api::WorkerExecutorAvailabilityStateV1::Ready,
                        reason: None,
                    },
                    continue_session: hiroute_application_api::WorkerExecutorCapabilityAvailabilityV1 {
                        state: hiroute_application_api::WorkerExecutorAvailabilityStateV1::Unknown,
                        reason: Some(hiroute_application_api::WorkerExecutorAvailabilityReasonV1::CapabilityUnverified),
                    },
                    restricted_policy: hiroute_application_api::WorkerExecutorCapabilityAvailabilityV1 {
                        state: hiroute_application_api::WorkerExecutorAvailabilityStateV1::Unknown,
                        reason: Some(hiroute_application_api::WorkerExecutorAvailabilityReasonV1::RestrictedPolicyUnverified),
                    },
                },
                hiroute_application_api::WorkerExecutorAvailabilityV1 {
                    harness: WorkerHarnessV1::ClaudeCode,
                    state: hiroute_application_api::WorkerExecutorAvailabilityStateV1::Unavailable,
                    reason: Some(hiroute_application_api::WorkerExecutorAvailabilityReasonV1::InstallationNotConfigured),
                    start_approve_all: hiroute_application_api::WorkerExecutorCapabilityAvailabilityV1 {
                        state: hiroute_application_api::WorkerExecutorAvailabilityStateV1::Unavailable,
                        reason: Some(hiroute_application_api::WorkerExecutorAvailabilityReasonV1::InstallationNotConfigured),
                    },
                    cancel: hiroute_application_api::WorkerExecutorCapabilityAvailabilityV1 {
                        state: hiroute_application_api::WorkerExecutorAvailabilityStateV1::Unavailable,
                        reason: Some(hiroute_application_api::WorkerExecutorAvailabilityReasonV1::InstallationNotConfigured),
                    },
                    continue_session: hiroute_application_api::WorkerExecutorCapabilityAvailabilityV1 {
                        state: hiroute_application_api::WorkerExecutorAvailabilityStateV1::Unavailable,
                        reason: Some(hiroute_application_api::WorkerExecutorAvailabilityReasonV1::InstallationNotConfigured),
                    },
                    restricted_policy: hiroute_application_api::WorkerExecutorCapabilityAvailabilityV1 {
                        state: hiroute_application_api::WorkerExecutorAvailabilityStateV1::Unavailable,
                        reason: Some(hiroute_application_api::WorkerExecutorAvailabilityReasonV1::InstallationNotConfigured),
                    },
                },
            ],
        };
        assert_eq!(
            worker_availability(Some(&configured), WorkerHarnessV1::CodexCli),
            (WorkPlanAvailabilityV1::Ready, None)
        );
        assert_eq!(
            worker_availability(Some(&configured), WorkerHarnessV1::ClaudeCode).0,
            WorkPlanAvailabilityV1::Unavailable
        );
    }
}
