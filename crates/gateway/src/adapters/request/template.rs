use std::collections::BTreeMap;
use std::sync::Arc;

use crate::content_ref::{ContentRef, ContentValueExt, JsonValueExt, MARKER_PREFIX};
use crate::server::core_runtime::model_ir::{
    ContentPart, ImageSource, ModelRequestIRV1, ToolKindV1, ToolOutput,
};
use crate::server::core_runtime::profiles::{
    CandidateContextDemand, CandidateProtocolProfile, ContextProjector,
};
use crate::server::request_plan::IngressProtocol;

use super::{
    ChatToolProjection, ProtocolAdapterError, serialize_chat, serialize_messages,
    serialize_responses, validate_message_shapes,
};

#[derive(Clone, Debug)]
pub struct PreparedNativeTemplate {
    pub protocol: IngressProtocol,
    pub path: String,
    pub(crate) bytes: Arc<[u8]>,
    pub(crate) replacements: Arc<[TemplateReplacement]>,
    pub wire_len: usize,
    pub context: CandidateContextDemand,
    pub capability_id: String,
    pub connector_id: String,
    pub adapter_revision: String,
    pub serializer_revision: String,
    pub decoder_revision: String,
    pub reasoning_profile_id: String,
    pub(crate) chat_tool_projection: Option<ChatToolProjection>,
}

