//! Observed input boundaries do not prove an accepted response predecessor.
use super::*;
use hiroute_domain::AgentTurnOpeningCauseV1 as Cause;

use crate::agent_turn_history::projection::AnalyzedRequest;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RequestPrefix {
    Match,
    Changed,
    Shortened,
    Unavailable,
}

/// Native activation IDs are association hints, not evidence of new input.
/// A complete current window must start after the previously observed request.
pub(super) fn has_appended_input(
    checkpoint: &TranscriptCheckpoint,
    analyzed: &AnalyzedRequest,
) -> bool {
    analyzed
        .current_input_start
        .is_some_and(|first| first >= checkpoint.message_count)
}

pub(super) fn request_prefix(
    checkpoint: &TranscriptCheckpoint,
    analyzed: &AnalyzedRequest,
) -> RequestPrefix {
    if analyzed.message_count < checkpoint.message_count {
        return RequestPrefix::Shortened;
    }
    let Some((anchors, saved)) = analyzed.anchors.as_ref().zip(checkpoint.digest) else {
        return RequestPrefix::Unavailable;
    };
    let incoming = if checkpoint.message_count == analyzed.message_count {
        Some(anchors.complete)
    } else {
        anchors
            .prefixes
            .iter()
            .find_map(|(count, digest)| (*count == checkpoint.message_count).then_some(*digest))
    };
    // Projection runs outside the store lock. A checkpoint may have advanced
    // to a position that was not measured; that is not a changed prefix.
    match incoming {
        Some(digest) if digest == saved => RequestPrefix::Match,
        Some(_) => RequestPrefix::Changed,
        None => RequestPrefix::Unavailable,
    }
}

pub(super) fn fork_opening_cause<'a>(
    entries: impl Iterator<Item = &'a SessionEntry>,
    analyzed: &AnalyzedRequest,
) -> Cause {
    let mut agreed = None;
    for entry in entries {
        let cause = match request_prefix(&entry.checkpoint, analyzed) {
            RequestPrefix::Match if has_appended_input(&entry.checkpoint, analyzed) => {
                Cause::UserMessage
            }
            RequestPrefix::Changed => Cause::HistoryRewrite,
            RequestPrefix::Shortened => Cause::HistoryShortened,
            RequestPrefix::Match | RequestPrefix::Unavailable => Cause::Other,
        };
        match agreed {
            Some(previous) if previous != cause => return Cause::Other,
            None => agreed = Some(cause),
            _ => {}
        }
    }
    agreed.unwrap_or(Cause::NoComparableHistory)
}
