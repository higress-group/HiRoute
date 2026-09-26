use std::collections::HashSet;
use std::io::Read;

use hiroute_gateway_core::runtime::body::{MemoryRole, Reservation};
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, de};
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
    pub(crate) max_string_bytes: usize,
}

impl IngressDocumentStats {
    fn reserve_workspace(
        self,
        replay: &ReplayStore,
        raw_bytes: u64,
    ) -> Result<Reservation, ReplayError> {
        let raw_bytes = usize::try_from(raw_bytes).map_err(|_| ReplayError::LengthOverflow)?;
        // Canonicalizing a nested JSON string can briefly own its raw bytes,
        // a decoded Value, and structural nodes. If that cannot fit, the
        // original native argument string remains a valid Replay-backed value.
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

    pub(crate) fn reserve_streaming_workspace(
        self,
        replay: &ReplayStore,
    ) -> Result<Reservation, ReplayError> {
        // serde_json::from_reader may retain its largest decoded string in a
        // scratch buffer, not the entire request. Semantic strings move from
        // there to Replay; retained strings are charged separately.
        let bytes = self
            .structure_units
            .checked_mul(STRUCTURE_RESERVATION_BYTES)
            .and_then(|structure| self.max_string_bytes.checked_add(structure))
            .ok_or(ReplayError::LengthOverflow)?;
        Ok(replay.budget().reserve(MemoryRole::ModelIrBacking, bytes)?)
    }
}

pub(crate) struct IngressWorkspace {
    _retained_strings: Vec<Reservation>,
    generated_markers: HashSet<usize>,
}

impl IngressWorkspace {
    pub(crate) fn generated_markers(&mut self) -> &mut HashSet<usize> {
        &mut self.generated_markers
    }
}

/// Decode large semantic strings directly into the existing request-local
/// Replay owner. The serde parser remains the syntax authority; only content
/// fields are replaced, and the later protocol decoder still validates them.
pub(crate) fn parse_ingress_document(
    protocol: IngressProtocol,
    replay: &ReplayStore,
    raw: &super::ContentRef,
    stats: IngressDocumentStats,
) -> Result<(Value, IngressWorkspace), ReplayError> {
    let parse = stats.reserve_streaming_workspace(replay)?;
    let mut reader = replay.reader(raw)?;
    let mut state = IngressParserState {
        replay,
        pool: None,
        literal_pool: None,
        retained_strings: Vec::new(),
        generated_markers: HashSet::new(),
        inline_remaining: INLINE_CONTENT_BYTES,
        failure: None,
    };
    let mut deserializer = serde_json::Deserializer::from_reader(&mut reader);
    let parsed = IngressSeed {
        state: &mut state,
        field: None,
        depth: 0,
    }
    .deserialize(&mut deserializer);
    let mut document = match parsed.and_then(|value| {
        deserializer.end()?;
        Ok(value)
    }) {
        Ok(value) => value,
        Err(_) => return Err(state.failure.take().unwrap_or(ReplayError::InvalidJson)),
    };
    drop(deserializer);
    reader.verify_terminal()?;
    // serde's largest-string scratch is gone after the deserializer drops.
    // Restoration below may retain that string, but never overlaps the
    // parser's transient copy in the request memory budget.
    drop(parse);
    if let Some(pool) = state.pool.take() {
        pool.seal()?;
    }
    restore_special_subtrees(protocol, &mut document, &mut state, None, 0)?;
    if let Some(pool) = state.literal_pool.take() {
        pool.seal()?;
    }
    Ok((
        document,
        IngressWorkspace {
            _retained_strings: state.retained_strings,
            generated_markers: state.generated_markers,
        },
    ))
}

struct IngressParserState<'a> {
    replay: &'a ReplayStore,
    pool: Option<ReplayContentWriter>,
    literal_pool: Option<ReplayContentWriter>,
    retained_strings: Vec<Reservation>,
    generated_markers: HashSet<usize>,
    inline_remaining: usize,
    failure: Option<ReplayError>,
}

