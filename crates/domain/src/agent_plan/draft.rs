use super::{AgentPlanId, PlanVersionError};
use crate::{PlanEditorStateV2, WorkspaceId};
use serde::{Deserialize, Serialize};

pub const PLAN_DRAFT_SCHEMA_V1: &str = "hiroute.plan-draft/v1";

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDraftV1 {
    pub schema: String,
    pub workspace_id: WorkspaceId,
    pub draft_id: String,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<AgentPlanId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_head_revision: Option<u64>,
    pub editor: PlanEditorStateV2,
}

impl PlanDraftV1 {
    pub fn validate(&self) -> Result<(), PlanVersionError> {
        WorkspaceId::parse(self.workspace_id.as_str()).map_err(|_| PlanVersionError::Invalid)?;
        AgentPlanId::parse(&self.draft_id).map_err(|_| PlanVersionError::Invalid)?;
        if self.schema != PLAN_DRAFT_SCHEMA_V1
            || self.revision == 0
            || self.plan_id.is_some() != self.base_head_revision.is_some()
            || self.base_head_revision == Some(0)
        {
            return Err(PlanVersionError::Invalid);
        }
        if let Some(plan) = &self.plan_id {
            AgentPlanId::parse(plan.as_str()).map_err(|_| PlanVersionError::Invalid)?;
        }
        self.editor
            .validate_draft()
            .map_err(|_| PlanVersionError::Invalid)
    }
}

pub const PLAN_DRAFT_CHANGE_SCHEMA_V1: &str = "hiroute.plan-draft-change/v1";
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanDraftActionV1 {
    Save { draft: Box<PlanDraftV1> },
    Discard,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDraftChangeV1 {
    pub schema: String,
    pub workspace_id: WorkspaceId,
    pub draft_id: String,
    pub expected_revision: Option<u64>,
    pub action: PlanDraftActionV1,
}
impl PlanDraftChangeV1 {
    pub fn validate(&self) -> Result<(), PlanVersionError> {
        WorkspaceId::parse(self.workspace_id.as_str()).map_err(|_| PlanVersionError::Invalid)?;
        AgentPlanId::parse(&self.draft_id).map_err(|_| PlanVersionError::Invalid)?;
        if self.schema != PLAN_DRAFT_CHANGE_SCHEMA_V1 || self.expected_revision == Some(0) {
            return Err(PlanVersionError::Invalid);
        }
        match &self.action {
            PlanDraftActionV1::Save { draft } => {
                draft.validate()?;
                if draft.workspace_id != self.workspace_id
                    || draft.draft_id != self.draft_id
                    || self.expected_revision.unwrap_or(0).checked_add(1) != Some(draft.revision)
                {
                    return Err(PlanVersionError::Invalid);
                }
            }
            PlanDraftActionV1::Discard if self.expected_revision.is_none() => {
                return Err(PlanVersionError::Invalid);
            }
            PlanDraftActionV1::Discard => (),
        }
        Ok(())
    }
}
