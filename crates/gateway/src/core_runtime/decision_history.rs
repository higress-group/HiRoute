//! Optional history admission, separate from each request's routing authority.

use super::{
    AgentTurnBegin, AgentTurnHistoryError, AgentTurnRequestGuard, PlanSnapshot,
    ProductionGatewayRuntime, RequestObservation, model_ir, profiles,
};
use crate::agent_turn_history::{AgentTurnHistorySnapshot, TurnDecisionInputs};
use crate::context_hold::{HistoryContinuity, HoldTicket, PreparedHold};
use crate::replay::ReplayStore;
use std::time::Instant;

pub(super) struct DecisionHistory {
    pub(super) guard: Option<AgentTurnRequestGuard>,
    pub(super) history: Box<AgentTurnHistorySnapshot>,
    pub(super) correlated: Option<Box<profiles::CorrelatedBranchDecisionV1>>,
    pub(super) hold_ticket: Option<Box<HoldTicket>>,
}

impl DecisionHistory {
    fn detached() -> Self {
        Self {
            guard: None,
            history: Box::new(AgentTurnHistorySnapshot {
                previous_decision: None,
                visible_conversation: Vec::new(),
                history_partial: true,
                assessment_from: None,
                assessment_target: None,
                _pin: None,
            }),
            correlated: None,
            hold_ticket: None,
        }
    }
}

impl ProductionGatewayRuntime {
    pub(super) fn begin_decision_history(
        &self,
        observation: &RequestObservation,
        plan: PlanSnapshot,
        request: &model_ir::ModelRequestIRV1,
        replay: &ReplayStore,
        prepared_hold: Option<PreparedHold>,
    ) -> Result<DecisionHistory, AgentTurnHistoryError> {
        let Some(prepared) = prepared_hold.as_ref() else {
            return Ok(DecisionHistory::detached());
        };
        let Ok(key) = self
            .agent_turn_history
            .scope_key("authorized-context", &prepared.scope_id())
        else {
            return Ok(DecisionHistory::detached());
        };
        let history_continuity = prepared_hold.as_ref().map_or(
            HistoryContinuity::NoCheckpoint,
            PreparedHold::history_continuity,
        );
        let begin = self.agent_turn_history.begin_with_context(
            key,
            plan,
            request,
            replay,
            TurnDecisionInputs {
                history_continuity,
                reselect_on_user_message: true,
            },
            Instant::now(),
        );
        let (ticket, history, completed, correlated) = match begin {
            Ok(AgentTurnBegin::NewTurn {
                ticket,
                history,
                completed,
                inherited_decision,
            }) => (
                ticket,
                history,
                completed,
                inherited_decision.map(|decision| {
                    Box::new(profiles::CorrelatedBranchDecisionV1 {
                        kind: profiles::ContinuationKindV1::TaskRoot,
                        decision,
                    })
                }),
            ),
            Ok(AgentTurnBegin::Continuation { ticket, decision }) => {
                let kind = if request.messages.iter().any(|message| {
                    message
                        .content
                        .iter()
                        .any(|part| matches!(part, model_ir::ContentPart::ToolResult { .. }))
                }) {
                    profiles::ContinuationKindV1::ToolContinuation
                } else {
                    profiles::ContinuationKindV1::TaskRoot
                };
                (
                    ticket,
                    DecisionHistory::detached().history,
                    None,
                    Some(Box::new(profiles::CorrelatedBranchDecisionV1 {
                        kind,
                        decision,
                    })),
                )
            }
            Err(AgentTurnHistoryError::Integrity) => {
                // Replay corruption remains a request-integrity failure, not a cache miss.
                return Err(AgentTurnHistoryError::Integrity);
            }
            Err(
                AgentTurnHistoryError::SessionTurnConflict
                | AgentTurnHistoryError::TurnContextUnavailable
                | AgentTurnHistoryError::Resource
                | AgentTurnHistoryError::NoUserTurn,
            ) => return Ok(DecisionHistory::detached()),
        };
        let guard = AgentTurnRequestGuard::new(self.agent_turn_history.clone(), ticket);
        let hold_ticket = if let Some(prepared) = prepared_hold {
            let Some(ticket) = self.context_holds.commit(prepared, Instant::now()) else {
                // Admission owns the history lease before any checkpoint can change.
                // A stale cache scan cannot authorize inherited routing or late writes.
                drop(guard);
                return Ok(DecisionHistory::detached());
            };
            if ticket.history_continuity != history_continuity {
                drop(guard);
                return Ok(DecisionHistory::detached());
            }
            Some(Box::new(ticket))
        } else {
            None
        };
        if let Some(completed) = completed {
            observation.agent_turn_finished(&completed);
        }
        Ok(DecisionHistory {
            guard: Some(guard),
            history,
            correlated,
            hold_ticket,
        })
    }
}
