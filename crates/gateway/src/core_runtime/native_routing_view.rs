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

#[path = "native_routing_view/codex.rs"]
mod codex;

pub(super) struct NativeRoutingView {
    current_input: Option<Vec<ContentPart>>,
    current_input_index: Option<usize>,
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
        if let Some(input) = crate::context_hold::native_input(headers) {
            let selected = match input {
                Ok(crate::context_hold::NativeInput::NativeMessages(window)) => {
                    window_input(request, replay, budget, &window)
                }
                _ => None,
            };
            return match selected {
                Some((parts, reservation)) => Self {
                    current_input: Some(parts),
                    current_input_index: None,
                    origin_unverified: true,
                    _reservation: Some(reservation),
                },
                None => Self {
                    current_input: None,
                    current_input_index: None,
                    origin_unverified: true,
                    _reservation: None,
                },
            };
        }
        if let Some(peer) = codex::prepare(request, replay, budget) {
            return peer;
        }
        if crate::context_hold::hermes_observed_input(headers).is_some() {
            // v1 clean-text hints are correlation only. A text hash cannot prove
            // which repeated/decorated callback input is the current wire event.
            return Self {
                current_input: None,
                current_input_index: None,
                origin_unverified: true,
                _reservation: None,
            };
        }
        let Some(proof) = dsh_user_turns(headers) else {
            return Self {
                current_input: None,
                current_input_index: None,
                origin_unverified: request.native_only,
                _reservation: None,
            };
        };
        let projected = proof
            .ok()
            .and_then(|turns| project(request, replay, budget, &turns));
        match projected {
            Some((parts, reservation)) => Self {
                current_input: Some(parts),
                current_input_index: None,
                origin_unverified: true,
                _reservation: Some(reservation),
            },
            None => Self {
                current_input: None,
                current_input_index: None,
                origin_unverified: true,
                _reservation: None,
            },
        }
    }

    pub(super) fn current_input(&self) -> Option<&[ContentPart]> {
        self.current_input.as_deref()
    }

    pub(super) fn current_input_index(&self) -> Option<usize> {
        self.current_input_index
    }

    pub(super) fn request<'a>(&'a self, original: &'a ModelRequestIRV1) -> &'a ModelRequestIRV1 {
        original
    }

    pub(super) fn origin_unverified(&self) -> bool {
        self.origin_unverified
    }
}

fn window_input(
    request: &ModelRequestIRV1,
    replay: &ReplayStore,
    budget: &StreamBudget,
    window: &crate::context_hold::NativeMessageWindow,
) -> Option<(Vec<ContentPart>, Reservation)> {
    let users = request
        .messages
        .iter()
        .filter(|message| {
            message.role == MessageRole::User
                && message.content.iter().any(|part| {
                    matches!(part, ContentPart::Text { .. } | ContentPart::Image { .. })
                })
        })
        .take(MAX_USER_MESSAGES + 1)
        .collect::<Vec<_>>();
    if users.len() > MAX_USER_MESSAGES {
        return None;
    }
    let bytes = users.iter().try_fold(0usize, |total, message| {
        message.content.iter().try_fold(total, |total, part| {
            let ContentPart::Text { text } = part else {
                return None;
            };
            let length = text
                .content_ref()
                .map_or(Some(text.len()), |r| usize::try_from(r.byte_len()).ok())?;
            total
                .checked_add(length)
                .filter(|n| *n <= MAX_USER_TEXT_BYTES)
        })
    })?;
    let reservation = budget
        .reserve(
            MemoryRole::ModelIrBacking,
            bytes.checked_mul(4)?.checked_add(32 * 1024)?,
        )
        .ok()?;
    let digests = users
        .iter()
        .map(|message| text_digest(&message.content, replay))
        .collect::<Option<Vec<_>>>()?;
    let encoded = serde_json::to_vec(&digests).ok()?;
    if format!("{:x}", Sha256::digest(&encoded)) != window.scope_digest {
        return None;
    }
    let mut parts = Vec::new();
    for message in &window.messages {
        if digests.get(message.index)? != &message.sha256 {
            return None;
        }
        parts.extend(users[message.index].content.clone());
    }
    Some((parts, reservation))
}

fn project(
    request: &ModelRequestIRV1,
    replay: &ReplayStore,
    budget: &StreamBudget,
    turns: &[NativeUserTurn],
) -> Option<(Vec<ContentPart>, Reservation)> {
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
    let mut parts = Vec::new();
    for message in &turns.last()?.messages {
        parts.extend(
            request.messages[*users.get(message.index)?]
                .content
                .iter()
                .cloned(),
        );
    }
    Some((parts, reservation))
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
