use super::*;

pub(super) fn decode_responses_input(
    value: &Value,
    messages: &mut Vec<CanonicalMessage>,
    state_owner: Option<&ExactProviderPathV1>,
    item_status: Option<&str>,
) -> Result<(), ModelIrError> {
    let object = value
        .as_object()
        .ok_or(ModelIrError::InvalidField("input[]"))?;
    let item_type = required_string(object, "type")?;
    match item_type.as_str() {
        "message" => {
            ensure_keys(
                object,
                &[
                    "type",
                    "role",
                    "content",
                    "name",
                    "id",
                    "status",
                    "phase",
                    "internal_chat_message_metadata_passthrough",
                ],
                "responses message",
            )?;
            messages.push(CanonicalMessage {
                role: decode_role(required_string(object, "role")?.as_str())?,
                content: decode_responses_content(
                    object
                        .get("content")
                        .ok_or(ModelIrError::InvalidField("content"))?,
                )?,
                name: optional_string(object, "name")?,
            });
        }
        "function_call" => {
            ensure_keys(
                object,
                &[
                    "type",
                    "call_id",
                    "namespace",
                    "name",
                    "arguments",
                    "id",
                    "status",
                    "internal_chat_message_metadata_passthrough",
                ],
                "responses function_call",
            )?;
            let arguments = parse_json_string(required_string(object, "arguments")?, "arguments")?;
            messages.push(CanonicalMessage {
                role: MessageRole::Assistant,
                content: vec![ContentPart::ToolCall {
                    logical_id: required_string(object, "call_id")?,
                    tool_kind: ToolKindV1::Function,
                    namespace: optional_string(object, "namespace")?,
                    name: required_string(object, "name")?,
                    arguments,
                }],
                name: None,
            });
        }
        "function_call_output" => {
            ensure_keys(
                object,
                &[
                    "type",
                    "call_id",
                    "output",
                    "id",
                    "status",
                    "internal_chat_message_metadata_passthrough",
                ],
                "responses function_call_output",
            )?;
            messages.push(CanonicalMessage {
                role: MessageRole::User,
                content: vec![ContentPart::ToolResult {
                    logical_id: required_string(object, "call_id")?,
                    tool_kind: ToolKindV1::Function,
                    namespace: None,
                    output: decode_tool_output(
                        object
                            .get("output")
                            .ok_or(ModelIrError::InvalidField("output"))?,
                    ),
                    status: match item_status {
                        Some("completed") => ToolResultStatusV1::Completed,
                        Some("incomplete") => ToolResultStatusV1::Failed,
                        Some("in_progress") | None => ToolResultStatusV1::Unknown,
                        Some(_) => {
                            return Err(ModelIrError::InvalidField("Responses input item status"));
                        }
                    },
                }],
                name: None,
            });
        }
        "custom_tool_call" => {
            ensure_keys(
                object,
                &[
                    "type",
                    "call_id",
                    "namespace",
                    "name",
                    "input",
                    "id",
                    "internal_chat_message_metadata_passthrough",
                ],
                "responses custom_tool_call",
            )?;
            messages.push(CanonicalMessage {
                role: MessageRole::Assistant,
                content: vec![ContentPart::ToolCall {
                    logical_id: required_string(object, "call_id")?,
                    tool_kind: ToolKindV1::Custom,
                    namespace: optional_string(object, "namespace")?,
                    name: required_string(object, "name")?,
                    arguments: Value::String(required_string(object, "input")?),
                }],
                name: None,
            });
        }
        "custom_tool_call_output" => {
            ensure_keys(
                object,
                &[
                    "type",
                    "call_id",
                    "output",
                    "id",
                    "internal_chat_message_metadata_passthrough",
                ],
                "responses custom_tool_call_output",
            )?;
            messages.push(CanonicalMessage {
                role: MessageRole::User,
                content: vec![ContentPart::ToolResult {
                    logical_id: required_string(object, "call_id")?,
                    tool_kind: ToolKindV1::Custom,
                    namespace: None,
                    output: decode_tool_output(
                        object
                            .get("output")
                            .ok_or(ModelIrError::InvalidField("output"))?,
                    ),
                    status: ToolResultStatusV1::Unknown,
                }],
                name: None,
            });
        }
        "reasoning" => {
            let encrypted = match object.get("encrypted_content") {
                None | Some(Value::Null) => None,
                Some(Value::String(value)) if value.is_empty() => None,
                Some(Value::String(value)) => Some(value.clone()),
                Some(_) => return Err(ModelIrError::InvalidField("encrypted_content")),
            };
            let content = if let Some(encrypted) = encrypted {
                let owner = state_owner
                    .filter(|owner| {
                        owner.is_complete() && owner.upstream_protocol == IngressProtocol::Responses
                    })
                    .cloned()
                    .ok_or(ModelIrError::ProviderStateOwnershipRequired)?;
                vec![ContentPart::ProviderState {
                    state: Box::new(OpaqueProviderState {
                        owner,
                        block_index: None,
                        kind: "encrypted_content".into(),
                        value: Value::String(encrypted),
                    }),
                }]
            } else {
                Vec::new()
            };
            messages.push(CanonicalMessage {
                role: MessageRole::Assistant,
                content,
                name: None,
            });
        }
        other => {
            return Err(ModelIrError::UnsupportedValue(format!(
                "responses input {other}"
            )));
        }
    }
    Ok(())
}

