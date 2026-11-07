//! Preserve native payloads; only routing and configured controls belong to us.
use crate::responses_reasoning::{ReasoningReplayAction, ReasoningReplayProjection};

use super::super::FunctionEncodingProjection;
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

pub(super) fn protect_projected_controls(
    request: &ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
    reasoning: &ReasoningProfileCapability,
    body: &mut Value,
) -> Result<(), ProtocolAdapterError> {
    let mut configured = controls(request, profile, reasoning)?;
    if request.native_body.is_some()
        && request.ingress_protocol == profile.capability.upstream_protocol
        && request.requested_reasoning.disposition
            == RequestedReasoningDisposition::AppliedToFixedBinding
    {
        // This branch retains the original native reasoning, including its
        // genuine Replay references. Only the model string was newly produced.
        configured
            .as_object_mut()
            .expect("controls are an object")
            .retain(|key, _| key == "model");
    }
    super::protect_control_literals(body, &configured);
    Ok(())
}

pub(super) fn project(
    request: &ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
    canonical: Value,
    omit_prefix: Option<usize>,
    reasoning_replay: ReasoningReplayProjection<'_>,
    unbacked: bool,
) -> Result<(Value, FunctionEncodingProjection, usize), ProtocolAdapterError> {
    // Ingress classifies this carrier before Replay externalization. Use the
    // retained canonical range, not presence or shifted indexes in a native tree.
    if super::super::responses_dialect::standard_json(profile)
        && request
            .messages
            .iter()
            .enumerate()
            .skip(omit_prefix.unwrap_or(0))
            .any(|(index, message)| {
                if reasoning_replay.action(index) == ReasoningReplayAction::Foreign {
                    return false;
                }
                request.responses_reasoning_history.get(&index).is_some_and(|history| {
                    history.encrypted_content == ResponsesReasoningEncryptedContentV1::Opaque
                }) || message.content.iter().any(|part| {
                    // Legacy canonical callers may have no reasoning sidecar.
                    matches!(part, ContentPart::ProviderState { state } if state.kind == "encrypted_content")
                })
            })
    {
        return Err(ModelIrError::ProviderStateNotPortable.into());
    }
    if request.ingress_protocol != profile.capability.upstream_protocol {
        if request.native_only {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "native request extensions have no cross-protocol mapping".into(),
            ));
        }
        let mut canonical = canonical;
        let negotiated = if request.ingress_protocol == IngressProtocol::Responses {
            super::super::responses_dialect::project_portable_tools(
                &mut canonical,
                profile.capability.upstream_protocol,
            )?
        } else {
            false
        };
        super::super::structured_output::project_messages_format(
            request,
            profile.capability.upstream_protocol,
            &mut canonical,
            unbacked,
        );
        normalize_codex_replay(&mut canonical, profile, request);
        return project_dialect(
            canonical,
            profile,
            if negotiated {
                FunctionEncodingProjection::Plaintext
            } else {
                FunctionEncodingProjection::Preserve
            },
            0,
        );
    }
    let Some(original) = &request.native_body else {
        let mut canonical = canonical;
        normalize_codex_replay(&mut canonical, profile, request);
        return project_dialect(canonical, profile, FunctionEncodingProjection::Preserve, 0);
    };
    let mut body = original.clone();
    if unbacked {
        crate::content_ref::preserve_unbacked_wire_envelopes(&mut body);
    }
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
        if request.ingress_protocol != IngressProtocol::ChatCompletions {
            object.remove("enable_thinking");
            if let Some(thinking) = object.get_mut("thinking").and_then(Value::as_object_mut) {
                thinking.remove("enabled");
            }
        }
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
            normalize_messages_thinking(
                object,
                request.requested_reasoning.messages_omit_thinking,
            )?;
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
    // Status sidecars use the original canonical indexes. Normalize before
    // prefix cleanup removes reasoning items and shifts later native positions.
    normalize_codex_replay(&mut body, profile, request);
    if omit_prefix.is_some() && request.native_only {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "unknown history cannot be cleaned".into(),
        ));
    }
    let removed = if omit_prefix.is_some() || !reasoning_replay.is_empty() {
        project_reasoning_history(
            &mut body,
            request,
            omit_prefix.unwrap_or(0),
            reasoning_replay,
        )
    } else {
        0
    };
    project_dialect(body, profile, FunctionEncodingProjection::Preserve, removed)
}

fn project_dialect(
    mut body: Value,
    profile: &CandidateProtocolProfile,
    mut projection: FunctionEncodingProjection,
    reasoning_removed: usize,
) -> Result<(Value, FunctionEncodingProjection, usize), ProtocolAdapterError> {
    if super::super::responses_dialect::standard_json(profile) {
        projection = super::super::responses_dialect::project_request(&mut body)?;
    }
    Ok((body, projection, reasoning_removed))
}

