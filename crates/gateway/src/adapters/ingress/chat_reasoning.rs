//! Chat reasoning aliases share one canonical state; native copies remain untouched.

use std::io::{Read, Write};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::DecodeContext;
use crate::content_ref::JsonValueExt;
use crate::replay::ReplayStore;

pub(super) fn canonical_reasoning<'a>(
    context: &DecodeContext,
    object: &'a Map<String, Value>,
    role: &str,
) -> Option<&'a Value> {
    let content = object.get("reasoning_content");
    let Some(alias) = object.get("reasoning") else {
        return content;
    };
    // Unknown shapes and conflicting carriers remain native payload. Never
    // choose one and silently discard potentially different historical state.
    let alias_is_text = alias.is_string()
        || (alias.content_ref().is_some()
            && (content.is_some() || text_digest(alias, context.replay).is_some()));
    if role != "assistant"
        || !alias_is_text
        || content.is_some_and(|content| !same_text(content, alias, context.replay))
    {
        context.mark_native_only();
        return content;
    }
    content.or(Some(alias))
}

fn same_text(left: &Value, right: &Value, replay: Option<&ReplayStore>) -> bool {
    if let (Some(left), Some(right)) = (left.as_str(), right.as_str()) {
        return left == right;
    }
    // Replay locations are not content identities: equal strings can occupy
    // different ranges. Resolve their bytes with bounded memory before admitting
    // an alias. Failed or unavailable backing never establishes equivalence.
    match (text_digest(left, replay), text_digest(right, replay)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn text_digest(value: &Value, replay: Option<&ReplayStore>) -> Option<(u64, [u8; 32])> {
    let Some(reference) = value.content_ref() else {
        let mut writer = TextDigest::default();
        // Whole-JSON backing contains quotes and escapes. Stream inline text
        // in the same representation instead of materializing another string.
        serde_json::to_writer(&mut writer, value.as_str()?).ok()?;
        return Some((writer.bytes, writer.digest.finalize().into()));
    };
    let mut reader = replay?.reader(&reference).ok()?;
    // Only string backing produced by the ingress JSON owner is a text alias;
    // a whole-JSON reference can otherwise carry an opaque object or array.
    let mut first = [0];
    reader.read_exact(&mut first).ok()?;
    if first[0] != b'"' {
        return None;
    }
    let mut digest = Sha256::new();
    digest.update(first);
    let mut buffer = [0; 4096];
    loop {
        let read = reader.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    reader.verify_terminal().ok()?;
    Some((reference.byte_len(), digest.finalize().into()))
}

#[derive(Default)]
struct TextDigest {
    bytes: u64,
    digest: Sha256,
}

impl Write for TextDigest {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.digest.update(bytes);
        self.bytes += bytes.len() as u64;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
