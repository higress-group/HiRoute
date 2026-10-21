//! Optional history admission, separate from each request's routing authority.

use super::{
    AgentTurnBegin, AgentTurnHistoryError, AgentTurnRequestGuard, PlanSnapshot,
    ProductionGatewayRuntime, RequestObservation, model_ir, profiles,
};
use crate::agent_turn_history::{AgentTurnHistorySnapshot, TurnDecisionInputs};
use crate::context_hold::{HoldTicket, PreparedHold};
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
    pub(super) fn note_decision_history_gap(
        &self,
        authorized: &crate::server::request_plan::AuthorizedRequestPlan,
        protocol: crate::server::request_plan::IngressProtocol,
        identity: &crate::context_hold::ContextIdentityFacts,
    ) {
        let Some(scope) = self.context_holds.digest_key().and_then(|digest_key| {
            crate::context_hold::ContextHoldKey::capture_gap_scope(
                authorized,
                protocol,
                identity,
                &digest_key,
            )
        }) else {
            return;
        };
        if let Ok(key) = self
            .agent_turn_history
            .scope_key("authorized-context", &scope)
        {
            self.agent_turn_history.note_capture_gap(&key);
        }
    }

    pub(super) fn begin_decision_history(
        &self,
        observation: &RequestObservation,
        plan: PlanSnapshot,
        request: &model_ir::ModelRequestIRV1,
        replay: &ReplayStore,
        prepared_hold: Option<PreparedHold>,
    ) -> Result<DecisionHistory, AgentTurnHistoryError> {
        let Some(prepared) = prepared_hold else {
            return Ok(DecisionHistory::detached());
        };
        let Ok(key) = self
            .agent_turn_history
            .scope_key("authorized-context", &prepared.scope_id())
        else {
            return Ok(DecisionHistory::detached());
        };
        let gap_key = key.clone();
        let history_continuity = prepared.history_continuity();
        let begin =
            crate::agent_turn_history::AgentTurnHistoryStore::prepare_request(request, replay)
                .and_then(|analyzed| {
                    self.agent_turn_history.begin_with_admission(
                        key,
                        plan,
                        analyzed,
                        TurnDecisionInputs {
                            history_continuity,
                            reselect_on_user_message: true,
                        },
                        Instant::now(),
                        || self.context_holds.reserve_commit(prepared, Instant::now()),
                    )
                });
        let (begin, hold_commit) = match begin {
            Ok(Some(admitted)) => admitted,
            Ok(None) => return Ok(DecisionHistory::detached()),
            Err(AgentTurnHistoryError::Integrity) => {
                // Replay corruption remains a request-integrity failure, not a cache miss.
                return Err(AgentTurnHistoryError::Integrity);
            }
            Err(
                AgentTurnHistoryError::SessionTurnConflict
                | AgentTurnHistoryError::TurnContextUnavailable
                | AgentTurnHistoryError::Resource
                | AgentTurnHistoryError::NoUserTurn,
            ) => {
                self.agent_turn_history.note_capture_gap(&gap_key);
                return Ok(DecisionHistory::detached());
            }
        };
        let (ticket, history, completed, correlated) = match begin {
            AgentTurnBegin::NewTurn {
                ticket,
                history,
                completed,
                inherited_decision,
            } => (
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
            AgentTurnBegin::Continuation { ticket, decision } => {
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
        };
        let guard = AgentTurnRequestGuard::new(self.agent_turn_history.clone(), ticket);
        // Both stores admitted the request under nonblocking locks. Publishing
        // the reserved checkpoint cannot fail after history has been changed.
        let hold_ticket = hold_commit.commit();
        debug_assert_eq!(hold_ticket.history_continuity, history_continuity);
        if let Some(completed) = completed {
            observation.agent_turn_finished(&completed);
        }
        Ok(DecisionHistory {
            guard: Some(guard),
            history,
            correlated,
            hold_ticket: Some(Box::new(hold_ticket)),
        })
    }
}

#[cfg(test)]
#[path = "decision_history/tests.rs"]
mod tests;