/// Codex subscription input rejects the output-only reasoning status, including
/// explicit null. Omit only completed or null: null carries no completion proof,
/// and unfinished states are never normalized into completed history. Keep the
/// original request and every other field; other endpoints retain native status.
fn normalize_codex_replay(
    body: &mut Value,
    profile: &CandidateProtocolProfile,
    request: &ModelRequestIRV1,
) {
    if profile.capability.upstream_protocol != IngressProtocol::Responses
        || profile.connector.connector_id != "connector.cpa.codex"
    {
        return;
    }
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };
    for (index, item) in input
        .iter_mut()
        // Tool declarations do not occupy canonical message positions.
        .filter(|item| item.get("type").and_then(Value::as_str) != Some("additional_tools"))
        .enumerate()
    {
        // Native leaves may already be Replay refs. Lifecycle admission retains
        // the bounded status beside the original canonical message; never inspect
        // a marker string as if it were the original status.
        let status = if request.ingress_protocol == IngressProtocol::Responses
            && request.native_body.is_some()
        {
            request
                .responses_item_statuses
                .get(&index)
                .map(String::as_str)
        } else {
            item.get("status").and_then(Value::as_str)
        };
        // JSON null cannot become a Replay marker. Unlike status strings it
        // remains directly distinguishable from an absent field after backing.
        let null_status = item.get("status").is_some_and(Value::is_null);
        if item.get("type").and_then(Value::as_str) == Some("reasoning")
            && (status == Some("completed") || null_status)
        {
            item.as_object_mut()
                .expect("typed item is an object")
                .remove("status");
        }
    }
}

fn normalize_messages_thinking(
    object: &mut Map<String, Value>,
    omit_thinking: bool,
) -> Result<(), ProtocolAdapterError> {
    let Some(thinking) = object.get_mut("thinking").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    match thinking.get("type").and_then(Value::as_str) {
        Some("enabled") => (),
        Some("adaptive") => {
            thinking.remove("budget_tokens");
        }
        None | Some("disabled") => {
            thinking.remove("budget_tokens");
            if thinking.contains_key("display") {
                if !omit_thinking {
                    return Err(ProtocolAdapterError::ClientUnrepresentable(
                        "thinking.display cannot be preserved with the selected Plan mode".into(),
                    ));
                }
                thinking.remove("display");
            }
        }
        Some(_) if thinking.contains_key("display") => {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "thinking.display has no mapping for the selected Plan mode".into(),
            ));
        }
        Some(_) => (),
    }
    if thinking.is_empty() {
        object.remove("thinking");
    } else if omit_thinking && !thinking.contains_key("type") {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "thinking extensions require a selected Plan thinking mode".into(),
        ));
    }
    Ok(())
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

fn project_reasoning_history(
    body: &mut Value,
    request: &ModelRequestIRV1,
    end: usize,
    reasoning_replay: ReasoningReplayProjection<'_>,
) -> usize {
    let protocol = request.ingress_protocol;
    let key = if protocol == IngressProtocol::Responses {
        "input"
    } else {
        "messages"
    };
    let Some(items) = body.get_mut(key).and_then(Value::as_array_mut) else {
        return 0;
    };
    let portable_layout = reasoning_replay
        .untracked_summary_ids()
        .then(|| request.native_history_layout())
        .flatten();
    let mut removed = 0;
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
        let position = index;
        index += 1;
        let clean = position < end;
        if protocol == IngressProtocol::Responses && item["type"] == "reasoning" {
            if clean {
                return false;
            }
            let Some(history) = request.responses_reasoning_history.get(&position) else {
                return true;
            };
            let remove_id = match reasoning_replay.action(position) {
                ReasoningReplayAction::Foreign => {
                    if history.encrypted_content == ResponsesReasoningEncryptedContentV1::Opaque {
                        removed += 1;
                        return false;
                    }
                    true
                }
                ReasoningReplayAction::PortableSummary => {
                    portable_layout
                        .and_then(|layout| layout.get(position))
                        .is_some_and(|item| item.modeled)
                        && portable_summary_reference(request, position, history)
                }
                ReasoningReplayAction::Preserve => false,
            };
            if remove_id
                && item
                    .as_object_mut()
                    .expect("typed reasoning item")
                    .remove("id")
                    .is_some()
            {
                removed += 1;
            }
            return true;
        }
        if !clean {
            return true;
        }
        if let Some(object) = item.as_object_mut() {
            object.remove("reasoning_content");
            if protocol == IngressProtocol::ChatCompletions {
                object.remove("reasoning");
            }
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
    removed
}

fn portable_summary_reference(
    request: &ModelRequestIRV1,
    position: usize,
    history: &ResponsesReasoningHistoryV1,
) -> bool {
    matches!(
        history.encrypted_content,
        ResponsesReasoningEncryptedContentV1::Absent | ResponsesReasoningEncryptedContentV1::Null
    ) && request
        .responses_item_ids
        .get(&position)
        .and_then(|id| id.strip_prefix("rs_"))
        .is_some_and(|suffix| !suffix.is_empty())
        && request
            .responses_item_statuses
            .get(&position)
            .is_none_or(|status| status == "completed")
        && history
            .native_fields
            .get("summary")
            .and_then(Value::as_array)
            .is_some_and(|summary| !summary.is_empty())
        && history.native_fields.get("content").is_none_or(|content| {
            content.is_null() || content.as_array().is_some_and(Vec::is_empty)
        })
}
