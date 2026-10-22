//! Comparable accepted response boundaries. Unsupported output loses an edge,
//! never capture or forwarding. Input text alone cannot establish this edge.
use std::collections::BTreeMap;
use std::io::{Read, Write};

use sha2::{Digest, Sha256};

use crate::content_ref::ContentValueExt;
use crate::context_hold::fingerprint::FingerprintBudget;
use crate::replay::{ReplayStore, write_canonical_json};
use crate::server::core_runtime::model_ir::{
    ContentPart, MessageRole, ModelEvent, ModelRequestIRV1, ToolKindV1,
};

const MAX_BLOCKS: usize = 128;

#[derive(Default)]
pub(super) struct BoundaryCapture {
    blocks: BTreeMap<u32, Block>,
    unavailable: bool,
    budget: FingerprintBudget,
}

enum Block {
    Text(Sha256),
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
            ModelEvent::TextDelta { index, text } => {
                if self.budget.bytes(text.len()).is_none() {
                    self.invalidate();
                    return;
                }
                match self
                    .blocks
                    .entry(*index)
                    .or_insert_with(|| Block::Text(Sha256::new()))
                {
                    Block::Text(hash) => hash.update(text.as_bytes()),
                    _ => self.unavailable = true,
                }
            }
            ModelEvent::TextFinished {
                index,
                text,
                annotations,
                ..
            } => {
                if self.budget.bytes(text.len()).is_none() {
                    self.invalidate();
                    return;
                }
                if !annotations.is_empty() {
                    self.unavailable = true;
                }
                self.blocks.insert(
                    *index,
                    Block::Text(Sha256::new_with_prefix(text.as_bytes())),
                );
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
            ModelEvent::ReasoningDelta { .. }
            | ModelEvent::ReasoningFinished { .. }
            | ModelEvent::RefusalDelta { .. }
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

    pub(super) fn digest(&self) -> Option<[u8; 32]> {
        if self.unavailable || self.blocks.is_empty() {
            return None;
        }
        let mut hash = Sha256::new_with_prefix(b"accepted-response-boundary/v2");
        for block in self.blocks.values() {
            match block {
                Block::Text(text) => {
                    hash.update(b"text");
                    hash.update(text.clone().finalize());
                }
                Block::Tool { digest, .. } => {
                    hash.update(b"tool");
                    hash.update(digest.as_ref()?);
                }
            }
        }
        Some(hash.finalize().into())
    }
}

pub(super) fn incoming_boundary(
    request: &ModelRequestIRV1,
    start: usize,
    replay: &ReplayStore,
    budget: &mut FingerprintBudget,
) -> Option<[u8; 32]> {
    let mut hash = Sha256::new_with_prefix(b"accepted-response-boundary/v2");
    let mut count = 0;
    for (index, message) in request.messages.iter().enumerate().skip(start) {
        if message.role != MessageRole::Assistant {
            break;
        }
        if message.name.is_some()
            || message.content.is_empty()
            || request.responses_reasoning_history.contains_key(&index)
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
                    hash.update(b"text");
                    hash.update(text_digest(text, Some(replay), budget)?);
                }
                ContentPart::ToolCall {
                    logical_id,
                    tool_kind,
                    namespace,
                    name,
                    arguments,
                    raw_arguments: None,
                } => {
                    hash.update(b"tool");
                    hash.update(tool_digest(
                        logical_id,
                        *tool_kind,
                        namespace.as_deref(),
                        name,
                        arguments,
                        Some(replay),
                        budget,
                    )?);
                }
                _ => return None,
            }
        }
    }
    (count > 0).then(|| hash.finalize().into())
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
