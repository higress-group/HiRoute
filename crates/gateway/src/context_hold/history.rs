use std::io::{Read, Write};

use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

use crate::content_ref::{ContentRef, ContentValueExt, JsonValueExt};
use crate::replay::{ReplayStore, write_canonical_json};
use crate::server::core_runtime::model_ir::{
    CanonicalMessage, ContentPart, ImageSource, InstructionRole, MessageRole, ModelRequestIRV1,
    OpaqueProviderState, ResponsesReasoningEncryptedContentV1, ResponsesToolOrderEntryV1,
    ToolChoice, ToolKindV1, ToolOutput, ToolResultStatusV1, UrlCitationKind, UrlCitationV1,
    WebSearchAction, WebSearchCallV1, WebSearchSourceKind, WebSearchStatus,
};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub(crate) struct HistoryEvidence<'a> {
    pub(crate) instruction_digest: [u8; 32],
    request: &'a ModelRequestIRV1,
    key: &'a [u8; 32],
    replay: Option<&'a ReplayStore>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct HistoryDigests {
    pub(crate) message_count: usize,
    pub(crate) prefix_digest: Option<[u8; 32]>,
    pub(crate) complete_digest: [u8; 32],
}

impl HistoryEvidence<'_> {
    #[cfg(test)]
    pub(crate) fn message_count(&self) -> usize {
        self.request.messages.len()
    }

    /// Hashes the requested stored prefix and the complete visible history in
    /// one streaming pass. No per-message digest array survives the call.
    pub(crate) fn measure(&self, prefix_count: Option<usize>) -> Option<HistoryDigests> {
        let mut history = Encoder::new(self.key, b"visible-history/v1/messages", self.replay)?;
        let mut prefix_digest = match prefix_count {
            Some(0) => Some(history.snapshot()?),
            _ => None,
        };
        for (index, message) in self.request.messages.iter().enumerate() {
            history.history_item(self.request, index, message)?;
            if prefix_count == Some(index.saturating_add(1)) {
                prefix_digest = Some(history.snapshot()?);
            }
        }
        Some(HistoryDigests {
            message_count: self.request.messages.len(),
            prefix_digest,
            complete_digest: history.finish()?,
        })
    }

    #[cfg(test)]
    fn complete_digest(&self) -> [u8; 32] {
        self.measure(None)
            .expect("history remains encodable")
            .complete_digest
    }

    #[cfg(test)]
    fn digest_at(&self, message_count: usize) -> Option<[u8; 32]> {
        self.measure(Some(message_count))?.prefix_digest
    }
}

#[cfg(test)]
pub(crate) fn visible_history<'a>(
    request: &'a ModelRequestIRV1,
    key: &'a [u8; 32],
) -> Option<HistoryEvidence<'a>> {
    visible_history_with_replay(request, key, None)
}

pub(crate) fn visible_history_with_replay<'a>(
    request: &'a ModelRequestIRV1,
    key: &'a [u8; 32],
    replay: Option<&'a ReplayStore>,
) -> Option<HistoryEvidence<'a>> {
    let mut instructions = Encoder::new(key, b"visible-history/v1/instructions", replay)?;
    instructions.usize(request.instructions.len());
    for instruction in &request.instructions {
        instructions.tag(match instruction.role {
            InstructionRole::System => b"system",
            InstructionRole::Developer => b"developer",
        });
        instructions.parts(&instruction.content)?;
    }
    // Tool declarations and stable provider state are part of the effective
    // model setup. Treating their change as a rebuild is conservative for KV
    // affinity and avoids claiming equivalence across transport rewrites.
    instructions.usize(request.tools.len());
    for tool in &request.tools {
        instructions.tag(match tool.kind {
            ToolKindV1::Function => b"function",
            ToolKindV1::Custom => b"custom",
        });
        instructions.string(&tool.name);
        instructions.optional_string(tool.description.as_deref());
        match &tool.input_schema {
            Some(value) => {
                instructions.tag(b"input-schema");
                instructions.json(value)?;
            }
            None => instructions.tag(b"no-input-schema"),
        }
        instructions.optional_bool(tool.strict);
        match &tool.format {
            Some(value) => {
                instructions.tag(b"custom-format");
                instructions.json(value)?;
            }
            None => instructions.tag(b"no-custom-format"),
        }
    }
    instructions.usize(request.tool_namespaces.len());
    for namespace in &request.tool_namespaces {
        instructions.string(&namespace.name);
        instructions.optional_string(namespace.description.as_deref());
        instructions.usize(namespace.tools.len());
        for tool in &namespace.tools {
            instructions.tag(match tool.kind {
                ToolKindV1::Function => b"function",
                ToolKindV1::Custom => b"custom",
            });
            instructions.string(&tool.name);
            instructions.optional_string(tool.description.as_deref());
            match &tool.input_schema {
                Some(value) => {
                    instructions.tag(b"input-schema");
                    instructions.json(value)?;
                }
                None => instructions.tag(b"no-input-schema"),
            }
            instructions.optional_bool(tool.strict);
            match &tool.format {
                Some(value) => {
                    instructions.tag(b"custom-format");
                    instructions.json(value)?;
                }
                None => instructions.tag(b"no-custom-format"),
            }
        }
    }
    instructions.usize(request.responses_tool_order.len());
    for entry in &request.responses_tool_order {
        match entry {
            ResponsesToolOrderEntryV1::Tool { index } => {
                instructions.tag(b"responses-flat-tool");
                instructions.u64(u64::from(*index));
            }
            ResponsesToolOrderEntryV1::Namespace { index } => {
                instructions.tag(b"responses-namespace");
                instructions.u64(u64::from(*index));
            }
            ResponsesToolOrderEntryV1::WebSearch => instructions.tag(b"responses-web-search"),
        }
    }
    match &request.tool_choice {
        ToolChoice::None => instructions.tag(b"tool-choice-none"),
        ToolChoice::Auto => instructions.tag(b"tool-choice-auto"),
        ToolChoice::RequiredAny => instructions.tag(b"tool-choice-required-any"),
        ToolChoice::RequiredNamed { tool_kind, name } => {
            instructions.tag(b"tool-choice-required-named");
            instructions.tag(match tool_kind {
                ToolKindV1::Function => b"function",
                ToolKindV1::Custom => b"custom",
            });
            instructions.string(name);
        }
    }
    instructions.bool(request.parallel_tool_calls);
    // The sealed Agent Plan owns reasoning effort/budget. The typed Responses
    // options below retain and hash the supported summary sibling.
    match &request.responses_options {
        Some(options) => {
            instructions.tag(b"responses-options");
            instructions.json(&serde_json::to_value(options).ok()?)?;
        }
        None => instructions.tag(b"no-responses-options"),
    }
    instructions.usize(request.provider_state.len());
    for state in &request.provider_state {
        instructions.provider_state(state)?;
    }
    match &request.web_search {
        Some(search) => {
            instructions.tag(b"web-search-tool");
            instructions.optional_bool(search.external_web_access);
        }
        None => instructions.tag(b"no-web-search-tool"),
    }
    let instruction_digest = instructions.finish()?;
    Some(HistoryEvidence {
        instruction_digest,
        request,
        key,
        replay,
    })
}

