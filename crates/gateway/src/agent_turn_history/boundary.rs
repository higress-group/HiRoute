//! Comparable accepted response boundaries. Unsupported output loses an edge,
//! never capture or forwarding. Input text alone cannot establish this edge.
use std::collections::BTreeMap;
use std::io::{Read, Write};

use sha2::{Digest, Sha256};

use crate::content_ref::{ContentValueExt, unsigned_messages_thinking_text};
use crate::context_hold::fingerprint::FingerprintBudget;
use crate::replay::{ReplayStore, write_canonical_json};
use crate::server::core_runtime::model_ir::{
    ContentPart, MessageRole, ModelEvent, ModelRequestIRV1, ResponsesReasoningEncryptedContentV1,
    ToolKindV1,
};

mod json_text;

const MAX_BLOCKS: usize = 128;

#[derive(Default)]
pub(super) struct BoundaryCapture {
    blocks: BTreeMap<u32, Block>,
    unavailable: bool,
    budget: FingerprintBudget,
}

enum Block {
    Text {
        hash: Sha256,
        nonempty: bool,
        reasoning: bool,
    },
    Tool {
        logical_id: String,
        digest: Option<[u8; 32]>,
    },
}

impl BoundaryCapture {
    pub(super) fn bytes(&self) -> usize {
        self.blocks
            .values()
            .map(|b| {
                512 + match b {
                    Block::Tool { logical_id, .. } => logical_id.capacity(),
                    _ => 0,
                }
            })
            .sum()
    }

    pub(super) fn invalidate(&mut self) {
        self.blocks.clear();
        self.unavailable = true;
    }

    pub(super) fn accept(&mut self, event: &ModelEvent) {
        if self.unavailable {
            return;
        }
        match event {
            ModelEvent::TextDelta { index, text } => self.text(*index, text, false, false),
            ModelEvent::ReasoningDelta { index, text } => self.text(*index, text, true, false),
            ModelEvent::TextFinished {
                index,
                text,
                annotations,
                ..
            } => {
                if !annotations.is_empty() {
                    self.unavailable = true;
                }
                self.text(*index, text, false, true);
            }
            ModelEvent::ReasoningFinished { index, text, .. } => {
                self.text(*index, text, true, true)
            }
            ModelEvent::ToolCallStarted {
                index, logical_id, ..
            } => {
                if logical_id.len() > 256 {
                    self.invalidate();
                    return;
                }
                self.blocks.insert(
                    *index,
                    Block::Tool {
                        logical_id: logical_id.clone(),
                        digest: None,
                    },
                );
            }
            ModelEvent::ToolCallFinished {
                index,
                tool_kind,
                namespace,
                name,
                arguments,
                ..
            } => {
                if let Some(Block::Tool { logical_id, digest }) = self.blocks.get_mut(index) {
                    *digest = tool_digest(
                        logical_id,
                        *tool_kind,
                        namespace.as_deref(),
                        name,
                        arguments,
                        None,
                        &mut self.budget,
                    );
                    self.unavailable = digest.is_none();
                } else {
                    self.unavailable = true;
                }
            }
            ModelEvent::RefusalDelta { .. }
            | ModelEvent::RefusalFinished { .. }
            | ModelEvent::ProviderState { .. }
            | ModelEvent::WebSearch { .. }
            | ModelEvent::TextAnnotation { .. }
            | ModelEvent::ResponseFailed { .. } => {
                self.unavailable = true;
            }
            _ => {}
        }
        if self.blocks.len() > MAX_BLOCKS || self.unavailable {
            self.blocks.clear();
            self.unavailable = true;
        }
    }

