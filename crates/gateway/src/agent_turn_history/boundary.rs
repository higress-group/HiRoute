//! Comparable accepted response boundaries. Unsupported output loses an edge,
//! never capture or forwarding. Input text alone cannot establish this edge.
use std::collections::BTreeMap;
use std::io::Write;

use crate::runtime_fingerprint::RuntimeHash;

use crate::content_ref::{ContentValueExt, unsigned_messages_thinking_text};
use crate::context_hold::fingerprint::FingerprintBudget;
use crate::replay::{ReplayStore, write_canonical_json};
use crate::server::core_runtime::model_ir::{
    ContentPart, MessageRole, ModelEvent, ModelRequestIRV1, ResponseBlockKind,
    ResponsesReasoningEncryptedContentV1, ToolKindV1,
};

mod json_text;

const MAX_BLOCKS: usize = 128;

#[derive(Default)]
pub(super) struct BoundaryCapture {
    blocks: BTreeMap<u32, Block>,
    encrypted: BTreeMap<u32, [u8; 32]>,
    unavailable: bool,
    budget: FingerprintBudget,
}

enum Block {
    Text {
        hash: RuntimeHash,
        nonempty: bool,
        reasoning: bool,
    },
    Tool {
        logical_id: String,
        digest: Option<[u8; 32]>,
        plaintext_digest: Option<[u8; 32]>,
        encrypted: bool,
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
            .sum::<usize>()
            + self.encrypted.len() * 256
    }

    pub(super) fn invalidate(&mut self) {
        self.blocks.clear();
        self.encrypted.clear();
        self.unavailable = true;
    }

    pub(super) fn accept(&mut self, event: &ModelEvent) {
        if self.unavailable {
            return;
        }
        match event {
            ModelEvent::ContentBlockStarted {
                index,
                block_kind: ResponseBlockKind::Reasoning,
                ..
            } => self.text(*index, "", true, false),
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
            ModelEvent::ProviderState { state } if state.kind == "encrypted_content" => {
                let digest = state
                    .value
                    .as_str()
                    .and_then(|text| text_digest(text, None, &mut self.budget));
                match (state.block_index, digest) {
                    (Some(index), Some(digest))
                        if matches!(
                            self.blocks.get(&index),
                            Some(Block::Text {
                                reasoning: true,
                                ..
                            })
                        ) =>
                    {
                        if self
                            .encrypted
                            .insert(index, digest)
                            .is_some_and(|old| old != digest)
                        {
                            self.unavailable = true;
                        }
                    }
                    _ => self.unavailable = true,
                }
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
                        plaintext_digest: None,
                        encrypted: false,
                    },
                );
            }
            ModelEvent::ToolCallFinished {
                index,
                tool_kind,
                namespace,
                name,
                arguments,
                encrypted_function_args,
                ..
            } => {
                if let Some(Block::Tool {
                    logical_id,
                    digest,
                    plaintext_digest,
                    encrypted,
                }) = self.blocks.get_mut(index)
                {
                    *encrypted = encrypted_function_args
                        .as_ref()
                        .is_some_and(|paths| !paths.is_empty());
                    let base = tool_digest(
                        logical_id,
                        *tool_kind,
                        namespace.as_deref(),
                        name,
                        arguments,
                        None,
                        &mut self.budget,
                    );
                    *digest = base.and_then(|value| {
                        function_encoding_digest(
                            value,
                            encrypted_function_args.as_deref(),
                            &mut self.budget,
                        )
                    });
                    // Reuse the already bounded canonical argument digest. This
                    // additional proof never rereads Replay or changes exact bytes.
                    *plaintext_digest = if *tool_kind == ToolKindV1::Function {
                        base
                    } else {
                        *digest
                    };
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
            self.encrypted.clear();
            self.unavailable = true;
        }
    }