struct Encoder<'a> {
    mac: HmacSha256,
    replay: Option<&'a ReplayStore>,
    valid: bool,
}

impl<'a> Encoder<'a> {
    fn new(key: &[u8; 32], domain: &[u8], replay: Option<&'a ReplayStore>) -> Option<Self> {
        let mut mac = HmacSha256::new_from_slice(key).ok()?;
        frame(&mut mac, b"domain", domain);
        Some(Self {
            mac,
            replay,
            valid: true,
        })
    }

    fn finish(self) -> Option<[u8; 32]> {
        self.valid.then(|| self.mac.finalize().into_bytes().into())
    }

    fn snapshot(&self) -> Option<[u8; 32]> {
        self.valid
            .then(|| self.mac.clone().finalize().into_bytes().into())
    }

    fn tag(&mut self, value: &[u8]) {
        frame(&mut self.mac, b"tag", value);
    }

    fn bytes(&mut self, value: &[u8]) {
        frame(&mut self.mac, b"bytes", value);
    }

    fn string(&mut self, value: &str) {
        self.tag(b"utf8");
        if let Some(reference) = value.content_ref() {
            self.valid &= self.hash_reference(&reference).is_some();
        } else {
            self.bytes(value.as_bytes());
        }
    }

    fn optional_string(&mut self, value: Option<&str>) {
        match value {
            Some(value) => {
                self.tag(b"some");
                self.string(value);
            }
            None => self.tag(b"none"),
        }
    }

    fn optional_bool(&mut self, value: Option<bool>) {
        match value {
            Some(value) => {
                self.tag(b"some");
                self.bool(value);
            }
            None => self.tag(b"none"),
        }
    }

    fn bool(&mut self, value: bool) {
        self.tag(if value { b"true" } else { b"false" });
    }

    fn usize(&mut self, value: usize) {
        self.u64(u64::try_from(value).unwrap_or(u64::MAX));
    }

    fn u64(&mut self, value: u64) {
        frame(&mut self.mac, b"u64", &value.to_be_bytes());
    }

    fn json(&mut self, value: &Value) -> Option<()> {
        self.tag(b"json");
        if let Some(reference) = value.content_ref() {
            return self.hash_reference(&reference);
        }
        let mut counter = ByteCounter(0);
        if contains_content_ref(value) {
            write_canonical_json_with_replay(&mut counter, value, self.replay?)?;
        } else {
            write_canonical_json(&mut counter, value).ok()?;
        }
        frame_header(&mut self.mac, b"bytes", counter.0);
        if contains_content_ref(value) {
            write_canonical_json_with_replay(&mut MacWriter(&mut self.mac), value, self.replay?)?;
        } else {
            write_canonical_json(&mut MacWriter(&mut self.mac), value).ok()?;
        }
        Some(())
    }

