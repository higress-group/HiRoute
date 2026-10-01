use serde_json::{Map, Value, json};

use crate::content_ref::JsonValueExt;
use crate::server::core_runtime::model_ir::{
    CanonicalTool, CanonicalToolNamespaceV1, ModelRequestIRV1, ResponsesToolOrderEntryV1,
    ToolChoice, ToolKindV1,
};
use crate::server::request_plan::IngressProtocol;

use super::{ChatToolProjection, ProtocolAdapterError};

pub(super) fn insert_responses_tools(
    body: &mut Map<String, Value>,
    request: &ModelRequestIRV1,
) -> Result<(), ProtocolAdapterError> {
    if request.tools.is_empty()
        && request.tool_namespaces.is_empty()
        && request.web_search.is_none()
    {
        return Ok(());
    }
    let tools = render_responses_tools_in_order(request)?;
    body.insert("tools".into(), Value::Array(tools));
    body.insert(
        "tool_choice".into(),
        render_tool_choice(IngressProtocol::Responses, &request.tool_choice),
    );
    body.insert(
        "parallel_tool_calls".into(),
        Value::Bool(request.parallel_tool_calls),
    );
    Ok(())
}

pub(super) fn insert_chat_tools(
    body: &mut Map<String, Value>,
    request: &ModelRequestIRV1,
    projection: &ChatToolProjection,
) -> Result<(), ProtocolAdapterError> {
    if projection.is_empty() {
        return Ok(());
    }
    body.insert(
        "tools".into(),
        Value::Array(projection.render_tools(request)?),
    );
    let choice = match projection.named_choice(&request.tool_choice)? {
        Some(name) => json!({"type": "function", "function": {"name": name}}),
        None => render_tool_choice(IngressProtocol::ChatCompletions, &request.tool_choice),
    };
    body.insert("tool_choice".into(), choice);
    body.insert(
        "parallel_tool_calls".into(),
        Value::Bool(request.parallel_tool_calls),
    );
    Ok(())
}

pub(super) fn insert_messages_tools(
    body: &mut Map<String, Value>,
    request: &ModelRequestIRV1,
) -> Result<(), ProtocolAdapterError> {
    if request.tools.is_empty() {
        return Ok(());
    }
    if request.tool_choice == ToolChoice::None {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "Messages cannot express Tool definitions with Tool choice none".into(),
        ));
    }
    if request
        .tools
        .iter()
        .any(|tool| tool.kind != ToolKindV1::Function)
    {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "Messages cannot express Responses custom tools".into(),
        ));
    }
    body.insert(
        "tools".into(),
        Value::Array(
            request
                .tools
                .iter()
                .map(render_messages_tool)
                .collect::<Result<Vec<_>, _>>()?,
        ),
    );
    body.insert(
        "tool_choice".into(),
        render_messages_tool_choice(&request.tool_choice, request.parallel_tool_calls),
    );
    Ok(())
}

fn render_tool_choice(protocol: IngressProtocol, choice: &ToolChoice) -> Value {
    match (protocol, choice) {
        (_, ToolChoice::None) => Value::String("none".into()),
        (IngressProtocol::Messages, ToolChoice::Auto) => json!({"type": "auto"}),
        (_, ToolChoice::Auto) => Value::String("auto".into()),
        (IngressProtocol::Messages, ToolChoice::RequiredAny) => json!({"type": "any"}),
        (_, ToolChoice::RequiredAny) => Value::String("required".into()),
        (IngressProtocol::Responses, ToolChoice::RequiredNamed { tool_kind, name }) => {
            json!({"type": tool_kind_label(*tool_kind), "name": name})
        }
        (IngressProtocol::ChatCompletions, ToolChoice::RequiredNamed { name, .. }) => {
            json!({"type": "function", "function": {"name": name}})
        }
        (IngressProtocol::Messages, ToolChoice::RequiredNamed { name, .. }) => {
            json!({"type": "tool", "name": name})
        }
    }
}

fn render_messages_tool_choice(choice: &ToolChoice, parallel: bool) -> Value {
    let mut value = match choice {
        ToolChoice::Auto => json!({"type": "auto"}),
        ToolChoice::RequiredAny => json!({"type": "any"}),
        ToolChoice::RequiredNamed { name, .. } => json!({"type": "tool", "name": name}),
        ToolChoice::None => unreachable!("Tool choice none is rejected before rendering"),
    };
    value
        .as_object_mut()
        .expect("tool choice renderer creates an object")
        .insert("disable_parallel_tool_use".into(), Value::Bool(!parallel));
    value
}

fn render_responses_tool(tool: &CanonicalTool) -> Result<Value, ProtocolAdapterError> {
    let mut object = Map::new();
    object.insert(
        "type".into(),
        Value::String(tool_kind_label(tool.kind).into()),
    );
    object.insert("name".into(), Value::String(tool.name.clone()));
    if let Some(description) = &tool.description {
        object.insert("description".into(), Value::String(description.clone()));
    }
    match tool.kind {
        ToolKindV1::Function => {
            if tool.format.is_some() {
                return Err(ProtocolAdapterError::ClientUnrepresentable(
                    "function tool cannot carry custom format".into(),
                ));
            }
            object.insert(
                "parameters".into(),
                tool.input_schema
                    .as_ref()
                    .ok_or_else(|| {
                        ProtocolAdapterError::ClientUnrepresentable(
                            "function tool requires an input schema".into(),
                        )
                    })?
                    .wire_value(),
            );
            if let Some(strict) = tool.strict {
                object.insert("strict".into(), Value::Bool(strict));
            }
        }
        ToolKindV1::Custom => {
            if tool.input_schema.is_some() || tool.strict.is_some() {
                return Err(ProtocolAdapterError::ClientUnrepresentable(
                    "custom tool cannot carry function schema or strict".into(),
                ));
            }
            if let Some(format) = &tool.format {
                object.insert("format".into(), format.wire_value());
            }
        }
    }
    Ok(Value::Object(object))
}

