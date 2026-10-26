use serde_json::{Map, Value};

use crate::server::core_runtime::model_ir::{FinishReason, ModelError, ModelIrError, ModelUsage};

use super::ProtocolAdapterError;

pub(super) fn decode_responses_usage(value: &Value) -> Result<ModelUsage, ProtocolAdapterError> {
    let object = checked_object(value)?;
    allow(
        object,
        &[
            "input_tokens",
            "output_tokens",
            "total_tokens",
            "input_tokens_details",
            "output_tokens_details",
        ],
    )?;
    allow_optional_object(
        object,
        "input_tokens_details",
        &["cached_tokens", "cache_write_tokens"],
    )?;
    allow_optional_object(object, "output_tokens_details", &["reasoning_tokens"])?;
    validate_total(object, "input_tokens", "output_tokens")?;
    Ok(ModelUsage {
        input_tokens: optional_u64(object, "input_tokens")?,
        output_tokens: optional_u64(object, "output_tokens")?,
        reasoning_tokens: nested_u64(object, "output_tokens_details", "reasoning_tokens"),
        cache_read_tokens: nested_u64(object, "input_tokens_details", "cached_tokens"),
        cache_write_tokens: nested_u64(object, "input_tokens_details", "cache_write_tokens"),
    })
}

pub(super) fn decode_chat_usage(value: &Value) -> Result<ModelUsage, ProtocolAdapterError> {
    let object = checked_object(value)?;
    allow(
        object,
        &[
            "prompt_tokens",
            "completion_tokens",
            "total_tokens",
            "prompt_tokens_details",
            "completion_tokens_details",
        ],
    )?;
    allow_optional_object(
        object,
        "prompt_tokens_details",
        &["cached_tokens", "cache_write_tokens"],
    )?;
    allow_optional_object(object, "completion_tokens_details", &["reasoning_tokens"])?;
    validate_total(object, "prompt_tokens", "completion_tokens")?;
    Ok(ModelUsage {
        input_tokens: optional_u64(object, "prompt_tokens")?,
        output_tokens: optional_u64(object, "completion_tokens")?,
        cache_read_tokens: nested_u64(object, "prompt_tokens_details", "cached_tokens"),
        cache_write_tokens: nested_u64(object, "prompt_tokens_details", "cache_write_tokens"),
        reasoning_tokens: nested_u64(object, "completion_tokens_details", "reasoning_tokens"),
    })
}

pub(super) fn decode_messages_usage(value: &Value) -> Result<ModelUsage, ProtocolAdapterError> {
    let object = checked_object(value)?;
    allow(
        object,
        &[
            "input_tokens",
            "output_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
            "server_tool_use",
            "service_tier",
        ],
    )?;
    allow_optional_object(object, "server_tool_use", &["web_search_requests"])?;
    let _ = optional_str(object, "service_tier")?;
    let provider_input = optional_u64(object, "input_tokens")?;
    let cache_read = optional_u64(object, "cache_read_input_tokens")?;
    let cache_write = optional_u64(object, "cache_creation_input_tokens")?;
    // Anthropic reports uncached input separately from both cache buckets.
    // HiRoute's shared input dimension is the complete input consumed by the
    // request, while the two cache dimensions remain available independently.
    let input_tokens = provider_input
        .map(|input| {
            input
                .checked_add(cache_read.unwrap_or_default())
                .and_then(|total| total.checked_add(cache_write.unwrap_or_default()))
                .ok_or_else(|| {
                    ProtocolAdapterError::from(ModelIrError::InvalidField("input_tokens"))
                })
        })
        .transpose()?;
    Ok(ModelUsage {
        input_tokens,
        output_tokens: optional_u64(object, "output_tokens")?,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        reasoning_tokens: None,
    })
}