impl IngressParserState<'_> {
    fn string<E: de::Error>(&mut self, value: &str, externalize: bool) -> Result<Value, E> {
        if externalize && (value.len() > self.inline_remaining || value.contains(MARKER_PREFIX)) {
            if self.pool.is_none() {
                self.pool = Some(self.replay.begin_content_pool().map_err(|error| {
                    self.failure = Some(error);
                    E::custom("ingress content backing unavailable")
                })?);
            }
            let reference = self
                .pool
                .as_mut()
                .expect("content pool initialized")
                .append(value.as_bytes())
                .map_err(|error| {
                    self.failure = Some(error);
                    E::custom("ingress content backing unavailable")
                })?;
            let marker = reference.wire_marker();
            self.generated_markers.insert(marker.as_ptr() as usize);
            return Ok(Value::String(marker));
        }
        if externalize {
            self.inline_remaining = self.inline_remaining.saturating_sub(value.len());
        }
        let reservation = self
            .replay
            .budget()
            .reserve(MemoryRole::ModelIrBacking, value.len())
            .map_err(|error| {
                self.failure = Some(error.into());
                E::custom("ingress string budget unavailable")
            })?;
        self.retained_strings.push(reservation);
        Ok(Value::String(value.to_owned()))
    }
}

struct IngressSeed<'a, 'b> {
    state: &'a mut IngressParserState<'b>,
    field: Option<String>,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for IngressSeed<'_, '_> {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for IngressSeed<'_, '_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("invalid JSON number"))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
        let externalize = self.field.as_deref().is_some_and(|field| {
            content_string_field(field) || matches!(field, "image_url" | "url")
        });
        self.state.string(value, externalize)
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Value, E> {
        self.visit_str(&value)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Value, A::Error> {
        let IngressSeed {
            state,
            field,
            depth,
        } = self;
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(IngressSeed {
            state: &mut *state,
            field: field.clone(),
            depth: depth + 1,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let IngressSeed { state, depth, .. } = self;
        let mut object = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            let reservation = state
                .replay
                .budget()
                .reserve(MemoryRole::ModelIrBacking, key.len())
                .map_err(|error| {
                    state.failure = Some(error.into());
                    de::Error::custom("ingress field budget unavailable")
                })?;
            state.retained_strings.push(reservation);
            let value = map.next_value_seed(IngressSeed {
                state: &mut *state,
                field: Some(key.clone()),
                depth: depth + 1,
            })?;
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
}

fn restore_special_subtrees(
    protocol: IngressProtocol,
    value: &mut Value,
    state: &mut IngressParserState<'_>,
    field: Option<&str>,
    depth: usize,
) -> Result<(), ReplayError> {
    if matches!(field, Some("image_url" | "url")) {
        return restore_generated(
            value,
            state.replay,
            &mut state.generated_markers,
            &mut state.retained_strings,
        );
    }
    if field == Some("arguments")
        && let Value::String(text) = value
    {
        if state.generated_markers.contains(&(text.as_ptr() as usize)) {
            let reference =
                super::ContentRef::from_wire_marker(text).ok_or(ReplayError::Integrity)?;
            if !valid_json_argument(state.replay, &reference)? {
                restore_generated(
                    value,
                    state.replay,
                    &mut state.generated_markers,
                    &mut state.retained_strings,
                )?;
            }
        }
        return Ok(());
    }
    if field.is_some_and(|field| whole_json_field(protocol, field, depth, value)) {
        return restore_generated(
            value,
            state.replay,
            &mut state.generated_markers,
            &mut state.retained_strings,
        );
    }
    match value {
        Value::Object(object) => {
            let item_type = object
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if protocol == IngressProtocol::Messages
                && matches!(item_type.as_deref(), Some("thinking" | "redacted_thinking"))
            {
                // The decoder clones these native blocks into ProviderState.
                // Keep large thinking/signature/data leaves backed by Replay.
                for child in object.values_mut() {
                    isolate_native_literal_markers(
                        child,
                        state.replay,
                        &mut state.generated_markers,
                        &mut state.literal_pool,
                    )?;
                }
                return Ok(());
            }
            if protocol == IngressProtocol::Responses && item_type.as_deref() == Some("reasoning") {
                // Native reasoning history is copied into Model IR as well.
                // Preserve its semantic leaves as refs, including summaries.
                for child in object.values_mut() {
                    isolate_native_literal_markers(
                        child,
                        state.replay,
                        &mut state.generated_markers,
                        &mut state.literal_pool,
                    )?;
                }
                return Ok(());
            }
            for (key, child) in object {
                let whole_item = (item_type.as_deref() == Some("tool_use") && key == "input")
                    || (item_type.as_deref() == Some("tool_result")
                        && key == "content"
                        && !child.is_string())
                    || (item_type.as_deref() == Some("function_call_output")
                        && key == "output"
                        && !child.is_string());
                if whole_item {
                    restore_generated(
                        child,
                        state.replay,
                        &mut state.generated_markers,
                        &mut state.retained_strings,
                    )?;
                } else {
                    restore_special_subtrees(protocol, child, state, Some(key), depth + 1)?;
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                restore_special_subtrees(protocol, child, state, field, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn isolate_native_literal_markers(
    value: &mut Value,
    replay: &ReplayStore,
    generated: &mut HashSet<usize>,
    pool: &mut Option<ReplayContentWriter>,
) -> Result<(), ReplayError> {
    match value {
        Value::String(text)
            if text.contains(MARKER_PREFIX) && !generated.contains(&(text.as_ptr() as usize)) =>
        {
            if pool.is_none() {
                *pool = Some(replay.begin_content_pool()?);
            }
            let reference = pool
                .as_mut()
                .ok_or(ReplayError::Integrity)?
                .append(text.as_bytes())?;
            *text = reference.wire_marker();
            generated.insert(text.as_ptr() as usize);
        }
        Value::Array(values) => {
            for value in values {
                isolate_native_literal_markers(value, replay, generated, pool)?;
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                isolate_native_literal_markers(value, replay, generated, pool)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn restore_generated(
    value: &mut Value,
    replay: &ReplayStore,
    generated: &mut HashSet<usize>,
    retained: &mut Vec<Reservation>,
) -> Result<(), ReplayError> {
    match value {
        Value::String(text) if generated.remove(&(text.as_ptr() as usize)) => {
            let reference =
                super::ContentRef::from_wire_marker(text).ok_or(ReplayError::Integrity)?;
            let length =
                usize::try_from(reference.byte_len()).map_err(|_| ReplayError::LengthOverflow)?;
            let reservation = replay
                .budget()
                .reserve(MemoryRole::ModelIrBacking, length)?;
            let mut restored = String::new();
            replay.reader(&reference)?.read_to_string(&mut restored)?;
            *text = restored;
            retained.push(reservation);
        }
        Value::Object(object) => {
            for child in object.values_mut() {
                restore_generated(child, replay, generated, retained)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                restore_generated(child, replay, generated, retained)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn valid_json_argument(
    replay: &ReplayStore,
    reference: &super::ContentRef,
) -> Result<bool, ReplayError> {
    let stats = match scan_ingress_document(replay.reader(reference)?) {
        Ok(stats) => stats,
        Err(ReplayError::Integrity | ReplayError::StructureLimit) => return Ok(false),
        Err(error) => return Err(error),
    };
    let _scratch = match stats.reserve_streaming_workspace(replay) {
        Ok(scratch) => scratch,
        // A native argument remains usable on its original protocol even if
        // there is insufficient scratch to inspect its inner JSON here.
        Err(ReplayError::Body(_)) => return Ok(true),
        Err(error) => return Err(error),
    };
    let mut reader = replay.reader(reference)?;
    // Validate with the same number/string semantics as serde_json::Value,
    // without materializing a second copy of a large tool argument.
    let mut deserializer = serde_json::Deserializer::from_reader(&mut reader);
    let valid = ValidModelJson::deserialize(&mut deserializer)
        .and_then(|_| deserializer.end())
        .is_ok();
    drop(deserializer);
    reader.verify_terminal()?;
    Ok(valid)
}

struct ValidModelJson;

impl<'de> Deserialize<'de> for ValidModelJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ValidModelJson)
    }
}

impl<'de> Visitor<'de> for ValidModelJson {
    type Value = Self;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a serde_json::Value-compatible value")
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self, E> {
        Ok(self)
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self, E> {
        Ok(self)
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self, E> {
        Ok(self)
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self, E> {
        serde_json::Number::from_f64(value)
            .map(|_| self)
            .ok_or_else(|| E::custom("invalid JSON number"))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self, E> {
        Ok(self)
    }

    fn visit_none<E: de::Error>(self) -> Result<Self, E> {
        Ok(self)
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<Self, E> {
        Ok(self)
    }

    fn visit_string<E: de::Error>(self, _: String) -> Result<Self, E> {
        Ok(self)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self, A::Error> {
        while sequence.next_element::<Self>()?.is_some() {}
        Ok(self)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self, A::Error> {
        while map.next_entry::<ValidModelJsonKey, Self>()?.is_some() {}
        Ok(self)
    }
}

struct ValidModelJsonKey;

impl<'de> Deserialize<'de> for ValidModelJsonKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(ValidModelJsonKey)
    }
}

impl<'de> Visitor<'de> for ValidModelJsonKey {
    type Value = Self;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a JSON object key")
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<Self, E> {
        Ok(self)
    }

    fn visit_borrowed_str<E: de::Error>(self, _: &'de str) -> Result<Self, E> {
        Ok(self)
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
#[cfg(test)]
pub(crate) fn compact_ingress_document(
    protocol: IngressProtocol,
    document: &mut Value,
    replay: &ReplayStore,
) -> Result<(), ReplayError> {
    compact_ingress_document_with_markers(protocol, document, replay, &mut HashSet::new())
}

pub(crate) fn compact_ingress_document_with_markers(
    protocol: IngressProtocol,
    document: &mut Value,
    replay: &ReplayStore,
    generated_markers: &mut HashSet<usize>,
) -> Result<(), ReplayError> {
    let mut compactor = Compactor {
        protocol,
        replay,
        pool: None,
        inline_remaining: INLINE_CONTENT_BYTES,
        generated_markers,
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
    generated_markers: &'a mut HashSet<usize>,
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
                if self.generated_markers.contains(&(text.as_ptr() as usize)) {
                    if field == Some("arguments") && self.protocol != IngressProtocol::Messages {
                        return self.canonicalize_generated_argument(text);
                    }
                    return Ok(());
                }
                if field == Some("arguments") && self.protocol != IngressProtocol::Messages {
                    self.externalize_argument_string(text)
                } else if field.is_some_and(content_string_field) {
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
                // These native continuation blocks become one ProviderState
                // value (and Responses summary becomes one native JSON field)
                // after decode. Keep their bytes intact until Model IR moves
                // each complete value to Replay; leaf markers inside an opaque
                // block cannot be expanded by the upstream template.
                if (self.protocol == IngressProtocol::Responses
                    && item_type.as_deref() == Some("reasoning"))
                    || (self.protocol == IngressProtocol::Messages
                        && matches!(item_type.as_deref(), Some("thinking" | "redacted_thinking")))
                {
                    return Ok(());
                }
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
        if self.generated_markers.contains(&(value.as_ptr() as usize)) {
            return Ok(());
        }
        if value.len() <= self.inline_remaining && !value.contains(MARKER_PREFIX) {
            self.inline_remaining -= value.len();
            return Ok(());
        }
        let reference = self.pool()?.append(value.as_bytes())?;
        *value = reference.wire_marker();
        Ok(())
    }

    fn externalize_argument_string(&mut self, value: &mut String) -> Result<(), ReplayError> {
        if value.len() <= self.inline_remaining && !value.contains(MARKER_PREFIX) {
            self.inline_remaining -= value.len();
            return Ok(());
        }
        // Arguments are a native protocol string. A malformed string stays
        // inline for the adapter's raw-argument path. Canonicalize valid JSON
        // while its decoded Value fits the shared request budget; otherwise
        // retain exact native bytes rather than rejecting a usable request.
        let mut deserializer = serde_json::Deserializer::from_str(value);
        let valid = ValidModelJson::deserialize(&mut deserializer)
            .and_then(|_| deserializer.end())
            .is_ok();
        if !valid {
            return if value.contains(MARKER_PREFIX) {
                Err(ReplayError::InvalidJson)
            } else {
                Ok(())
            };
        }
        let mut scanner = StructuralScanner::default();
        scanner.feed(value.as_bytes())?;
        let stats = scanner.finish()?;
        let reference = match stats.reserve_workspace(self.replay, value.len() as u64) {
            Ok(_scratch) => match serde_json::from_str::<Value>(value) {
                Ok(parsed) => self.pool()?.append_json(&parsed, true)?,
                Err(_) => return Ok(()),
            },
            Err(ReplayError::Body(_)) => self.pool()?.append(value.as_bytes())?,
            Err(error) => return Err(error),
        };
        *value = reference.wire_marker();
        Ok(())
    }

    fn canonicalize_generated_argument(&mut self, value: &mut String) -> Result<(), ReplayError> {
        let reference = super::ContentRef::from_wire_marker(value).ok_or(ReplayError::Integrity)?;
        let stats = scan_ingress_document(self.replay.reader(&reference)?)?;
        let _scratch = match stats.reserve_workspace(self.replay, reference.byte_len()) {
            Ok(scratch) => scratch,
            // Native tool arguments are already valid and backed by Replay.
            // Canonicalization only improves history equivalence; it must not
            // turn an otherwise valid large request into a 503.
            Err(ReplayError::Body(_)) => return Ok(()),
            Err(error) => return Err(error),
        };
        let mut reader = self.replay.reader(&reference)?;
        let mut deserializer = serde_json::Deserializer::from_reader(&mut reader);
        let parsed = Value::deserialize(&mut deserializer).and_then(|value| {
            deserializer.end()?;
            Ok(value)
        });
        drop(deserializer);
        reader.verify_terminal()?;
        let Ok(parsed) = parsed else {
            // Syntax-only validation must never turn a native raw argument
            // into a request failure if Value rejects its numeric shape.
            return Ok(());
        };
        let canonical = self.pool()?.append_json(&parsed, true)?.wire_marker();
        self.generated_markers.remove(&(value.as_ptr() as usize));
        *value = canonical;
        self.generated_markers.insert(value.as_ptr() as usize);
        Ok(())
    }

    fn externalize_json(&mut self, value: &mut Value) -> Result<(), ReplayError> {
        // ContextHold hashes these request-local JSON ranges as canonical
        // bytes, independent of the caller's object insertion order.
        let reference = self.pool()?.append_json(value, true)?;
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
#[path = "ingress/tests.rs"]
mod tests;

#[derive(Default)]
struct StructuralScanner {
    structure_units: usize,
    depth: usize,
    max_depth: usize,
    current_string_bytes: usize,
    max_string_bytes: usize,
    in_string: bool,
    escaped: bool,
    unicode_digits: u8,
    unicode_value: u16,
    pending_high_surrogate: bool,
}

impl StructuralScanner {
    fn feed(&mut self, bytes: &[u8]) -> Result<(), ReplayError> {
        for &byte in bytes {
            if self.in_string {
                if self.unicode_digits > 0 {
                    self.unicode_value = self
                        .unicode_value
                        .wrapping_mul(16)
                        .wrapping_add(u16::from((byte as char).to_digit(16).unwrap_or(0) as u8));
                    self.unicode_digits -= 1;
                    if self.unicode_digits == 0 {
                        let code = self.unicode_value;
                        let width = if (0xd800..=0xdbff).contains(&code) {
                            self.pending_high_surrogate = true;
                            4
                        } else if (0xdc00..=0xdfff).contains(&code) && self.pending_high_surrogate {
                            self.pending_high_surrogate = false;
                            0
                        } else if code <= 0x7f {
                            1
                        } else if code <= 0x7ff {
                            2
                        } else {
                            3
                        };
                        self.add_string_bytes(width)?;
                    }
                    continue;
                }
                if self.escaped {
                    self.escaped = false;
                    if byte == b'u' {
                        self.unicode_digits = 4;
                        self.unicode_value = 0;
                    } else {
                        self.add_string_bytes(1)?;
                    }
                    continue;
                } else if byte == b'\\' {
                    self.escaped = true;
                    continue;
                } else if byte == b'"' {
                    self.in_string = false;
                    self.max_string_bytes = self.max_string_bytes.max(self.current_string_bytes);
                    continue;
                }
                self.add_string_bytes(1)?;
                continue;
            }
            match byte {
                b'"' => {
                    self.in_string = true;
                    self.current_string_bytes = 0;
                }
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

    fn add_string_bytes(&mut self, count: usize) -> Result<(), ReplayError> {
        self.current_string_bytes = self
            .current_string_bytes
            .checked_add(count)
            .ok_or(ReplayError::LengthOverflow)?;
        Ok(())
    }

    fn finish(self) -> Result<IngressDocumentStats, ReplayError> {
        if self.in_string || self.escaped || self.unicode_digits > 0 || self.depth != 0 {
            return Err(ReplayError::Integrity);
        }
        Ok(IngressDocumentStats {
            structure_units: self.structure_units,
            max_depth: self.max_depth,
            max_string_bytes: self.max_string_bytes,
        })
    }
}
