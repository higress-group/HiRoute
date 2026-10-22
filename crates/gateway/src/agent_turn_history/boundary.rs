//! Comparable accepted response boundaries. Unsupported output loses an edge,
//! never capture or forwarding. Input text alone cannot establish this edge.
use std::collections::BTreeMap;
use std::io::{Read, Write};

use sha2::{Digest, Sha256};

use crate::content_ref::{ContentValueExt, JsonValueExt};
use crate::replay::{ReplayStore, write_canonical_json};
use crate::server::core_runtime::model_ir::{
    ContentPart, MessageRole, ModelEvent, ModelRequestIRV1, ToolKindV1,
};

const MAX_BLOCKS: usize = 128;

#[derive(Default)]
pub(super) struct BoundaryCapture {
    blocks: BTreeMap<u32, Block>,
    unavailable: bool,
}

enum Block {
    Text(Sha256),
    Tool {
        native_id: String,
        digest: Option<[u8; 32]>,
    },
}

impl BoundaryCapture {
    pub(super) fn bytes(&self) -> usize {
        self.blocks
            .values()
            .map(|b| {
                512 + match b {
                    Block::Tool { native_id, .. } => native_id.capacity(),
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
                if !annotations.is_empty() {
                    self.unavailable = true;
                }
                self.blocks.insert(
                    *index,
                    Block::Text(Sha256::new_with_prefix(text.as_bytes())),
                );
            }
            ModelEvent::ToolCallStarted {
                index, native_id, ..
            } => {
                if native_id.len() > 256 {
                    self.invalidate();
                    return;
                }
                self.blocks.insert(
                    *index,
                    Block::Tool {
                        native_id: native_id.clone(),
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
                if let Some(Block::Tool { native_id, digest }) = self.blocks.get_mut(index) {
                    *digest = tool_digest(
                        native_id,
                        *tool_kind,
                        namespace.as_deref(),
                        name,
                        arguments,
                        None,
                    );
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
            if count > MAX_BLOCKS {
                return None;
            }
            match part {
                ContentPart::Text { text } => {
                    hash.update(b"text");
                    hash.update(text_digest(text, Some(replay))?);
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
                    )?);
                }
                _ => return None,
            }
        }
    }
    (count > 0).then(|| hash.finalize().into())
}

fn text_digest(text: &str, replay: Option<&ReplayStore>) -> Option<[u8; 32]> {
    let mut hash = Sha256::new();
    if let Some(reference) = text.content_ref() {
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
) -> Option<[u8; 32]> {
    let mut hash = Sha256::new_with_prefix(b"tool-boundary/v2");
    hash.update(text_digest(id, None)?);
    hash.update(match kind {
        ToolKindV1::Function => b"function".as_slice(),
        ToolKindV1::Custom => b"custom".as_slice(),
    });
    hash.update([u8::from(namespace.is_some())]);
    if let Some(namespace) = namespace {
        hash.update(text_digest(namespace, replay)?);
    }
    hash.update(text_digest(name, replay)?);
    // Canonical JSON includes arguments and their types, never only the tool name.
    if contains_reference(args) {
        return None;
    }
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
    write_canonical_json(&mut Sink(&mut hash), args).ok()?;
    Some(hash.finalize().into())
}

fn contains_reference(value: &serde_json::Value) -> bool {
    if value.content_ref().is_some() {
        return true;
    }
    match value {
        serde_json::Value::String(s) => s.content_ref().is_some(),
        serde_json::Value::Array(a) => a.iter().any(contains_reference),
        serde_json::Value::Object(o) => o.values().any(contains_reference),
        _ => false,
    }
}