/// Structural JSON plus replay-backed substitutions shared by provider and
/// classifier requests. The template itself stays bounded by request shape;
/// referenced content is read only by the sequential body encoder.
#[derive(Clone, Debug)]
pub(crate) struct PreparedReplayTemplate {
    pub(crate) bytes: Arc<[u8]>,
    pub(crate) replacements: Arc<[TemplateReplacement]>,
    pub(crate) wire_len: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct TemplateReplacement {
    pub start: usize,
    pub end: usize,
    pub content: ContentRef,
    pub encoding: ReplacementEncoding,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReplacementEncoding {
    JsonString,
    RawJson,
    /// The replay bytes are one complete JSON value embedded inside a JSON
    /// object that is itself serialized into an outer JSON string.
    RawJsonInJsonString,
}

impl PreparedNativeTemplate {
    pub(crate) fn retained_bytes(&self) -> Option<usize> {
        [
            self.bytes.len(),
            self.replacements
                .len()
                .checked_mul(std::mem::size_of::<TemplateReplacement>())?,
            self.path.capacity(),
            self.capability_id.capacity(),
            self.connector_id.capacity(),
            self.adapter_revision.capacity(),
            self.serializer_revision.capacity(),
            self.decoder_revision.capacity(),
            self.reasoning_profile_id.capacity(),
            match &self.chat_tool_projection {
                Some(projection) => projection.retained_bytes()?,
                None => 0,
            },
            4 * std::mem::size_of::<usize>(),
        ]
        .into_iter()
        .try_fold(0_usize, usize::checked_add)
    }
}

impl PreparedReplayTemplate {
    pub(crate) fn retained_bytes(&self) -> Option<usize> {
        self.replacements
            .len()
            .checked_mul(std::mem::size_of::<TemplateReplacement>())?
            .checked_add(self.bytes.len())
            .and_then(|bytes| bytes.checked_add(3 * std::mem::size_of::<usize>()))
    }

    pub(crate) fn release_storage(&mut self) {
        self.bytes = Arc::from([]);
        self.replacements = Arc::from([]);
    }
}

/// Produces a bounded structural JSON template. Large strings remain markers
/// whose exact escaped lengths are already carried by `ContentRef`; no content
/// bytes are read while Planner computes candidate context demand.
pub fn project_candidate_request_template(
    request: &ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
) -> Result<PreparedNativeTemplate, ProtocolAdapterError> {
    let requirements = request.requirements();
    let reasoning = profile.validate(&requirements)?;
    validate_message_shapes(request, profile.capability.upstream_protocol)?;
    let chat_tool_projection =
        if profile.capability.upstream_protocol == IngressProtocol::ChatCompletions {
            Some(ChatToolProjection::for_request(request)?)
        } else {
            None
        };
    let body = match profile.capability.upstream_protocol {
        IngressProtocol::Responses => serialize_responses(request, profile, reasoning)?,
        IngressProtocol::ChatCompletions => serialize_chat(
            request,
            profile,
            reasoning,
            chat_tool_projection
                .as_ref()
                .expect("Chat projection was constructed"),
        )?,
        IngressProtocol::Messages => serialize_messages(request, profile, reasoning)?,
    };
    let refs = request_content_refs(request, profile.capability.upstream_protocol);
    let replay_template = prepare_replay_json_template(&body, refs)?;
    let wire_len = replay_template.wire_len;
    let PreparedReplayTemplate {
        bytes,
        replacements,
        ..
    } = replay_template;
    let context = ContextProjector::project_serialized_len(
        wire_len as u64,
        &profile.capability.context,
        reasoning,
    )?;
    Ok(PreparedNativeTemplate {
        protocol: profile.capability.upstream_protocol,
        path: profile.connector.request_path.clone(),
        bytes,
        replacements,
        wire_len,
        context,
        capability_id: profile.capability.capability_id.clone(),
        connector_id: profile.connector.connector_id.clone(),
        adapter_revision: profile.adapter_revision.clone(),
        serializer_revision: profile.serializer_revision.clone(),
        decoder_revision: profile.decoder_revision.clone(),
        reasoning_profile_id: reasoning.profile_id.clone(),
        chat_tool_projection,
    })
}

pub(crate) fn prepare_replay_json_template(
    body: &serde_json::Value,
    refs: impl IntoIterator<Item = RequestedReplacement>,
) -> Result<PreparedReplayTemplate, ProtocolAdapterError> {
    let bytes = serde_json::to_vec(body)
        .map_err(|error| ProtocolAdapterError::Serialization(error.to_string()))?;
    let mut expected = BTreeMap::<ContentRef, (ReplacementEncoding, usize)>::new();
    for requested in refs {
        match expected.entry(requested.content) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert((requested.encoding, 1));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let (encoding, count) = entry.get_mut();
                if *encoding != requested.encoding {
                    return Err(ProtocolAdapterError::Serialization(
                        "ContentRef is requested with conflicting encodings".into(),
                    ));
                }
                *count = count.checked_add(1).ok_or_else(|| {
                    ProtocolAdapterError::Serialization(
                        "ContentRef replacement count overflow".into(),
                    )
                })?;
            }
        }
    }
    let mut replacements = Vec::with_capacity(expected.len());
    let mut cursor = 0_usize;
    while let Some(relative) = find_subslice(&bytes[cursor..], MARKER_PREFIX.as_bytes()) {
        let marker_start = cursor + relative;
        let suffix_start = marker_start + MARKER_PREFIX.len();
        let suffix = find_subslice(&bytes[suffix_start..], b"__").ok_or_else(|| {
            ProtocolAdapterError::Serialization("ContentRef marker is not terminated".into())
        })?;
        let marker_end = suffix_start + suffix + 2;
        let marker = std::str::from_utf8(&bytes[marker_start..marker_end]).map_err(|_| {
            ProtocolAdapterError::Serialization("ContentRef marker is not ASCII".into())
        })?;
        let content = ContentRef::from_wire_marker(marker).ok_or_else(|| {
            ProtocolAdapterError::Serialization("ContentRef marker is malformed".into())
        })?;
        let (encoding, count) = expected.get_mut(&content).ok_or_else(|| {
            ProtocolAdapterError::Serialization(
                "ContentRef marker collides with inline content".into(),
            )
        })?;
        if *count == 0 {
            return Err(ProtocolAdapterError::Serialization(
                "ContentRef marker occurs more often than requested".into(),
            ));
        }
        *count -= 1;
        let encoding = *encoding;
        let (start, end) = match encoding {
            ReplacementEncoding::JsonString => (marker_start, marker_end),
            ReplacementEncoding::RawJson => {
                if marker_start == 0
                    || marker_end >= bytes.len()
                    || bytes[marker_start - 1] != b'"'
                    || bytes[marker_end] != b'"'
                {
                    return Err(ProtocolAdapterError::Serialization(
                        "raw ContentRef is not a complete JSON string placeholder".into(),
                    ));
                }
                (marker_start - 1, marker_end + 1)
            }
            ReplacementEncoding::RawJsonInJsonString => {
                if marker_start < 2
                    || marker_end + 1 >= bytes.len()
                    || &bytes[marker_start - 2..marker_start] != b"\\\""
                    || &bytes[marker_end..marker_end + 2] != b"\\\""
                {
                    return Err(ProtocolAdapterError::Serialization(
                        "nested raw ContentRef is not a complete escaped JSON string placeholder"
                            .into(),
                    ));
                }
                (marker_start - 2, marker_end + 2)
            }
        };
        replacements.push(TemplateReplacement {
            start,
            end,
            content,
            encoding,
        });
        cursor = end;
    }
    if expected.values().any(|(_, count)| *count != 0) {
        return Err(ProtocolAdapterError::Serialization(
            "ContentRef marker is missing from the native template".into(),
        ));
    }
    if replacements
        .windows(2)
        .any(|pair| pair[0].end > pair[1].start)
    {
        return Err(ProtocolAdapterError::Serialization(
            "ContentRef template replacements overlap".into(),
        ));
    }
    let wire_len = replacements
        .iter()
        .try_fold(bytes.len() as u64, |length, replacement| {
            length
                .checked_sub((replacement.end - replacement.start) as u64)
                .and_then(|value| {
                    value.checked_add(match replacement.encoding {
                        ReplacementEncoding::JsonString
                        | ReplacementEncoding::RawJsonInJsonString => {
                            replacement.content.json_escaped_len()
                        }
                        ReplacementEncoding::RawJson => replacement.content.byte_len(),
                    })
                })
        })
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| {
            ProtocolAdapterError::Serialization("native request length overflow".into())
        })?;
    Ok(PreparedReplayTemplate {
        bytes: bytes.into(),
        replacements: replacements.into(),
        wire_len,
    })
}

