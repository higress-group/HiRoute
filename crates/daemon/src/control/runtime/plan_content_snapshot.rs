//! Read one consistent authoring view across routing facts and durable Plan records.
use super::LocalControlAdapter;
use hiroute_application::control::{ControlReadError, RoutingFactsPort};
use hiroute_application::routing::PlanAuthoringSnapshotV2;
use hiroute_application_api::{PlanContentChangeV2, PlanContentTargetV2};
use hiroute_domain::{AliasRegistryV1, WorkspaceId};

impl LocalControlAdapter {
    pub(super) fn plan_content_snapshot(
        &self,
        workspace: &WorkspaceId,
        change: &PlanContentChangeV2,
    ) -> Result<PlanAuthoringSnapshotV2, ControlReadError> {
        hiroute_diagnostics::publication::measure(
            &self
                .publication_diagnostics
                .lock()
                .map(|p| p.clone())
                .unwrap_or_default(),
            hiroute_diagnostics::publication::PublicationStage::PlanAuthoringSnapshot,
            None,
            None,
            || {
                let selected_service = change
                    .editor
                    .effective()
                    .ok()
                    .and_then(|configuration| configuration.decision_service().cloned());
                let before = self.routing_compilation_snapshot(workspace)?;
                let read_rows = || {
                    let stores = self.stores_lock().map_err(super::map_port)?;
                    let head = match &change.target {
                        PlanContentTargetV2::Create { .. } => None,
                        PlanContentTargetV2::Update { plan_id, .. } => stores
                            .control()
                            .plan_head(workspace, plan_id)
                            .map_err(|_| ControlReadError::Corrupt)?,
                    };
                    let draft = change
                        .consumed_draft
                        .as_ref()
                        .map(|draft| {
                            stores
                                .control()
                                .plan_draft(workspace, &draft.draft_id)
                                .map_err(|_| ControlReadError::Corrupt)
                        })
                        .transpose()?
                        .flatten();
                    let heads = stores
                        .control()
                        .plan_heads(workspace)
                        .map_err(|_| ControlReadError::Corrupt)?;
                    let decision_service = selected_service
                        .as_ref()
                        .map(|service| {
                            stores
                                .control()
                                .decision_service(workspace, &service.id, service.revision)
                                .map_err(super::map_port)
                        })
                        .transpose()?
                        .flatten();
                    Ok::<_, ControlReadError>((head, draft, heads, decision_service))
                };
                let (mut current_head, draft, mut heads, decision_service) = read_rows()?;
                let after = self.routing_compilation_snapshot(workspace)?;
                let repeated = read_rows()?;
                if before.facts != after.facts
                    || before.expected_revisions != after.expected_revisions
                    || before.active_publication != after.active_publication
                    || repeated
                        != (
                            current_head.clone(),
                            draft.clone(),
                            heads.clone(),
                            decision_service.clone(),
                        )
                {
                    return Err(ControlReadError::SnapshotChanged);
                }
                if let Some(head) = &current_head {
                    let compiled = after.active_publication.as_ref().and_then(|publication| {
                        publication
                            .plans
                            .iter()
                            .find(|plan| plan.agent_plan_id() == &head.reference.plan_id)
                    });
                    if !compiled.is_some_and(|plan| {
                        plan.body.agent_plan_revision == head.reference.content_revision
                            && plan.model_alias() == &head.model_alias
                    }) {
                        return Err(ControlReadError::SnapshotChanged);
                    }
                }
                if let Some(publication) = &after.active_publication {
                    for plan in &publication.plans {
                        if !heads
                            .iter()
                            .any(|h| &h.reference.plan_id == plan.agent_plan_id())
                        {
                            let version =
                                hiroute_domain::PlanVersionV1::from_unversioned_compiled_recovery(
                                    workspace.clone(),
                                    plan.clone(),
                                )
                                .map_err(|_| ControlReadError::Corrupt)?;
                            heads.push(hiroute_domain::PlanHeadV1 {
                                head_revision: version.reference.content_revision,
                                model_alias: plan.model_alias().clone(),
                                reference: version.reference,
                                status: hiroute_domain::PlanLifecycleV1::Enabled,
                            });
                        }
                    }
                }
                let legacy_source = if current_head.is_none() {
                    if let PlanContentTargetV2::Update { plan_id, .. } = &change.target {
                        let compiled = after
                            .active_publication
                            .as_ref()
                            .and_then(|p| p.plans.iter().find(|p| p.agent_plan_id() == plan_id))
                            .ok_or(ControlReadError::NotFound)?;
                        let version =
                            hiroute_domain::PlanVersionV1::from_unversioned_compiled_recovery(
                                workspace.clone(),
                                compiled.clone(),
                            )
                            .map_err(|_| ControlReadError::Corrupt)?;
                        current_head = heads
                            .iter()
                            .find(|h| &h.reference.plan_id == plan_id)
                            .cloned();
                        Some(version)
                    } else {
                        None
                    }
                } else {
                    None
                };
                Ok(PlanAuthoringSnapshotV2 {
                    workspace: workspace.clone(),
                    aliases: after
                        .active_publication
                        .as_ref()
                        .map(|p| p.alias_registry.clone())
                        .unwrap_or_else(AliasRegistryV1::default),
                    active_publication: after.active_publication,
                    current_head,
                    legacy_source,
                    plan_heads: heads,
                    draft,
                    facts: after.facts,
                    decision_service,
                    expected_revisions: after.expected_revisions,
                })
            },
        )
    }
}
