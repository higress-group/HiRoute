//! Read-only current policy and retained Worker state for Application evidence.
use super::*;
use hiroute_application::trust::context::{
    TrustAnnouncementV1, TrustContextV1, TrustRunV1, announcement_cohort_start,
};
use hiroute_domain::delegation::DelegationRuntimePort;
use std::collections::BTreeMap;

impl LocalControlAdapter {
    pub(super) fn read_trust_context(
        &self,
        workspace: &WorkspaceId,
        plan_id: &str,
    ) -> Result<TrustContextV1, ControlReadError> {
        let plan_id =
            hiroute_domain::AgentPlanId::parse(plan_id).map_err(|_| ControlReadError::NotFound)?;
        let catalog = self
            .current_catalog()
            .ok_or(ControlReadError::Unavailable)?;
        let stores = self
            .stores
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?;
        let head = stores
            .control()
            .plan_head(workspace, &plan_id)
            .map_err(|_| ControlReadError::Unavailable)?
            .ok_or(ControlReadError::NotFound)?;
        let version = stores
            .control()
            .lookup_exact_plan_version(&head.reference)
            .map_err(|_| ControlReadError::Unavailable)?;
        let candidates = version
            .compiled
            .body
            .materialized
            .attempt_owned
            .groups
            .iter()
            .flat_map(|g| &g.candidates);
        let binding_models = candidates
            .map(|c| (c.binding_id.clone(), c.model_configuration_id.clone()))
            .collect();
        let identity_stability = catalog
            .catalog
            .model_data()
            .models
            .iter()
            .map(|m| {
                (
                    m.model_configuration_id.clone(),
                    catalog
                        .catalog
                        .model_identity_stability(&m.model_configuration_id)
                        .into(),
                )
            })
            .collect();
        let announcements = catalog
            .catalog
            .featured_models()
            .iter()
            .filter(|f| f.kind == hiroute_integrations::model_feed::FeaturedKind::ModelUpdate)
            .map(|f| {
                Ok(TrustAnnouncementV1 {
                    model_configuration_id: f.model_configuration_id.clone(),
                    included_on: f.included_on.clone(),
                    cohort_from_ms: announcement_cohort_start(&f.included_on)
                        .ok_or(ControlReadError::Corrupt)?,
                })
            })
            .collect::<Result<Vec<_>, ControlReadError>>()?;
        let generation = hiroute_domain::CanonicalDigest::of(&(&head, catalog.generation))
            .map_err(|_| ControlReadError::Corrupt)?;
        Ok(TrustContextV1 {
            active: head.reference,
            enabled: head.status == hiroute_domain::PlanLifecycleV1::Enabled,
            configuration: version.configuration,
            binding_models,
            identity_stability,
            announcements,
            generation,
        })
    }
    pub(super) fn read_trust_primary_bindings(
        &self,
        workspace: &WorkspaceId,
        plan_id: &str,
        revision: Option<u64>,
        branch_id: Option<&str>,
    ) -> Result<Vec<String>, ControlReadError> {
        use hiroute_domain::*;
        let plan = AgentPlanId::parse(plan_id).map_err(|_| ControlReadError::NotFound)?;
        let stores = self.stores_lock().map_err(map_port)?;
        let versions = stores
            .control()
            .published_plan_versions(workspace)
            .map_err(|_| ControlReadError::Unavailable)?
            .into_iter()
            .filter(|v| {
                v.reference.plan_id == plan
                    && revision.is_none_or(|n| v.reference.content_revision == n)
            });
        let mut bindings = std::collections::BTreeSet::new();
        for version in versions {
            match &version.configuration.strategy {
                AgentPlanStrategyV2::SmartSaving { primary, .. }
                    if branch_id.is_none_or(|b| b == SMART_SAVING_SCOPE_ID) =>
                {
                    bindings.extend(primary.iter().map(|c| c.binding_id.clone()))
                }
                AgentPlanStrategyV2::Branches { routing } => {
                    for branch in routing
                        .branches
                        .iter()
                        .filter(|b| branch_id.is_none_or(|id| id == b.id))
                    {
                        if !branch.candidates.is_empty() && branch.primary_candidates.is_empty() {
                            return Ok(Vec::new());
                        }
                        bindings.extend(
                            branch
                                .primary_candidates
                                .iter()
                                .map(|c| c.binding_id.clone()),
                        );
                    }
                }
                _ => return Ok(Vec::new()),
            }
        }
        Ok(bindings.into_iter().collect())
    }
    pub(super) fn read_trust_runs(
        &self,
        workspace: &WorkspaceId,
        ids: &[String],
    ) -> Result<BTreeMap<String, TrustRunV1>, ControlReadError> {
        if ids.len() > 50_000 {
            return Err(ControlReadError::Unavailable);
        }
        let stores = self
            .stores
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?;
        let mut runs = BTreeMap::new();
        for id in ids {
            if let Some(run) = stores
                .runtime()
                .run(workspace, id)
                .map_err(|_| ControlReadError::Unavailable)?
            {
                runs.insert(
                    id.clone(),
                    TrustRunV1 {
                        run_id: run.run_id,
                        task_id: run.task_id,
                        ordinal: run.ordinal,
                        state: run.progress.state,
                    },
                );
            }
        }
        Ok(runs)
    }
}