fn decode_responses_content(value: &Value) -> Result<Vec<ContentPart>, ModelIrError> {
    let values = match value {
        Value::String(text) => {
            return Ok(vec![ContentPart::Text { text: text.clone() }]);
        }
        Value::Array(values) => values,
        _ => return Err(ModelIrError::InvalidField("content")),
    };
    values
        .iter()
        .map(|value| {
            let object = value
                .as_object()
                .ok_or(ModelIrError::InvalidField("content[]"))?;
            match required_string(object, "type")?.as_str() {
                "input_text" | "output_text" => {
                    ensure_keys(object, &["type", "text", "annotations"], "responses text")?;
                    Ok(ContentPart::Text {
                        text: required_string(object, "text")?,
                    })
                }
                "input_image" => {
                    ensure_keys(object, &["type", "image_url"], "responses image")?;
                    Ok(ContentPart::Image {
                        source: decode_image_url(required_string(object, "image_url")?)?,
                    })
                }
                other => Err(ModelIrError::UnsupportedValue(format!(
                    "responses content {other}"
                ))),
            }
        })
        .collect()
}

pub(super) struct DecodedResponsesTools {
    pub(super) tools: Vec<CanonicalTool>,
    pub(super) namespaces: Vec<CanonicalToolNamespaceV1>,
    pub(super) web_search: Option<WebSearchToolV1>,
    pub(super) order: Vec<ResponsesToolOrderEntryV1>,
}

pub(super) fn decode_responses_tools(
    value: Option<&Value>,
    additional: &[&Value],
) -> Result<DecodedResponsesTools, ModelIrError> {
    let mut decoded = DecodedResponsesTools {
        tools: Vec::new(),
        namespaces: Vec::new(),
        web_search: None,
        order: Vec::new(),
    };
    if let Some(value) = value {
        merge_responses_tools(value, &mut decoded)?;
    }
    for value in additional {
        merge_responses_tools(value, &mut decoded)?;
    }
    // Flat tools already retain their native order in `tools`. Keep the
    // Responses-only order ledger only when it is needed to interleave a
    // namespace or hosted search declaration with those portable tools.
    if decoded.namespaces.is_empty() && decoded.web_search.is_none() {
        decoded.order.clear();
    }
    Ok(decoded)
}

