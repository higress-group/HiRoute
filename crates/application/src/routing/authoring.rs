//! Pure content Preview: computes and seals the exact alias without reserving it.
use hiroute_application_api::*;
use hiroute_domain::*;
use thiserror::Error;

use crate::compiler::{AgentPlanCompilationFactsV1, AgentPlanCompilerError, compile_agent_plan_v2};

pub struct PlanAuthoringSnapshotV2 {
    pub workspace: WorkspaceId,
    pub aliases: AliasRegistryV1,
    pub active_publication: Option<GatewayPublicationV1>,
    pub plan_heads: Vec<PlanHeadV1>,
    pub current_head: Option<PlanHeadV1>,
    /// Authenticated source for a publication that predates durable PlanVersion rows. It can be
    /// the original V1 compiled value or the same executable value already currentized to V2.
    pub legacy_source: Option<PlanVersionV1>,
    pub draft: Option<PlanDraftV1>,
    pub facts: AgentPlanCompilationFactsV1,
    /// Exact saved connection revision selected by the effective editor, not its latest version.
    pub decision_service: Option<DecisionServiceV1>,
    pub expected_revisions: RevisionSetV1,
}

pub(crate) fn created_plan_id(
    workspace: &WorkspaceId,
    creation_key: &str,
) -> Result<AgentPlanId, PlanPreviewError> {
    let digest = CanonicalDigest::of(&("hiroute.created-plan/v2", workspace, creation_key))
        .map_err(|_| PlanPreviewError::Invalid)?;
    AgentPlanId::parse(format!("plan/{}", &digest.as_str()[7..31]))
        .map_err(|_| PlanPreviewError::Invalid)
}

