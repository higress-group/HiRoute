//! Optional history admission, separate from each request's routing authority.

use super::{
    AgentTurnBegin, AgentTurnHistoryError, AgentTurnRequestGuard, PlanSnapshot,
    ProductionGatewayRuntime, RequestObservation, model_ir, profiles,
};
use crate::agent_turn_history::{
    AgentTurnHistorySnapshot, CurrentInputPosition, TurnDecisionInputs,
};
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

pub(super) struct HistoryInput<'a> {
    pub(super) request: &'a model_ir::ModelRequestIRV1,
    pub(super) replay: &'a ReplayStore,
    pub(super) current_input: Option<&'a [model_ir::ContentPart]>,
    pub(super) current_input_position: Option<CurrentInputPosition>,
    pub(super) current_input_unavailable: bool,
}

impl ProductionGatewayRuntime {
    #[cfg(test)]
    pub(super) fn begin_decision_history(
        &self,
        observation: &RequestObservation,
        plan: PlanSnapshot,
        request: &model_ir::ModelRequestIRV1,
        replay: &ReplayStore,
        prepared_hold: Option<PreparedHold>,
    ) -> Result<DecisionHistory, AgentTurnHistoryError> {
        let capture = prepared_hold.as_ref().map(CaptureScope::from_prepared);
        self.begin_authorized_decision_history(
            observation,
            plan,
            capture,
            prepared_hold,
            HistoryInput {
                request,
                replay,
                current_input: None,
                current_input_position: None,
                current_input_unavailable: false,
            },
        )
    }

    pub(super) fn begin_authorized_decision_history(
        &self,
        observation: &RequestObservation,
        plan: PlanSnapshot,
        capture: Option<CaptureScope>,
        prepared_hold: Option<PreparedHold>,
        input: HistoryInput<'_>,
    ) -> Result<DecisionHistory, AgentTurnHistoryError> {
        let HistoryInput {
            request,
            replay,
            current_input,
            current_input_position,
            current_input_unavailable,
        } = input;
        let Some(capture) = capture else {
            return Ok(DecisionHistory::detached());
        };
        let Ok(key) = self
            .agent_turn_history
            .scope_key("authorized-context", &capture.scope_id)
        else {
            return Ok(DecisionHistory::detached());
        };
        let gap_key = key.clone();

        let history_continuity = capture.continuity;
        let native_turn = capture.native_turn;
        let user_origin_unverified = capture.origin_unverified;
        let uncaptured_native_prefix = capture.missing_prefix;
        let begin = self
            .agent_turn_history
            .prepare_scoped_request(&key, request, replay)
            .and_then(|mut analyzed| {
                analyzed.set_branch_cursor(capture.branch_cursor.0, capture.branch_cursor.1);
                if current_input_unavailable {
                    let Some(CurrentInputPosition::Event(index)) = current_input_position else {
                        return Err(AgentTurnHistoryError::NoUserTurn);
                    };
                    analyzed.select_unavailable_event(index, request)?;
                } else if capture.unavailable_input {
                    analyzed.mark_current_input_unavailable();
                } else if let Some(parts) = current_input {
                    analyzed.select_current_input(
                        parts,
                        current_input_position,
                        request,
                        replay,
                    )?;
                }
                if user_origin_unverified {
                    analyzed.mark_user_origin_unverified();
                }
                if uncaptured_native_prefix {
                    analyzed.mark_prior_history_missing();
                }
                self.agent_turn_history.begin_with_admission(
                    key,
                    plan,
                    analyzed,
                    TurnDecisionInputs {
                        history_continuity,
                        native_turn,
                        reselect_on_user_message: true,
                    },
                    Instant::now(),
                    || Some(()),
                )
            });
        let (begin, ()) = match begin {
            Ok(Some(admitted)) => admitted,
            Ok(None) => return Ok(DecisionHistory::detached()),
            Err(AgentTurnHistoryError::Integrity) => {
                // Replay corruption remains a request-integrity failure, not a cache miss.
                return Err(AgentTurnHistoryError::Integrity);
            }
            Err(
                AgentTurnHistoryError::TurnContextUnavailable
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
        if let Some(admission) = self
            .agent_turn_history
            .bind_request(&ticket, observation.logical_request_id())
        {
            observation.agent_turn_admitted(&admission);
        }
        let branch_digest = *ticket.key.branch_digest();
        let allow_affinity = ticket.allow_affinity;
        let guard = AgentTurnRequestGuard::new(self.agent_turn_history.clone(), ticket);
        // Affinity is subordinate to execution capture. A stale or busy cache
        // can only lose its hint, never the admitted request or its snapshot.
        let hold_ticket = prepared_hold
            .filter(|_| {
                allow_affinity
                    && (!(capture.unavailable_input || current_input_unavailable)
                        || correlated.is_some())
            })
            .and_then(|prepared| {
                self.context_holds.commit_on_branch(
                    prepared,
                    &branch_digest,
                    request,
                    replay,
                    Instant::now(),
                )
            });
        if let Some(completed) = completed {
            observation.agent_turn_finished(&completed);
        }
        Ok(DecisionHistory {
            guard: Some(guard),
            history,
            correlated,
            hold_ticket: hold_ticket.map(Box::new),
        })
    }
}

/// Authorization-owned capture scope. It is available even when the optional
/// affinity cache cannot prepare or commit a checkpoint.
pub(super) struct CaptureScope {
    scope_id: String,
    native_turn: Option<[u8; 32]>,
    branch_cursor: (Option<[u8; 32]>, Option<[u8; 32]>),
    origin_unverified: bool,
    missing_prefix: bool,
    unavailable_input: bool,
    continuity: crate::context_hold::HistoryContinuity,
}

impl CaptureScope {
    #[cfg(test)]
    fn from_prepared(prepared: &PreparedHold) -> Self {
        Self {
            scope_id: prepared.scope_id(),
            native_turn: prepared.native_turn(),
            branch_cursor: (None, None),
            origin_unverified: prepared.user_origin_unverified(),
            missing_prefix: prepared.uncaptured_native_prefix(),
            unavailable_input: prepared.native_user_unavailable(),
            continuity: prepared.history_continuity(),
        }
    }

    pub(super) fn authorized(
        authorized: &crate::server::request_plan::AuthorizedRequestPlan,
        protocol: crate::server::request_plan::IngressProtocol,
        identity: &crate::context_hold::ContextIdentityFacts,
        digest_key: &[u8; 32],
        prepared: Option<&PreparedHold>,
    ) -> Option<Self> {
        let key = crate::context_hold::ContextHoldKey::routing_history(
            authorized, protocol, identity, digest_key,
        )?;
        let native = identity.native_checkpoint();
        Some(Self {
            scope_id: key.scope_id(),
            native_turn: native.turn,
            branch_cursor: (native.branch_leaf, native.branch_path),
            origin_unverified: identity.input_origin_unverified(prepared),
            missing_prefix: prepared
                .map_or(native.compacted, PreparedHold::uncaptured_native_prefix),
            unavailable_input: identity.requires_captured_input(),
            continuity: prepared.map_or(
                crate::context_hold::HistoryContinuity::NoCheckpoint,
                PreparedHold::history_continuity,
            ),
        })
    }
}

#[cfg(test)]
#[path = "decision_history/tests.rs"]
mod tests;
