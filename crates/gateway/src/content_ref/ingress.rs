use std::collections::HashSet;
use std::io::{BufReader, Read};

use hiroute_gateway_core::runtime::body::{
    ChargedBytes, ChargedBytesBuilder, MemoryRole, Reservation,
};
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, de};
use serde_json::Value;

use crate::replay::{ReplayContentWriter, ReplayError, ReplayReader, ReplayStore};
use crate::server::request_plan::IngressProtocol;

use super::MARKER_PREFIX;
mod scanner;
use scanner::StructuralScanner;

const MAX_JSON_DEPTH: usize = 128;
const INLINE_CONTENT_BYTES: usize = 8 * 1024;
const STRUCTURE_RESERVATION_BYTES: usize = 128;
const MAX_SLICE_PARSE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IngressDocumentStats {
    pub(crate) structure_units: usize,
    pub(crate) max_depth: usize,
    pub(crate) max_string_bytes: usize,
}

/// Retention bound for an already serde-validated inline JSON tree and its
/// encoded bytes. Sizing uncertainty disables optional reuse, never admission.
pub(crate) fn inline_document_retention_bytes(bytes: &[u8]) -> Option<usize> {
    let mut scanner = StructuralScanner::default();
    scanner.feed(bytes).ok()?;
    let stats = scanner.finish().ok()?;
    stats
        .structure_units
        .checked_mul(STRUCTURE_RESERVATION_BYTES)?
        .checked_add(bytes.len().checked_mul(4)?)?
        .checked_add(4096)
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

    pub(crate) fn native_literals(
        &self,
        document: &Value,
        replay: &ReplayStore,
    ) -> Result<super::NativeLiteralNodes, ReplayError> {
        super::NativeLiteralNodes::capture(document, &self.generated_markers, replay)
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
    let mut state = IngressParserState {
        replay,
        pool: None,
        literal_pool: None,
        retained_strings: Vec::new(),
        generated_markers: HashSet::new(),
        inline_remaining: INLINE_CONTENT_BYTES,
        failure: None,
    };
    let mut document = if let Some(bytes) = memory_parse_input(replay, raw, stats)? {
        let mut decoder = serde_json::Deserializer::from_slice(bytes.bytes());
        decode_document(&mut decoder, &mut state)?
    } else {
        // Disk and budget-constrained requests keep the bounded reader path.
        let capacity = raw.byte_len().min(8 * 1024) as usize;
        let _buffer = replay
            .budget()
            .reserve(MemoryRole::ModelIrBacking, capacity)?;
        let mut reader = BufReader::with_capacity(capacity, replay.reader(raw)?);
        let mut decoder = serde_json::Deserializer::from_reader(&mut reader);
        let document = decode_document(&mut decoder, &mut state)?;
        drop(decoder);
        reader.get_mut().verify_terminal()?;
        document
    };
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

fn decode_document<'de, R: serde_json::de::Read<'de>>(
    decoder: &mut serde_json::Deserializer<R>,
    state: &mut IngressParserState<'_>,
) -> Result<Value, ReplayError> {
    let parsed = IngressSeed {
        state,
        field: None,
        depth: 0,
    }
    .deserialize(&mut *decoder);
    parsed
        .and_then(|value| {
            decoder.end()?;
            Ok(value)
        })
        .map_err(|_| state.failure.take().unwrap_or(ReplayError::InvalidJson))
}

fn memory_parse_input(
    replay: &ReplayStore,
    raw: &super::ContentRef,
    stats: IngressDocumentStats,
) -> Result<Option<ChargedBytes>, ReplayError> {
    let length = usize::try_from(raw.byte_len()).map_err(|_| ReplayError::LengthOverflow)?;
    if length > MAX_SLICE_PARSE_BYTES || replay.snapshot().disk_backed {
        return Ok(None);
    }
    // Leave conservative room for semantic backing, range metadata, retained
    // strings and the next Replay segment. Scratch refusal is not an error.
    let headroom = length
        .saturating_mul(3)
        .saturating_add(stats.structure_units.saturating_mul(256))
        .saturating_add(64 * 1024);
    if replay.budget().available_bytes()? < headroom {
        return Ok(None);
    }
    let Ok(mut bytes) =
        ChargedBytesBuilder::new(replay.budget(), MemoryRole::ModelIrBacking, length)
    else {
        return Ok(None);
    };
    bytes.resize_zeroed(length)?;
    let mut reader = replay.reader(raw)?;
    reader.read_exact(bytes.as_mut_slice())?;
    reader.verify_terminal()?;
    Ok(Some(bytes.finish()))
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
            if let Some(previous) = object.insert(key, value) {
                forget_generated(&previous, &mut state.generated_markers);
            }
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
    if let Some(object) = value.as_object()
        && object.len() == 1
        && (object.contains_key(super::JSON_MARKER_KEY)
            || object.contains_key(super::LITERAL_WIRE_KEY)
            || object.contains_key(super::WIRE_STRING_PARTS_KEY))
    {
        let trusted = object
            .get(super::JSON_MARKER_KEY)
            .and_then(Value::as_str)
            .is_some_and(|text| generated.contains(&(text.as_ptr() as usize)));
        if trusted {
            return Ok(());
        }
        let reference = super::literal_json::append_literal_json(value, generated, replay, pool)?;
        forget_generated(value, generated);
        *value = reference.json_marker();
        let marker = value[super::JSON_MARKER_KEY]
            .as_str()
            .ok_or(ReplayError::Integrity)?;
        generated.insert(marker.as_ptr() as usize);
        return Ok(());
    }
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
    // Validate with the same number/string semantics as serde_json::Value,
    // without materializing a second copy of a large tool argument.
    match decode_json_argument::<ValidModelJson>(replay, reference, stats) {
        Ok(_) => Ok(true),
        Err(ReplayError::InvalidJson) => Ok(false),
        Err(error) => Err(error),
    }
}

fn decode_json_argument<T: serde::de::DeserializeOwned>(
    replay: &ReplayStore,
    reference: &super::ContentRef,
    stats: IngressDocumentStats,
) -> Result<T, ReplayError> {
    // Reuse ingress's charged, optional small-memory path. Disk and pressure
    // keep the same bounded Reader, syntax/end check and terminal verification.
    if let Some(bytes) = memory_parse_input(replay, reference, stats)? {
        let mut decoder = serde_json::Deserializer::from_slice(bytes.bytes());
        return T::deserialize(&mut decoder)
            .and_then(|value| decoder.end().map(|()| value))
            .map_err(|_| ReplayError::InvalidJson);
    }
    let source = replay.reader(reference)?;
    let capacity = reference.byte_len().min(8 * 1024) as usize;
    let buffer = replay
        .budget()
        .reserve(MemoryRole::ModelIrBacking, capacity)
        .ok();
    let mut reader = BufReader::with_capacity(if buffer.is_some() { capacity } else { 0 }, source);
    let mut decoder = serde_json::Deserializer::from_reader(&mut reader);
    let parsed = T::deserialize(&mut decoder).and_then(|value| decoder.end().map(|()| value));
    drop(decoder);
    reader.get_mut().verify_terminal()?;
    parsed.map_err(|_| ReplayError::InvalidJson)
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

mod compactor;
#[cfg(test)]
pub(crate) use compactor::compact_ingress_document;
pub(crate) use compactor::compact_ingress_document_with_markers;

fn forget_generated(value: &Value, generated: &mut HashSet<usize>) {
    match value {
        Value::String(text) => {
            generated.remove(&(text.as_ptr() as usize));
        }
        Value::Array(values) => values
            .iter()
            .for_each(|value| forget_generated(value, generated)),
        Value::Object(values) => values
            .values()
            .for_each(|value| forget_generated(value, generated)),
        _ => {}
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
            && matches!(field, "conversation" | "previous_response_id"))
        // Messages thinking/output_config and Responses reasoning contain
        // controls decoded or merged by name. Keep their object structure
        // visible; replacing it with a Replay marker invents a native extension.
        // Large content leaves still use the existing request-local backing.
        || (depth == 1
            && matches!(field, "thinking" | "output_config")
            && protocol != IngressProtocol::Messages)
        // Responses reasoning currently has only the bounded `effort` and
        // `summary` controls. Keep that small object visible so ingress can
        // validate and retain summary before the sealed Plan replaces effort.
        // Other protocols still use backing for their native reasoning shapes.
        || (depth == 1 && field == "reasoning" && protocol != IngressProtocol::Responses)
        || (protocol == IngressProtocol::ChatCompletions
            && depth > 1
            && field == "reasoning"
            && value.is_string())
        || (field == "reasoning_content" && !value.is_null())
}

#[cfg(test)]
#[path = "ingress/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ingress/parser_path_tests.rs"]
mod parser_path_tests;
