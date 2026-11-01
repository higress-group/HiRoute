//! Bounded semantic segments reuse ModelEvent and its existing product reader.
//! No frame/event log and no whole-response buffer are needed.
use super::{CONTENT_CHUNK_BYTES, ModelEvent, ModelStreamEventV1};
use crate::server::core_runtime::observation::{
    crypto::WorkspaceHmac, producer::ObservationWorkReservation,
};
use std::collections::BTreeMap;

const MAX_BLOCKS: usize = 128;
pub(super) const RESERVATION_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Key(u32, u8, String);
struct Proof {
    hash: WorkspaceHmac,
    bytes: usize,
}

pub(in crate::server::core_runtime::observation) struct SemanticResponse {
    pub(super) attempt: String,
    pub(super) partial: bool,
    key: [u8; 32],
    proofs: BTreeMap<Key, Proof>,
    pending: Option<(Key, ModelStreamEventV1)>,
    // This owner outlives a queued delivery and releases its memory exactly
    // once on terminal, cancellation, capture failure or request destruction.
    _reservation: ObservationWorkReservation,
}

fn text(event: &ModelEvent) -> Option<(&str, u32, u8, bool)> {
    match event {
        ModelEvent::TextDelta { index, text } => Some((text, *index, 0, false)),
        ModelEvent::ReasoningDelta { index, text } => Some((text, *index, 1, false)),
        ModelEvent::RefusalDelta { index, text } => Some((text, *index, 2, false)),
        ModelEvent::ToolArgumentsDelta { index, delta, .. } => Some((delta, *index, 3, false)),
        ModelEvent::TextFinished { index, text, .. } => Some((text, *index, 0, true)),
        ModelEvent::ReasoningFinished { index, text, .. } => Some((text, *index, 1, true)),
        ModelEvent::RefusalFinished { index, text, .. } => Some((text, *index, 2, true)),
        _ => None,
    }
}
fn text_mut(event: &mut ModelEvent) -> Option<&mut String> {
    match event {
        ModelEvent::TextDelta { text, .. }
        | ModelEvent::ReasoningDelta { text, .. }
        | ModelEvent::RefusalDelta { text, .. }
        | ModelEvent::TextFinished { text, .. }
        | ModelEvent::ReasoningFinished { text, .. }
        | ModelEvent::RefusalFinished { text, .. } => Some(text),
        ModelEvent::ToolArgumentsDelta { delta, .. } => Some(delta),
        _ => None,
    }
}

impl SemanticResponse {
    pub(super) fn new(
        attempt: &str,
        key: [u8; 32],
        reservation: ObservationWorkReservation,
    ) -> Self {
        Self {
            attempt: attempt.into(),
            key,
            partial: false,
            proofs: BTreeMap::new(),
            pending: None,
            _reservation: reservation,
        }
    }

    fn flush_into(&mut self, output: &mut Vec<ModelStreamEventV1>) {
        if let Some((_, event)) = self.pending.take() {
            output.push(event);
        }
    }

    pub(super) fn accept(&mut self, mut event: ModelStreamEventV1) -> Vec<ModelStreamEventV1> {
        let mut output = Vec::new();
        let Some((value, index, kind, finished)) = text(&event.event) else {
            self.flush_into(&mut output);
            output.push(event);
            return output;
        };
        if matches!(&event.event, ModelEvent::ToolArgumentsDelta{logical_id,..} if logical_id.len()>4096)
        {
            self.partial = true;
            self.flush_into(&mut output);
            output.push(event);
            return output;
        }
        let tool = if let ModelEvent::ToolArgumentsDelta { logical_id, .. } = &event.event {
            let mut hash = WorkspaceHmac::new(&self.key, b"semantic-tool-identity");
            hash.update(logical_id.as_bytes());
            hash.finish()
        } else {
            String::new()
        };
        let identity = Key(index, kind, tool);
        if finished {
            self.flush_into(&mut output);
            if let Some(proof) = self.proofs.get(&identity) {
                let mut full = WorkspaceHmac::new(&self.key, b"semantic-accepted-text");
                full.update_stream(value.as_bytes());
                if value.len() == proof.bytes && full.finish() == proof.hash.clone().finish() {
                    // Retain status/annotations/phase without another full copy
                    // of already accepted text. Empty finals never erase deltas.
                    text_mut(&mut event.event).expect("text final").clear();
                } else {
                    self.partial = true;
                }
            }
            output.push(event);
            return output;
        }
        if !self.proofs.contains_key(&identity) && self.proofs.len() >= MAX_BLOCKS {
            self.partial = true;
            self.flush_into(&mut output);
            output.push(event);
            return output;
        }
        let proof = self
            .proofs
            .entry(identity.clone())
            .or_insert_with(|| Proof {
                hash: WorkspaceHmac::new(&self.key, b"semantic-accepted-text"),
                bytes: 0,
            });
        proof.hash.update_stream(value.as_bytes());
        proof.bytes = proof.bytes.saturating_add(value.len());
        let value = std::mem::take(text_mut(&mut event.event).expect("delta"));
        if self
            .pending
            .as_ref()
            .is_some_and(|(key, _)| key != &identity)
        {
            self.flush_into(&mut output);
        }
        // Split only on UTF-8 boundaries. A single incoming event may produce
        // several parts; its queued reservation owns that bounded source work.
        let mut remaining = value.as_str();
        loop {
            let (_, pending) = self
                .pending
                .get_or_insert_with(|| (identity.clone(), event.clone()));
            let pending_text = text_mut(&mut pending.event).expect("pending delta");
            let space = CONTENT_CHUNK_BYTES.saturating_sub(pending_text.len());
            let mut take = space.min(remaining.len());
            while !remaining.is_char_boundary(take) {
                take -= 1;
            }
            if take == 0 && !remaining.is_empty() {
                self.flush_into(&mut output);
                continue;
            }
            pending_text.push_str(&remaining[..take]);
            remaining = &remaining[take..];
            if pending_text.len() == CONTENT_CHUNK_BYTES {
                self.flush_into(&mut output);
            }
            if remaining.is_empty() {
                break;
            }
        }
        output
    }

    pub(super) fn finish(&mut self) -> Vec<ModelStreamEventV1> {
        let mut output = Vec::new();
        self.flush_into(&mut output);
        output
    }
}

#[cfg(test)]
#[path = "semantic/tests.rs"]
mod tests;
