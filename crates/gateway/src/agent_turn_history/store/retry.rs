//! A failed native request can retry without inventing an accepted response edge.
use super::*;
use crate::agent_turn_history::projection::AnalyzedRequest;

pub(super) fn same_native_request(
    entry: &SessionEntry,
    analyzed: &AnalyzedRequest,
    native_turn: Option<[u8; 32]>,
) -> bool {
    analyzed.measured
        && native_turn.is_some()
        && native_turn == entry.active.native_turn
        && analyzed.retry_setup.is_some()
        && analyzed.retry_setup == entry.checkpoint.retry_setup
        && analyzed.anchors.as_ref().is_some_and(|anchors| {
            !anchors.contextual
                && entry.checkpoint.digest == Some(anchors.complete)
                && entry.checkpoint.message_count == analyzed.message_count
        })
}

pub(super) fn failed_before_output(entry: &SessionEntry) -> bool {
    entry.pending_request.is_none()
        && entry.active.last_request_status == AgentTurnStatus::Failed
        && !entry.active.request_output_accepted
        && !entry.active.capture_partial
}
