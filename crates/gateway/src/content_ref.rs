use std::io::Write;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::replay::{ReplayContentWriter, ReplayError, ReplayStore};
use crate::server::core_runtime::model_ir::{
    ContentPart, ImageSource, ModelRequestIRV1, ToolOutput,
};

pub(crate) const MARKER_PREFIX: &str = "__hiroute_content_ref_v2_";
const MARKER_SUFFIX: &str = "__";
const JSON_MARKER_KEY: &str = "__hiroute_json_content_ref_v2";
pub const MAX_CONTENT_FIELDS: usize = 16_384;
const CONTENT_FIELD_METADATA_BYTES: usize = 192;

mod ingress;

pub(crate) use ingress::{compact_ingress_document, scan_ingress_document};

/// Stable, path-free locator for one range in the request's replay owner. It
/// deliberately carries no content digest, key, file name, or runtime session
/// identifier and is useful only together with that request-local owner.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentRef {
    stream_ordinal: u64,
    byte_offset: u64,
    byte_len: u64,
    json_escaped_len: u64,
}

impl ContentRef {
    pub(crate) fn new(
        stream_ordinal: u64,
        byte_offset: u64,
        byte_len: u64,
        json_escaped_len: u64,
    ) -> Self {
        Self {
            stream_ordinal,
            byte_offset,
            byte_len,
            json_escaped_len,
        }
    }

    pub fn stream_ordinal(&self) -> u64 {
        self.stream_ordinal
    }

    pub fn byte_len(&self) -> u64 {
        self.byte_len
    }

    pub fn byte_offset(&self) -> u64 {
        self.byte_offset
    }

    pub fn json_escaped_len(&self) -> u64 {
        self.json_escaped_len
    }

    /// Compact structural token held by the Model IR after the corresponding
    /// large string moves into Replay. The request-local owner validates the
    /// complete reference before opening a reader, so a forged token can only
    /// fail closed.
    pub(crate) fn wire_marker(&self) -> String {
        format!(
            "{MARKER_PREFIX}{}_{}_{}_{}{MARKER_SUFFIX}",
            self.stream_ordinal, self.byte_offset, self.byte_len, self.json_escaped_len
        )
    }

    pub(crate) fn from_wire_marker(value: &str) -> Option<Self> {
        let payload = value
            .strip_prefix(MARKER_PREFIX)?
            .strip_suffix(MARKER_SUFFIX)?;
        let mut fields = payload.split('_');
        let stream_ordinal = fields.next()?.parse().ok()?;
        let byte_offset = fields.next()?.parse().ok()?;
        let byte_len = fields.next()?.parse().ok()?;
        let json_escaped_len = fields.next()?.parse().ok()?;
        if fields.next().is_some() {
            return None;
        }
        Some(Self::new(
            stream_ordinal,
            byte_offset,
            byte_len,
            json_escaped_len,
        ))
    }

    pub(crate) fn json_marker(&self) -> Value {
        let mut marker = Map::new();
        marker.insert(JSON_MARKER_KEY.into(), Value::String(self.wire_marker()));
        Value::Object(marker)
    }
}

/// Preserve the existing public Model IR construction surface (`String`) while
/// representing externalized values as compact, stable ContentRef tokens.
/// This avoids forcing unrelated request producers to know replay internals.
pub type ContentValue = String;

pub(crate) trait ContentValueExt {
    fn content_ref(&self) -> Option<ContentRef>;
    fn wire_value(&self) -> String;
}

pub(crate) trait JsonValueExt {
    fn content_ref(&self) -> Option<ContentRef>;
    fn wire_value(&self) -> Value;
}

impl ContentValueExt for str {
    fn content_ref(&self) -> Option<ContentRef> {
        ContentRef::from_wire_marker(self)
    }

    fn wire_value(&self) -> String {
        self.to_owned()
    }
}

impl JsonValueExt for Value {
    fn content_ref(&self) -> Option<ContentRef> {
        let object = self.as_object()?;
        if object.len() != 1 {
            return None;
        }
        ContentRef::from_wire_marker(object.get(JSON_MARKER_KEY)?.as_str()?)
    }

    fn wire_value(&self) -> Value {
        self.content_ref().map_or_else(
            || self.clone(),
            |content| Value::String(content.wire_marker()),
        )
    }
}

