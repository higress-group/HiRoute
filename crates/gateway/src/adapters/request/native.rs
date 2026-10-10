//! Preserve native payloads; only routing and configured controls belong to us.
use super::*;

pub(super) fn controls(
    request: &ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
    reasoning: &ReasoningProfileCapability,
) -> Result<Value, ProtocolAdapterError> {
    let protocol = profile.capability.upstream_protocol;
    let mut body = Map::new();
    body.insert("model".into(), json!(profile.capability.native_model));
    body.insert("stream".into(), json!(request.stream));
    let cap = candidate_max_output(request, profile);
    if protocol == IngressProtocol::Messages && cap.is_none() {
        return Err(ContextProjectionError::UnknownLimit("Messages max_tokens").into());
    }
    if let Some(cap) = cap {
        let key = match protocol {
            IngressProtocol::Responses => "max_output_tokens",
            IngressProtocol::ChatCompletions => "max_completion_tokens",
            IngressProtocol::Messages => "max_tokens",
        };
        body.insert(key.into(), json!(cap));
    }
    render_reasoning(&mut body, reasoning, protocol)?;
    Ok(Value::Object(body))
}

pub(super) fn project(
    request: &ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
    canonical: Value,
    omit_prefix: Option<usize>,
) -> Result<Value, ProtocolAdapterError> {
    if request.ingress_protocol != profile.capability.upstream_protocol {
        if request.native_only {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "native request extensions have no cross-protocol mapping".into(),
            ));
        }
        let mut canonical = canonical;
        super::super::structured_output::project_messages_format(
            request,
            profile.capability.upstream_protocol,
            &mut canonical,
        );
        return Ok(canonical);
    }
    let Some(original) = &request.native_body else {
        return Ok(canonical);
    };
    let mut body = original.clone();
    let object = body.as_object_mut().ok_or(ModelIrError::ExpectedObject)?;
    // Use one current Chat output-cap spelling, never contradictory limits.
    if request.ingress_protocol == IngressProtocol::ChatCompletions {
        object.remove("max_tokens");
    }
    for key in [
        "model",
        "stream",
        "max_output_tokens",
        "max_tokens",
        "max_completion_tokens",
    ] {
        if let Some(value) = canonical.get(key) {
            object.insert(key.into(), value.clone());
        }
    }
    if request.requested_reasoning.disposition
        != RequestedReasoningDisposition::AppliedToFixedBinding
    {
        // Plan controls own effort/budget, not unrelated native siblings such as
        // output formatting. Start each attempt from the immutable original tree.
        for (root, fields) in [
            ("reasoning", &["effort"][..]),
            ("thinking", &["type", "budget_tokens"][..]),
            ("output_config", &["effort"][..]),
        ] {
            if let Some(native) = object.get_mut(root).and_then(Value::as_object_mut) {
                for field in fields {
                    native.remove(*field);
                }
            }
            if let Some(configured) = canonical.get(root).and_then(Value::as_object) {
                let native = object.entry(root).or_insert_with(|| json!({}));
                let native = native
                    .as_object_mut()
                    .ok_or(ModelIrError::InvalidField("reasoning control"))?;
                for field in fields {
                    if let Some(value) = configured.get(*field) {
                        native.insert((*field).into(), value.clone());
                    }
                }
            }
            if object
                .get(root)
                .and_then(Value::as_object)
                .is_some_and(Map::is_empty)
            {
                object.remove(root);
            }
        }
        object.remove("reasoning_effort");
        if let Some(value) = canonical.get("reasoning_effort") {
            object.insert("reasoning_effort".into(), value.clone());
        }
        // Profiles can also own provider controls such as enable_thinking.
        // Reuse the existing assignment renderer, preserving unrelated siblings.
        let reasoning = profile.selected_reasoning()?;
        for field in reasoning.render.fields() {
            remove_assignment(object, &field.path);
        }
        render_reasoning(object, reasoning, request.ingress_protocol)?;
        if request.ingress_protocol == IngressProtocol::Messages {
            normalize_messages_context_edits(object);
        }
    }

    // Preserve the existing Claude instruction-reminder normalization.
    if request.ingress_protocol == IngressProtocol::Messages
        && object
            .get("messages")
            .and_then(Value::as_array)
            .is_some_and(|messages| {
                messages
                    .iter()
                    .any(|m| matches!(m["role"].as_str(), Some("system" | "developer")))
            })
    {
        // Keep blocks separate: concatenating Replay markers into one string
        // loses their exact replacement boundaries and rejects large histories.
        let mut system = Vec::new();
        append_instruction_blocks(&mut system, object.get("system"))?;
        let messages = object
            .get_mut("messages")
            .and_then(Value::as_array_mut)
            .expect("checked messages");
        for message in messages
            .iter()
            .filter(|m| matches!(m["role"].as_str(), Some("system" | "developer")))
        {
            append_instruction_blocks(&mut system, message.get("content"))?;
        }
        messages.retain(|m| !matches!(m["role"].as_str(), Some("system" | "developer")));
        object.insert("system".into(), Value::Array(system));
    }
    if let Some(end) = omit_prefix {
        if request.native_only {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "unknown history cannot be cleaned".into(),
            ));
        }
        clean_prefix(&mut body, request.ingress_protocol, end);
    }
    Ok(body)
}