fn render_responses_namespace(
    namespace: &CanonicalToolNamespaceV1,
) -> Result<Value, ProtocolAdapterError> {
    let mut object = Map::new();
    object.insert("type".into(), Value::String("namespace".into()));
    object.insert("name".into(), Value::String(namespace.name.clone()));
    if let Some(description) = &namespace.description {
        object.insert("description".into(), Value::String(description.clone()));
    }
    object.insert(
        "tools".into(),
        Value::Array(
            namespace
                .tools
                .iter()
                .map(render_responses_tool)
                .collect::<Result<Vec<_>, _>>()?,
        ),
    );
    Ok(Value::Object(object))
}

fn render_responses_tools_in_order(
    request: &ModelRequestIRV1,
) -> Result<Vec<Value>, ProtocolAdapterError> {
    if request.responses_tool_order.is_empty() {
        if !request.tool_namespaces.is_empty() {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "Responses namespace tools require an exact native order".into(),
            ));
        }
        let mut tools = request
            .tools
            .iter()
            .map(render_responses_tool)
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(search) = &request.web_search {
            tools.push(
                serde_json::to_value(search)
                    .map_err(|_| ProtocolAdapterError::Serialization("web search".into()))?,
            );
        }
        return Ok(tools);
    }

    let mut flat_seen = vec![false; request.tools.len()];
    let mut namespace_seen = vec![false; request.tool_namespaces.len()];
    let mut search_seen = false;
    let mut tools = Vec::with_capacity(request.responses_tool_order.len());
    for entry in &request.responses_tool_order {
        match *entry {
            ResponsesToolOrderEntryV1::Tool { index } => {
                let index = usize::try_from(index).map_err(|_| {
                    ProtocolAdapterError::ClientUnrepresentable(
                        "Responses Tool order index is invalid".into(),
                    )
                })?;
                let tool = request.tools.get(index).ok_or_else(|| {
                    ProtocolAdapterError::ClientUnrepresentable(
                        "Responses Tool order references a missing tool".into(),
                    )
                })?;
                if std::mem::replace(&mut flat_seen[index], true) {
                    return Err(ProtocolAdapterError::ClientUnrepresentable(
                        "Responses Tool order repeats a tool".into(),
                    ));
                }
                tools.push(render_responses_tool(tool)?);
            }
            ResponsesToolOrderEntryV1::Namespace { index } => {
                let index = usize::try_from(index).map_err(|_| {
                    ProtocolAdapterError::ClientUnrepresentable(
                        "Responses Tool order index is invalid".into(),
                    )
                })?;
                let namespace = request.tool_namespaces.get(index).ok_or_else(|| {
                    ProtocolAdapterError::ClientUnrepresentable(
                        "Responses Tool order references a missing namespace".into(),
                    )
                })?;
                if std::mem::replace(&mut namespace_seen[index], true) {
                    return Err(ProtocolAdapterError::ClientUnrepresentable(
                        "Responses Tool order repeats a namespace".into(),
                    ));
                }
                tools.push(render_responses_namespace(namespace)?);
            }
            ResponsesToolOrderEntryV1::WebSearch => {
                let search = request.web_search.as_ref().ok_or_else(|| {
                    ProtocolAdapterError::ClientUnrepresentable(
                        "Responses Tool order references missing web search".into(),
                    )
                })?;
                if std::mem::replace(&mut search_seen, true) {
                    return Err(ProtocolAdapterError::ClientUnrepresentable(
                        "Responses Tool order repeats web search".into(),
                    ));
                }
                tools.push(
                    serde_json::to_value(search)
                        .map_err(|_| ProtocolAdapterError::Serialization("web search".into()))?,
                );
            }
        }
    }
    if flat_seen.iter().any(|seen| !seen)
        || namespace_seen.iter().any(|seen| !seen)
        || search_seen != request.web_search.is_some()
    {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "Responses Tool order is incomplete".into(),
        ));
    }
    Ok(tools)
}

fn render_messages_tool(tool: &CanonicalTool) -> Result<Value, ProtocolAdapterError> {
    // The cross-protocol serializer has no strict-schema mapping. Native
    // Messages requests bypass this serializer and preserve provider fields.
    if tool.strict == Some(true) {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "strict function schema has no Messages conversion".into(),
        ));
    }
    if tool.format.is_some() {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "Messages function tool cannot carry custom format".into(),
        ));
    }
    let mut object = Map::new();
    object.insert("name".into(), Value::String(tool.name.clone()));
    object.insert(
        "input_schema".into(),
        tool.input_schema
            .as_ref()
            .ok_or_else(|| {
                ProtocolAdapterError::ClientUnrepresentable(
                    "Messages function tool requires an input schema".into(),
                )
            })?
            .wire_value(),
    );
    if let Some(description) = &tool.description {
        object.insert("description".into(), Value::String(description.clone()));
    }
    Ok(Value::Object(object))
}

fn tool_kind_label(kind: ToolKindV1) -> &'static str {
    match kind {
        ToolKindV1::Function => "function",
        ToolKindV1::Custom => "custom",
    }
}
