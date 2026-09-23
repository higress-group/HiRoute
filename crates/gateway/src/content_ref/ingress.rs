use std::io::Read;

use hiroute_gateway_core::runtime::body::{MemoryRole, Reservation};
use serde_json::Value;

use crate::replay::{ReplayContentWriter, ReplayError, ReplayReader, ReplayStore};
use crate::server::request_plan::IngressProtocol;

use super::MARKER_PREFIX;

const MAX_JSON_DEPTH: usize = 128;
const INLINE_CONTENT_BYTES: usize = 8 * 1024;
const STRUCTURE_RESERVATION_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IngressDocumentStats {
    pub(crate) structure_units: usize,
    pub(crate) max_depth: usize,
}

impl IngressDocumentStats {
    pub(crate) fn reserve_workspace(
        self,
        replay: &ReplayStore,
        raw_bytes: u64,
    ) -> Result<Reservation, ReplayError> {
        let raw_bytes = usize::try_from(raw_bytes).map_err(|_| ReplayError::LengthOverflow)?;
        // One full Value may exist briefly during serde decode, and one field
        // may be copied into the encrypted writer while its old String is
        // dropped. Both transient owners, plus structural nodes, are charged
        // to the same stream budget that owns Replay and Attempt buffers.
        let bytes = raw_bytes
            .checked_mul(2)
            .and_then(|bytes| {
                self.structure_units
                    .checked_mul(STRUCTURE_RESERVATION_BYTES)
                    .and_then(|structure| bytes.checked_add(structure))
            })
            .ok_or(ReplayError::LengthOverflow)?;
        Ok(replay.budget().reserve(MemoryRole::ModelIrBacking, bytes)?)
    }
}

/// Performs a bounded, allocation-free structural pass over the authenticated
/// raw stream before serde or any content pool/file is created. This is not a
/// second parser; serde remains the syntax authority. It is the pre-allocation
/// DoS gate and also forces terminal length verification of the raw backing.
pub(crate) fn scan_ingress_document(
    mut reader: ReplayReader,
) -> Result<IngressDocumentStats, ReplayError> {
    let mut scanner = StructuralScanner::default();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        scanner.feed(&buffer[..read])?;
    }
    reader.verify_terminal()?;
    scanner.finish()
}

/// Replaces only MVP protocol fields that may carry large semantic content.
/// The surrounding JSON structure remains a normal serde Value so existing
/// protocol validation stays authoritative, while decode/Planner only see
/// compact request-local markers. This boundary accepts client JSON, so even
/// text matching an existing locator must be externalized as literal content.
pub(crate) fn compact_ingress_document(
    protocol: IngressProtocol,
    document: &mut Value,
    replay: &ReplayStore,
) -> Result<(), ReplayError> {
    let mut compactor = Compactor {
        protocol,
        replay,
        pool: None,
        inline_remaining: INLINE_CONTENT_BYTES,
    };
    compactor.walk(document, None, 0)?;
    if let Some(pool) = compactor.pool {
        pool.seal()?;
    }
    Ok(())
}

struct Compactor<'a> {
    protocol: IngressProtocol,
    replay: &'a ReplayStore,
    pool: Option<ReplayContentWriter>,
    inline_remaining: usize,
}