    fn text(&mut self, index: u32, text: &str, reasoning: bool, finished: bool) {
        if self.budget.bytes(text.len()).is_none() {
            self.invalidate();
            return;
        }
        let block = self.blocks.entry(index).or_insert_with(|| Block::Text {
            hash: RuntimeHash::new(),
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
                    *hash = RuntimeHash::new_with_prefix(text.as_bytes());
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
        for (index, block) in &self.blocks {
            match block {
                Block::Text {
                    nonempty: false,
                    reasoning: true,
                    ..
                } => hash.empty_reasoning(),
                Block::Text {
                    nonempty: false, ..
                } => {}
                Block::Text {
                    hash: text,
                    reasoning,
                    ..
                } => {
                    hash.part(
                        if *reasoning { b"reasoning" } else { b"text" },
                        text.clone().finalize(),
                    );
                }
                Block::Tool {
                    digest,
                    plaintext_digest,
                    encrypted,
                    ..
                } => {
                    hash.part_with_plaintext(b"tool", (*digest)?, (*plaintext_digest)?);
                    hash.encrypted |= *encrypted;
                }
            }
            if let Some(digest) = self.encrypted.get(index) {
                hash.part(b"encrypted_content", *digest);
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
    let native = request.native_history_layout();
    if request.native_only && native.is_none() {
        return None;
    }
    let mut hash = BoundaryHasher::default();
    let mut count = 0;
    for (index, message) in request.messages.iter().enumerate().skip(start) {
        if message.role != MessageRole::Assistant {
            break;
        }
        if native.is_some_and(|items| !items[index].modeled) {
            return None;
        }
        if let Some(history) = request.responses_reasoning_history.get(&index) {
            // Compare accepted plaintext and opaque state independently. Exact
            // ciphertext equality proves replay, never its meaning or affinity.
            // Native producers
            // can retain explicit empty fields without adding provider state or
            // content. Keep this evidence projection separate from wire replay.
            if message.name.is_some()
                || (history.encrypted_content != ResponsesReasoningEncryptedContentV1::Opaque
                    && !message.content.is_empty())
                || history
                    .native_fields
                    .iter()
                    .any(|(key, value)| match key.as_str() {
                        "summary" | "content" => false,
                        "status" => !value.is_null(),
                        _ => true,
                    })
            {
                return None;
            }
            let encrypted =
                if history.encrypted_content == ResponsesReasoningEncryptedContentV1::Opaque {
                    let [ContentPart::ProviderState { state }] = message.content.as_slice() else {
                        return None;
                    };
                    if state.kind != "encrypted_content" {
                        return None;
                    }
                    Some(json_text::digest(&state.value, replay, budget)?.0)
                } else {
                    None
                };
            hash.reasoning = true;
            if let Some(text) = reasoning_text(&history.native_fields, budget)? {
                let digest = text_digest(text, Some(replay), budget)?;
                if !text.is_empty() && !text.content_ref().is_some_and(|r| r.byte_len() == 0) {
                    hash.part(b"reasoning", digest);
                } else {
                    hash.empty_reasoning();
                }
            } else {
                hash.empty_reasoning();
            }
            if let Some(digest) = encrypted {
                hash.part(b"encrypted_content", digest);
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
                    encrypted_function_args,
                    logical_id,
                    tool_kind,
                    namespace,
                    name,
                    arguments,
                    raw_arguments: None,
                } => {
                    hash.encrypted |= encrypted_function_args
                        .as_ref()
                        .is_some_and(|paths| !paths.is_empty());
                    let base = tool_digest(
                        logical_id,
                        *tool_kind,
                        namespace.as_deref(),
                        name,
                        arguments,
                        Some(replay),
                        budget,
                    )?;
                    let exact =
                        function_encoding_digest(base, encrypted_function_args.as_deref(), budget)?;
                    hash.part_with_plaintext(
                        b"tool",
                        exact,
                        if *tool_kind == ToolKindV1::Function {
                            base
                        } else {
                            exact
                        },
                    );
                }
                ContentPart::ProviderState { state } => {
                    hash.reasoning = true;
                    if state.kind == "reasoning_content" {
                        // Chat aliases have already been normalized by ingress.
                        // A JSON string reference is storage, not opaque state.
                        let (digest, nonempty) = json_text::digest(&state.value, replay, budget)?;
                        if nonempty {
                            hash.part(b"reasoning", digest);
                        } else {
                            hash.empty_reasoning();
                        }
                    } else {
                        let text = unsigned_messages_thinking_text(state)?;
                        let digest = text_digest(text, Some(replay), budget)?;
                        if !text.is_empty()
                            && !text.content_ref().is_some_and(|r| r.byte_len() == 0)
                        {
                            hash.part(b"reasoning", digest);
                        } else {
                            hash.empty_reasoning();
                        }
                    }
                }
                _ => return None,
            }
        }
    }
    hash.finish().map(|mut evidence| {
        evidence.contextual = request.native_only;
        evidence
    })
}

/// Responses clients replay either the summary or the actual plaintext content.
/// Compare the one retained text with accepted output; never prefer one populated
/// carrier over another or silently discard unknown fields or additional parts.
fn reasoning_text<'a>(
    fields: &'a serde_json::Map<String, serde_json::Value>,
    budget: &mut FingerprintBudget,
) -> Option<Option<&'a str>> {
    let mut text = None;
    for (field, kind) in [("summary", "summary_text"), ("content", "reasoning_text")] {
        let Some(value) = fields.get(field).filter(|value| !value.is_null()) else {
            continue;
        };
        let parts = value.as_array()?;
        budget.items(parts.len())?;
        if parts.len() > 1 {
            return None;
        }
        if let Some(part) = parts.first() {
            let part = part.as_object()?;
            if text.is_some() || part.len() != 2 || part.get("type")?.as_str()? != kind {
                return None;
            }
            text = Some(part.get("text")?.as_str()?);
        }
    }
    Some(text)
}

/// Bounded proofs of the same ordered output. Only the original full digest is
/// exact; plaintext encoding and visible-only associations remain uncertain.
#[derive(Clone, Copy)]
pub(super) struct BoundaryEvidence {
    full: [u8; 32],
    plaintext_full: [u8; 32],
    visible: Option<[u8; 32]>,
    reasoning: bool,
    contextual: bool,
    encrypted: bool,
}

impl BoundaryEvidence {
    /// Some(false) is exact agreement; Some(true) is contextual association.
    /// Encoding compatibility keeps reasoning intact; omission remains directional.
    pub(super) fn matches(self, incoming: Self) -> Option<bool> {
        if self.full == incoming.full {
            Some(self.encrypted || incoming.encrypted || incoming.contextual)
        } else if !self.encrypted
            && !incoming.encrypted
            && (self.plaintext_full == incoming.plaintext_full
                || self.reasoning
                    && !incoming.reasoning
                    && self.visible.is_some()
                    && self.visible == incoming.visible)
        {
            Some(true)
        } else {
            None
        }
    }
}

struct BoundaryHasher {
    full: RuntimeHash,
    plaintext_full: RuntimeHash,
    visible: RuntimeHash,
    content: bool,
    visible_content: bool,
    reasoning: bool,
    encrypted: bool,
}

impl Default for BoundaryHasher {
    fn default() -> Self {
        Self {
            full: RuntimeHash::new_with_prefix(b"accepted-response-boundary/v2"),
            plaintext_full: RuntimeHash::new_with_prefix(
                b"accepted-plaintext-encoding-boundary/v1",
            ),
            visible: RuntimeHash::new_with_prefix(b"accepted-response-boundary/v2"),
            content: false,
            visible_content: false,
            reasoning: false,
            encrypted: false,
        }
    }
}

impl BoundaryHasher {
    fn empty_reasoning(&mut self) {
        // Only the new proof retains empty reasoning positions. Original full
        // and visible hashes deliberately keep their existing zero-content fold.
        self.plaintext_full.update(b"reasoning");
        self.plaintext_full.update(RuntimeHash::digest(b""));
    }

    fn part(&mut self, kind: &[u8], digest: [u8; 32]) {
        self.part_with_plaintext(kind, digest, digest);
    }

    fn part_with_plaintext(&mut self, kind: &[u8], digest: [u8; 32], plaintext: [u8; 32]) {
        self.full.update(kind);
        self.full.update(digest);
        self.plaintext_full.update(kind);
        self.plaintext_full.update(plaintext);
        self.content = true;
        if kind == b"reasoning" || kind == b"encrypted_content" {
            self.reasoning = true;
            self.encrypted |= kind == b"encrypted_content";
        } else {
            self.visible.update(kind);
            self.visible.update(digest);
            self.visible_content = true;
        }
    }

    fn finish(self) -> Option<BoundaryEvidence> {
        self.content.then(|| BoundaryEvidence {
            full: self.full.finalize(),
            plaintext_full: self.plaintext_full.finalize(),
            visible: self.visible_content.then(|| self.visible.finalize()),
            reasoning: self.reasoning,
            contextual: false,
            encrypted: self.encrypted,
        })
    }
}

fn text_digest(
    text: &str,
    replay: Option<&ReplayStore>,
    budget: &mut FingerprintBudget,
) -> Option<[u8; 32]> {
    let mut hash = RuntimeHash::new();
    if let Some(reference) = text.content_ref() {
        budget.bytes(usize::try_from(reference.byte_len()).ok()?)?;
        return replay?.runtime_fingerprint(&reference).ok();
    } else {
        budget.bytes(text.len())?;
        if let Some(replay) = replay {
            return Some(replay.runtime_bytes_fingerprint(text.as_bytes()));
        }
        hash.update(text.as_bytes());
    }
    Some(hash.finalize())
}

fn function_encoding_digest(
    base: [u8; 32],
    encoding: Option<&[String]>,
    budget: &mut FingerprintBudget,
) -> Option<[u8; 32]> {
    let Some(paths) = encoding else {
        return Some(base);
    };
    budget.items(paths.len())?;
    let mut hash = RuntimeHash::new_with_prefix(b"tool-function-encoding/v1");
    hash.update(base);
    hash.update((paths.len() as u64).to_be_bytes());
    for path in paths {
        hash.update(text_digest(path, None, budget)?);
    }
    Some(hash.finalize())
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
    let mut hash = RuntimeHash::new_with_prefix(b"tool-boundary/v2");
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
    struct Sink<'a>(&'a mut RuntimeHash);
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
    Some(hash.finalize())
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
            final_phase: None,
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
            final_phase: None,
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
            final_phase: None,
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