fn normalize_messages_context_edits(object: &mut Map<String, Value>) {
    if matches!(
        object
            .get("thinking")
            .and_then(|value| value.get("type"))
            .and_then(Value::as_str),
        Some("enabled" | "adaptive")
    ) {
        return;
    }
    let Some(context) = object
        .get_mut("context_management")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let Some(edits) = context.get_mut("edits").and_then(Value::as_array_mut) else {
        return;
    };
    let original_len = edits.len();
    edits
        .retain(|edit| edit.get("type").and_then(Value::as_str) != Some("clear_thinking_20251015"));
    if edits.len() == original_len {
        return;
    }
    if edits.is_empty() {
        context.remove("edits");
    }
    if context.is_empty() {
        object.remove("context_management");
    }
}

fn append_instruction_blocks(
    blocks: &mut Vec<Value>,
    value: Option<&Value>,
) -> Result<(), ProtocolAdapterError> {
    match value {
        None => (),
        Some(Value::String(text)) => blocks.push(json!({"type":"text","text":text})),
        Some(Value::Array(parts)) => blocks.extend(parts.iter().cloned()),
        Some(_) => return Err(ModelIrError::InvalidField("instruction").into()),
    }
    Ok(())
}

fn remove_assignment(object: &mut Map<String, Value>, path: &[String]) {
    if let Some((head, tail)) = path.split_first() {
        if tail.is_empty() {
            object.remove(head);
        } else if let Some(child) = object.get_mut(head).and_then(Value::as_object_mut) {
            remove_assignment(child, tail);
        }
    }
}

fn clean_prefix(body: &mut Value, protocol: IngressProtocol, end: usize) {
    let key = if protocol == IngressProtocol::Responses {
        "input"
    } else {
        "messages"
    };
    let Some(items) = body.get_mut(key).and_then(Value::as_array_mut) else {
        return;
    };
    let mut index = 0usize;
    items.retain_mut(|item| {
        // Chat's leading instructions are outside canonical message indexes;
        // later instruction reminders are ordinary indexed history entries.
        if protocol == IngressProtocol::ChatCompletions
            && index == 0
            && matches!(item["role"].as_str(), Some("system" | "developer"))
        {
            return true;
        }
        if protocol == IngressProtocol::Responses && item["type"] == "additional_tools" {
            return true;
        }
        let clean = index < end;
        index += 1;
        if !clean {
            return true;
        }
        if protocol == IngressProtocol::Responses && item["type"] == "reasoning" {
            return false;
        }
        if let Some(object) = item.as_object_mut() {
            object.remove("reasoning_content");
        }
        if let Some(parts) = item.get_mut("content").and_then(Value::as_array_mut) {
            let original_len = parts.len();
            parts.retain(|part| {
                !matches!(
                    part["type"].as_str(),
                    Some("thinking" | "redacted_thinking")
                )
            });
            if protocol == IngressProtocol::Messages && original_len > 0 && parts.is_empty() {
                return false;
            }
        }
        true
    });
}