/// Moves large canonical strings into the same request-owned replay backing.
/// The raw ingress stream is still available to the core body lifecycle until
/// EOS; it is released immediately afterwards, leaving only canonical refs for
/// long responses.
pub fn externalize_model_request(
    request: &mut ModelRequestIRV1,
    replay: &ReplayStore,
    inline_limit: usize,
) -> Result<(), ReplayError> {
    let field_count = content_field_count(request)?;
    if field_count > MAX_CONTENT_FIELDS {
        return Err(ReplayError::StructureLimit);
    }
    replay.charge_metadata(
        field_count
            .checked_mul(CONTENT_FIELD_METADATA_BYTES)
            .ok_or(ReplayError::LengthOverflow)?,
    )?;
    // This is one aggregate allowance, not a per-field threshold. Otherwise a
    // request can split a large prompt across many individually-small fields
    // and make every Attempt rebuild a full-size structural wire buffer.
    let mut inline_remaining = inline_limit;
    let mut pool = None;
    for instruction in &mut request.instructions {
        externalize_parts(
            &mut instruction.content,
            replay,
            &mut pool,
            &mut inline_remaining,
        )?;
    }
    for message in &mut request.messages {
        if let Some(name) = &mut message.name {
            externalize_value(name, replay, &mut pool, &mut inline_remaining)?;
        }
        externalize_parts(
            &mut message.content,
            replay,
            &mut pool,
            &mut inline_remaining,
        )?;
    }
    for history in request.responses_reasoning_history.values_mut() {
        for value in history.native_fields.values_mut() {
            externalize_json(value, replay, &mut pool, &mut inline_remaining, false)?;
        }
    }
    // Tool kind/name/namespace values are request-scoped identity keys. Keep
    // them directly comparable across declarations, choices, calls, results,
    // continuation bindings and candidate projections; independent replay
    // markers for equal strings are intentionally not identity-equivalent.
    for tool in &mut request.tools {
        if let Some(description) = &mut tool.description {
            externalize_value(description, replay, &mut pool, &mut inline_remaining)?;
        }
        for value in [&mut tool.input_schema, &mut tool.format]
            .into_iter()
            .flatten()
        {
            externalize_json(value, replay, &mut pool, &mut inline_remaining, false)?;
        }
    }
    for namespace in &mut request.tool_namespaces {
        if let Some(description) = &mut namespace.description {
            externalize_value(description, replay, &mut pool, &mut inline_remaining)?;
        }
        for tool in &mut namespace.tools {
            if let Some(description) = &mut tool.description {
                externalize_value(description, replay, &mut pool, &mut inline_remaining)?;
            }
            for value in [&mut tool.input_schema, &mut tool.format]
                .into_iter()
                .flatten()
            {
                externalize_json(value, replay, &mut pool, &mut inline_remaining, false)?;
            }
        }
    }
    if let Some(value) = &mut request.requested_reasoning.native_value {
        externalize_json(value, replay, &mut pool, &mut inline_remaining, false)?;
    }
    for state in &mut request.provider_state {
        externalize_json(
            &mut state.value,
            replay,
            &mut pool,
            &mut inline_remaining,
            false,
        )?;
    }
    if let Some(pool) = pool {
        pool.seal()?;
    }
    Ok(())
}

pub fn model_content_refs(request: &ModelRequestIRV1) -> Vec<ContentRef> {
    let mut refs = Vec::new();
    for instruction in &request.instructions {
        collect_part_content_refs(&instruction.content, &mut refs);
    }
    for message in &request.messages {
        if let Some(content) = message
            .name
            .as_deref()
            .and_then(ContentValueExt::content_ref)
        {
            refs.push(content);
        }
        collect_part_content_refs(&message.content, &mut refs);
    }
    for history in request.responses_reasoning_history.values() {
        for value in history.native_fields.values() {
            collect_json_content_ref(value, &mut refs);
        }
    }
    for tool in &request.tools {
        if let Some(description) = &tool.description {
            collect_string_content_ref(description, &mut refs);
        }
        if let Some(value) = &tool.input_schema {
            collect_json_content_ref(value, &mut refs);
        }
        if let Some(value) = &tool.format {
            collect_json_content_ref(value, &mut refs);
        }
    }
    for namespace in &request.tool_namespaces {
        if let Some(description) = &namespace.description {
            collect_string_content_ref(description, &mut refs);
        }
        for tool in &namespace.tools {
            if let Some(description) = &tool.description {
                collect_string_content_ref(description, &mut refs);
            }
            if let Some(value) = &tool.input_schema {
                collect_json_content_ref(value, &mut refs);
            }
            if let Some(value) = &tool.format {
                collect_json_content_ref(value, &mut refs);
            }
        }
    }
    if let Some(value) = &request.requested_reasoning.native_value {
        collect_json_content_ref(value, &mut refs);
    }
    for state in &request.provider_state {
        collect_json_content_ref(&state.value, &mut refs);
    }
    refs
}