impl Compactor<'_> {
    fn walk(
        &mut self,
        value: &mut Value,
        field: Option<&str>,
        depth: usize,
    ) -> Result<(), ReplayError> {
        if depth > MAX_JSON_DEPTH {
            return Err(ReplayError::StructureLimit);
        }
        if field.is_some_and(|field| whole_json_field(self.protocol, field, depth, value)) {
            return self.externalize_json(value);
        }
        match value {
            Value::String(text) => {
                if field.is_some_and(content_string_field) {
                    self.externalize_string(text)
                } else {
                    Ok(())
                }
            }
            Value::Array(values) => values
                .iter_mut()
                .try_for_each(|value| self.walk(value, field, depth + 1)),
            Value::Object(object) => {
                let item_type = object
                    .get("type")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let handled_image_field =
                    self.compact_image_source(object, item_type.as_deref())?;
                for (key, value) in object.iter_mut() {
                    if handled_image_field == Some(key.as_str()) {
                        continue;
                    }
                    let externalize_whole = (item_type.as_deref() == Some("tool_use")
                        && key == "input")
                        || (item_type.as_deref() == Some("tool_result")
                            && key == "content"
                            && !value.is_string())
                        || (item_type.as_deref() == Some("function_call_output")
                            && key == "output"
                            && !value.is_string());
                    if externalize_whole {
                        self.externalize_json(value)?;
                    } else {
                        self.walk(value, Some(key), depth + 1)?;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Preserve the semantic discriminator before replacing image bytes with
    /// a request-local marker. Responses and Chat encode both URL and Base64
    /// forms in one string, while Messages carries an explicit source type.
    fn compact_image_source(
        &mut self,
        object: &mut serde_json::Map<String, Value>,
        item_type: Option<&str>,
    ) -> Result<Option<&'static str>, ReplayError> {
        match (self.protocol, item_type) {
            (IngressProtocol::Responses, Some("input_image")) => {
                if let Some(Value::String(image_url)) = object.get_mut("image_url") {
                    self.externalize_image_url(image_url)?;
                }
                Ok(Some("image_url"))
            }
            (IngressProtocol::ChatCompletions, Some("image_url")) => {
                if let Some(source) = object.get_mut("image_url").and_then(Value::as_object_mut)
                    && let Some(Value::String(url)) = source.get_mut("url")
                {
                    self.externalize_image_url(url)?;
                }
                Ok(Some("image_url"))
            }
            (IngressProtocol::Messages, Some("image")) => {
                if let Some(source) = object.get_mut("source").and_then(Value::as_object_mut) {
                    match source.get("type").and_then(Value::as_str) {
                        Some("url") => {
                            if let Some(Value::String(url)) = source.get_mut("url") {
                                self.externalize_image_url(url)?;
                            }
                        }
                        Some("base64") => {
                            if let Some(Value::String(data)) = source.get_mut("data") {
                                self.externalize_string(data)?;
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Some("source"))
            }
            _ => Ok(None),
        }
    }

    fn externalize_image_url(&mut self, value: &mut String) -> Result<(), ReplayError> {
        if let Some(rest) = value.strip_prefix("data:") {
            let Some((media_type, data)) = rest.split_once(";base64,") else {
                // Leave malformed data URIs intact for the protocol decoder to
                // reject with its existing client-facing semantic error.
                return Ok(());
            };
            if media_type.is_empty() || data.is_empty() {
                return Ok(());
            }
            let payload_start = value
                .len()
                .checked_sub(data.len())
                .ok_or(ReplayError::LengthOverflow)?;
            return self.externalize_string_suffix(value, payload_start);
        }
        if value.starts_with("https://") || value.starts_with("http://") {
            return self.externalize_string(value);
        }
        // Unsupported schemes remain visible to the protocol decoder. They
        // must never become an opaque URL marker that bypasses validation.
        Ok(())
    }

    fn externalize_string_suffix(
        &mut self,
        value: &mut String,
        payload_start: usize,
    ) -> Result<(), ReplayError> {
        let payload = value.get(payload_start..).ok_or(ReplayError::InvalidUtf8)?;
        if payload.len() <= self.inline_remaining && !payload.contains(MARKER_PREFIX) {
            self.inline_remaining -= payload.len();
            return Ok(());
        }
        let reference = self.pool()?.append(payload.as_bytes())?;
        value.truncate(payload_start);
        value.push_str(&reference.wire_marker());
        Ok(())
    }

    fn externalize_string(&mut self, value: &mut String) -> Result<(), ReplayError> {
        if value.len() <= self.inline_remaining && !value.contains(MARKER_PREFIX) {
            self.inline_remaining -= value.len();
            return Ok(());
        }
        let reference = self.pool()?.append(value.as_bytes())?;
        *value = reference.wire_marker();
        Ok(())
    }

    fn externalize_json(&mut self, value: &mut Value) -> Result<(), ReplayError> {
        let reference = self.pool()?.append_json(value, false)?;
        *value = reference.json_marker();
        Ok(())
    }

    fn pool(&mut self) -> Result<&mut ReplayContentWriter, ReplayError> {
        if self.pool.is_none() {
            self.pool = Some(self.replay.begin_content_pool()?);
        }
        self.pool.as_mut().ok_or(ReplayError::Integrity)
    }
}

fn content_string_field(field: &str) -> bool {
    matches!(
        field,
        "arguments"
            | "content"
            | "data"
            | "description"
            | "input"
            | "instructions"
            | "output"
            | "system"
            | "text"
            | "thinking"
            | "signature"
            | "encrypted_content"
    )
}

fn whole_json_field(protocol: IngressProtocol, field: &str, depth: usize, value: &Value) -> bool {
    matches!(field, "input_schema" | "parameters")
        || (depth == 1
            && matches!(
                field,
                "conversation"
                    | "output_config"
                    | "previous_response_id"
                    | "thinking"
            ))
        // Responses reasoning currently has only the bounded `effort` and
        // `summary` controls. Keep that small object visible so ingress can
        // validate and retain summary before the sealed Plan replaces effort.
        // Other protocols still use the request-local JSON backing for their
        // larger native reasoning shapes.
        || (depth == 1 && field == "reasoning" && protocol != IngressProtocol::Responses)
        || (field == "reasoning_content" && !value.is_null())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_gateway_core::runtime::body::BudgetTree;
    use serde_json::json;

    #[test]
    fn long_control_strings_are_preserved_without_a_generic_length_limit() {
        let root = std::env::temp_dir().join(format!(
            "hiroute-long-control-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
            root: root.clone(),
            memory_threshold_bytes: 128,
            record_bytes: 31,
            orphan_ttl: std::time::Duration::from_secs(60),
        })
        .unwrap();
        let original = "control 中文 ".repeat(4096);
        for protocol in [
            IngressProtocol::Responses,
            IngressProtocol::Messages,
            IngressProtocol::ChatCompletions,
        ] {
            let tree = BudgetTree::new(1024 * 1024, 1024 * 1024).unwrap();
            let store = manager
                .begin_request(tree.stream(1024 * 1024).unwrap())
                .unwrap();
            let mut document = json!({"model":original,"metadata":{"extension":original}});
            let expected = document.clone();
            compact_ingress_document(protocol, &mut document, &store).unwrap();
            assert_eq!(document, expected);
        }
        drop(manager);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn long_native_reasoning_history_is_content_not_control() {
        let root = std::env::temp_dir().join(format!(
            "hiroute-long-reasoning-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
            root: root.clone(),
            memory_threshold_bytes: 128,
            record_bytes: 31,
            orphan_ttl: std::time::Duration::from_secs(60),
        })
        .unwrap();
        for (protocol, field) in [
            (IngressProtocol::Messages, "thinking"),
            (IngressProtocol::Messages, "signature"),
            (IngressProtocol::Responses, "encrypted_content"),
        ] {
            let tree = BudgetTree::new(1024 * 1024, 1024 * 1024).unwrap();
            let store = manager
                .begin_request(tree.stream(1024 * 1024).unwrap())
                .unwrap();
            let original = "reasoning 中文 ".repeat(1024);
            let mut document = json!({"model":"alias", "history":[{field:original}]});
            compact_ingress_document(protocol, &mut document, &store).unwrap();
            let marker = document["history"][0][field].as_str().unwrap();
            let reference = crate::content_ref::ContentRef::from_wire_marker(marker).unwrap();
            let mut actual = String::new();
            store
                .reader(&reference)
                .unwrap()
                .read_to_string(&mut actual)
                .unwrap();
            assert_eq!(actual, original);
        }
        drop(manager);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn responses_reasoning_stays_visible_for_typed_summary_decode() {
        let root = std::env::temp_dir().join(format!(
            "hiroute-responses-reasoning-compaction-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
            root: root.clone(),
            memory_threshold_bytes: 128,
            record_bytes: 31,
            orphan_ttl: std::time::Duration::from_secs(60),
        })
        .expect("replay manager");
        let tree = BudgetTree::new(1024 * 1024, 1024 * 1024).expect("budget tree");
        let budget = tree.stream(1024 * 1024).expect("stream budget");
        let store = manager.begin_request(budget).expect("replay store");
        let reasoning = json!({"effort":"low","summary":"auto"});
        let mut document = json!({
            "model":"alias",
            "input":"hello",
            "reasoning":reasoning,
        });

        compact_ingress_document(IngressProtocol::Responses, &mut document, &store)
            .expect("compact Responses document");
        assert_eq!(document["reasoning"], reasoning);
        let request = crate::server::core_runtime::adapters::decode_ingress_request(
            IngressProtocol::Responses,
            &document,
        )
        .expect("typed Responses decode");
        assert_eq!(
            request
                .responses_options
                .and_then(|options| options.reasoning_summary),
            Some("auto".into())
        );

        drop(store);
        drop(manager);
        std::fs::remove_dir_all(root).expect("remove replay root");
    }
}

#[derive(Default)]
struct StructuralScanner {
    structure_units: usize,
    depth: usize,
    max_depth: usize,
    in_string: bool,
    escaped: bool,
}

impl StructuralScanner {
    fn feed(&mut self, bytes: &[u8]) -> Result<(), ReplayError> {
        for &byte in bytes {
            if self.in_string {
                if self.escaped {
                    self.escaped = false;
                } else if byte == b'\\' {
                    self.escaped = true;
                } else if byte == b'"' {
                    self.in_string = false;
                }
                continue;
            }
            match byte {
                b'"' => self.in_string = true,
                b'{' | b'[' => {
                    self.depth = self
                        .depth
                        .checked_add(1)
                        .ok_or(ReplayError::StructureLimit)?;
                    self.max_depth = self.max_depth.max(self.depth);
                    self.bump()?;
                    if self.depth > MAX_JSON_DEPTH {
                        return Err(ReplayError::StructureLimit);
                    }
                }
                b'}' | b']' => {
                    self.depth = self
                        .depth
                        .checked_sub(1)
                        .ok_or(ReplayError::StructureLimit)?;
                }
                b':' | b',' => self.bump()?,
                _ => {}
            }
        }
        Ok(())
    }

    fn bump(&mut self) -> Result<(), ReplayError> {
        self.structure_units = self
            .structure_units
            .checked_add(1)
            .ok_or(ReplayError::StructureLimit)?;
        Ok(())
    }

    fn finish(self) -> Result<IngressDocumentStats, ReplayError> {
        if self.in_string || self.escaped || self.depth != 0 {
            return Err(ReplayError::Integrity);
        }
        Ok(IngressDocumentStats {
            structure_units: self.structure_units,
            max_depth: self.max_depth,
        })
    }
}