    fn hash_reference(&mut self, reference: &ContentRef) -> Option<()> {
        let mut reader = self.replay?.reader(reference).ok()?;
        frame_header(&mut self.mac, b"bytes", reference.byte_len());
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let size = reader.read(&mut buffer).ok()?;
            if size == 0 {
                break;
            }
            self.mac.update(&buffer[..size]);
        }
        reader.verify_terminal().ok()
    }

    fn parts(&mut self, parts: &[ContentPart]) -> Option<()> {
        self.usize(parts.len());
        for part in parts {
            match part {
                ContentPart::Text { text } => {
                    self.tag(b"text");
                    self.string(text);
                }
                ContentPart::Image {
                    source: ImageSource::Url { url },
                } => {
                    self.tag(b"image-url");
                    self.string(url);
                }
                ContentPart::Image {
                    source: ImageSource::Base64 { media_type, data },
                } => {
                    self.tag(b"image-base64");
                    self.bytes(media_type.as_bytes());
                    self.string(data);
                }
                ContentPart::ToolCall {
                    logical_id,
                    tool_kind,
                    namespace,
                    name,
                    arguments,
                    raw_arguments,
                } => {
                    self.tag(b"tool-call");
                    self.tag(match tool_kind {
                        ToolKindV1::Function => b"function",
                        ToolKindV1::Custom => b"custom",
                    });
                    self.bytes(logical_id.as_bytes());
                    self.optional_string(namespace.as_deref());
                    self.string(name);
                    if let Some(raw) = raw_arguments {
                        self.tag(b"raw-arguments");
                        self.string(raw);
                    } else {
                        self.json(arguments)?;
                    }
                }
                ContentPart::ToolResult {
                    logical_id,
                    tool_kind,
                    output,
                    status,
                } => {
                    self.tag(b"tool-result");
                    self.tag(match tool_kind {
                        ToolKindV1::Function => b"function",
                        ToolKindV1::Custom => b"custom",
                    });
                    self.bytes(logical_id.as_bytes());
                    self.tag(match status {
                        ToolResultStatusV1::Completed => b"completed",
                        ToolResultStatusV1::Failed => b"failed",
                        ToolResultStatusV1::Unknown => b"unknown",
                    });
                    match output {
                        ToolOutput::Text(value) => {
                            self.tag(b"text");
                            self.string(value);
                        }
                        ToolOutput::Json(value) => {
                            self.tag(b"json");
                            self.json(value)?;
                        }
                    }
                }
                ContentPart::ProviderState { state } => {
                    self.tag(b"provider-state");
                    self.provider_state(state)?;
                }
            }
        }
        Some(())
    }

    fn message(&mut self, message: &CanonicalMessage) -> Option<()> {
        self.tag(b"message");
        self.tag(match message.role {
            MessageRole::User => b"user",
            MessageRole::Assistant => b"assistant",
            MessageRole::System => b"system",
            MessageRole::Developer => b"developer",
        });
        self.optional_string(message.name.as_deref());
        self.parts(&message.content)
    }

    fn history_item(
        &mut self,
        request: &ModelRequestIRV1,
        message_index: usize,
        message: &CanonicalMessage,
    ) -> Option<()> {
        self.tag(b"history-item");
        if let Some(search) = request.responses_search_history.get(&message_index) {
            self.web_search_call(search);
        } else {
            self.message(message)?;
        }
        match request.responses_annotations.get(&message_index) {
            Some(parts) => {
                self.tag(b"responses-annotations");
                self.usize(parts.len());
                for (part_index, annotations) in parts {
                    self.usize(*part_index);
                    self.usize(annotations.len());
                    for annotation in annotations {
                        self.url_citation(annotation);
                    }
                }
            }
            None => self.tag(b"no-responses-annotations"),
        }
        match request.responses_reasoning_history.get(&message_index) {
            Some(history) => {
                self.tag(b"responses-reasoning-history");
                // Native fields may each be Replay-backed (notably a long
                // Responses summary). Hash the sorted fields individually so
                // the request-local JSON markers never stand in for content.
                self.usize(history.native_fields.len());
                let mut keys = history.native_fields.keys().collect::<Vec<_>>();
                keys.sort_unstable();
                for key in keys {
                    self.bytes(key.as_bytes());
                    self.json(&history.native_fields[key])?;
                }
                self.tag(match history.encrypted_content {
                    ResponsesReasoningEncryptedContentV1::Opaque => b"opaque",
                    ResponsesReasoningEncryptedContentV1::Absent => b"absent",
                    ResponsesReasoningEncryptedContentV1::Null => b"null",
                    ResponsesReasoningEncryptedContentV1::Empty => b"empty",
                });
            }
            None => self.tag(b"no-responses-reasoning-history"),
        }
        Some(())
    }

    fn web_search_call(&mut self, call: &WebSearchCallV1) {
        self.tag(b"responses-web-search-call");
        self.string(&call.id);
        self.tag(match call.status {
            WebSearchStatus::InProgress => b"in-progress",
            WebSearchStatus::Searching => b"searching",
            WebSearchStatus::Completed => b"completed",
            WebSearchStatus::Failed => b"failed",
        });
        match &call.action {
            Some(WebSearchAction::Search {
                query,
                queries,
                sources,
            }) => {
                self.tag(b"search");
                self.optional_string(query.as_deref());
                self.optional_strings(queries.as_deref());
                match sources {
                    Some(sources) => {
                        self.tag(b"some-sources");
                        self.usize(sources.len());
                        for source in sources {
                            self.tag(match &source.kind {
                                WebSearchSourceKind::Url => b"url",
                            });
                            self.string(&source.url);
                        }
                    }
                    None => self.tag(b"no-sources"),
                }
            }
            Some(WebSearchAction::OpenPage { url }) => {
                self.tag(b"open-page");
                self.string(url);
            }
            Some(WebSearchAction::Find { url, pattern }) => {
                self.tag(b"find");
                self.string(url);
                self.string(pattern);
            }
            None => self.tag(b"no-action"),
        }
    }

    fn optional_strings(&mut self, values: Option<&[String]>) {
        match values {
            Some(values) => {
                self.tag(b"some-strings");
                self.usize(values.len());
                for value in values {
                    self.string(value);
                }
            }
            None => self.tag(b"no-strings"),
        }
    }

    fn url_citation(&mut self, citation: &UrlCitationV1) {
        self.tag(match &citation.kind {
            UrlCitationKind::UrlCitation => b"url-citation",
        });
        self.u64(u64::from(citation.start_index));
        self.u64(u64::from(citation.end_index));
        self.string(&citation.title);
        self.string(&citation.url);
    }

    fn provider_state(&mut self, state: &OpaqueProviderState) -> Option<()> {
        let owner = &state.owner;
        for value in [
            owner.provider_id.as_str(),
            owner.endpoint_id.as_str(),
            owner.entitlement_id.as_str(),
            owner.connector_id.as_str(),
            owner.connector_revision.as_str(),
            owner.capability_id.as_str(),
            owner.capability_revision.as_str(),
            owner.model_configuration_id.as_str(),
            owner.native_model.as_str(),
            owner.adapter_revision.as_str(),
            owner.serializer_revision.as_str(),
            owner.decoder_revision.as_str(),
        ] {
            self.bytes(value.as_bytes());
        }
        self.tag(format!("{:?}", owner.upstream_protocol).as_bytes());
        match state.block_index {
            Some(index) => {
                self.tag(b"some-block");
                self.u64(u64::from(index));
            }
            None => self.tag(b"no-block"),
        }
        self.bytes(state.kind.as_bytes());
        self.optional_string(state.messages_thinking.as_deref());
        self.json(&state.value)
    }
}

