//! The explicit standard-Responses JSON dialect owns both directions of negotiation.
//! Native Responses is deliberately not an alias for this contract.
use serde_json::{Map, Value, json};

use super::{ModelIrError, ProtocolAdapterError};
use crate::server::core_runtime::profiles::CandidateProtocolProfile;
use crate::server::request_plan::IngressProtocol;

pub(super) fn standard_json(profile: &CandidateProtocolProfile) -> bool {
    profile.capability.upstream_protocol == IngressProtocol::Responses
        && profile.adapter_revision == hiroute_domain::STANDARD_RESPONSES_JSON_ADAPTER_V1
}

pub(super) fn encoding(item: &Map<String, Value>) -> Result<Option<Vec<String>>, ModelIrError> {
    item.get("encrypted_function_args")
        .map(|value| {
            value
                .as_array()
                .ok_or(ModelIrError::InvalidField("encrypted_function_args"))?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or(ModelIrError::InvalidField("encrypted_function_args[]"))
                })
                .collect()
        })
        .transpose()
}

pub(super) fn response_encoding(
    item: &Map<String, Value>,
    standard: bool,
) -> Result<Option<Vec<String>>, ModelIrError> {
    let encoding = encoding(item)?;
    if standard {
        if encoding.as_ref().is_some_and(|paths| !paths.is_empty()) {
            return Err(ModelIrError::UnsupportedField(
                "standard JSON endpoint returned encrypted function arguments".into(),
            ));
        }
        Ok(Some(Vec::new()))
    } else {
        Ok(encoding)
    }
}

