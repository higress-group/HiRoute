//! Request-owned background is never admitted to execution history or assessment membership.
use super::{CallFailure, DecisionInput, openai_decisions, protocol};
use crate::agent_turn_history::CurrentInputPosition;
use crate::content_ref::ContentValueExt;
use crate::server::core_runtime::adapters::RequestedReplacement;
use crate::server::core_runtime::model_ir::{ContentPart, MessageRole};
use crate::server::request_plan::RestBranchClassifierAuthorityV1;
use serde_json::{Value, json};

const MAX_MESSAGES: usize = 16;
const MAX_SCAN_MESSAGES: usize = 256;
const MAX_PARTS: usize = 64;

pub(super) fn empty() -> Value {
    json!({"source":"request_replay", "messages":[], "truncated":false,
        "boundary_unavailable":false})
}

pub(super) fn append(
    authority: &RestBranchClassifierAuthorityV1,
    input: DecisionInput<'_>,
    body: &mut Value,
    refs: &mut Vec<RequestedReplacement>,
    mut budget: usize,
) -> Result<(), CallFailure> {
    // Native projections may precede trailing synthetic User envelopes. Use
    // their validated position, never a text match or the final User as a proxy.
    let boundary = match (input.current, input.position) {
        (Some(_), Some(CurrentInputPosition::Event(index))) => Some(index),
        (Some(_), Some(CurrentInputPosition::Window { first, last }))
            if first <= last && last < input.request.messages.len() =>
        {
            Some(first)
        }
        (Some(_), _) => None,
        (None, _) => super::super::native_routing_view::observed_input_index(input.request),
    }
    .filter(|index| {
        input
            .request
            .messages
            .get(*index)
            .is_some_and(|m| m.role == MessageRole::User)
    });
    let Some(boundary) = boundary else {
        body["state"]["request_context"]["boundary_unavailable"] = json!(true);
        return Ok(());
    };
    let mut wire_len =
        openai_decisions::prepare_template(authority.wire_protocol, body, refs.clone())?.wire_len;
    let mut messages = Vec::new();
    let scan_start = boundary.saturating_sub(MAX_SCAN_MESSAGES);
    body["state"]["request_context"]["truncated"] = json!(scan_start > 0);
    for (ordinal, message) in input.request.messages[..boundary]
        .iter()
        .enumerate()
        .skip(scan_start)
        .rev()
    {
        let role = match message.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            _ => continue,
        };
        if message.content.len() > MAX_PARTS {
            body["state"]["request_context"]["truncated"] = json!(true);
            break;
        }
        let texts = message
            .content
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } if !text.is_empty() => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        if texts.is_empty() {
            continue;
        }
        if messages.len() == MAX_MESSAGES
            || texts
                .iter()
                .try_fold(0usize, |sum, text| {
                    let bytes = text
                        .content_ref()
                        .map_or(text.len() as u64, |r| r.byte_len());
                    sum.checked_add(usize::try_from(bytes).ok()?)
                })
                .is_none_or(|bytes| bytes > budget)
        {
            body["state"]["request_context"]["truncated"] = json!(true);
            break;
        }
        let mut candidate_refs = refs.clone();
        let parts = texts
            .iter()
            .map(|text| protocol::project_text(text, &mut candidate_refs))
            .collect::<Vec<_>>();
        messages.insert(
            0,
            json!({"role":role, "message_ordinal":ordinal, "content":parts}),
        );
        body["state"]["request_context"]["messages"] = json!(messages);
        // Use the actual selected wire codec, including escaping and Replay
        // lengths. At most sixteen bounded candidates are serialized, with no
        // materialization of Replay-backed transcript text or extra provider call.
        let candidate = openai_decisions::prepare_template(
            authority.wire_protocol,
            body,
            candidate_refs.clone(),
        );
        let next = candidate.ok().and_then(|p| {
            p.wire_len
                .checked_sub(wire_len)
                .filter(|bytes| *bytes <= budget)
                .map(|bytes| (p.wire_len, bytes))
        });
        let Some((next_len, used)) = next else {
            messages.remove(0);
            body["state"]["request_context"]["messages"] = json!(messages);
            body["state"]["request_context"]["truncated"] = json!(true);
            break;
        };
        wire_len = next_len;
        budget -= used;
        *refs = candidate_refs;
    }
    Ok(())
}
