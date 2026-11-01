//! The C v4 bounded replay channel, measured once per admitted message fragment.
use super::{
    CallFailure, DecisionInput,
    context_budget::{Cost, Remaining},
    protocol, request_context,
};
use crate::content_ref::ContentValueExt;
use crate::server::core_runtime::adapters::{RequestedReplacement, prepare_replay_json_template};
use crate::server::core_runtime::model_ir::{ContentPart, MessageRole};
use serde_json::{Value, json};

pub(super) struct ReplaySelection {
    pub value: Value,
    pub refs: Vec<RequestedReplacement>,
    pub retained: usize,
    pub omitted: Option<usize>,
    pub scanned: usize,
    pub unscanned: Option<usize>,
    pub truncated: bool,
    pub boundary_unavailable: bool,
    pub budget_omitted: bool,
}

impl ReplaySelection {
    pub fn empty() -> Self {
        Self {
            value: request_context::empty(),
            refs: vec![],
            retained: 0,
            omitted: Some(0),
            scanned: 0,
            unscanned: Some(0),
            truncated: false,
            boundary_unavailable: false,
            budget_omitted: false,
        }
    }
}

pub(super) fn admit(
    input: DecisionInput<'_>,
    budget: &mut Remaining,
) -> Result<ReplaySelection, CallFailure> {
    let mut result = ReplaySelection::empty();
    let Some(boundary) = request_context::boundary(input) else {
        result.value["boundary_unavailable"] = json!(true);
        result.boundary_unavailable = true;
        result.omitted = None;
        result.unscanned = None;
        return Ok(result);
    };
    let scan_start = boundary.saturating_sub(256);
    result.truncated = scan_start > 0;
    let mut messages = Vec::new();
    for (ordinal, message) in input.request.messages[..boundary]
        .iter()
        .enumerate()
        .skip(scan_start)
        .rev()
    {
        result.scanned += 1;
        let role = match message.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            _ => continue,
        };
        if message.content.len() > 64 {
            result.truncated = true;
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
        if messages.len() == 16 {
            result.truncated = true;
            break;
        }
        let raw_fits = texts
            .iter()
            .try_fold(0usize, |used, text| {
                let bytes = text
                    .content_ref()
                    .map_or(text.len() as u64, |content| content.json_escaped_len());
                used.checked_add(usize::try_from(bytes).ok()?)
            })
            .is_some_and(|bytes| bytes <= budget.bytes);
        if !raw_fits {
            result.truncated = true;
            result.budget_omitted = true;
            break;
        }
        let mut refs = Vec::new();
        let parts = texts
            .iter()
            .map(|text| protocol::project_text(text, &mut refs))
            .collect::<Vec<_>>();
        let message = json!({"role":role,"message_ordinal":ordinal,"content":parts});
        let template = prepare_replay_json_template(&message, refs.clone())
            .map_err(|_| CallFailure::RejectedInput)?;
        // An object, role, ordinal, content array and three values per text part.
        let cost = Cost {
            bytes: template.wire_len,
            values: 4 + texts.len() * 3,
        }
        .comma(!messages.is_empty())?;
        if !budget.admit(cost) {
            result.truncated = true;
            result.budget_omitted = true;
            break;
        }
        result.refs.extend(refs);
        messages.push(message);
    }
    result.retained = messages.len();
    result.unscanned = Some(boundary - result.scanned);
    if result.truncated {
        result.omitted = None;
    }
    messages.reverse();
    result.value["messages"] = Value::Array(messages);
    result.value["truncated"] = json!(result.truncated);
    Ok(result)
}