pub(super) fn project_request(body: &mut Value) -> Result<(), ProtocolAdapterError> {
    let object = body.as_object_mut().ok_or(ModelIrError::ExpectedObject)?;
    if let Some(tools) = object.get_mut("tools") {
        for tool in tools
            .as_array_mut()
            .ok_or(ModelIrError::InvalidField("tools"))?
        {
            project_tool(tool)?;
        }
    }
    if let Some(input) = object.get_mut("input").and_then(Value::as_array_mut) {
        for item in input {
            match item.get("type").and_then(Value::as_str) {
                Some("agent_message") => project_peer(item)?,
                Some("function_call") => {
                    let call = item
                        .as_object_mut()
                        .ok_or(ModelIrError::InvalidField("function_call"))?;
                    if encoding(call)?
                        .as_ref()
                        .is_some_and(|paths| !paths.is_empty())
                    {
                        return Err(ModelIrError::UnsupportedField(
                            "encrypted function replay on standard JSON endpoint".into(),
                        )
                        .into());
                    }
                    // Consume only an explicit plaintext declaration. Missing is never filled
                    // in historical input, nor interpreted as permission to decrypt anything.
                    call.remove("encrypted_function_args");
                }
                Some("reasoning") if item.get("encrypted_content").is_some() => {
                    return Err(ModelIrError::ProviderStateNotPortable.into());
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn project_tool(tool: &mut Value) -> Result<(), ModelIrError> {
    let object = tool
        .as_object_mut()
        .ok_or(ModelIrError::InvalidField("tools[]"))?;
    match object.get("type").and_then(Value::as_str) {
        Some("function") => {
            if let Some(schema) = object.get_mut("parameters") {
                schema_node(schema)?;
            }
        }
        Some("namespace") => {
            for tool in object
                .get_mut("tools")
                .and_then(Value::as_array_mut)
                .ok_or(ModelIrError::InvalidField("namespace.tools"))?
            {
                project_tool(tool)?;
            }
        }
        // These declarations retain their complete original native shape.
        Some("custom" | "web_search" | "web_search_preview") => {}
        _ => {
            return Err(ModelIrError::UnsupportedField(
                "standard Responses tool declaration".into(),
            ));
        }
    }
    Ok(())
}

fn schema_node(value: &mut Value) -> Result<(), ModelIrError> {
    if value.is_boolean() {
        return Ok(());
    }
    let schema = value
        .as_object_mut()
        .ok_or(ModelIrError::InvalidField("function.parameters schema"))?;
    if schema.get("encrypted").is_some_and(|v| !v.is_boolean()) {
        return Err(ModelIrError::InvalidField("schema.encrypted"));
    }
    schema.remove("encrypted");
    // Visit schema positions, never example/default data or a property named "encrypted".
    for key in [
        "properties",
        "patternProperties",
        "$defs",
        "definitions",
        "dependentSchemas",
    ] {
        if let Some(children) = schema.get_mut(key) {
            for child in children
                .as_object_mut()
                .ok_or(ModelIrError::InvalidField("schema map"))?
                .values_mut()
            {
                schema_node(child)?;
            }
        }
    }
    for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(children) = schema.get_mut(key) {
            for child in children
                .as_array_mut()
                .ok_or(ModelIrError::InvalidField("schema array"))?
            {
                schema_node(child)?;
            }
        }
    }
    for key in [
        "items",
        "contains",
        "additionalProperties",
        "unevaluatedProperties",
        "unevaluatedItems",
        "not",
        "if",
        "then",
        "else",
        "propertyNames",
    ] {
        if let Some(child) = schema.get_mut(key) {
            if let Some(children) = child.as_array_mut() {
                for child in children {
                    schema_node(child)?;
                }
            } else {
                schema_node(child)?;
            }
        }
    }
    Ok(())
}

fn project_peer(item: &mut Value) -> Result<(), ModelIrError> {
    let peer = item
        .as_object()
        .ok_or(ModelIrError::InvalidField("agent_message"))?;
    if peer.keys().any(|key| {
        !matches!(
            key.as_str(),
            "type" | "id" | "author" | "recipient" | "content"
        )
    }) {
        return Err(ModelIrError::UnsupportedField(
            "agent_message extension on standard Responses".into(),
        ));
    }
    let author = peer
        .get("author")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or(ModelIrError::InvalidField("agent_message.author"))?;
    let recipient = peer
        .get("recipient")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or(ModelIrError::InvalidField("agent_message.recipient"))?;
    let mut header = json!({"type":"agent_message","author":author,"recipient":recipient});
    if let Some(id) = peer.get("id") {
        header["id"] = id.clone();
    }
    // Separate metadata and ordered text blocks keep Replay string references exact.
    // This is peer-authored assistant content, never a new user or invented tool result.
    let mut content = vec![json!({"type":"output_text","text":header.to_string()})];
    for part in peer
        .get("content")
        .and_then(Value::as_array)
        .ok_or(ModelIrError::InvalidField("agent_message.content"))?
    {
        let part = part
            .as_object()
            .ok_or(ModelIrError::InvalidField("agent_message.content[]"))?;
        if part.len() != 2 || part.get("type").and_then(Value::as_str) != Some("input_text") {
            return Err(ModelIrError::UnsupportedField(
                "non-plaintext agent_message on standard Responses".into(),
            ));
        }
        let text = part
            .get("text")
            .filter(|v| v.is_string())
            .ok_or(ModelIrError::InvalidField("agent_message.text"))?;
        content.push(json!({"type":"output_text","text":text}));
    }
    *item = json!({"type":"message","role":"assistant","content":content});
    Ok(())
}

// Chat function.arguments and Messages tool_use.input define JSON plaintext. The
// request adapter consumes Codex schema annotations when selecting either mapping.
pub(super) fn project_portable_tools(
    body: &mut Value,
    protocol: IngressProtocol,
) -> Result<(), ModelIrError> {
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            let schema = match protocol {
                IngressProtocol::ChatCompletions => tool
                    .get_mut("function")
                    .and_then(|f| f.get_mut("parameters")),
                IngressProtocol::Messages => tool.get_mut("input_schema"),
                IngressProtocol::Responses => None,
            };
            if let Some(schema) = schema {
                schema_node(schema)?;
            }
        }
    }
    Ok(())
}
