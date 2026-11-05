use super::*;

#[path = "schema.rs"]
mod schema;

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
    compactor.walk(document, None, 0, WalkScope::Root)?;
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

#[derive(Clone, Copy)]
enum WalkScope {
    Root,
    ResponsesInput,
    ChatMessage,
    ChatToolCall,
    ChatFunction,
    JsonArgument,
    Other,
}

impl WalkScope {
    fn child(self, protocol: IngressProtocol, key: &str, item_type: Option<&str>) -> Self {
        match (self, protocol, key) {
            (Self::Root, IngressProtocol::Responses, "input") => Self::ResponsesInput,
            (Self::Root, IngressProtocol::ChatCompletions, "messages") => Self::ChatMessage,
            (Self::ResponsesInput, IngressProtocol::Responses, "arguments")
                if item_type == Some("function_call") =>
            {
                Self::JsonArgument
            }
            (Self::ChatMessage, IngressProtocol::ChatCompletions, "tool_calls") => {
                Self::ChatToolCall
            }
            (Self::ChatToolCall, IngressProtocol::ChatCompletions, "function")
                if matches!(item_type, None | Some("function")) =>
            {
                Self::ChatFunction
            }
            (Self::ChatFunction, IngressProtocol::ChatCompletions, "arguments") => {
                Self::JsonArgument
            }
            _ => Self::Other,
        }
    }
}

impl Compactor<'_> {
    fn walk(
        &mut self,
        value: &mut Value,
        field: Option<&str>,
        depth: usize,
        scope: WalkScope,
    ) -> Result<(), ReplayError> {
        if depth > MAX_JSON_DEPTH {
            return Err(ReplayError::StructureLimit);
        }
        if field.is_some_and(|field| whole_json_field(self.protocol, field, depth, value)) {
            if matches!(field, Some("parameters" | "input_schema")) {
                return self.schema(value, depth);
            }
            return self.externalize_json(value);
        }
        match value {
            Value::String(text) => {
                if self.generated_markers.contains(&(text.as_ptr() as usize)) {
                    if matches!(scope, WalkScope::JsonArgument) {
                        return self.canonicalize_generated_argument(text);
                    }
                    return Ok(());
                }
                if matches!(scope, WalkScope::JsonArgument) {
                    self.externalize_argument_string(text)
                } else if field.is_some_and(content_string_field) {
                    self.externalize_string(text)
                } else {
                    Ok(())
                }
            }
            Value::Array(values) => values
                .iter_mut()
                .try_for_each(|value| self.walk(value, field, depth + 1, scope)),
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
                        self.walk(
                            value,
                            Some(key),
                            depth + 1,
                            scope.child(self.protocol, key, item_type.as_deref()),
                        )?;
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
        let marker = reference.wire_marker();
        let mut compact = String::with_capacity(payload_start + marker.len());
        compact.push_str(&value[..payload_start]);
        compact.push_str(&marker);
        *value = compact;
        self.generated_markers.insert(value.as_ptr() as usize);
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
        self.generated_markers.insert(value.as_ptr() as usize);
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
                Ok(parsed) => self.pool()?.append_canonical_json(&parsed)?,
                Err(_) => return Ok(()),
            },
            Err(ReplayError::Body(_)) => self.pool()?.append(value.as_bytes())?,
            Err(error) => return Err(error),
        };
        *value = reference.wire_marker();
        self.generated_markers.insert(value.as_ptr() as usize);
        Ok(())
    }

    fn canonicalize_generated_argument(&mut self, value: &mut String) -> Result<(), ReplayError> {
        let reference = crate::content_ref::ContentRef::from_wire_marker(value)
            .ok_or(ReplayError::Integrity)?;
        let stats = scan_ingress_document(self.replay.reader(&reference)?)?;
        let _scratch = match stats.reserve_workspace(self.replay, reference.byte_len()) {
            Ok(scratch) => scratch,
            // Native tool arguments are already valid and backed by Replay.
            // Canonicalization only improves history equivalence; it must not
            // turn an otherwise valid large request into a 503.
            Err(ReplayError::Body(_)) => return Ok(()),
            Err(error) => return Err(error),
        };
        let parsed = match decode_json_argument::<Value>(self.replay, &reference, stats) {
            Ok(parsed) => parsed,
            // Syntax-only validation must never turn a native raw argument
            // into a request failure if Value rejects its numeric shape.
            Err(ReplayError::InvalidJson) => return Ok(()),
            Err(error) => return Err(error),
        };
        let canonical = self.pool()?.append_canonical_json(&parsed)?.wire_marker();
        self.generated_markers.remove(&(value.as_ptr() as usize));
        *value = canonical;
        self.generated_markers.insert(value.as_ptr() as usize);
        Ok(())
    }

    fn externalize_json(&mut self, value: &mut Value) -> Result<(), ReplayError> {
        // ContextHold hashes these request-local JSON ranges as canonical
        // bytes, independent of the caller's object insertion order.
        let reference = self.pool()?.append_canonical_json(value)?;
        forget_generated(value, self.generated_markers);
        *value = reference.json_marker();
        let marker = value
            .get(crate::content_ref::JSON_MARKER_KEY)
            .and_then(Value::as_str)
            .ok_or(ReplayError::Integrity)?;
        self.generated_markers.insert(marker.as_ptr() as usize);
        Ok(())
    }

    fn pool(&mut self) -> Result<&mut ReplayContentWriter, ReplayError> {
        if self.pool.is_none() {
            self.pool = Some(self.replay.begin_content_pool()?);
        }
        self.pool.as_mut().ok_or(ReplayError::Integrity)
    }
}