fn frame(mac: &mut HmacSha256, kind: &[u8], value: &[u8]) {
    frame_header(mac, kind, value.len() as u64);
    mac.update(value);
}

fn frame_header(mac: &mut HmacSha256, kind: &[u8], length: u64) {
    mac.update(&(kind.len() as u64).to_be_bytes());
    mac.update(kind);
    mac.update(&length.to_be_bytes());
}

fn contains_content_ref(value: &Value) -> bool {
    if value.content_ref().is_some() {
        return true;
    }
    match value {
        Value::String(value) => value.content_ref().is_some(),
        Value::Array(values) => values.iter().any(contains_content_ref),
        Value::Object(values) => values.values().any(contains_content_ref),
        _ => false,
    }
}

fn write_canonical_json_with_replay(
    writer: &mut impl Write,
    value: &Value,
    replay: &ReplayStore,
) -> Option<()> {
    if let Some(reference) = value.content_ref() {
        let mut reader = replay.reader(&reference).ok()?;
        std::io::copy(&mut reader, writer).ok()?;
        reader.verify_terminal().ok()?;
        return Some(());
    }
    match value {
        Value::String(value) if value.content_ref().is_some() => {
            let reference = value.content_ref()?;
            let mut reader = replay.reader(&reference).ok()?;
            writer.write_all(b"\"").ok()?;
            let mut buffer = [0_u8; 16 * 1024];
            loop {
                let count = reader.read(&mut buffer).ok()?;
                if count == 0 {
                    break;
                }
                let mut start = 0;
                for (index, &byte) in buffer[..count].iter().enumerate() {
                    let escape = match byte {
                        b'"' => Some(b"\\\"".as_slice()),
                        b'\\' => Some(b"\\\\".as_slice()),
                        b'\x08' => Some(b"\\b".as_slice()),
                        b'\x0c' => Some(b"\\f".as_slice()),
                        b'\n' => Some(b"\\n".as_slice()),
                        b'\r' => Some(b"\\r".as_slice()),
                        b'\t' => Some(b"\\t".as_slice()),
                        0x00..=0x1f => None,
                        _ => continue,
                    };
                    writer.write_all(&buffer[start..index]).ok()?;
                    if let Some(escape) = escape {
                        writer.write_all(escape).ok()?;
                    } else {
                        const HEX: &[u8; 16] = b"0123456789abcdef";
                        writer
                            .write_all(&[
                                b'\\',
                                b'u',
                                b'0',
                                b'0',
                                HEX[(byte >> 4) as usize],
                                HEX[(byte & 0xf) as usize],
                            ])
                            .ok()?;
                    }
                    start = index + 1;
                }
                writer.write_all(&buffer[start..count]).ok()?;
            }
            reader.verify_terminal().ok()?;
            writer.write_all(b"\"").ok()?;
        }
        Value::Array(values) => {
            writer.write_all(b"[").ok()?;
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    writer.write_all(b",").ok()?;
                }
                write_canonical_json_with_replay(writer, value, replay)?;
            }
            writer.write_all(b"]").ok()?;
        }
        Value::Object(values) => {
            writer.write_all(b"{").ok()?;
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    writer.write_all(b",").ok()?;
                }
                serde_json::to_writer(&mut *writer, key).ok()?;
                writer.write_all(b":").ok()?;
                write_canonical_json_with_replay(writer, &values[key], replay)?;
            }
            writer.write_all(b"}").ok()?;
        }
        _ => write_canonical_json(writer, value).ok()?,
    }
    Some(())
}

struct ByteCounter(u64);

impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| std::io::Error::other("JSON length overflow"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct MacWriter<'a>(&'a mut HmacSha256);