    fn text(&mut self, index: u32, text: &str, reasoning: bool, finished: bool) {
        if self.budget.bytes(text.len()).is_none() {
            self.invalidate();
            return;
        }
        let block = self.blocks.entry(index).or_insert_with(|| Block::Text {
            hash: Sha256::new(),
            nonempty: false,
            reasoning,
        });
        match block {
            Block::Text {
                hash,
                nonempty,
                reasoning: original,
            } if *original == reasoning => {
                if finished {
                    // A terminal cannot erase accepted nonempty deltas.
                    self.unavailable |= *nonempty && text.is_empty();
                    *hash = Sha256::new_with_prefix(text.as_bytes());
                    *nonempty = !text.is_empty();
                } else {
                    hash.update(text.as_bytes());
                    *nonempty |= !text.is_empty();
                }
            }
            _ => self.unavailable = true,
        }
    }

    #[cfg(test)]
    pub(super) fn digest(&self) -> Option<[u8; 32]> {
        self.evidence().map(|e| e.full)
    }

    pub(super) fn evidence(&self) -> Option<BoundaryEvidence> {
        if self.unavailable || self.blocks.is_empty() {
            return None;
        }
        let mut hash = BoundaryHasher::default();
        for block in self.blocks.values() {
            match block {
                Block::Text {
                    nonempty: false, ..
                } => continue,
                Block::Text {
                    hash: text,
                    reasoning,
                    ..
                } => {
                    hash.part(
                        if *reasoning { b"reasoning" } else { b"text" },
                        text.clone().finalize().into(),
                    );
                }
                Block::Tool { digest, .. } => hash.part(b"tool", (*digest)?),
            }
        }
        hash.finish()
    }
}

#[cfg(test)]
pub(super) fn incoming_boundary(
    request: &ModelRequestIRV1,
    start: usize,
    replay: &ReplayStore,
    budget: &mut FingerprintBudget,
) -> Option<[u8; 32]> {
    incoming_evidence(request, start, replay, budget).map(|e| e.full)
}

pub(super) fn incoming_evidence(
    request: &ModelRequestIRV1,
    start: usize,
    replay: &ReplayStore,
    budget: &mut FingerprintBudget,
) -> Option<BoundaryEvidence> {
    let mut hash = BoundaryHasher::default();
    let mut count = 0;
    for (index, message) in request.messages.iter().enumerate().skip(start) {
        if message.role != MessageRole::Assistant {
            break;
        }
        if let Some(history) = request.responses_reasoning_history.get(&index) {
            // Compare the bounded accepted plaintext summary. Native producers
            // can retain explicit empty fields without adding provider state or
            // content. Keep this evidence projection separate from wire replay.
            if message.name.is_some()
                || !message.content.is_empty()
                || history.encrypted_content == ResponsesReasoningEncryptedContentV1::Opaque
                || history
                    .native_fields
                    .iter()
                    .any(|(key, value)| match key.as_str() {
                        "summary" => false,
                        "status" => !value.is_null(),
                        "content" => {
                            !value.is_null() && !value.as_array().is_some_and(Vec::is_empty)
                        }
                        _ => true,
                    })
            {
                return None;
            }
            hash.reasoning = true;
            let summary = history.native_fields.get("summary")?.as_array()?;
            budget.items(summary.len())?;
            if summary.len() > 1 {
                return None;
            }
            if let Some(part) = summary.first() {
                let part = part.as_object()?;
                if part.len() != 2 || part.get("type")?.as_str()? != "summary_text" {
                    return None;
                }
                let text = part.get("text")?.as_str()?;
                let digest = text_digest(text, Some(replay), budget)?;
                if !text.is_empty() && !text.content_ref().is_some_and(|r| r.byte_len() == 0) {
                    hash.part(b"reasoning", digest);
                }
            }
            continue;
        }
        if message.name.is_some()
            || message.content.is_empty()
            || request.responses_search_history.contains_key(&index)
            || request.responses_annotations.contains_key(&index)
        {
            return None;
        }
        for part in &message.content {
            count += 1;
            budget.items(1)?;
            if count > MAX_BLOCKS {
                return None;
            }
            match part {
                ContentPart::Text { text } => {
                    let digest = text_digest(text, Some(replay), budget)?;
                    // Native clients can replay null tool-response content as
                    // an empty Text placeholder. Compare its semantic absence
                    // without changing the forwarded or captured message.
                    if text.is_empty() || text.content_ref().is_some_and(|r| r.byte_len() == 0) {
                        continue;
                    }
                    hash.part(b"text", digest);
                }
                ContentPart::ToolCall {
                    logical_id,
                    tool_kind,
                    namespace,
                    name,
                    arguments,
                    raw_arguments: None,
                } => {
                    hash.part(
                        b"tool",
                        tool_digest(
                            logical_id,
                            *tool_kind,
                            namespace.as_deref(),
                            name,
                            arguments,
                            Some(replay),
                            budget,
                        )?,
                    );
                }
                ContentPart::ProviderState { state } => {
                    hash.reasoning = true;
                    if state.kind == "reasoning_content" {
                        if request.native_only {
                            return None;
                        }
                        // Chat aliases have already been normalized by ingress.
                        // A JSON string reference is storage, not opaque state.
                        let (digest, nonempty) = json_text::digest(&state.value, replay, budget)?;
                        if nonempty {
                            hash.part(b"reasoning", digest);
                        }
                    } else {
                        let text = unsigned_messages_thinking_text(state)?;
                        let digest = text_digest(text, Some(replay), budget)?;
                        if !text.is_empty()
                            && !text.content_ref().is_some_and(|r| r.byte_len() == 0)
                        {
                            hash.part(b"reasoning", digest);
                        }
                    }
                }
                _ => return None,
            }
        }
    }
    hash.finish().map(|mut evidence| {
        evidence.projection_allowed = !request.native_only;
        evidence
    })
}