pub(super) fn request_has_content_refs(request: &ModelRequestIRV1) -> bool {
    !request_content_refs(request, request.ingress_protocol).is_empty()
        || request
            .requested_reasoning
            .native_value
            .as_ref()
            .and_then(JsonValueExt::content_ref)
            .is_some()
}

#[derive(Clone, Debug)]
pub(crate) struct RequestedReplacement {
    pub(crate) content: ContentRef,
    pub(crate) encoding: ReplacementEncoding,
}

fn request_content_refs(
    request: &ModelRequestIRV1,
    target: IngressProtocol,
) -> Vec<RequestedReplacement> {
    let mut refs = Vec::new();
    for instruction in &request.instructions {
        for part in &instruction.content {
            collect_part_refs(part, target, &mut refs);
        }
    }
    for message in &request.messages {
        collect_string_ref(message.name.as_deref(), &mut refs);
        for part in &message.content {
            collect_part_refs(part, target, &mut refs);
        }
    }
    for history in request.responses_reasoning_history.values() {
        for value in history.native_fields.values() {
            collect_json_ref(value, ReplacementEncoding::RawJson, &mut refs);
        }
    }
    for tool in &request.tools {
        collect_string_ref(tool.description.as_deref(), &mut refs);
        if let Some(schema) = &tool.input_schema {
            collect_json_ref(schema, ReplacementEncoding::RawJson, &mut refs);
        }
        if let Some(format) = &tool.format {
            collect_json_ref(format, ReplacementEncoding::RawJson, &mut refs);
        }
    }
    for namespace in &request.tool_namespaces {
        if target == IngressProtocol::Responses {
            collect_string_ref(namespace.description.as_deref(), &mut refs);
        }
        for tool in &namespace.tools {
            collect_string_ref(tool.description.as_deref(), &mut refs);
            if let Some(schema) = &tool.input_schema {
                collect_json_ref(schema, ReplacementEncoding::RawJson, &mut refs);
            }
            if let Some(format) = &tool.format {
                collect_json_ref(format, ReplacementEncoding::RawJson, &mut refs);
            }
        }
    }
    for state in &request.provider_state {
        collect_json_ref(&state.value, ReplacementEncoding::RawJson, &mut refs);
    }
    refs
}

fn collect_part_refs(
    part: &ContentPart,
    target: IngressProtocol,
    refs: &mut Vec<RequestedReplacement>,
) {
    match part {
        ContentPart::Text { text } => collect_string_ref(Some(text), refs),
        ContentPart::Image {
            source: ImageSource::Url { url },
        } => collect_string_ref(Some(url), refs),
        ContentPart::Image {
            source: ImageSource::Base64 { data, .. },
        } => collect_string_ref(Some(data), refs),
        ContentPart::ToolCall {
            tool_kind,
            arguments,
            ..
        } => {
            collect_json_ref(
                arguments,
                match (tool_kind, target) {
                    (ToolKindV1::Custom, IngressProtocol::Responses) => {
                        ReplacementEncoding::RawJson
                    }
                    (ToolKindV1::Custom, IngressProtocol::ChatCompletions) => {
                        ReplacementEncoding::RawJsonInJsonString
                    }
                    (_, IngressProtocol::Messages) => ReplacementEncoding::RawJson,
                    _ => ReplacementEncoding::JsonString,
                },
                refs,
            );
        }
        ContentPart::ToolResult {
            output: ToolOutput::Text(value),
            ..
        } => collect_string_ref(Some(value), refs),
        ContentPart::ToolResult {
            output: ToolOutput::Json(value),
            ..
        } => collect_json_ref(value, ReplacementEncoding::JsonString, refs),
        ContentPart::ProviderState { state } => {
            collect_json_ref(&state.value, ReplacementEncoding::RawJson, refs);
        }
    }
}

fn collect_string_ref(value: Option<&str>, refs: &mut Vec<RequestedReplacement>) {
    if let Some(content) = value.and_then(ContentValueExt::content_ref) {
        refs.push(RequestedReplacement {
            content,
            encoding: ReplacementEncoding::JsonString,
        });
    }
}

fn collect_json_ref(
    value: &serde_json::Value,
    encoding: ReplacementEncoding,
    refs: &mut Vec<RequestedReplacement>,
) {
    if let Some(content) = value.content_ref() {
        refs.push(RequestedReplacement { content, encoding });
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