impl Write for MacWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::core_runtime::model_ir::{
        MODEL_REQUEST_IR_SCHEMA, RequestedReasoningControl,
    };
    use crate::server::request_plan::IngressProtocol;

    fn request(messages: Vec<CanonicalMessage>) -> ModelRequestIRV1 {
        ModelRequestIRV1 {
            schema_version: MODEL_REQUEST_IR_SCHEMA.into(),
            ingress_protocol: IngressProtocol::Responses,
            served_model_id: "agent/test".into(),
            stream: false,
            instructions: Vec::new(),
            messages,
            tools: Vec::new(),
            tool_namespaces: Vec::new(),
            responses_tool_order: Vec::new(),
            web_search: None,
            responses_search_history: Default::default(),
            responses_annotations: Default::default(),
            responses_message_phases: Default::default(),
            responses_internal_chat_message_metadata: Default::default(),
            responses_reasoning_history: Default::default(),
            tool_choice: ToolChoice::None,
            parallel_tool_calls: false,
            requested_reasoning: RequestedReasoningControl::absent(),
            requested_max_output_tokens: None,
            provider_state: Vec::new(),
            responses_options: None,
            responses_item_ids: Default::default(),
            responses_item_statuses: Default::default(),
        }
    }

    fn text(role: MessageRole, value: impl Into<String>) -> CanonicalMessage {
        CanonicalMessage {
            role,
            content: vec![ContentPart::Text { text: value.into() }],
            name: None,
        }
    }

    #[test]
    fn full_tail_and_message_order_affect_prefix_without_truncation() {
        let key = [7; 32];
        let common = "x".repeat(8_192);
        let one_request = request(vec![
            text(MessageRole::User, common.clone() + "a"),
            text(MessageRole::Assistant, "answer"),
        ]);
        let two_request = request(vec![
            text(MessageRole::User, common + "b"),
            text(MessageRole::Assistant, "answer"),
        ]);
        let one = visible_history(&one_request, &key).unwrap();
        let two = visible_history(&two_request, &key).unwrap();
        assert_ne!(one.digest_at(1), two.digest_at(1));
        assert_ne!(one.complete_digest(), two.complete_digest());

        let appended_request = request(vec![
            text(MessageRole::User, "first"),
            text(MessageRole::Assistant, "second"),
        ]);
        let prefix_request = request(vec![text(MessageRole::User, "first")]);
        let appended = visible_history(&appended_request, &key).unwrap();
        let prefix = visible_history(&prefix_request, &key).unwrap();
        assert_eq!(appended.digest_at(1), Some(prefix.complete_digest()));
    }

    #[test]
    fn content_refs_disable_history_equivalence_without_hashing_replay_bytes() {
        let key = [8; 32];
        let marker = crate::content_ref::ContentRef::new(1, 0, 5, 5).wire_marker();
        let request = request(vec![text(MessageRole::User, marker)]);
        let history = visible_history(&request, &key).expect("instruction history");
        assert!(
            history.measure(None).is_none(),
            "a request-local locator cannot prove cross-request content equivalence"
        );
    }

    #[test]
    fn signed_messages_summary_changes_history_even_with_the_same_ciphertext() {
        use crate::replay::{ReplayConfig, ReplayManager};
        use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
        use hiroute_gateway_core::runtime::body::BudgetTree;

        let owner = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Messages,
            IngressProtocol::Responses,
            "luna",
            fixed_reasoning("fixed"),
        )
        .exact_provider_path()
        .unwrap();
        let make_request = |summary: String| {
            let mut value = request(vec![
                CanonicalMessage {
                    role: MessageRole::Assistant,
                    content: vec![ContentPart::ProviderState {
                        state: Box::new(OpaqueProviderState {
                            owner: owner.clone(),
                            block_index: Some(0),
                            kind: "encrypted_content".into(),
                            value: serde_json::json!("same-signature"),
                            messages_thinking: Some(summary),
                        }),
                    }],
                    name: None,
                },
                text(MessageRole::User, "continue"),
            ]);
            value.ingress_protocol = IngressProtocol::Messages;
            value
        };
        let key = [33; 32];
        let original = "long summary 中文 ".repeat(1000);
        let changed = original.replace("中文", "更新");
        let original_request = make_request(original.clone());
        let changed_request = make_request(changed.clone());
        let original_history = visible_history(&original_request, &key).unwrap();
        let changed_history = visible_history(&changed_request, &key).unwrap();
        assert_ne!(
            original_history.complete_digest(),
            changed_history.complete_digest()
        );

        let root = std::env::temp_dir().join(format!(
            "hiroute-signed-history-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = ReplayManager::open(ReplayConfig {
            root: root.clone(),
            memory_threshold_bytes: 128,
            record_bytes: 31,
            orphan_ttl: std::time::Duration::from_secs(60),
        })
        .unwrap();
        let tree = BudgetTree::new(1024 * 1024, 1024 * 1024).unwrap();
        let store = manager
            .begin_request(tree.stream(1024 * 1024).unwrap())
            .unwrap();
        let original_ref = make_request(
            store
                .store_content(original.as_bytes())
                .unwrap()
                .wire_marker(),
        );
        let changed_ref = make_request(
            store
                .store_content(changed.as_bytes())
                .unwrap()
                .wire_marker(),
        );
        assert_eq!(
            visible_history_with_replay(&original_ref, &key, Some(&store))
                .unwrap()
                .complete_digest(),
            original_history.complete_digest()
        );
        assert_eq!(
            visible_history_with_replay(&changed_ref, &key, Some(&store))
                .unwrap()
                .complete_digest(),
            changed_history.complete_digest()
        );
        assert!(
            visible_history(&original_ref, &key)
                .unwrap()
                .measure(None)
                .is_none()
        );
        drop((store, manager));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replay_content_keeps_long_codex_instruction_tools_and_message_prefix_equivalent() {
        use crate::replay::{ReplayConfig, ReplayManager};
        use crate::server::core_runtime::model_ir::{CanonicalInstruction, CanonicalTool};
        use hiroute_gateway_core::runtime::body::BudgetTree;

        let root = std::env::temp_dir().join(format!(
            "hiroute-history-replay-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = ReplayManager::open(ReplayConfig {
            root: root.clone(),
            memory_threshold_bytes: 128,
            record_bytes: 31,
            orphan_ttl: std::time::Duration::from_secs(60),
        })
        .unwrap();
        let tree = BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024).unwrap();
        let first_store = manager
            .begin_request(tree.stream(1024 * 1024).unwrap())
            .unwrap();
        let second_store = manager
            .begin_request(tree.stream(1024 * 1024).unwrap())
            .unwrap();
        // Distinct request-local stream ordinals must not become the digest.
        second_store
            .store_content(b"unrelated prior range")
            .unwrap();
        let instruction = "system prompt 中文 ".repeat(2_000);
        let first_user = "first user content ".repeat(1_000);
        let schema = serde_json::json!({"type":"object","properties":{"query":{"type":"string"}}});
        let reordered_schema =
            serde_json::json!({"properties":{"query":{"type":"string"}},"type":"object"});
        let make_request = |store: &crate::replay::ReplayStore, appended: bool| {
            let mut request = request(vec![text(
                MessageRole::User,
                store
                    .store_content(first_user.as_bytes())
                    .unwrap()
                    .wire_marker(),
            )]);
            if appended {
                request.messages.extend([
                    text(MessageRole::Assistant, "first answer"),
                    text(MessageRole::User, "new question"),
                ]);
            }
            request.instructions.push(CanonicalInstruction {
                role: InstructionRole::System,
                content: vec![ContentPart::Text {
                    text: store
                        .store_content(instruction.as_bytes())
                        .unwrap()
                        .wire_marker(),
                }],
            });
            let mut pool = store.begin_content_pool().unwrap();
            let schema_ref = pool
                .append_json(if appended { &reordered_schema } else { &schema }, true)
                .unwrap();
            pool.seal().unwrap();
            request.tools.push(CanonicalTool {
                kind: ToolKindV1::Function,
                name: "lookup".into(),
                description: None,
                input_schema: Some(schema_ref.json_marker()),
                strict: None,
                format: None,
            });
            request
        };
        let first = make_request(&first_store, false);
        let continued = make_request(&second_store, true);
        let key = [20; 32];
        let first_history = visible_history_with_replay(&first, &key, Some(&first_store)).unwrap();
        let continued_history =
            visible_history_with_replay(&continued, &key, Some(&second_store)).unwrap();
        assert_eq!(
            first_history.instruction_digest,
            continued_history.instruction_digest
        );
        assert_eq!(
            continued_history.digest_at(first.messages.len()),
            Some(first_history.complete_digest())
        );
        let mut inline = first.clone();
        inline.messages[0] = text(MessageRole::User, first_user.clone());
        inline.instructions[0].content = vec![ContentPart::Text {
            text: instruction.clone(),
        }];
        inline.tools[0].input_schema = Some(reordered_schema);
        let inline_history = visible_history(&inline, &key).unwrap();
        assert_eq!(
            inline_history.instruction_digest,
            first_history.instruction_digest
        );
        assert_eq!(
            inline_history.complete_digest(),
            first_history.complete_digest()
        );
        assert!(
            visible_history_with_replay(&continued, &key, None).is_none(),
            "a bare request-local locator cannot establish continuity"
        );

        let mut changed = continued.clone();
        changed.messages[0] = text(MessageRole::User, "replaced first message");
        let changed_history =
            visible_history_with_replay(&changed, &key, Some(&second_store)).unwrap();
        assert_ne!(
            changed_history.digest_at(first.messages.len()),
            Some(first_history.complete_digest())
        );
        drop((first_store, second_store, manager));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nested_native_state_ref_hashes_like_the_same_inline_json() {
        use crate::replay::{ReplayConfig, ReplayManager};
        use hiroute_gateway_core::runtime::body::BudgetTree;

        let root = std::env::temp_dir().join(format!(
            "hiroute-history-escaped-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = ReplayManager::open(ReplayConfig {
            root: root.clone(),
            memory_threshold_bytes: 128,
            record_bytes: 31,
            orphan_ttl: std::time::Duration::from_secs(60),
        })
        .unwrap();
        let tree = BudgetTree::new(1024 * 1024, 1024 * 1024).unwrap();
        let store = manager
            .begin_request(tree.stream(1024 * 1024).unwrap())
            .unwrap();
        let original = "opaque \"user\\ text\n中文";
        let marker = store
            .store_content(original.as_bytes())
            .unwrap()
            .wire_marker();
        let externalized = serde_json::json!({"thinking": marker});
        let inline = serde_json::json!({"thinking": original});
        let key = [21; 32];
        let mut replay_hash = Encoder::new(&key, b"json-string", Some(&store)).unwrap();
        replay_hash.json(&externalized).unwrap();
        let mut inline_hash = Encoder::new(&key, b"json-string", None).unwrap();
        inline_hash.json(&inline).unwrap();
        assert_eq!(replay_hash.finish(), inline_hash.finish());
        drop((store, manager));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn compacted_responses_tool_arguments_and_long_reasoning_keep_prefix() {
        use crate::content_ref::{
            compact_ingress_document_with_markers, parse_ingress_document, scan_ingress_document,
        };
        use crate::replay::{ReplayConfig, ReplayManager};
        use crate::server::core_runtime::adapters::decode_ingress_request;
        use hiroute_gateway_core::runtime::body::BudgetTree;

        let root = std::env::temp_dir().join(format!(
            "hiroute-history-responses-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = ReplayManager::open(ReplayConfig {
            root: root.clone(),
            memory_threshold_bytes: 128,
            record_bytes: 31,
            orphan_ttl: std::time::Duration::from_secs(60),
        })
        .unwrap();
        let tree = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024).unwrap();
        let first_store = manager
            .begin_request(tree.stream(4 * 1024 * 1024).unwrap())
            .unwrap();
        let second_store = manager
            .begin_request(tree.stream(4 * 1024 * 1024).unwrap())
            .unwrap();
        let argument_text = "tool argument 中文 ".repeat(700);
        let encoded = serde_json::to_string(&argument_text).unwrap();
        let first_arguments = format!("{{\"b\": {encoded}, \"a\": 1}}");
        let second_arguments = format!("{{\"a\":1,\"b\":{encoded}}}");
        let summary = "reasoning summary \"\\\n🙂 ".repeat(700);
        let make_request = |store: &ReplayStore, arguments: String, appended: bool| {
            let mut input = vec![
                serde_json::json!({"type":"message","role":"user","content":[{"type":"input_text","text":"first"}]}),
                serde_json::json!({"type":"function_call","call_id":"call-1","name":"lookup","arguments":arguments}),
                serde_json::json!({"type":"reasoning","summary":[{"type":"summary_text","text":summary}]}),
            ];
            if appended {
                input.push(serde_json::json!({"type":"message","role":"user","content":[{"type":"input_text","text":"follow up"}]}));
            }
            let body = serde_json::to_vec(&serde_json::json!({"model":"agent/test","input":input}))
                .unwrap();
            let mut writer = store.begin_raw().unwrap();
            writer.append(&body).unwrap();
            let raw = writer.seal().unwrap();
            let stats = scan_ingress_document(store.reader(&raw).unwrap()).unwrap();
            let (mut document, mut workspace) =
                parse_ingress_document(IngressProtocol::Responses, store, &raw, stats).unwrap();
            compact_ingress_document_with_markers(
                IngressProtocol::Responses,
                &mut document,
                store,
                workspace.generated_markers(),
            )
            .unwrap();
            assert!(
                document["input"][1]["arguments"]
                    .as_str()
                    .and_then(ContentValueExt::content_ref)
                    .is_some()
            );
            assert!(
                document["input"][2]["summary"][0]["text"]
                    .as_str()
                    .and_then(ContentValueExt::content_ref)
                    .is_some()
            );
            decode_ingress_request(IngressProtocol::Responses, &document).unwrap()
        };
        let first = make_request(&first_store, first_arguments, false);
        let continued = make_request(&second_store, second_arguments, true);
        let key = [22; 32];
        let first_history = visible_history_with_replay(&first, &key, Some(&first_store)).unwrap();
        let continued_history =
            visible_history_with_replay(&continued, &key, Some(&second_store)).unwrap();
        assert_eq!(
            continued_history.digest_at(first.messages.len()),
            Some(first_history.complete_digest())
        );
        drop((first_store, second_store, manager));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_json_sorts_object_keys_and_instruction_change_rebuilds() {
        let key = [9; 32];
        let tool_message = |arguments| CanonicalMessage {
            role: MessageRole::Assistant,
            content: vec![ContentPart::ToolCall {
                logical_id: "call".into(),
                tool_kind: ToolKindV1::Function,
                namespace: None,
                name: "tool".into(),
                arguments,
                raw_arguments: None,
            }],
            name: None,
        };
        let left_request = request(vec![tool_message(serde_json::json!({"a":1,"b":2}))]);
        let right_request = request(vec![tool_message(serde_json::json!({"b":2,"a":1}))]);
        let left = visible_history(&left_request, &key).unwrap();
        let right = visible_history(&right_request, &key).unwrap();
        assert_eq!(left.complete_digest(), right.complete_digest());

        let mut changed = request(vec![text(MessageRole::User, "same")]);
        let original_instruction_digest =
            visible_history(&changed, &key).unwrap().instruction_digest;
        changed.instructions.push(
            crate::server::core_runtime::model_ir::CanonicalInstruction {
                role: InstructionRole::Developer,
                content: vec![ContentPart::Text { text: "new".into() }],
            },
        );
        assert_ne!(
            original_instruction_digest,
            visible_history(&changed, &key).unwrap().instruction_digest
        );
    }

    #[test]
    fn evidence_is_fixed_size_and_ignored_request_effort_is_not_effective_setup() {
        let key = [11; 32];
        let small_request = request(vec![text(MessageRole::User, "small")]);
        let large_request = request(
            (0..4_096)
                .map(|index| text(MessageRole::User, format!("message-{index}")))
                .collect(),
        );
        let small = visible_history(&small_request, &key).unwrap();
        let large = visible_history(&large_request, &key).unwrap();
        assert_eq!(std::mem::size_of_val(&small), std::mem::size_of_val(&large));
        assert_eq!(large.message_count(), 4_096);

        let mut low = request(vec![text(MessageRole::User, "same")]);
        low.requested_reasoning =
            RequestedReasoningControl::overridden(serde_json::json!({"effort":"low"}));
        let mut high = low.clone();
        high.requested_reasoning =
            RequestedReasoningControl::overridden(serde_json::json!({"effort":"high"}));
        let low = visible_history(&low, &key).unwrap();
        let high = visible_history(&high, &key).unwrap();
        assert_eq!(low.instruction_digest, high.instruction_digest);
        assert_eq!(low.complete_digest(), high.complete_digest());
    }

    #[test]
    fn responses_wire_search_and_annotations_extend_their_ordered_history_position() {
        use crate::server::core_runtime::adapters::decode_ingress_request;
        use serde_json::json;

        let key = [13; 32];
        let initial_wire = json!({
            "model":"agent/test",
            "input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"weather"}]}],
            "reasoning":{"effort":"low"}
        });
        let continued_wire = json!({
            "model":"agent/test",
            "input":[
                {"type":"message","role":"user","content":[{"type":"input_text","text":"weather"}]},
                {"type":"web_search_call","id":"search-1","status":"completed","action":{"type":"search","query":"weather today"}},
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":"sunny","annotations":[{"type":"url_citation","start_index":0,"end_index":5,"title":"forecast","url":"https://example.test/forecast"}]}]},
                {"type":"message","role":"user","content":[{"type":"input_text","text":"and tomorrow?"}]}
            ],
            "reasoning":{"effort":"high"}
        });
        let initial = decode_ingress_request(IngressProtocol::Responses, &initial_wire).unwrap();
        let continued =
            decode_ingress_request(IngressProtocol::Responses, &continued_wire).unwrap();
        let initial_history = visible_history(&initial, &key).unwrap();
        let continued_history = visible_history(&continued, &key).unwrap();
        assert_eq!(
            continued_history.digest_at(initial.messages.len()),
            Some(initial_history.complete_digest()),
            "appended search history and a changed ignored effort remain a continuation"
        );

        let mut changed_search_wire = continued_wire.clone();
        changed_search_wire["input"][1]["action"]["query"] = json!("different query");
        let changed_search =
            decode_ingress_request(IngressProtocol::Responses, &changed_search_wire).unwrap();
        assert_ne!(
            continued_history.digest_at(2),
            visible_history(&changed_search, &key).unwrap().digest_at(2)
        );

        let mut changed_citation_wire = continued_wire;
        changed_citation_wire["input"][2]["content"][0]["annotations"][0]["url"] =
            json!("https://example.test/changed");
        let changed_citation =
            decode_ingress_request(IngressProtocol::Responses, &changed_citation_wire).unwrap();
        assert_ne!(
            continued_history.digest_at(3),
            visible_history(&changed_citation, &key)
                .unwrap()
                .digest_at(3)
        );
    }

    #[test]
    fn later_reused_id_does_not_change_earlier_result_fingerprint() {
        use crate::server::core_runtime::adapters::decode_ingress_request;
        use serde_json::json;
        let mut wire = json!({"model":"alias","input":[
            {"type":"function_call_output","call_id":"same","output":"first"}
        ]});
        let first = decode_ingress_request(IngressProtocol::Responses, &wire).unwrap();
        wire["input"].as_array_mut().unwrap().extend([
            json!({"type":"function_call","call_id":"same","namespace":"new-group","name":"lookup","arguments":"{}"}),
            json!({"type":"function_call_output","call_id":"same","output":"second"}),
        ]);
        let continued = decode_ingress_request(IngressProtocol::Responses, &wire).unwrap();
        let key = [19; 32];
        assert_eq!(
            visible_history(&continued, &key)
                .unwrap()
                .digest_at(first.messages.len()),
            Some(visible_history(&first, &key).unwrap().complete_digest())
        );
    }

    #[test]
    fn namespace_schema_order_and_supported_responses_options_rebuild_context() {
        use crate::server::core_runtime::adapters::decode_ingress_request;
        use serde_json::json;

        let key = [17; 32];
        let base = json!({
            "model":"agent/test",
            "input":"hello",
            "store":false,
            "prompt_cache_key":"cache-a",
            "client_metadata":{"session_id":"one"},
            "reasoning":{"effort":"low","summary":"auto"},
            "tools":[
                {"type":"namespace","name":"group","tools":[
                    {"type":"function","name":"child","parameters":{"type":"object","properties":{"value":{"type":"string"}}},"strict":true}
                ]},
                {"type":"function","name":"flat","parameters":{"type":"object"}}
            ]
        });
        let digest = |wire: &Value| {
            let request = decode_ingress_request(IngressProtocol::Responses, wire).unwrap();
            visible_history(&request, &key).unwrap().instruction_digest
        };
        let base_digest = digest(&base);

        let mut effort = base.clone();
        effort["reasoning"]["effort"] = json!("high");
        assert_eq!(base_digest, digest(&effort));

        let mut summary = base.clone();
        summary["reasoning"]["summary"] = json!("detailed");
        assert_ne!(base_digest, digest(&summary));

        let mut schema = base.clone();
        schema["tools"][0]["tools"][0]["parameters"]["properties"]["value"]["type"] =
            json!("number");
        assert_ne!(base_digest, digest(&schema));

        let mut order = base.clone();
        order["tools"].as_array_mut().unwrap().swap(0, 1);
        assert_ne!(base_digest, digest(&order));

        let mut cache = base;
        cache["prompt_cache_key"] = json!("cache-b");
        assert_ne!(base_digest, digest(&cache));
    }
}