pub(super) fn validate_responses_envelope_metadata(
    object: &Map<String, Value>,
) -> Result<(), ProtocolAdapterError> {
    if object
        .get("metadata")
        .is_some_and(|value| !value.is_null() && !value.is_object())
    {
        return Err(ModelIrError::InvalidField("metadata").into());
    }
    let _ = optional_str(object, "service_tier")?;
    Ok(())
}

pub(super) fn decode_error(
    value: &Value,
    status: Option<u16>,
) -> Result<ModelError, ProtocolAdapterError> {
    let outer = checked_object(value)?;
    let error = if let Some(error) = outer.get("error") {
        allow(outer, &["type", "error"])?;
        checked_object(error)?
    } else {
        outer
    };
    allow(error, &["type", "code", "message", "param", "retryable"])?;
    let retryable = match error.get("retryable") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(value)) => Some(*value),
        Some(_) => return Err(ModelIrError::InvalidField("retryable").into()),
    };
    Ok(ModelError {
        status,
        code: error
            .get("code")
            .or_else(|| error.get("type"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        message: error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned),
        retryable,
    })
}

pub(super) fn decode_finish_reason(value: &str) -> FinishReason {
    match value {
        "stop" | "end_turn" | "completed" => FinishReason::Stop,
        "length" | "max_tokens" | "max_output_tokens" => FinishReason::Length,
        "tool_calls" | "tool_use" => FinishReason::ToolCall,
        "content_filter" | "refusal" => FinishReason::Refusal,
        "cancelled" => FinishReason::Cancelled,
        other => FinishReason::Other(other.into()),
    }
}

pub(super) fn parse_data(data: &[u8]) -> Result<Value, ProtocolAdapterError> {
    serde_json::from_slice(data)
        .map_err(|error| ModelIrError::InvalidJson(error.to_string()).into())
}

pub(super) fn checked_object(value: &Value) -> Result<&Map<String, Value>, ProtocolAdapterError> {
    value
        .as_object()
        .ok_or_else(|| ModelIrError::ExpectedObject.into())
}

pub(super) fn object_field<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a Map<String, Value>, ProtocolAdapterError> {
    object
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| ModelIrError::InvalidField(key).into())
}

pub(super) fn array<'a>(
    value: &'a Value,
    key: &'static str,
) -> Result<&'a Vec<Value>, ProtocolAdapterError> {
    value
        .as_array()
        .ok_or_else(|| ModelIrError::InvalidField(key).into())
}

pub(super) fn array_field<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
    optional: bool,
) -> Result<&'a [Value], ProtocolAdapterError> {
    match object.get(key) {
        Some(value) => Ok(array(value, key)?),
        None if optional => Ok(&[]),
        None => Err(ModelIrError::InvalidField(key).into()),
    }
}

pub(super) fn required_str<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a str, ProtocolAdapterError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ModelIrError::InvalidField(key).into())
}

pub(super) fn optional_str<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
) -> Result<Option<&'a str>, ProtocolAdapterError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(ModelIrError::InvalidField(key).into()),
    }
}

pub(super) fn required_u32(
    object: &Map<String, Value>,
    key: &'static str,
) -> Result<u32, ProtocolAdapterError> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| ModelIrError::InvalidField(key).into())
}

pub(super) fn optional_u64(
    object: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<u64>, ProtocolAdapterError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| ModelIrError::InvalidField(key).into()),
        Some(_) => Err(ModelIrError::InvalidField(key).into()),
    }
}

pub(super) fn allow(
    _object: &Map<String, Value>,
    _known_fields: &[&str],
) -> Result<(), ProtocolAdapterError> {
    // Provider responses are an external, additively evolving contract. The
    // listed fields document what HiRoute consumes; unknown sibling metadata
    // is tolerated. Required field types, event/content discriminators,
    // lifecycle ordering, identities, and explicitly unsupported semantic
    // values remain fail-closed at their individual decode sites.
    Ok(())
}