/// Two bounded projections of the same accepted output. Only the full digest is
/// an exact response proof; visible-only association is explicitly uncertain.
#[derive(Clone, Copy)]
pub(super) struct BoundaryEvidence {
    full: [u8; 32],
    visible: Option<[u8; 32]>,
    reasoning: bool,
    projection_allowed: bool,
}

impl BoundaryEvidence {
    /// Some(false) is full agreement; Some(true) is omitted plaintext reasoning.
    /// Present (even empty) reasoning never authorizes a projection fallback.
    pub(super) fn matches(self, incoming: Self) -> Option<bool> {
        if !incoming.projection_allowed {
            return None;
        }
        if self.full == incoming.full {
            Some(false)
        } else if self.reasoning
            && incoming.projection_allowed
            && !incoming.reasoning
            && self.visible.is_some()
            && self.visible == incoming.visible
        {
            Some(true)
        } else {
            None
        }
    }
}

struct BoundaryHasher {
    full: Sha256,
    visible: Sha256,
    content: bool,
    visible_content: bool,
    reasoning: bool,
}

impl Default for BoundaryHasher {
    fn default() -> Self {
        Self {
            full: Sha256::new_with_prefix(b"accepted-response-boundary/v2"),
            visible: Sha256::new_with_prefix(b"accepted-response-boundary/v2"),
            content: false,
            visible_content: false,
            reasoning: false,
        }
    }
}

impl BoundaryHasher {
    fn part(&mut self, kind: &[u8], digest: [u8; 32]) {
        self.full.update(kind);
        self.full.update(digest);
        self.content = true;
        if kind == b"reasoning" {
            self.reasoning = true;
        } else {
            self.visible.update(kind);
            self.visible.update(digest);
            self.visible_content = true;
        }
    }

    fn finish(self) -> Option<BoundaryEvidence> {
        self.content.then(|| BoundaryEvidence {
            full: self.full.finalize().into(),
            visible: self.visible_content.then(|| self.visible.finalize().into()),
            reasoning: self.reasoning,
            projection_allowed: true,
        })
    }
}

