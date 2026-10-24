//! An admitted request keeps capture authority while its optional index is busy.
use super::{AgentTurnHistoryError, AgentTurnRequestGuard, profiles};

impl AgentTurnRequestGuard {
    pub(super) fn withhold_pending_affinity(
        &self,
        hold_ticket: &mut Option<Box<crate::context_hold::HoldTicket>>,
        input: &mut profiles::PlannerInputV1,
    ) {
        if self.decision_deferred {
            // Publication can still fail its budget after delivery. Suppress
            // planner hints and the ticket consumed by HoldCompletion together,
            // before the single planner/execution call. Capture stays admitted.
            *hold_ticket = None;
            input.context_hold = None;
            input.previous_success_candidate_id = None;
        }
    }

    pub(super) fn commit_decision_or_defer(
        &mut self,
        decision: &profiles::BranchDecisionV1,
    ) -> Result<(), AgentTurnHistoryError> {
        match self.store.commit_decision(self.ticket(), decision.clone()) {
            Ok(()) => Ok(()),
            Err(AgentTurnHistoryError::Resource) => {
                // The request already owns a unique, budgeted capture ticket.
                // Keep recording its accepted output. The frozen planner input
                // owns the decision; do not allocate a second deferred history.
                self.decision_deferred = true;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn finish_deferred_decision(
        &mut self,
        decision: Option<&profiles::BranchDecisionV1>,
    ) -> Result<(), AgentTurnHistoryError> {
        if self.decision_deferred {
            let decision = decision.ok_or(AgentTurnHistoryError::Integrity)?;
            // Delivery has ended. Finalization validates the same ticket and
            // memory ceiling; exhaustion leaves a gap through the armed guard.
            self.store
                .commit_decision_after_delivery(self.ticket(), decision.clone())?;
            self.decision_deferred = false;
        }
        Ok(())
    }
}
