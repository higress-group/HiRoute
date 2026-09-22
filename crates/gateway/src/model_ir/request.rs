use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::content_ref::ContentValue;
use crate::server::request_plan::IngressProtocol;

use super::ModelIrError;

pub const MODEL_REQUEST_IR_SCHEMA: &str = "hiroute.model-request-ir/v1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRequestIRV1 {
    pub schema_version: String,
    pub ingress_protocol: IngressProtocol,
    pub served_model_id: String,
    pub stream: bool,
    pub instructions: Vec<CanonicalInstruction>,
    pub messages: Vec<CanonicalMessage>,
    pub tools: Vec<CanonicalTool>,
    /// Responses-only one-level tool namespaces. They remain distinct from
    /// flat tools because `(kind, namespace, name)` is the callable identity.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_namespaces: Vec<CanonicalToolNamespaceV1>,
    /// Native top-level Responses tool order. Every flat tool and
    /// namespace is referenced exactly once; WebSearch is present iff the
    /// typed hosted search declaration is present.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub responses_tool_order: Vec<ResponsesToolOrderEntryV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_search: Option<super::WebSearchToolV1>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub responses_search_history: std::collections::BTreeMap<usize, super::WebSearchCallV1>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub responses_annotations: std::collections::BTreeMap<
        usize,
        std::collections::BTreeMap<usize, Vec<super::UrlCitationV1>>,
    >,
    pub tool_choice: ToolChoice,
    pub parallel_tool_calls: bool,
    pub requested_reasoning: RequestedReasoningControl,
    /// Recorded for audit only. P0 projection always uses the candidate cap.
    pub requested_max_output_tokens: Option<u64>,
    pub provider_state: Vec<OpaqueProviderState>,
    pub tool_id_map: Vec<ToolIdMapEntryV1>,
    /// Native Responses transport controls are retained, never silently dropped on conversion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responses_options: Option<ResponsesRequestOptionsV1>,
    /// Client-supplied Responses item IDs, indexed by canonical message position. These
    /// are not provider-state authority or logical Tool call identities.
    #[serde(
        default,
        alias = "responses_message_ids",
        skip_serializing_if = "std::collections::BTreeMap::is_empty"
    )]
    pub responses_item_ids: std::collections::BTreeMap<usize, String>,
    /// Explicit completion status of a native Responses input item.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub responses_item_statuses: std::collections::BTreeMap<usize, String>,
    /// Responses message phases are native input-item semantics. They are indexed by the
    /// canonical message produced from that exact input item and may only be replayed to a
    /// Responses upstream.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub responses_message_phases: std::collections::BTreeMap<usize, ResponsesMessagePhaseV1>,
    /// Bounded Codex transport metadata carried by Responses history items. This is neither
    /// provider state nor HiRoute identity and is preserved only on the same native protocol.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub responses_internal_chat_message_metadata:
        std::collections::BTreeMap<usize, ResponsesInternalChatMessageMetadataV1>,
    /// Native Responses reasoning-item history retained for an exact same-provider
    /// continuation. The opaque encrypted state remains a ProviderState content part;
    /// this sidecar preserves other native fields without interpreting their format.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub responses_reasoning_history: std::collections::BTreeMap<usize, ResponsesReasoningHistoryV1>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponsesRequestOptionsV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_metadata: Option<std::collections::BTreeMap<String, String>>,
    /// Responses reasoning history policy is native to this transport; never project it to
    /// Chat Completions or Messages, where the same continuity cannot be promised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_context: Option<String>,
    /// Supported Responses reasoning sibling that is retained while the
    /// sealed Agent Plan replaces effort/budget fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_summary: Option<ContentValue>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponsesInternalChatMessageMetadataV1 {
    pub turn_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponsesReasoningHistoryV1 {
    /// Native, non-authority fields of a Responses reasoning item. These are
    /// returned only to a Responses upstream; the Gateway does not interpret
    /// a provider's plain reasoning format.
    pub native_fields: serde_json::Map<String, serde_json::Value>,
    /// Wire shape of the native `encrypted_content` sibling. Only `Opaque`
    /// corresponds to ProviderState and therefore requires exact-owner authority.
    pub encrypted_content: ResponsesReasoningEncryptedContentV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponsesReasoningEncryptedContentV1 {
    Opaque,
    Absent,
    Null,
    Empty,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponsesMessagePhaseV1 {
    Commentary,
    FinalAnswer,
}

impl ModelRequestIRV1 {
    pub fn requirements(&self) -> RequestCapabilityRequirementsV1 {
        // Responses may put instruction-role input items before the first conversation
        // message. Their position is initial even though the item remains a message in
        // the IR so native Responses identity and metadata can be preserved. An
        // instruction after user/assistant history is genuinely mid-conversation.
        let mut conversation_started = false;
        let mut initial_input_instructions = false;
        let mut mid_conversation_instructions = false;
        for message in &self.messages {
            if matches!(message.role, MessageRole::System | MessageRole::Developer) {
                if conversation_started {
                    mid_conversation_instructions = true;
                } else {
                    initial_input_instructions = true;
                }
            } else {
                conversation_started = true;
            }
        }
        let namespace_functions = self
            .tool_namespaces
            .iter()
            .flat_map(|namespace| namespace.tools.iter());
        let function_tools = !self.tools.is_empty()
            || self
                .tool_namespaces
                .iter()
                .any(|namespace| !namespace.tools.is_empty());
        let active_function_tools = function_tools && self.tool_choice != ToolChoice::None;
        let mut requirements = RequestCapabilityRequirementsV1 {
            ingress_protocol: self.ingress_protocol,
            text: false,
            initial_instructions: !self.instructions.is_empty() || initial_input_instructions,
            mid_conversation_instructions,
            image_url: false,
            image_base64: false,
            image_media_types: Vec::new(),
            function_tools,
            strict_tools: self
                .tools
                .iter()
                .chain(namespace_functions)
                .any(|tool| tool.kind == ToolKindV1::Function && tool.strict.is_some()),
            tool_choice: self.tool_choice.clone(),
            parallel_tools: self.parallel_tool_calls,
            tool_roundtrip: active_function_tools || !self.responses_search_history.is_empty(),
            tool_result_text: false,
            tool_result_json: false,
            logical_tool_id_mapping: active_function_tools
                || !self.responses_search_history.is_empty(),
            streaming: self.stream,
            stream_text: self.stream,
            stream_tool_arguments: self.stream
                && function_tools
                && self.tool_choice != ToolChoice::None,
            stream_reasoning: self.stream,
            stream_usage: self.stream,
            provider_state: !self.provider_state.is_empty(),
        };
        let mut media_types = BTreeSet::new();
        for content in self
            .instructions
            .iter()
            .flat_map(|instruction| instruction.content.iter())
            .chain(
                self.messages
                    .iter()
                    .flat_map(|message| message.content.iter()),
            )
        {
            match content {
                ContentPart::Text { .. } => requirements.text = true,
                ContentPart::Image {
                    source: ImageSource::Url { .. },
                } => {
                    requirements.image_url = true;
                }
                ContentPart::Image {
                    source: ImageSource::Base64 { media_type, .. },
                } => {
                    requirements.image_base64 = true;
                    media_types.insert(media_type.clone());
                }
                ContentPart::ToolCall { .. } => {
                    requirements.tool_roundtrip = true;
                    requirements.logical_tool_id_mapping = true;
                }
                ContentPart::ToolResult { output, .. } => {
                    requirements.tool_roundtrip = true;
                    requirements.logical_tool_id_mapping = true;
                    match output {
                        ToolOutput::Text(_) => requirements.tool_result_text = true,
                        ToolOutput::Json(_) => requirements.tool_result_json = true,
                    }
                }
                ContentPart::ProviderState { .. } => requirements.provider_state = true,
            }
        }
        requirements.image_media_types = media_types.into_iter().collect();
        requirements
    }

    /// Returns every client-visible Tool identity that requires trusted
    /// continuation recovery. One historical call and one matching result are
    /// the only unambiguous repeated use of the same logical ID.
    pub fn continuation_logical_ids(&self) -> Result<Vec<String>, ModelIrError> {
        let mut identities = std::collections::BTreeMap::<String, (bool, bool)>::new();
        for search in self.responses_search_history.values() {
            if identities
                .insert(search.id.clone(), (true, false))
                .is_some()
            {
                return Err(ModelIrError::ToolContinuationConflict);
            }
        }
        for part in self
            .instructions
            .iter()
            .flat_map(|instruction| instruction.content.iter())
            .chain(
                self.messages
                    .iter()
                    .flat_map(|message| message.content.iter()),
            )
        {
            let (logical_id, is_call) = match part {
                ContentPart::ToolCall { logical_id, .. } => (logical_id, true),
                ContentPart::ToolResult { logical_id, .. } => (logical_id, false),
                ContentPart::Text { .. }
                | ContentPart::Image { .. }
                | ContentPart::ProviderState { .. } => continue,
            };
            if logical_id.trim().is_empty() {
                return Err(ModelIrError::ToolContinuationConflict);
            }
            let presence = identities.entry(logical_id.clone()).or_default();
            let duplicate = if is_call {
                std::mem::replace(&mut presence.0, true)
            } else {
                std::mem::replace(&mut presence.1, true)
            };
            if duplicate {
                return Err(ModelIrError::ToolContinuationConflict);
            }
        }
        Ok(identities.into_keys().collect())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionRole {
    System,
    Developer,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalInstruction {
    pub role: InstructionRole,
    pub content: Vec<ContentPart>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Developer,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalMessage {
    pub role: MessageRole,
    pub content: Vec<ContentPart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<ContentValue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContentPart {
    Text {
        text: ContentValue,
    },
    Image {
        source: ImageSource,
    },
    ToolCall {
        logical_id: String,
        tool_kind: ToolKindV1,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<ContentValue>,
        name: ContentValue,
        arguments: Value,
    },
    ToolResult {
        logical_id: String,
        tool_kind: ToolKindV1,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<ContentValue>,
        output: ToolOutput,
        #[serde(default, skip_serializing_if = "ToolResultStatusV1::is_unknown")]
        status: ToolResultStatusV1,
    },
    ProviderState {
        state: Box<OpaqueProviderState>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImageSource {
    Url {
        url: ContentValue,
    },
    Base64 {
        media_type: String,
        data: ContentValue,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ToolOutput {
    Text(ContentValue),
    Json(Value),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalTool {
    pub kind: ToolKindV1,
    pub name: ContentValue,
    pub description: Option<ContentValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalToolNamespaceV1 {
    pub name: ContentValue,
    pub description: Option<ContentValue>,
    pub tools: Vec<CanonicalTool>,
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKindV1 {
    #[default]
    Function,
    Custom,
}

/// Protocol-neutral status evidence for one client-supplied Tool result.
///
/// `Unknown` is intentionally distinct from `Completed`: protocols without a
/// structured result status must not turn the mere presence of output into a
/// success signal for Agent-turn classification.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultStatusV1 {
    Completed,
    Failed,
    #[default]
    Unknown,
}

impl ToolResultStatusV1 {
    fn is_unknown(&self) -> bool {
        *self == Self::Unknown
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResponsesToolOrderEntryV1 {
    Tool { index: u32 },
    Namespace { index: u32 },
    WebSearch,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolChoice {
    None,
    Auto,
    RequiredAny,
    RequiredNamed {
        tool_kind: ToolKindV1,
        name: ContentValue,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedReasoningDisposition {
    Absent,
    OverriddenByAgentPlan,
    AppliedToFixedBinding,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestedReasoningControl {
    pub native_value: Option<Value>,
    pub disposition: RequestedReasoningDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fixed_profile_digest: Option<hiroute_domain::CanonicalDigest>,
}

impl RequestedReasoningControl {
    pub fn absent() -> Self {
        Self {
            native_value: None,
            disposition: RequestedReasoningDisposition::Absent,
            fixed_profile_digest: None,
        }
    }

    pub fn overridden(native_value: Value) -> Self {
        Self {
            native_value: Some(native_value),
            disposition: RequestedReasoningDisposition::OverriddenByAgentPlan,
            fixed_profile_digest: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpaqueProviderState {
    pub owner: ExactProviderPathV1,
    pub block_index: Option<u32>,
    pub kind: String,
    pub value: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactProviderPathV1 {
    pub provider_id: String,
    pub endpoint_id: String,
    pub entitlement_id: String,
    pub connector_id: String,
    pub connector_revision: String,
    pub capability_id: String,
    pub capability_revision: String,
    pub model_configuration_id: String,
    pub native_model: String,
    pub upstream_protocol: IngressProtocol,
    pub adapter_revision: String,
    pub serializer_revision: String,
    pub decoder_revision: String,
}

impl ExactProviderPathV1 {
    pub fn is_complete(&self) -> bool {
        [
            self.provider_id.as_str(),
            self.endpoint_id.as_str(),
            self.entitlement_id.as_str(),
            self.connector_id.as_str(),
            self.connector_revision.as_str(),
            self.capability_id.as_str(),
            self.capability_revision.as_str(),
            self.model_configuration_id.as_str(),
            self.native_model.as_str(),
            self.adapter_revision.as_str(),
            self.serializer_revision.as_str(),
            self.decoder_revision.as_str(),
        ]
        .into_iter()
        .all(|value| !value.trim().is_empty())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolIdMapEntryV1 {
    pub logical_id: String,
    pub native_id: String,
    pub kind: ToolKindV1,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub owner: ExactProviderPathV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestCapabilityRequirementsV1 {
    pub ingress_protocol: IngressProtocol,
    pub text: bool,
    pub initial_instructions: bool,
    pub mid_conversation_instructions: bool,
    pub image_url: bool,
    pub image_base64: bool,
    pub image_media_types: Vec<String>,
    pub function_tools: bool,
    pub strict_tools: bool,
    pub tool_choice: ToolChoice,
    pub parallel_tools: bool,
    pub tool_roundtrip: bool,
    pub tool_result_text: bool,
    pub tool_result_json: bool,
    pub logical_tool_id_mapping: bool,
    pub streaming: bool,
    pub stream_text: bool,
    pub stream_tool_arguments: bool,
    pub stream_reasoning: bool,
    pub stream_usage: bool,
    pub provider_state: bool,
}
