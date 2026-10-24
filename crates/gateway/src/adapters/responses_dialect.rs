//! The explicit standard-Responses JSON dialect owns both directions of negotiation.
//! Native Responses is deliberately not an alias for this contract.
use super::schema_encoding::project_schema as schema_node;
use crate::content_ref::MARKER_PREFIX;
use serde_json::{Map, Value, json};

use super::{ModelIrError, ProtocolAdapterError};
use crate::server::core_runtime::profiles::CandidateProtocolProfile;
use crate::server::request_plan::IngressProtocol;

/// Per-attempt result of consuming an explicit client encoding declaration.
/// This is not a provider capability or a client-name heuristic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum FunctionEncodingProjection {
    #[default]
    Preserve,
    Plaintext,
}

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
    projection: FunctionEncodingProjection,
) -> Result<Option<Vec<String>>, ModelIrError> {
    let encoding = encoding(item)?;
    if standard {
        if encoding.as_ref().is_some_and(|paths| !paths.is_empty()) {
            return Err(ModelIrError::UnsupportedField(
                "standard JSON endpoint returned encrypted function arguments".into(),
            ));
        }
        Ok((projection == FunctionEncodingProjection::Plaintext).then(Vec::new))
    } else {
        Ok(encoding)
    }
}

pub(super) fn project_request(
    body: &mut Value,
) -> Result<FunctionEncodingProjection, ProtocolAdapterError> {
    let object = body.as_object_mut().ok_or(ModelIrError::ExpectedObject)?;
    let mut negotiated = false;
    if let Some(tools) = object.get_mut("tools") {
        for tool in tools
            .as_array_mut()
            .ok_or(ModelIrError::InvalidField("tools"))?
        {
            negotiated |= project_tool(tool)?;
        }
    }
    // Native peer items and the encoding extension are explicit dialect declarations.
    // Ordinary standard Responses history legitimately omits this Codex-only field.
    if let Some(input) = object.get("input").and_then(Value::as_array) {
        negotiated |= input.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("agent_message")
                || item.get("encrypted_function_args").is_some()
        });
    }
    if let Some(input) = object.get_mut("input").and_then(Value::as_array_mut) {
        for item in input {
            match item.get("type").and_then(Value::as_str) {
                Some("agent_message") => project_peer(item)?,
                Some("function_call") => {
                    let call = item
                        .as_object_mut()
                        .ok_or(ModelIrError::InvalidField("function_call"))?;
                    let declared = encoding(call)?;
                    // Codex's custom-provider request builder drops this optional
                    // vendor metadata even when its recorded response carried [].
                    // Schema negotiation does not change standard JSON history.
                    if declared.as_ref().is_some_and(|paths| !paths.is_empty()) {
                        return Err(ModelIrError::ProviderStateNotPortable.into());
                    }
                    // Consume the negotiated extension; ordinary standard replay stays ordinary.
                    call.remove("encrypted_function_args");
                }
                _ => {}
            }
        }
    }
    Ok(if negotiated {
        FunctionEncodingProjection::Plaintext
    } else {
        FunctionEncodingProjection::Preserve
    })
}

fn project_tool(tool: &mut Value) -> Result<bool, ModelIrError> {
    let object = tool
        .as_object_mut()
        .ok_or(ModelIrError::InvalidField("tools[]"))?;
    let mut negotiated = false;
    match object.get("type").and_then(Value::as_str) {
        Some("function") => {
            if let Some(schema) = object.get_mut("parameters") {
                negotiated |= schema_node(schema)?;
            }
        }
        Some("namespace") => {
            for tool in object
                .get_mut("tools")
                .and_then(Value::as_array_mut)
                .ok_or(ModelIrError::InvalidField("namespace.tools"))?
            {
                negotiated |= project_tool(tool)?;
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
    Ok(negotiated)
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
    // Metadata is encoded into one text block below. Native identity strings are
    // budgeted inline by the request owner; never embed a Replay marker as text.
    // Reserved marker literals remain portable on native passthrough, but this
    // metadata-text representation cannot consume their backed identity safely.
    for key in ["author", "recipient", "id"] {
        if let Some(value) = peer.get(key)
            && value
                .as_str()
                .is_none_or(|text| text.is_empty() || text.contains(MARKER_PREFIX))
        {
            return Err(ModelIrError::UnsupportedField(
                "agent_message identity representation".into(),
            ));
        }
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
) -> Result<bool, ModelIrError> {
    let mut negotiated = false;
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
                negotiated |= schema_node(schema)?;
            }
        }
    }
    Ok(negotiated)
}