fn text_digest(
    text: &str,
    replay: Option<&ReplayStore>,
    budget: &mut FingerprintBudget,
) -> Option<[u8; 32]> {
    let mut hash = Sha256::new();
    if let Some(reference) = text.content_ref() {
        budget.bytes(usize::try_from(reference.byte_len()).ok()?)?;
        let mut reader = replay?.reader(&reference).ok()?;
        let mut buffer = [0u8; 8192];
        loop {
            let n = reader.read(&mut buffer).ok()?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        reader.verify_terminal().ok()?;
    } else {
        budget.bytes(text.len())?;
        hash.update(text.as_bytes());
    }
    Some(hash.finalize().into())
}

fn tool_digest(
    id: &str,
    kind: ToolKindV1,
    namespace: Option<&str>,
    name: &str,
    args: &serde_json::Value,
    replay: Option<&ReplayStore>,
    budget: &mut FingerprintBudget,
) -> Option<[u8; 32]> {
    let mut hash = Sha256::new_with_prefix(b"tool-boundary/v2");
    hash.update(text_digest(id, None, budget)?);
    hash.update(match kind {
        ToolKindV1::Function => b"function".as_slice(),
        ToolKindV1::Custom => b"custom".as_slice(),
    });
    hash.update([u8::from(namespace.is_some())]);
    if let Some(namespace) = namespace {
        hash.update(text_digest(namespace, replay, budget)?);
    }
    hash.update(text_digest(name, replay, budget)?);
    // Canonical JSON includes arguments and their types, never only the tool name.
    struct Sink<'a>(&'a mut Sha256);
    impl Write for Sink<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    if budget.json(args)? {
        crate::context_hold::write_canonical_json_with_replay(&mut Sink(&mut hash), args, replay?)?;
    } else {
        write_canonical_json(&mut Sink(&mut hash), args).ok()?;
    }
    Some(hash.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_hold::fingerprint::MAX_BYTES;
    use serde_json::json;

    #[test]
    fn empty_text_alone_cannot_establish_an_accepted_response_edge() {
        let mut capture = BoundaryCapture::default();
        capture.accept(&ModelEvent::TextDelta {
            index: 0,
            text: String::new(),
        });
        assert!(capture.digest().is_none());
        capture.accept(&ModelEvent::TextFinished {
            index: 0,
            text: String::new(),
            annotations: Vec::new(),
            status: crate::server::core_runtime::model_ir::ResponseItemStatus::Completed,
        });
        assert!(capture.digest().is_none());
    }

    #[test]
    fn empty_terminal_cannot_erase_an_accepted_delta() {
        let mut capture = BoundaryCapture::default();
        capture.accept(&ModelEvent::TextDelta {
            index: 0,
            text: "accepted".into(),
        });
        assert!(capture.digest().is_some());
        capture.accept(&ModelEvent::TextFinished {
            index: 0,
            text: String::new(),
            annotations: Vec::new(),
            status: crate::server::core_runtime::model_ir::ResponseItemStatus::Completed,
        });
        assert!(
            capture.digest().is_none(),
            "empty terminal cannot erase an accepted delta"
        );
    }

    #[test]
    fn comparisons_share_one_budget_and_large_output_cannot_establish_an_edge() {
        let mut budget = FingerprintBudget::default();
        budget.bytes(MAX_BYTES - 3).unwrap();
        assert!(text_digest("abc", None, &mut budget).is_some());
        assert!(text_digest("abc", None, &mut budget).is_none());
        let mut captured = BoundaryCapture::default();
        captured.accept(&ModelEvent::TextDelta {
            index: 0,
            text: "x".repeat(MAX_BYTES + 1),
        });
        captured.accept(&ModelEvent::TextFinished {
            index: 0,
            text: "small suffix".into(),
            annotations: Vec::new(),
            status: crate::server::core_runtime::model_ir::ResponseItemStatus::Completed,
        });
        assert!(captured.digest().is_none());
    }

    #[test]
    fn tool_json_limit_does_not_admit_a_truncated_digest() {
        let args = json!({"text":"x".repeat(MAX_BYTES / 6 + 1)});
        assert!(
            tool_digest(
                "call",
                ToolKindV1::Function,
                None,
                "tool",
                &args,
                None,
                &mut FingerprintBudget::default()
            )
            .is_none()
        );
        assert!(
            tool_digest(
                "call",
                ToolKindV1::Function,
                None,
                "tool",
                &json!({"text":"small"}),
                None,
                &mut FingerprintBudget::default()
            )
            .is_some()
        );
    }
}
