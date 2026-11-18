//! Client-executed Responses discovery. Its tool definitions take effect at the
//! result position and are forwarded in their native envelope.
use super::*;

fn require_client(object: &Map<String, Value>) -> Result<(), ModelIrError> {
    match object.get("execution").and_then(Value::as_str) {
        Some("client") => Ok(()),
        None if object.contains_key("execution") && !object["execution"].is_null() => {
            Err(ModelIrError::InvalidField("tool_search.execution"))
        }
        _ => Err(ModelIrError::UnsupportedValue(
            "non-client tool_search execution".into(),
        )),
    }
}

pub(super) fn declaration(
    context: &DecodeContext,
    object: &Map<String, Value>,
) -> Result<(), ModelIrError> {
    require_client(object)?;
    ensure_keys(
        context,
        object,
        &["type", "execution", "description", "parameters"],
        "tool_search",
    )?;
    if object
        .get("description")
        .is_some_and(|value| !value.is_null() && !value.is_string())
        || object
            .get("parameters")
            .is_some_and(|value| !value.is_null() && !value.is_object())
    {
        return Err(ModelIrError::InvalidField("tool_search declaration"));
    }
    context.client_tool_search.set(true);
    context.mark_modeled_native_only();
    Ok(())
}

pub(super) fn history(
    context: &DecodeContext,
    object: &Map<String, Value>,
    item_type: &str,
    status: Option<&str>,
) -> Result<NativeHandoffItem, ModelIrError> {
    require_client(object)?;
    if object
        .get("call_id")
        .and_then(Value::as_str)
        .is_none_or(|id| id.trim().is_empty())
    {
        return Err(ModelIrError::InvalidField("tool_search.call_id"));
    }
    let call = item_type == "tool_search_call";
    let payload = if call { "arguments" } else { "tools" };
    let before = context.unknown_generation.get();
    ensure_keys(
        context,
        object,
        &[
            "type",
            "id",
            "call_id",
            "execution",
            "status",
            payload,
            "internal_chat_message_metadata_passthrough",
        ],
        "tool_search history",
    )?;
    let complete =
        before == context.unknown_generation.get() && status.is_none_or(|s| s == "completed");
    let item = if call {
        if !object.get(payload).is_some_and(Value::is_object) {
            return Err(ModelIrError::InvalidField("tool_search_call.arguments"));
        }
        NativeHandoffItem::ToolSearchCall
    } else {
        let tools = object
            .get(payload)
            .and_then(Value::as_array)
            .ok_or(ModelIrError::InvalidField("tool_search_output.tools"))?;
        for tool in tools {
            validate_discovered(context, tool, true)?;
        }
        NativeHandoffItem::ToolSearchOutput
    };
    context.client_tool_search.set(true);
    context.mark_native_only();
    Ok(if complete {
        item
    } else {
        NativeHandoffItem::None
    })
}

/// Only inspect execution carriers here. Nullable descriptions, schema details
/// and other ordinary definition fields belong to the receiving provider.
fn validate_discovered(
    context: &DecodeContext,
    tool: &Value,
    allow_namespace: bool,
) -> Result<(), ModelIrError> {
    let object = tool
        .as_object()
        .ok_or(ModelIrError::InvalidField("tool_search_output.tools[]"))?;
    ensure_absent(object, &["execution"], "discovered client tool")?;
    let fields: &[&str] = match object.get("type").and_then(Value::as_str) {
        Some("function") => &[
            "type",
            "name",
            "description",
            "parameters",
            "strict",
            "defer_loading",
        ],
        Some("custom") => &["type", "name", "description", "format", "defer_loading"],
        Some("namespace") if allow_namespace => {
            let children = object
                .get("tools")
                .and_then(Value::as_array)
                .ok_or(ModelIrError::InvalidField("namespace tools"))?;
            for child in children {
                validate_discovered(context, child, false)?;
            }
            &["type", "name", "description", "tools"]
        }
        _ => {
            return Err(ModelIrError::UnsupportedValue(
                "non-client tool in tool_search_output".into(),
            ));
        }
    };
    if object
        .get("name")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err(ModelIrError::InvalidField("tool_search_output tool name"));
    }
    ensure_keys(context, object, fields, "discovered client tool")
}
