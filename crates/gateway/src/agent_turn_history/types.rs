use serde::Serialize;
use std::sync::Arc;

use crate::server::core_runtime::profiles::BranchDecisionV1;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum VisibleContentPart {
    Text { text: String },
    ToolActivity { tool: String, status: ToolStatus },
    Unavailable { source_kind: String },
}

impl VisibleContentPart {
    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Self::Text { text } => text.capacity(),
                Self::ToolActivity { tool, .. } => tool.capacity(),
                Self::Unavailable { source_kind } => source_kind.capacity(),
            }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolStatus {
    Completed,
    Failed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentTurnStatus {
    Completed,
    Failed,
    Interrupted,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VisibleAgentTurn {
    pub(crate) branch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) executed_branch_id: Option<String>,
    pub(crate) user: Vec<VisibleContentPart>,
    pub(crate) status: AgentTurnStatus,
    pub(crate) steps: Vec<Vec<VisibleContentPart>>,
}

impl VisibleAgentTurn {
    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.branch_id.as_ref().map_or(0, String::capacity)
            + self.executed_branch_id.as_ref().map_or(0, String::capacity)
            + self.user.capacity() * std::mem::size_of::<VisibleContentPart>()
            + self
                .user
                .iter()
                .map(VisibleContentPart::retained_bytes)
                .sum::<usize>()
            + self.steps.capacity() * std::mem::size_of::<Vec<VisibleContentPart>>()
            + self
                .steps
                .iter()
                .map(|step| {
                    step.capacity() * std::mem::size_of::<VisibleContentPart>()
                        + step
                            .iter()
                            .map(VisibleContentPart::retained_bytes)
                            .sum::<usize>()
                })
                .sum::<usize>()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AgentTurnHistorySnapshot {
    pub(crate) visible_conversation: Vec<Arc<VisibleAgentTurn>>,
    pub(crate) history_partial: bool,
    pub(crate) assessment_from: Option<usize>,
    pub(crate) assessment_target: Option<AssessmentTarget>,
    /// Keeps Arc-backed turn bytes resident while one classifier request uses
    /// this immutable view. It is an accounting pin, not another history.
    pub(crate) _pin: Option<Arc<super::store::SnapshotPin>>,
}

#[derive(Clone, Debug)]
pub(crate) struct AssessmentTarget {
    pub(crate) segment_id: String,
    pub(crate) first_turn_id: String,
    pub(crate) through_turn_id: String,
    pub(crate) first_ordinal: u64,
    pub(crate) through_ordinal: u64,
    pub(crate) target_partial: bool,
    pub(crate) plan: PlanSnapshot,
    pub(crate) attribution: ExecutionAttribution,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PlanSnapshot {
    pub(crate) plan_id: String,
    pub(crate) plan_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExecutionAttribution {
    Single {
        selected_branch_id: String,
        executed_branch_id: String,
        model_configuration_id: String,
        profile_digest: String,
    },
    Mixed,
    Unknown,
}

#[derive(Clone, Debug)]
pub(crate) struct AgentTurnTicket {
    pub(crate) key: AgentTurnHistoryKey,
    pub(crate) entry_token: u64,
    pub(crate) revision: u64,
    pub(crate) request_token: u64,
    pub(crate) agent_turn_id: String,
    pub(crate) new_turn: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TurnDecisionInputs {
    pub(crate) message_history_continues: bool,
    pub(crate) reselect_on_user_message: bool,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct AgentTurnHistoryKey(pub(super) [u8; 32]);

#[derive(Clone, Debug)]
pub(crate) enum AgentTurnBegin {
    NewTurn {
        ticket: AgentTurnTicket,
        history: Box<AgentTurnHistorySnapshot>,
        completed: Option<Box<CompletedAgentTurn>>,
        inherited_decision: Option<BranchDecisionV1>,
    },
    Continuation {
        ticket: AgentTurnTicket,
        decision: BranchDecisionV1,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentTurnHistoryError {
    TurnContextUnavailable,
    SessionTurnConflict,
    Resource,
    Integrity,
}

#[derive(Clone, Debug)]
pub(crate) struct AcceptedExecution {
    pub(crate) model_configuration_id: String,
    pub(crate) profile_digest: String,
    pub(crate) executed_branch_id: String,
    pub(crate) request_id: String,
}

#[derive(Clone, Debug)]
pub(crate) struct CompletedAgentTurn {
    pub(crate) agent_turn_id: String,
    pub(crate) segment_id: String,
    pub(crate) ordinal: u64,
    pub(crate) plan: PlanSnapshot,
    pub(crate) selected_branch_id: String,
    pub(crate) attribution: ExecutionAttribution,
    pub(crate) started_at_ms: u64,
    pub(crate) finished_at_ms: u64,
    pub(crate) status: AgentTurnStatus,
    pub(crate) history_partial: bool,
    pub(crate) first_request_id: Option<String>,
    pub(crate) last_request_id: Option<String>,
}
