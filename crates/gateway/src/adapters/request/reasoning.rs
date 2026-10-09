//! Protocol-owned reasoning controls and supported released-profile normalization.
use super::*;
use crate::server::core_runtime::profiles::ReasoningControlKind;

pub(super) fn render_reasoning(
    body: &mut Map<String, Value>,
    reasoning: &ReasoningProfileCapability,
    protocol: IngressProtocol,
) -> Result<(), ProtocolAdapterError> {
    if reasoning
        .render
        .protocol()
        .is_some_and(|owner| owner != protocol)
    {
        return Err(invalid(
            "reasoning render belongs to another upstream protocol",
        ));
    }
    if protocol == IngressProtocol::Responses
        && matches!(reasoning.render, NativeReasoningRender::ExactBudget { .. })
    {
        return Err(invalid("Responses has no standard reasoning token budget"));
    }
    // Normalize the released toggle projection at the wire boundary. The stored
    // profile and its digest remain unchanged; explicitly selected effort does too.
    if reasoning.control_kind == ReasoningControlKind::Toggle
        && matches!(reasoning.profile_id.as_str(), "enabled" | "disabled")
        && protocol != IngressProtocol::ChatCompletions
    {
        let enabled = reasoning.profile_id == "enabled";
        match protocol {
            IngressProtocol::Responses => {
                insert_exact_field(
                    body,
                    &["reasoning".into(), "effort".into()],
                    json!(if enabled { "low" } else { "none" }),
                )?;
            }
            IngressProtocol::Messages => {
                insert_exact_field(
                    body,
                    &["thinking".into(), "type".into()],
                    json!(if enabled { "enabled" } else { "disabled" }),
                )?;
                if enabled {
                    insert_exact_field(
                        body,
                        &["thinking".into(), "budget_tokens".into()],
                        json!(1024),
                    )?;
                }
            }
            IngressProtocol::ChatCompletions => unreachable!(),
        }
        return complete_messages_thinking(body, protocol);
    }
    match &reasoning.render {
        NativeReasoningRender::NoControlParameter => {}
        NativeReasoningRender::ExactFields {
            protocol: owner,
            fields,
        }
        | NativeReasoningRender::ExactBudget {
            protocol: owner,
            fields,
            ..
        } if *owner == protocol => {
            for field in fields {
                insert_exact_field(body, &field.path, native_reasoning_value(&field.value))?;
            }
        }
        NativeReasoningRender::ExactFields { .. } | NativeReasoningRender::ExactBudget { .. } => {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "reasoning render belongs to another upstream protocol".into(),
            ));
        }
    }
    complete_messages_thinking(body, protocol)
}

fn native_reasoning_value(value: &NativeReasoningValue) -> Value {
    match value {
        NativeReasoningValue::Bool(value) => Value::Bool(*value),
        NativeReasoningValue::String(value) => Value::String(value.clone()),
        NativeReasoningValue::U64(value) => Value::from(*value),
    }
}

fn insert_exact_field(
    body: &mut Map<String, Value>,
    path: &[String],
    value: Value,
) -> Result<(), ProtocolAdapterError> {
    let Some((root, tail)) = path.split_first() else {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "reasoning field path is empty".into(),
        ));
    };
    if tail.is_empty() {
        if body.insert(root.clone(), value).is_some() {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "reasoning field collides with native request".into(),
            ));
        }
        return Ok(());
    }
    let root_value = body
        .entry(root.clone())
        .or_insert_with(|| Value::Object(Map::new()));
    let mut object = root_value.as_object_mut().ok_or_else(|| {
        ProtocolAdapterError::ClientUnrepresentable(
            "reasoning path collides with a non-object native field".into(),
        )
    })?;
    for part in &tail[..tail.len() - 1] {
        let child = object
            .entry(part.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        object = child.as_object_mut().ok_or_else(|| {
            ProtocolAdapterError::ClientUnrepresentable(
                "reasoning path collides with a non-object native field".into(),
            )
        })?;
    }
    if object.insert(tail[tail.len() - 1].clone(), value).is_some() {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "reasoning field is assigned more than once".into(),
        ));
    }
    Ok(())
}

fn invalid(message: &str) -> ProtocolAdapterError {
    ProtocolAdapterError::ClientUnrepresentable(message.into())
}

fn complete_messages_thinking(
    body: &mut Map<String, Value>,
    protocol: IngressProtocol,
) -> Result<(), ProtocolAdapterError> {
    if protocol != IngressProtocol::Messages {
        return Ok(());
    }
    let max_tokens = body.get("max_tokens").and_then(Value::as_u64);
    let Some(thinking) = body.get_mut("thinking").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    // Older budget profiles stored only budget_tokens. Adaptive is explicit and
    // must never acquire a manual budget from this normalization.
    if thinking.contains_key("budget_tokens") && !thinking.contains_key("type") {
        thinking.insert("type".into(), json!("enabled"));
    }
    match thinking.get("type").and_then(Value::as_str) {
        Some("enabled") => {
            let budget = thinking
                .entry("budget_tokens")
                .or_insert(json!(1024))
                .as_u64()
                .ok_or_else(|| invalid("Messages thinking budget must be an integer"))?;
            if budget < 1024 || max_tokens.is_none_or(|limit| budget >= limit) {
                return Err(invalid(
                    "Messages thinking budget must be at least 1024 and below max_tokens",
                ));
            }
        }
        Some("disabled") => {
            thinking.remove("budget_tokens");
        }
        _ => {}
    }
    Ok(())
}