fn content_field_count(request: &ModelRequestIRV1) -> Result<usize, ReplayError> {
    let mut count = 0_usize;
    for instruction in &request.instructions {
        count = count
            .checked_add(part_field_count(&instruction.content)?)
            .ok_or(ReplayError::LengthOverflow)?;
    }
    for message in &request.messages {
        let parts = part_field_count(&message.content)?;
        count = count
            .checked_add(usize::from(message.name.is_some()))
            .and_then(|value| value.checked_add(parts))
            .ok_or(ReplayError::LengthOverflow)?;
    }
    count = request
        .responses_reasoning_history
        .values()
        .try_fold(count, |count, history| {
            count
                .checked_add(history.native_fields.len())
                .ok_or(ReplayError::LengthOverflow)
        })?;
    for tool in &request.tools {
        count = count
            .checked_add(
                1 + usize::from(tool.description.is_some())
                    + usize::from(tool.input_schema.is_some())
                    + usize::from(tool.format.is_some()),
            )
            .ok_or(ReplayError::LengthOverflow)?;
    }
    for namespace in &request.tool_namespaces {
        count = count
            .checked_add(1 + usize::from(namespace.description.is_some()))
            .ok_or(ReplayError::LengthOverflow)?;
        for tool in &namespace.tools {
            count = count
                .checked_add(
                    1 + usize::from(tool.description.is_some())
                        + usize::from(tool.input_schema.is_some())
                        + usize::from(tool.format.is_some()),
                )
                .ok_or(ReplayError::LengthOverflow)?;
        }
    }
    count = count
        .checked_add(usize::from(matches!(
            request.tool_choice,
            crate::server::core_runtime::model_ir::ToolChoice::RequiredNamed { .. }
        )))
        .and_then(|value| {
            value.checked_add(usize::from(
                request.requested_reasoning.native_value.is_some(),
            ))
        })
        .and_then(|value| value.checked_add(request.provider_state.len()))
        .ok_or(ReplayError::LengthOverflow)?;
    Ok(count)
}

fn part_field_count(parts: &[ContentPart]) -> Result<usize, ReplayError> {
    parts.iter().try_fold(0_usize, |count, part| {
        count
            .checked_add(match part {
                ContentPart::ToolCall { namespace, .. } => 2 + usize::from(namespace.is_some()),
                ContentPart::ToolResult { namespace, .. } => 1 + usize::from(namespace.is_some()),
                ContentPart::Text { .. }
                | ContentPart::Image { .. }
                | ContentPart::ProviderState { .. } => 1,
            })
            .ok_or(ReplayError::LengthOverflow)
    })
}

fn collect_part_content_refs(parts: &[ContentPart], refs: &mut Vec<ContentRef>) {
    for part in parts {
        match part {
            ContentPart::Text { text } => collect_string_content_ref(text, refs),
            ContentPart::Image {
                source: ImageSource::Url { url },
            } => collect_string_content_ref(url, refs),
            ContentPart::Image {
                source: ImageSource::Base64 { data, .. },
            } => collect_string_content_ref(data, refs),
            ContentPart::ToolCall { arguments, .. } => collect_json_content_ref(arguments, refs),
            ContentPart::ToolResult {
                output: ToolOutput::Text(value),
                ..
            } => collect_string_content_ref(value, refs),
            ContentPart::ToolResult {
                output: ToolOutput::Json(value),
                ..
            } => collect_json_content_ref(value, refs),
            ContentPart::ProviderState { state } => {
                collect_json_content_ref(&state.value, refs);
            }
        }
    }
}

fn collect_string_content_ref(value: &str, refs: &mut Vec<ContentRef>) {
    if let Some(content) = value.content_ref() {
        refs.push(content);
    }
}