fn merge_responses_tools(
    value: &Value,
    decoded: &mut DecodedResponsesTools,
) -> Result<(), ModelIrError> {
    let values = value
        .as_array()
        .ok_or(ModelIrError::InvalidField("tools"))?;
    for value in values {
        let object = value
            .as_object()
            .ok_or(ModelIrError::InvalidField("tools[]"))?;
        match required_string(object, "type")?.as_str() {
            "function" | "custom" => {
                let tool = decode_responses_tool(object)?;
                if let Some(existing) = decoded
                    .tools
                    .iter()
                    .find(|existing| existing.name == tool.name)
                {
                    if existing == &tool {
                        continue;
                    }
                    return Err(ModelIrError::InvalidField(
                        "conflicting flat tool declaration",
                    ));
                }
                let index = u32::try_from(decoded.tools.len())
                    .map_err(|_| ModelIrError::InvalidField("tools"))?;
                decoded.tools.push(tool);
                decoded
                    .order
                    .push(ResponsesToolOrderEntryV1::Tool { index });
            }
            "namespace" => {
                ensure_keys(
                    object,
                    &["type", "name", "description", "tools"],
                    "responses namespace Tool",
                )?;
                let children = object
                    .get("tools")
                    .and_then(Value::as_array)
                    .filter(|children| !children.is_empty())
                    .ok_or(ModelIrError::InvalidField("namespace tools"))?;
                let mut tools = Vec::with_capacity(children.len());
                for child in children {
                    let tool = decode_responses_tool(
                        child
                            .as_object()
                            .ok_or(ModelIrError::InvalidField("namespace tools[]"))?,
                    )?;
                    if let Some(existing) = tools
                        .iter()
                        .find(|existing: &&CanonicalTool| existing.name == tool.name)
                    {
                        if existing == &tool {
                            continue;
                        }
                        return Err(ModelIrError::InvalidField(
                            "conflicting namespace tool declaration",
                        ));
                    }
                    tools.push(tool);
                }
                let namespace = CanonicalToolNamespaceV1 {
                    name: required_string(object, "name")?,
                    description: optional_string(object, "description")?,
                    tools,
                };
                if let Some(existing) = decoded
                    .namespaces
                    .iter()
                    .find(|existing| existing.name == namespace.name)
                {
                    if existing == &namespace {
                        continue;
                    }
                    return Err(ModelIrError::InvalidField(
                        "conflicting namespace declaration",
                    ));
                }
                let index = u32::try_from(decoded.namespaces.len())
                    .map_err(|_| ModelIrError::InvalidField("tools"))?;
                decoded.namespaces.push(namespace);
                decoded
                    .order
                    .push(ResponsesToolOrderEntryV1::Namespace { index });
            }
            "web_search" => {
                let search = serde_json::from_value(value.clone())
                    .map_err(|_| ModelIrError::InvalidField("web_search"))?;
                if let Some(existing) = &decoded.web_search {
                    if existing == &search {
                        continue;
                    }
                    return Err(ModelIrError::InvalidField("conflicting web_search"));
                }
                decoded.web_search = Some(search);
                decoded.order.push(ResponsesToolOrderEntryV1::WebSearch);
            }
            other => {
                return Err(ModelIrError::UnsupportedValue(format!(
                    "responses Tool {other}"
                )));
            }
        }
    }
    Ok(())
}

fn decode_responses_tool(object: &Map<String, Value>) -> Result<CanonicalTool, ModelIrError> {
    match required_string(object, "type")?.as_str() {
        "function" => {
            ensure_keys(
                object,
                &["type", "name", "description", "parameters", "strict"],
                "responses function Tool",
            )?;
            Ok(CanonicalTool {
                kind: ToolKindV1::Function,
                name: required_string(object, "name")?,
                description: optional_string(object, "description")?,
                input_schema: Some(
                    object
                        .get("parameters")
                        .cloned()
                        .ok_or(ModelIrError::InvalidField("parameters"))?,
                ),
                strict: optional_bool(object, "strict")?,
                format: None,
            })
        }
        "custom" => {
            ensure_keys(
                object,
                &["type", "name", "description", "format"],
                "responses custom Tool",
            )?;
            Ok(CanonicalTool {
                kind: ToolKindV1::Custom,
                name: required_string(object, "name")?,
                description: optional_string(object, "description")?,
                input_schema: None,
                strict: None,
                format: object
                    .get("format")
                    .filter(|value| !value.is_null())
                    .cloned(),
            })
        }
        "namespace" => Err(ModelIrError::UnsupportedValue(
            "nested responses namespace".into(),
        )),
        other => Err(ModelIrError::UnsupportedValue(format!(
            "responses namespace child {other}"
        ))),
    }
}
