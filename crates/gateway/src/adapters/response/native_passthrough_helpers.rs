use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::{
    IngressProtocol, ModelError, ModelIrError, NativeTerminalOutcome, ProjectionMetadata,
    ProtocolAdapterError,
};

impl ProjectionMetadata {
    pub(super) fn opaque() -> Self {
        Self {
            semantic: false,
            terminal: None,
            failure: None,
            source_bytes: 0,
        }
    }
}

pub(super) fn string_field<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, ProtocolAdapterError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ModelIrError::InvalidField(field).into())
}

pub(super) fn u32_field(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<u32, ProtocolAdapterError> {
    object
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| ModelIrError::InvalidField(field).into())
}

pub(super) fn object_field_mut<'a>(
    object: &'a mut Map<String, Value>,
    field: &'static str,
) -> Result<&'a mut Map<String, Value>, ProtocolAdapterError> {
    object
        .get_mut(field)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ModelIrError::InvalidField(field).into())
}

pub(super) fn responses_semantic_event(event: &str) -> bool {
    matches!(
        event,
        "response.output_text.delta"
            | "response.output_text.done"
            | "response.refusal.delta"
            | "response.refusal.done"
            | "response.reasoning_summary_text.delta"
            | "response.reasoning_summary_text.done"
            | "response.reasoning_text.delta"
            | "response.reasoning_text.done"
            | "response.function_call_arguments.delta"
            | "response.function_call_arguments.done"
            | "response.custom_tool_call_input.delta"
            | "response.custom_tool_call_input.done"
    )
}

pub(super) fn unowned_extension(protocol: IngressProtocol, event: &str) -> bool {
    match protocol {
        IngressProtocol::Responses => {
            !matches!(
                event,
                "response.created"
                    | "response.in_progress"
                    | "response.output_item.added"
                    | "response.output_item.done"
                    | "response.completed"
                    | "response.incomplete"
                    | "response.failed"
                    | "response.web_search_call.in_progress"
                    | "response.web_search_call.searching"
                    | "response.web_search_call.completed"
                    | "error"
            ) && !responses_semantic_event(event)
        }
        IngressProtocol::Messages => !matches!(
            event,
            "message_start"
                | "content_block_start"
                | "content_block_delta"
                | "content_block_stop"
                | "message_delta"
                | "message_stop"
                | "error"
        ),
        IngressProtocol::ChatCompletions => true,
    }
}

pub(super) fn digest_item(item: &Map<String, Value>) -> [u8; 32] {
    // A complete output_item.done is bounded by the SSE event limit. Hashing
    // it avoids retaining a second copy of the native response body.
    Sha256::digest(serde_json::to_vec(item).expect("JSON object serializes")).into()
}

pub(super) fn response_item_identity(item: &Map<String, Value>) -> Option<[u8; 32]> {
    let kind = item.get("type")?.as_str()?;
    let id = item.get("id")?.as_str()?;
    let mut digest = Sha256::new();
    digest.update(u32::try_from(kind.len()).ok()?.to_be_bytes());
    digest.update(kind.as_bytes());
    digest.update(id.as_bytes());
    Some(digest.finalize().into())
}

pub(super) fn responses_output_is_semantic(response: &Map<String, Value>) -> bool {
    response
        .get("output")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.as_object().is_some_and(response_item_is_semantic))
        })
}

pub(super) fn response_item_is_semantic(item: &Map<String, Value>) -> bool {
    match item.get("type").and_then(Value::as_str) {
        Some("function_call" | "custom_tool_call" | "web_search_call") => true,
        Some("message") => item
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|content| {
                content.iter().any(|block| {
                    block
                        .get("text")
                        .or_else(|| block.get("refusal"))
                        .and_then(Value::as_str)
                        .is_some_and(|value| !value.is_empty())
                })
            }),
        Some("reasoning") => {
            item.get("encrypted_content")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
                || item
                    .get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|content| {
                        content.iter().any(|part| {
                            part.get("text")
                                .and_then(Value::as_str)
                                .is_some_and(|text| !text.is_empty())
                        })
                    })
                || item
                    .get("summary")
                    .and_then(Value::as_array)
                    .is_some_and(|summary| {
                        summary.iter().any(|part| {
                            part.get("text")
                                .and_then(Value::as_str)
                                .is_some_and(|text| !text.is_empty())
                        })
                    })
        }
        _ => false,
    }
}