pub(super) fn unsupported(context: &str, value: &str) -> ProtocolAdapterError {
    ModelIrError::UnsupportedValue(format!("{context}: {value}")).into()
}

pub(super) fn reject_non_null(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<(), ProtocolAdapterError> {
    if object.get(field).is_some_and(|value| !value.is_null()) {
        Err(ModelIrError::UnsupportedField(field.into()).into())
    } else {
        Ok(())
    }
}

pub(super) fn reject_nonempty_array(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<(), ProtocolAdapterError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(()),
        Some(Value::Array(values)) if values.is_empty() => Ok(()),
        Some(Value::Array(_)) => Err(ModelIrError::UnsupportedField(field.into()).into()),
        Some(_) => Err(ModelIrError::InvalidField(field).into()),
    }
}

fn nested_u64(object: &Map<String, Value>, parent: &str, field: &str) -> Option<u64> {
    object
        .get(parent)
        .and_then(Value::as_object)
        .and_then(|details| details.get(field))
        .and_then(Value::as_u64)
}

fn allow_optional_object(
    object: &Map<String, Value>,
    field: &'static str,
    fields: &[&str],
) -> Result<(), ProtocolAdapterError> {
    if let Some(value) = object.get(field) {
        allow(checked_object(value)?, fields)?;
    }
    Ok(())
}

fn validate_total(
    object: &Map<String, Value>,
    input_field: &'static str,
    output_field: &'static str,
) -> Result<(), ProtocolAdapterError> {
    let Some(total) = optional_u64(object, "total_tokens")? else {
        return Ok(());
    };
    let input = optional_u64(object, input_field)?.unwrap_or_default();
    let output = optional_u64(object, output_field)?.unwrap_or_default();
    if input.checked_add(output) == Some(total) {
        Ok(())
    } else {
        Err(ModelIrError::InvalidField("total_tokens").into())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::decode_messages_usage;

    #[test]
    fn messages_usage_accepts_current_service_metadata_without_losing_tokens() {
        let usage = decode_messages_usage(&json!({
            "input_tokens": 17,
            "output_tokens": 44,
            "cache_read_input_tokens": 0,
            "server_tool_use": {"web_search_requests": 0},
            "service_tier": "standard"
        }))
        .unwrap();

        assert_eq!(usage.input_tokens, Some(17));
        assert_eq!(usage.output_tokens, Some(44));
        assert_eq!(usage.cache_read_tokens, Some(0));
    }

    #[test]
    fn messages_usage_normalizes_uncached_and_both_cache_buckets() {
        let usage = decode_messages_usage(&json!({
            "input_tokens": 300,
            "output_tokens": 44,
            "cache_read_input_tokens": 600,
            "cache_creation_input_tokens": 100
        }))
        .unwrap();

        assert_eq!(usage.input_tokens, Some(1000));
        assert_eq!(usage.cache_read_tokens, Some(600));
        assert_eq!(usage.cache_write_tokens, Some(100));
    }

    #[test]
    fn messages_usage_preserves_missing_base_and_explicit_zero() {
        let missing = decode_messages_usage(&json!({
            "cache_read_input_tokens": 9,
            "cache_creation_input_tokens": 3
        }))
        .unwrap();
        assert_eq!(missing.input_tokens, None);
        assert_eq!(missing.cache_read_tokens, Some(9));
        assert_eq!(missing.cache_write_tokens, Some(3));

        let zero = decode_messages_usage(&json!({
            "input_tokens": 0,
            "cache_read_input_tokens": 0,
            "cache_creation_input_tokens": 0
        }))
        .unwrap();
        assert_eq!(zero.input_tokens, Some(0));
    }

    #[test]
    fn messages_usage_rejects_total_input_overflow() {
        let error = decode_messages_usage(&json!({
            "input_tokens": u64::MAX,
            "cache_read_input_tokens": 1
        }))
        .unwrap_err();

        assert!(error.to_string().contains("input_tokens"));
    }
}