pub fn preview_plan_content(
    change: &PlanContentChangeV2,
    state: &PlanAuthoringSnapshotV2,
) -> Result<PlanContentPreviewV2, PlanPreviewError> {
    if change.schema != PLAN_CONTENT_CHANGE_SCHEMA_V2 {
        return Err(PlanPreviewError::Invalid);
    }
    state
        .aliases
        .validate()
        .map_err(|_| PlanPreviewError::Invalid)?;
    if let Some(legacy) = &state.legacy_source {
        legacy.validate().map_err(|_| PlanPreviewError::Invalid)?;
        if state.current_head.as_ref().map(|h| &h.reference) != Some(&legacy.reference)
            || *legacy
                != PlanVersionV1::from_unversioned_compiled_recovery(
                    state.workspace.clone(),
                    legacy.compiled.clone(),
                )
                .map_err(|_| PlanPreviewError::Invalid)?
        {
            return Err(PlanPreviewError::Invalid);
        }
    }
    let configuration = change
        .editor
        .effective()
        .map_err(|_| PlanPreviewError::Invalid)?;
    if let Some(service) = configuration.decision_service()
        && state.decision_service.as_ref() != Some(service)
    {
        return Err(PlanPreviewError::InvalidDecisionService);
    }
    if let Some(selected) = &change.consumed_draft {
        let draft = state.draft.as_ref().ok_or(PlanPreviewError::Stale)?;
        draft.validate().map_err(|_| PlanPreviewError::Invalid)?;
        if draft.workspace_id != state.workspace
            || draft.draft_id != selected.draft_id
            || draft.revision != selected.revision
            || draft.plan_id.as_ref() != state.current_head.as_ref().map(|h| &h.reference.plan_id)
            || draft.base_head_revision != state.current_head.as_ref().map(|h| h.head_revision)
        {
            return Err(PlanPreviewError::Stale);
        }
    }
    let (plan_id, content_revision, head_revision, status) = match &change.target {
        PlanContentTargetV2::Create { creation_key } => {
            if state.current_head.is_some()
                || creation_key.is_empty()
                || creation_key.len() > 128
                || creation_key.chars().any(char::is_control)
            {
                return Err(PlanPreviewError::Invalid);
            }
            let id = created_plan_id(&state.workspace, creation_key)?;
            if state.aliases.alias_for(&id).is_some()
                || state.aliases.retired_plan_ids.contains(&id)
            {
                return Err(PlanPreviewError::Stale);
            }
            (id, 1, 1, PlanLifecycleV1::Enabled)
        }
        PlanContentTargetV2::Update {
            plan_id,
            expected_head_revision,
        } => {
            let head = state.current_head.as_ref().ok_or(PlanPreviewError::Stale)?;
            head.validate().map_err(|_| PlanPreviewError::Invalid)?;
            if &head.reference.plan_id != plan_id
                || head.reference.workspace_id != state.workspace
                || head.head_revision != *expected_head_revision
                || head.status == PlanLifecycleV1::Deleted
                || state.aliases.alias_for(plan_id) != Some(&head.model_alias)
            {
                return Err(PlanPreviewError::Stale);
            }
            (
                plan_id.clone(),
                head.reference
                    .content_revision
                    .checked_add(1)
                    .ok_or(PlanPreviewError::Invalid)?,
                head.head_revision
                    .checked_add(1)
                    .ok_or(PlanPreviewError::Invalid)?,
                head.status,
            )
        }
    };
    let mut aliases = state.aliases.clone();
    let model_alias = if let Some(head) = &state.current_head {
        if change
            .editor
            .custom_alias
            .as_ref()
            .is_some_and(|alias| alias != head.model_alias.as_str())
        {
            return Err(PlanPreviewError::AliasImmutable);
        }
        head.model_alias.clone()
    } else if let Some(custom) = &change.editor.custom_alias {
        aliases
            .allocate_custom(
                plan_id.clone(),
                ModelAlias::parse_custom(custom).map_err(|_| PlanPreviewError::AliasUnavailable)?,
            )
            .map_err(|_| PlanPreviewError::AliasUnavailable)?
    } else {
        aliases
            .allocate_named(plan_id.clone(), configuration.display_name.as_str())
            .map_err(|_| PlanPreviewError::AliasUnavailable)?
    };
    let compiled = compile_agent_plan_v2(
        AgentPlanIdentityV1 {
            agent_plan_id: plan_id,
            model_alias: model_alias.clone(),
            display_name: configuration.display_name.clone(),
            purpose: configuration.purpose.clone(),
        },
        content_revision,
        &configuration,
        &state.facts,
    )?;
    let plan_version = PlanVersionV1::new(state.workspace.clone(), configuration, compiled)
        .map_err(|_| PlanPreviewError::Invalid)?;
    let plan_head = PlanHeadV1 {
        reference: plan_version.reference.clone(),
        head_revision,
        model_alias,
        status,
    };
    let alias_registry_digest =
        CanonicalDigest::of(&state.aliases).map_err(|_| PlanPreviewError::Invalid)?;
    let change_digest = plan_content_confirmation_digest(change, &state.expected_revisions)
        .map_err(|_| PlanPreviewError::Invalid)?;
    Ok(PlanContentPreviewV2 {
        schema: PLAN_CONTENT_PREVIEW_SCHEMA_V2.into(),
        change_digest,
        expected_revisions: state.expected_revisions.clone(),
        alias_registry_digest,
        before_head: state.current_head.clone(),
        legacy_source: state.legacy_source.clone(),
        plan_head,
        plan_version,
        consumed_draft: change.consumed_draft.clone(),
    })
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum PlanPreviewError {
    #[error("plan is referenced by Agent settings or retained execution versions")]
    Referenced,
    #[error("invalid plan content")]
    Invalid,
    #[error("select an unchanged saved decision connection revision")]
    InvalidDecisionService,
    #[error("plan, draft or alias registry changed; preview again")]
    Stale,
    #[error("the requested alias is unavailable")]
    AliasUnavailable,
    #[error("published aliases are immutable")]
    AliasImmutable,
    #[error(transparent)]
    Compiler(#[from] AgentPlanCompilerError),
}

#[cfg(test)]
mod tests;