pub(super) fn message_block_is_semantic(block: &Map<String, Value>) -> bool {
    match block.get("type").and_then(Value::as_str) {
        Some("tool_use") => true,
        Some("text") => block
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty()),
        Some("thinking") => block
            .get("thinking")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty()),
        Some("redacted_thinking") => block
            .get("data")
            .and_then(Value::as_str)
            .is_some_and(|data| !data.is_empty()),
        _ => false,
    }
}

pub(super) fn message_delta_is_semantic(delta: &Map<String, Value>) -> bool {
    ["text", "thinking", "partial_json"]
        .into_iter()
        .any(|field| {
            delta
                .get(field)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
        })
}

pub(super) fn chat_delta_is_semantic(delta: &Map<String, Value>) -> bool {
    ["content", "refusal", "reasoning", "reasoning_content"]
        .into_iter()
        .any(|field| {
            delta
                .get(field)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
        })
        || delta
            .get("tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|calls| {
                calls.iter().any(|call| {
                    call.get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| !id.is_empty())
                        || call
                            .get("function")
                            .and_then(Value::as_object)
                            .is_some_and(|function| {
                                function
                                    .get("arguments")
                                    .and_then(Value::as_str)
                                    .is_some_and(|args| !args.is_empty())
                            })
                })
            })
}

pub(super) fn merge_terminal_description(
    previous: Option<NativeTerminalOutcome>,
    observed: &mut Option<[u8; 32]>,
    description: &str,
    next: NativeTerminalOutcome,
) -> NativeTerminalOutcome {
    // Keep just the description fingerprint, not an unbounded provider string.
    let digest: [u8; 32] = Sha256::digest(description.as_bytes()).into();
    let changed = observed.replace(digest).is_some_and(|old| old != digest);
    if changed {
        return NativeTerminalOutcome::Unknown;
    }
    match previous {
        None => next,
        Some(previous) if previous == next => previous,
        Some(_) => NativeTerminalOutcome::Unknown,
    }
}

pub(super) fn classify_messages_finish(reason: &str) -> NativeTerminalOutcome {
    match reason {
        "end_turn" | "stop_sequence" | "tool_use" => NativeTerminalOutcome::Complete,
        "max_tokens" | "refusal" => NativeTerminalOutcome::Incomplete,
        _ => NativeTerminalOutcome::Unknown,
    }
}

pub(super) fn classify_chat_finish(reason: &str) -> NativeTerminalOutcome {
    match reason {
        "stop" | "tool_calls" | "function_call" => NativeTerminalOutcome::Complete,
        "length" | "content_filter" => NativeTerminalOutcome::Incomplete,
        _ => NativeTerminalOutcome::Unknown,
    }
}

pub(super) fn u64_value(object: &Map<String, Value>, field: &str) -> Option<u64> {
    object.get(field).and_then(Value::as_u64)
}

pub(super) fn nested_u64_value(
    object: &Map<String, Value>,
    outer: &str,
    inner: &str,
) -> Option<u64> {
    object
        .get(outer)
        .and_then(Value::as_object)
        .and_then(|nested| nested.get(inner))
        .and_then(Value::as_u64)
}

pub(super) fn loose_error(value: &Value) -> ModelError {
    let object = value.as_object().and_then(|object| {
        object
            .get("error")
            .and_then(Value::as_object)
            .or(Some(object))
    });
    ModelError {
        status: None,
        code: object
            .and_then(|object| object.get("code").or_else(|| object.get("type")))
            .and_then(Value::as_str)
            .map(str::to_owned),
        message: object
            .and_then(|object| object.get("message"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        retryable: object
            .and_then(|object| object.get("retryable"))
            .and_then(Value::as_bool),
    }
}
