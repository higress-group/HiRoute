//! Safe capacity facts for installed model consumers, never native configuration or authority.
use crate::{AgentKindV1, AgentPlanId, ModelAlias};
use serde::{Deserialize, Serialize};

pub const AGENT_MODEL_BUDGET_CONFLICT_SCHEMA_V1: &str = "hiroute.agent-model-budget-conflict/v1";
pub const AGENT_MODEL_BUDGET_CONFLICT_CODE: &str = "AGENT_MODEL_BUDGET_CONFLICT";

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentModelTokenBudgetV1 {
    pub context_window_tokens: u64,
    pub max_output_tokens: u64,
}

#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelBudgetReasonV1 {
    CapacityReduced,
    PlanMissing,
    BudgetUnavailable,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentModelBudgetBlockerV1 {
    pub agent_kind: AgentKindV1,
    pub context_id: String,
    pub plan_id: AgentPlanId,
    pub model_alias: ModelAlias,
    pub reason: AgentModelBudgetReasonV1,
    pub declared: AgentModelTokenBudgetV1,
    pub proposed: Option<AgentModelTokenBudgetV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentModelBudgetConflictV1 {
    pub schema: String,
    pub blockers: Vec<AgentModelBudgetBlockerV1>,
}

impl AgentModelBudgetConflictV1 {
    pub fn new(mut blockers: Vec<AgentModelBudgetBlockerV1>) -> Self {
        blockers.sort_by(|a, b| {
            (
                a.agent_kind,
                &a.context_id,
                &a.plan_id,
                &a.model_alias,
                a.reason,
            )
                .cmp(&(
                    b.agent_kind,
                    &b.context_id,
                    &b.plan_id,
                    &b.model_alias,
                    b.reason,
                ))
        });
        blockers.dedup();
        Self {
            schema: AGENT_MODEL_BUDGET_CONFLICT_SCHEMA_V1.into(),
            blockers,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.schema == AGENT_MODEL_BUDGET_CONFLICT_SCHEMA_V1
            && !self.blockers.is_empty()
            && self.blockers.iter().all(|blocker| {
                matches!(
                    blocker.agent_kind,
                    AgentKindV1::Qoder
                        | AgentKindV1::Pi
                        | AgentKindV1::DeepseekHarness
                        | AgentKindV1::OpenCode
                        | AgentKindV1::Hermes
                ) && blocker.context_id.starts_with("agent-context/")
                    && blocker.context_id.len() <= 256
                    && blocker
                        .context_id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"/-_:".contains(&b))
                    && AgentPlanId::parse(blocker.plan_id.as_str()).is_ok()
                    && ModelAlias::parse(blocker.model_alias.as_str()).is_ok()
                    && blocker.declared.context_window_tokens > 0
                    && blocker.declared.max_output_tokens > 0
                    && match (blocker.reason, blocker.proposed) {
                        (AgentModelBudgetReasonV1::CapacityReduced, Some(next)) => {
                            next.context_window_tokens > 0
                                && next.max_output_tokens > 0
                                && (next.context_window_tokens
                                    < blocker.declared.context_window_tokens
                                    || next.max_output_tokens < blocker.declared.max_output_tokens)
                        }
                        (
                            AgentModelBudgetReasonV1::PlanMissing
                            | AgentModelBudgetReasonV1::BudgetUnavailable,
                            None,
                        ) => true,
                        _ => false,
                    }
            })
    }

    /// Retain the released Qoder code only when every actual blocker belongs to Qoder.
    pub fn safe_code(&self) -> &'static str {
        if self
            .blockers
            .iter()
            .all(|blocker| blocker.agent_kind == AgentKindV1::Qoder)
        {
            "QODER_MODEL_BUDGET_CONFLICT"
        } else {
            AGENT_MODEL_BUDGET_CONFLICT_CODE
        }
    }

    /// A terminal result is diagnostic data. Parsing it never authorizes recovery effects.
    pub fn parse(encoded: &str) -> Option<Self> {
        let value: Self = serde_json::from_str(encoded).ok()?;
        value.is_valid().then_some(value)
    }
}
