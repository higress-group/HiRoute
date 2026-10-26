//! Optional native semantic user projection. Execution and Observation retain the original IR.
use std::io::{self, Read, Write};

use hiroute_diagnostics::event::NativeInputProjection;
use hiroute_gateway_core::runtime::body::{MemoryRole, Reservation, StreamBudget};
use http::HeaderMap;
use sha2::{Digest, Sha256};

use super::model_ir::{ContentPart, MessageRole, ModelRequestIRV1};
use crate::agent_turn_history::CurrentInputPosition;
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
    projection: NativeInputProjection,
    current_input_position: Option<CurrentInputPosition>,
    current_input_unavailable: bool,
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
        // A typed native peer is stronger event-kind evidence than optional
        // adapter enrichment. An unrelated/invalid window cannot select its
        // ancestor User or make opaque peer intent readable.
        if let Some(peer) = codex::prepare(request, replay, budget) {
            return peer;
        }
        if let Some(input) = crate::context_hold::native_input(headers) {
            let (selected, projection) = match input {
                Ok(Some(crate::context_hold::NativeInput::NativeMessages(window))) => {
                    let selected = window_input(request, replay, budget, &window);
                    let verdict = if selected.is_some() {
                        NativeInputProjection::WindowApplied
                    } else {
                        NativeInputProjection::WindowRejected
                    };
                    (selected, verdict)
                }
                Ok(_) => (None, NativeInputProjection::CorrelationOnly),
                Err(_) => (None, NativeInputProjection::InvalidInput),
            };
            return match selected {
                Some((parts, position, reservation)) => Self {
                    current_input: Some(parts),
                    projection,
                    current_input_position: Some(position),
                    current_input_unavailable: false,
                    origin_unverified: true,
                    _reservation: Some(reservation),
                },
                None => Self {
                    current_input: None,
                    projection,
                    current_input_position: None,
                    current_input_unavailable: false,
                    origin_unverified: true,
                    _reservation: None,
                },
            };
        }
        if crate::context_hold::hermes_observed_input(headers).is_some() {
            // v1 clean-text hints are correlation only. A text hash cannot prove
            // which repeated/decorated callback input is the current wire event.
            return Self {
                current_input: None,
                projection: NativeInputProjection::LegacyHintObserved,
                current_input_position: None,
                current_input_unavailable: false,
                origin_unverified: true,
                _reservation: None,
            };
        }
        let Some(proof) = dsh_user_turns(headers) else {
            return Self {
                current_input: None,
                projection: NativeInputProjection::Wire,
                current_input_position: None,
                current_input_unavailable: false,
                origin_unverified: request.native_only,
                _reservation: None,
            };
        };
        let projected = proof
            .ok()
            .and_then(|turns| project(request, replay, budget, &turns));
        match projected {
            Some((parts, position, reservation)) => Self {
                current_input: Some(parts),
                projection: NativeInputProjection::LegacyWindowApplied,
                current_input_position: Some(position),
                current_input_unavailable: false,
                origin_unverified: true,
                _reservation: Some(reservation),
            },
            None => Self {
                current_input: None,
                projection: NativeInputProjection::LegacyWindowRejected,
                current_input_position: None,
                current_input_unavailable: false,
                origin_unverified: true,
                _reservation: None,
            },
        }
    }

    pub(super) fn projection(&self) -> NativeInputProjection {
        self.projection
    }

    pub(super) fn current_input(&self) -> Option<&[ContentPart]> {
        self.current_input.as_deref()
    }

    pub(super) fn current_input_position(&self) -> Option<CurrentInputPosition> {
        self.current_input_position
    }

    pub(super) fn current_input_unavailable(&self) -> bool {
        self.current_input_unavailable
    }

    pub(super) fn excerpt_selection(
        &self,
        request: &ModelRequestIRV1,
        source_unverified: bool,
    ) -> hiroute_domain::RequestInputSelectionV1 {
        use hiroute_domain::RequestInputSelectionEvidenceV1 as E;
        let position = if self.current_input_unavailable {
            None
        } else {
            match self.current_input_position {
                Some(CurrentInputPosition::Event(index)) => Some((index, E::NativeMatched)),
                Some(CurrentInputPosition::Window { first, .. }) => Some((first, E::NativeMatched)),
                None if self.projection == NativeInputProjection::Wire => {
                    observed_input_index(request).map(|index| (index, E::WireObserved))
                }
                None => None,
            }
        };
        let selected = position.and_then(|(index, evidence)| {
            let message = request.messages.get(index)?;
            if message.role != MessageRole::User {
                return None;
            }
            let ordinal = u32::try_from(request.instructions.len().checked_add(index)?).ok()?;
            (ordinal <= 65_535).then_some((ordinal, evidence))
        });
        hiroute_domain::RequestInputSelectionV1 {
            version: 1,
            evidence: selected.map_or(E::Unavailable, |(_, evidence)| evidence),
            message_ordinal: selected.map(|(ordinal, _)| ordinal),
            source_unverified,
        }
    }

    pub(super) fn request<'a>(&'a self, original: &'a ModelRequestIRV1) -> &'a ModelRequestIRV1 {
        original
    }

    pub(super) fn origin_unverified(&self) -> bool {
        self.origin_unverified
    }
}

pub(super) fn observed_input_index(request: &ModelRequestIRV1) -> Option<usize> {
    request.messages.iter().rposition(|message| {
        message.role == MessageRole::User
            && message
                .content
                .iter()
                .any(|part| matches!(part, ContentPart::Text { .. } | ContentPart::Image { .. }))
    })
}

fn window_input(
    request: &ModelRequestIRV1,
    replay: &ReplayStore,
    budget: &StreamBudget,
    window: &crate::context_hold::NativeMessageWindow,
) -> Option<(Vec<ContentPart>, CurrentInputPosition, Reservation)> {
    let users = request
        .messages
        .iter()
        .enumerate()
        .filter(|(_, message)| {
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
    let bytes = users.iter().try_fold(0usize, |total, (_, message)| {
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
        .map(|(_, message)| text_digest(&message.content, replay))
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
        parts.extend(users[message.index].1.content.clone());
    }
    let first = users.get(window.messages.first()?.index)?.0;
    let last = users.get(window.messages.last()?.index)?.0;
    Some((
        parts,
        input_window_position(request, first, last)?,
        reservation,
    ))
}

fn input_window_position(
    request: &ModelRequestIRV1,
    first: usize,
    last: usize,
) -> Option<CurrentInputPosition> {
    // A batch cannot bridge model/tool execution and erase its capture boundary.
    if request.messages[first..=last]
        .iter()
        .enumerate()
        .any(|(offset, message)| {
            message.role == MessageRole::Assistant
                || message.content.iter().any(|part| {
                    matches!(
                        part,
                        ContentPart::ToolCall { .. }
                            | ContentPart::ToolResult { .. }
                            | ContentPart::ProviderState { .. }
                    )
                })
                || request
                    .responses_search_history
                    .contains_key(&(first + offset))
                || request
                    .responses_reasoning_history
                    .contains_key(&(first + offset))
        })
    {
        return None;
    }
    Some(CurrentInputPosition::Window { first, last })
}

fn project(
    request: &ModelRequestIRV1,
    replay: &ReplayStore,
    budget: &StreamBudget,
    turns: &[NativeUserTurn],
) -> Option<(Vec<ContentPart>, CurrentInputPosition, Reservation)> {
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
    let first = *users.get(turns.last()?.messages.first()?.index)?;
    let last = *users.get(turns.last()?.messages.last()?.index)?;
    Some((
        parts,
        input_window_position(request, first, last)?,
        reservation,
    ))
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
