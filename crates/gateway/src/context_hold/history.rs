use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

use crate::content_ref::{ContentValueExt, JsonValueExt};
use crate::server::core_runtime::model_ir::{
    CanonicalMessage, ContentPart, ImageSource, InstructionRole, MessageRole, ModelRequestIRV1,
    OpaqueProviderState, ResponsesReasoningEncryptedContentV1, ResponsesToolOrderEntryV1,
    ToolChoice, ToolKindV1, ToolOutput, ToolResultStatusV1, UrlCitationKind, UrlCitationV1,
    WebSearchAction, WebSearchCallV1, WebSearchSourceKind, WebSearchStatus,
};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug)]
pub(crate) struct HistoryEvidence<'a> {
    pub(crate) instruction_digest: [u8; 32],
    request: &'a ModelRequestIRV1,
    key: &'a [u8; 32],
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
        let mut history = Encoder::new(self.key, b"visible-history/v1/messages")?;
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

pub(crate) fn visible_history<'a>(
    request: &'a ModelRequestIRV1,
    key: &'a [u8; 32],
) -> Option<HistoryEvidence<'a>> {
    let mut instructions = Encoder::new(key, b"visible-history/v1/instructions")?;
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
    })
}

struct Encoder {
    mac: HmacSha256,
    content_ref_seen: bool,
}

impl Encoder {
    fn new(key: &[u8; 32], domain: &[u8]) -> Option<Self> {
        let mut mac = HmacSha256::new_from_slice(key).ok()?;
        frame(&mut mac, b"domain", domain);
        Some(Self {
            mac,
            content_ref_seen: false,
        })
    }

    fn finish(self) -> Option<[u8; 32]> {
        (!self.content_ref_seen).then(|| self.mac.finalize().into_bytes().into())
    }

    fn snapshot(&self) -> Option<[u8; 32]> {
        (!self.content_ref_seen).then(|| self.mac.clone().finalize().into_bytes().into())
    }

    fn tag(&mut self, value: &[u8]) {
        frame(&mut self.mac, b"tag", value);
    }

    fn bytes(&mut self, value: &[u8]) {
        frame(&mut self.mac, b"bytes", value);
    }

    fn string(&mut self, value: &str) {
        if value.content_ref().is_some() {
            self.content_ref_seen = true;
        } else {
            self.tag(b"utf8");
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
        if value.content_ref().is_some() {
            self.content_ref_seen = true;
            return Some(());
        }
        match value {
            Value::Null => self.tag(b"null"),
            Value::Bool(value) => self.bool(*value),
            Value::Number(value) => {
                self.tag(b"number");
                self.bytes(value.to_string().as_bytes());
            }
            Value::String(value) => self.string(value),
            Value::Array(values) => {
                self.tag(b"array");
                self.usize(values.len());
                for value in values {
                    self.json(value)?;
                }
            }
            Value::Object(values) => {
                self.tag(b"object");
                self.usize(values.len());
                let mut keys = values.keys().collect::<Vec<_>>();
                keys.sort_unstable();
                for key in keys {
                    self.bytes(key.as_bytes());
                    self.json(&values[key])?;
                }
            }
        }
        Some(())
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
                } => {
                    self.tag(b"tool-call");
                    self.tag(match tool_kind {
                        ToolKindV1::Function => b"function",
                        ToolKindV1::Custom => b"custom",
                    });
                    self.bytes(logical_id.as_bytes());
                    self.optional_string(namespace.as_deref());
                    self.string(name);
                    self.json(arguments)?;
                }
                ContentPart::ToolResult {
                    logical_id,
                    tool_kind,
                    namespace,
                    output,
                    status,
                } => {
                    self.tag(b"tool-result");
                    self.tag(match tool_kind {
                        ToolKindV1::Function => b"function",
                        ToolKindV1::Custom => b"custom",
                    });
                    self.bytes(logical_id.as_bytes());
                    self.optional_string(namespace.as_deref());
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
                self.json(&Value::Object(history.native_fields.clone()))?;
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
        self.json(&state.value)
    }
}

fn frame(mac: &mut HmacSha256, kind: &[u8], value: &[u8]) {
    mac.update(&(kind.len() as u64).to_be_bytes());
    mac.update(kind);
    mac.update(&(value.len() as u64).to_be_bytes());
    mac.update(value);
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
            tool_id_map: Vec::new(),
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
