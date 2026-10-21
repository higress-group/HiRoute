//! Optional native semantic user projection. Execution and Observation retain the original IR.
use std::io::{self, Read, Write};

use hiroute_gateway_core::runtime::body::{MemoryRole, Reservation, StreamBudget};
use http::HeaderMap;
use sha2::{Digest, Sha256};

use super::model_ir::{ContentPart, MessageRole, ModelRequestIRV1};
use crate::content_ref::ContentValueExt;
use crate::context_hold::{NativeUserTurn, dsh_user_turns};
use crate::replay::ReplayStore;

const MAX_COMPACT_IR_BYTES: usize = 256 * 1024;
const MAX_USER_TEXT_BYTES: usize = 2 * 1024 * 1024;
const MAX_USER_MESSAGES: usize = 128;

pub(super) struct NativeRoutingView {
    projected: Option<Box<ModelRequestIRV1>>,
    origin_unverified: bool,
    _reservation: Option<Reservation>,
}

impl NativeRoutingView {
    pub(super) fn prepare(
        headers: &HeaderMap,
        request: &ModelRequestIRV1,
        replay: &ReplayStore,
        budget: &StreamBudget,
    ) -> Self {
        // A copied ancestor User is not proof of the current peer task. Keep
        // routing usable on observed input and disclose the missing provenance.
        let peer_task = request.native_only
            && request.ingress_protocol == crate::server::request_plan::IngressProtocol::Responses
            && request
                .native_body
                .as_ref()
                .and_then(|body| body.get("input"))
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| {
                    items.iter().any(|item| {
                        item.get("type").and_then(serde_json::Value::as_str)
                            == Some("agent_message")
                    })
                });
        if peer_task {
            return Self {
                projected: None,
                origin_unverified: true,
                _reservation: None,
            };
        }
        let Some(proof) = dsh_user_turns(headers) else {
            return Self {
                projected: None,
                origin_unverified: request.native_only,
                _reservation: None,
            };
        };
        let projected = proof
            .ok()
            .and_then(|turns| project(request, replay, budget, &turns));
        match projected {
            Some((request, reservation)) => Self {
                projected: Some(request),
                origin_unverified: false,
                _reservation: Some(reservation),
            },
            None => Self {
                projected: None,
                origin_unverified: true,
                _reservation: None,
            },
        }
    }

    pub(super) fn request<'a>(&'a self, original: &'a ModelRequestIRV1) -> &'a ModelRequestIRV1 {
        self.projected.as_deref().unwrap_or(original)
    }

    pub(super) fn origin_unverified(&self) -> bool {
        self.origin_unverified
    }
}

fn project(
    request: &ModelRequestIRV1,
    replay: &ReplayStore,
    budget: &StreamBudget,
    turns: &[NativeUserTurn],
) -> Option<(Box<ModelRequestIRV1>, Reservation)> {
    let mut counter = BoundedCounter(0);
    // native_body is deliberately serde-skipped. A normal Clone would duplicate
    // that potentially large opaque execution document outside this bound.
    serde_json::to_writer(&mut counter, request).ok()?;
    let users = request
        .messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            (message.role == MessageRole::User
                && message.content.iter().any(|part| {
                    matches!(part, ContentPart::Text { .. } | ContentPart::Image { .. })
                }))
            .then_some(index)
        })
        .take(MAX_USER_MESSAGES + 1)
        .collect::<Vec<_>>();
    if users.len() > MAX_USER_MESSAGES {
        return None;
    }
    let text_bytes = users.iter().try_fold(0usize, |total, &index| {
        request.messages[index]
            .content
            .iter()
            .try_fold(total, |total, part| {
                let ContentPart::Text { text } = part else {
                    return None;
                };
                let bytes = match text.content_ref() {
                    Some(reference) => usize::try_from(reference.byte_len()).ok()?,
                    None => text.len(),
                };
                total
                    .checked_add(bytes)
                    .filter(|n| *n <= MAX_USER_TEXT_BYTES)
            })
    })?;
    // Includes the bounded encoded buffer, deserialized metadata/container
    // overhead, and temporary resolved user text. Failure is an optimization miss.
    let bytes = counter
        .0
        .checked_mul(64)?
        .checked_add(text_bytes.checked_mul(2)?)?
        .checked_add(4096)?;
    let reservation = budget.reserve(MemoryRole::ModelIrBacking, bytes).ok()?;
    for turn in turns {
        for message in &turn.messages {
            let index = *users.get(message.index)?;
            if text_digest(&request.messages[index].content, replay)? != message.sha256 {
                return None;
            }
        }
        let first = *users.get(turn.messages.first()?.index)?;
        let last = *users.get(turn.messages.last()?.index)?;
        // An interleaved model/tool exchange cannot be reordered into one input batch.
        if request.messages[first..=last].iter().any(|message| {
            message.role == MessageRole::Assistant
                || message.content.iter().any(|part| {
                    matches!(
                        part,
                        ContentPart::ToolCall { .. } | ContentPart::ToolResult { .. }
                    )
                })
        }) {
            return None;
        }
    }
    let encoded = serde_json::to_vec(request).ok()?;
    let mut view: Box<ModelRequestIRV1> = Box::new(serde_json::from_slice(&encoded).ok()?);
    view.native_only = request.native_only;
    // Retain positions and synthetic context for strict ContextHold fingerprints,
    // but only native semantic users may open routing turns. This view is never
    // serialized to a provider or captured as the observed model request.
    for &index in &users {
        view.messages[index].role = MessageRole::System;
    }
    for turn in turns {
        let last = *users.get(turn.messages.last()?.index)?;
        let mut content = Vec::new();
        for message in &turn.messages {
            let index = *users.get(message.index)?;
            content.extend(std::mem::take(&mut view.messages[index].content));
        }
        view.messages[last].content = content;
        view.messages[last].role = MessageRole::User;
    }
    Some((view, reservation))
}

fn text_digest(parts: &[ContentPart], replay: &ReplayStore) -> Option<String> {
    let mut strings = Vec::with_capacity(parts.len());
    for part in parts {
        let ContentPart::Text { text } = part else {
            return None;
        };
        let value = if let Some(reference) = text.content_ref() {
            if reference.byte_len() > MAX_USER_TEXT_BYTES as u64 {
                return None;
            }
            let mut reader = replay.reader(&reference).ok()?;
            let mut value = String::new();
            reader
                .by_ref()
                .take(MAX_USER_TEXT_BYTES as u64 + 1)
                .read_to_string(&mut value)
                .ok()?;
            if value.len() > MAX_USER_TEXT_BYTES {
                return None;
            }
            reader.verify_terminal().ok()?;
            value
        } else {
            text.clone()
        };
        strings.push(value);
    }
    let mut hash = HashWriter(Sha256::new());
    serde_json::to_writer(&mut hash, &strings).ok()?;
    Some(format!("{:x}", hash.0.finalize()))
}

struct BoundedCounter(usize);
impl Write for BoundedCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .filter(|n| *n <= MAX_COMPACT_IR_BYTES)
            .ok_or_else(|| io::Error::other("optional routing view capacity"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct HashWriter(Sha256);
impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "native_routing_view/tests.rs"]
mod tests;