fn collect_json_content_ref(value: &Value, refs: &mut Vec<ContentRef>) {
    if let Some(content) = value.content_ref() {
        refs.push(content);
    }
}

fn externalize_parts(
    parts: &mut [ContentPart],
    replay: &ReplayStore,
    pool: &mut Option<ReplayContentWriter>,
    inline_remaining: &mut usize,
) -> Result<(), ReplayError> {
    for part in parts {
        match part {
            ContentPart::Text { text } => externalize_value(text, replay, pool, inline_remaining)?,
            ContentPart::Image {
                source: ImageSource::Url { url },
            } => externalize_value(url, replay, pool, inline_remaining)?,
            ContentPart::Image {
                source: ImageSource::Base64 { data, .. },
            } => externalize_value(data, replay, pool, inline_remaining)?,
            ContentPart::ToolCall { arguments, .. } => {
                externalize_json(arguments, replay, pool, inline_remaining, false)?
            }
            ContentPart::ToolResult {
                output: ToolOutput::Text(value),
                ..
            } => externalize_value(value, replay, pool, inline_remaining)?,
            ContentPart::ToolResult {
                output: ToolOutput::Json(value),
                ..
            } => externalize_json(value, replay, pool, inline_remaining, true)?,
            ContentPart::ProviderState { state } => {
                externalize_json(&mut state.value, replay, pool, inline_remaining, false)?;
            }
        }
    }
    Ok(())
}

fn externalize_value(
    value: &mut ContentValue,
    replay: &ReplayStore,
    pool: &mut Option<ReplayContentWriter>,
    inline_remaining: &mut usize,
) -> Result<(), ReplayError> {
    if ContentRef::from_wire_marker(value)
        .as_ref()
        .is_some_and(|reference| replay.has_reference(reference))
    {
        return Ok(());
    }
    // The marker namespace is reserved. Externalize marker-containing user
    // content even when small so it cannot alias a ref created for an earlier
    // field in the same request.
    if value.len() <= *inline_remaining && !value.contains(MARKER_PREFIX) {
        *inline_remaining -= value.len();
        return Ok(());
    }
    let content_ref = append_to_pool(replay, pool, value.as_bytes())?;
    *value = content_ref.wire_marker();
    Ok(())
}

fn externalize_json(
    value: &mut Value,
    replay: &ReplayStore,
    pool: &mut Option<ReplayContentWriter>,
    inline_remaining: &mut usize,
    canonical: bool,
) -> Result<(), ReplayError> {
    if value
        .content_ref()
        .as_ref()
        .is_some_and(|reference| replay.has_reference(reference))
    {
        return Ok(());
    }
    // JSON-bearing fields can be arbitrarily large. Serialize them directly
    // into the aggregate replay stream instead of first building an ordered
    // clone and a second full Vec. Scalar JSON stays inline while the shared
    // allowance remains; compound JSON is always externalized so Planner and
    // every candidate hold only a compact marker. The marker namespace is
    // reserved for scalar strings here too, for the same reason as ordinary
    // ContentValue fields above.
    if !value.is_array() && !value.is_object() {
        let encoded_len = serialized_json_len(value)?;
        let marker_free = value
            .as_str()
            .is_none_or(|string| !string.contains(MARKER_PREFIX));
        if encoded_len <= *inline_remaining && marker_free {
            *inline_remaining -= encoded_len;
            return Ok(());
        }
    }
    if pool.is_none() {
        *pool = Some(replay.begin_content_pool()?);
    }
    let reference = pool
        .as_mut()
        .ok_or(ReplayError::Integrity)?
        .append_json(value, canonical)?;
    *value = reference.json_marker();
    Ok(())
}

fn serialized_json_len(value: &Value) -> Result<usize, ReplayError> {
    let mut counter = CountingWriter(0);
    serde_json::to_writer(&mut counter, value).map_err(|_| ReplayError::InvalidJson)?;
    Ok(counter.0)
}

struct CountingWriter(usize);

impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("JSON length overflow"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn append_to_pool(
    replay: &ReplayStore,
    pool: &mut Option<ReplayContentWriter>,
    bytes: &[u8],
) -> Result<ContentRef, ReplayError> {
    if pool.is_none() {
        *pool = Some(replay.begin_content_pool()?);
    }
    pool.as_mut().ok_or(ReplayError::Integrity)?.append(bytes)
}
