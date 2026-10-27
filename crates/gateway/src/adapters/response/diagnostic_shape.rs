//! Copy a closed shape from the decoder's existing parsed frame. No reparsing,
//! allocation, content hashes or protocol authority belongs to this diagnostic.

use hiroute_diagnostics::event::{ResponseEventKind as Kind, ResponseEventShape, ResponseItemKind};
use serde_json::Value;

pub(super) fn shape(event_type: Option<&str>, value: Option<&Value>) -> ResponseEventShape {
    let item = value.and_then(|value| value.get("item"));
    ResponseEventShape {
        event: kind(
            value
                .and_then(|value| value.get("type"))
                .and_then(Value::as_str),
        ),
        sse_event: event_type.map(|event| kind(Some(event))),
        output_index: value
            .and_then(|value| value.get("output_index"))
            .and_then(Value::as_u64),
        content_index: value
            .and_then(|value| value.get("content_index"))
            .and_then(Value::as_u64),
        summary_index: value
            .and_then(|value| value.get("summary_index"))
            .and_then(Value::as_u64),
        item: item.map(|item| match item.get("type").and_then(Value::as_str) {
            Some("message") => ResponseItemKind::Message,
            Some("reasoning") => ResponseItemKind::Reasoning,
            Some("function_call") => ResponseItemKind::FunctionCall,
            Some("custom_tool_call") => ResponseItemKind::CustomToolCall,
            Some("web_search_call") => ResponseItemKind::WebSearchCall,
            _ => ResponseItemKind::Unknown,
        }),
        content_parts: item
            .and_then(|item| item.get("content"))
            .and_then(Value::as_array)
            .map(|parts| parts.len() as u64),
        summary_parts: item
            .and_then(|item| item.get("summary"))
            .and_then(Value::as_array)
            .map(|parts| parts.len() as u64),
    }
}

fn kind(value: Option<&str>) -> Kind {
    match value {
        Some("response.created" | "response.in_progress") => Kind::ResponseStarted,
        Some("response.output_item.added") => Kind::OutputItemAdded,
        Some("response.output_item.done") => Kind::OutputItemDone,
        Some("response.content_part.added") => Kind::ContentPartAdded,
        Some("response.content_part.done") => Kind::ContentPartDone,
        Some("response.output_text.delta" | "response.refusal.delta") => Kind::TextDelta,
        Some("response.output_text.done" | "response.refusal.done") => Kind::TextDone,
        Some("response.reasoning_summary_part.added") => Kind::ReasoningPartAdded,
        Some("response.reasoning_summary_part.done") => Kind::ReasoningPartDone,
        Some("response.reasoning_text.delta" | "response.reasoning_summary_text.delta") => {
            Kind::ReasoningDelta
        }
        Some("response.reasoning_text.done" | "response.reasoning_summary_text.done") => {
            Kind::ReasoningDone
        }
        Some(
            "response.function_call_arguments.delta" | "response.custom_tool_call_input.delta",
        ) => Kind::ToolArgumentsDelta,
        Some("response.function_call_arguments.done" | "response.custom_tool_call_input.done") => {
            Kind::ToolArgumentsDone
        }
        Some("response.completed" | "response.incomplete" | "response.failed") => Kind::Terminal,
        Some("error") => Kind::Error,
        _ => Kind::Unknown,
    }
}
