use super::*;

pub(super) fn render_responses_json(
    alias: &str,
    response: &ModelResponseIRV1,
) -> Result<Value, ProtocolAdapterError> {
    let reason = response.finish_reason.as_ref().ok_or_else(|| {
        ProtocolAdapterError::ClientUnrepresentable(
            "Responses terminal response is missing a finish reason".into(),
        )
    })?;
    let terminal = responses_terminal(reason)?;
    let mut output = response
        .blocks
        .iter()
        .map(|block| render_responses_block(&response.response_id, block))
        .collect::<Result<Vec<_>, _>>()?;
    append_responses_state(&response.response_id, &mut output, &response.provider_state)?;
    let mut rendered = json!({
        "id": response.response_id,
        "object": "response",
        "created_at": 0,
        "status": terminal.status,
        "error": null,
        "incomplete_details": terminal.incomplete_details,
        "model": alias,
        "output": output,
        "usage": responses_usage(&response.usage),
    });
    if let Some(object) = rendered.as_object_mut() {
        for (key, value) in &response.responses_metadata {
            if !matches!(
                key.as_str(),
                "id" | "model" | "output" | "usage" | "status" | "error" | "incomplete_details"
            ) {
                object.insert(key.clone(), value.clone());
            }
        }
    }
    Ok(rendered)
}

pub(super) fn render_responses_stream(
    alias: &str,
    response: &ModelResponseIRV1,
) -> Result<Vec<RenderedSseEvent>, ProtocolAdapterError> {
    let mut events = Vec::new();
    let mut sequence = 0_u64;
    push_responses_event(
        &mut events,
        &mut sequence,
        "response.created",
        json!({"type":"response.created","response":{"id":response.response_id,"model":alias}}),
    );
    if response
        .provider_state
        .iter()
        .any(|state| state.block_index.is_none())
    {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "Responses streaming state requires a reasoning block binding".into(),
        ));
    }
    let mut final_items = response
        .blocks
        .iter()
        .map(|block| render_responses_block(&response.response_id, block))
        .collect::<Result<Vec<_>, _>>()?;
    append_responses_state(
        &response.response_id,
        &mut final_items,
        &response.provider_state,
    )?;
    for (position, block) in response.blocks.iter().enumerate() {
        let output_index = block.index();
        let final_item = &final_items[position];
        match block {
            ResponseBlock::WebSearch { item, .. } => {
                let mut start = item.wire_value();
                start["status"] = "in_progress".into();
                start.as_object_mut().unwrap().remove("action");
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.output_item.added",
                    json!({"type":"response.output_item.added","output_index":output_index,"item":start}),
                );
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.output_item.done",
                    json!({"type":"response.output_item.done","output_index":output_index,"item":item.wire_value()}),
                );
            }
            ResponseBlock::Text {
                item_id,
                text,
                annotations,
                ..
            } => {
                let item_id = response_item_id(&response.response_id, item_id, "msg", output_index);
                let mut start_item = final_item.clone();
                start_item["status"] = "in_progress".into();
                start_item["content"] = json!([]);
                let empty_part = json!({"type":"output_text","text":"","annotations":[]});
                let final_part =
                    json!({"type":"output_text","text":text,"annotations":annotations});
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.output_item.added",
                    json!({"type":"response.output_item.added","output_index":output_index,"item":start_item}),
                );
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.content_part.added",
                    json!({"type":"response.content_part.added","item_id":item_id,"output_index":output_index,"content_index":0,"part":empty_part}),
                );
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.output_text.delta",
                    json!({"type":"response.output_text.delta","item_id":item_id,"output_index":output_index,"content_index":0,"delta":text}),
                );
                for (position, annotation) in annotations.iter().enumerate() {
                    push_responses_event(
                        &mut events,
                        &mut sequence,
                        "response.output_text.annotation.added",
                        json!({"type":"response.output_text.annotation.added","item_id":item_id,"output_index":output_index,"content_index":0,"annotation_index":position,"annotation":annotation}),
                    );
                }
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.output_text.done",
                    json!({"type":"response.output_text.done","item_id":item_id,"output_index":output_index,"content_index":0,"text":text}),
                );
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.content_part.done",
                    json!({"type":"response.content_part.done","item_id":item_id,"output_index":output_index,"content_index":0,"part":final_part}),
                );
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.output_item.done",
                    json!({"type":"response.output_item.done","output_index":output_index,"item":final_item}),
                );
            }
            ResponseBlock::Reasoning { item_id, text, .. } => {
                let item_id = response_item_id(&response.response_id, item_id, "rs", output_index);
                let mut start_item = final_item.clone();
                start_item["status"] = "in_progress".into();
                start_item["summary"] = json!([]);
                let empty_part = json!({"type":"summary_text","text":""});
                let final_part = json!({"type":"summary_text","text":text});
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.output_item.added",
                    json!({"type":"response.output_item.added","output_index":output_index,"item":start_item}),
                );
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.reasoning_summary_part.added",
                    json!({"type":"response.reasoning_summary_part.added","item_id":item_id,"output_index":output_index,"summary_index":0,"part":empty_part}),
                );
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.reasoning_summary_text.delta",
                    json!({"type":"response.reasoning_summary_text.delta","item_id":item_id,"output_index":output_index,"summary_index":0,"delta":text}),
                );
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.reasoning_summary_text.done",
                    json!({"type":"response.reasoning_summary_text.done","item_id":item_id,"output_index":output_index,"summary_index":0,"text":text}),
                );
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.reasoning_summary_part.done",
                    json!({"type":"response.reasoning_summary_part.done","item_id":item_id,"output_index":output_index,"summary_index":0,"part":final_part}),
                );
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.output_item.done",
                    json!({"type":"response.output_item.done","output_index":output_index,"item":final_item}),
                );
            }
            ResponseBlock::Refusal { item_id, text, .. } => {
                let item_id = response_item_id(&response.response_id, item_id, "msg", output_index);
                let mut start_item = final_item.clone();
                start_item["status"] = "in_progress".into();
                start_item["content"] = json!([]);
                let empty_part = json!({"type":"refusal","refusal":""});
                let final_part = json!({"type":"refusal","refusal":text});
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.output_item.added",
                    json!({"type":"response.output_item.added","output_index":output_index,"item":start_item}),
                );
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.content_part.added",
                    json!({"type":"response.content_part.added","item_id":item_id,"output_index":output_index,"content_index":0,"part":empty_part}),
                );
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.refusal.delta",
                    json!({"type":"response.refusal.delta","item_id":item_id,"output_index":output_index,"content_index":0,"delta":text}),
                );
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.refusal.done",
                    json!({"type":"response.refusal.done","item_id":item_id,"output_index":output_index,"content_index":0,"refusal":text}),
                );
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.content_part.done",
                    json!({"type":"response.content_part.done","item_id":item_id,"output_index":output_index,"content_index":0,"part":final_part}),
                );
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.output_item.done",
                    json!({"type":"response.output_item.done","output_index":output_index,"item":final_item}),
                );
            }
            ResponseBlock::ToolCall {
                item_id,
                logical_id,
                tool_kind,
                namespace,
                name,
                arguments,
                ..
            } => {
                let (prefix, item_type, payload_field, delta_event, done_event, arguments) =
                    match tool_kind {
                        ToolKindV1::Function => (
                            "fc",
                            "function_call",
                            "arguments",
                            "response.function_call_arguments.delta",
                            "response.function_call_arguments.done",
                            serde_json::to_string(arguments).map_err(|error| {
                                ProtocolAdapterError::Serialization(error.to_string())
                            })?,
                        ),
                        ToolKindV1::Custom => (
                            "ct",
                            "custom_tool_call",
                            "input",
                            "response.custom_tool_call_input.delta",
                            "response.custom_tool_call_input.done",
                            arguments
                                .as_str()
                                .ok_or_else(|| {
                                    ProtocolAdapterError::ClientUnrepresentable(
                                        "custom tool input is not a string".into(),
                                    )
                                })?
                                .to_owned(),
                        ),
                    };
                let item_id =
                    response_item_id(&response.response_id, item_id, prefix, output_index);
                let mut added_item = json!({"type":item_type,"id":item_id,"call_id":logical_id,"name":name,"status":"in_progress"});
                added_item[payload_field] = Value::String(String::new());
                if let Some(namespace) = namespace {
                    added_item["namespace"] = Value::String(namespace.clone());
                }
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.output_item.added",
                    json!({"type":"response.output_item.added","output_index":output_index,"item":added_item}),
                );
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    delta_event,
                    json!({"type":delta_event,"item_id":item_id,"output_index":output_index,"delta":arguments}),
                );
                push_responses_event(&mut events, &mut sequence, done_event, {
                    let mut event =
                        json!({"type":done_event,"item_id":item_id,"output_index":output_index});
                    event[payload_field] = Value::String(arguments);
                    event
                });
                push_responses_event(
                    &mut events,
                    &mut sequence,
                    "response.output_item.done",
                    json!({"type":"response.output_item.done","output_index":output_index,"item":final_item}),
                );
            }
        }
    }
    let reason = response.finish_reason.as_ref().ok_or_else(|| {
        ProtocolAdapterError::ClientUnrepresentable(
            "Responses terminal response is missing a finish reason".into(),
        )
    })?;
    let terminal = responses_terminal(reason)?;
    push_responses_event(
        &mut events,
        &mut sequence,
        terminal.event,
        json!({"type":terminal.event,"response":render_responses_json(alias, response)?}),
    );
    Ok(events)
}

pub(super) fn render_responses_block(
    response_id: &str,
    block: &ResponseBlock,
) -> Result<Value, ProtocolAdapterError> {
    Ok(match block {
        ResponseBlock::WebSearch { item, .. } => item.wire_value(),
        ResponseBlock::Text {
            index,
            item_id,
            phase,
            text,
            annotations,
            status,
        } => {
            let mut item = json!({"type":"message","id":response_item_id(response_id, item_id, "msg", *index),"status":status.as_str(),"role":"assistant","content":[{"type":"output_text","text":text,"annotations":annotations}]});
            if let Some(phase) = phase {
                item["phase"] = Value::String(phase.clone());
            }
            item
        }
        ResponseBlock::Reasoning {
            index,
            item_id,
            text,
            status,
        } => {
            json!({"type":"reasoning","id":response_item_id(response_id, item_id, "rs", *index),"status":status.as_str(),"summary":[{"type":"summary_text","text":text}]})
        }
        ResponseBlock::Refusal {
            index,
            item_id,
            phase,
            text,
            status,
        } => {
            let mut item = json!({"type":"message","id":response_item_id(response_id, item_id, "msg", *index),"status":status.as_str(),"role":"assistant","content":[{"type":"refusal","refusal":text}]});
            if let Some(phase) = phase {
                item["phase"] = Value::String(phase.clone());
            }
            item
        }
        ResponseBlock::ToolCall {
            encrypted_function_args,
            index,
            item_id,
            logical_id,
            tool_kind,
            namespace,
            name,
            arguments,
            status,
        } => {
            let (prefix, item_type, payload_field, payload) = match tool_kind {
                ToolKindV1::Function => (
                    "fc",
                    "function_call",
                    "arguments",
                    serde_json::to_string(arguments)
                        .map_err(|error| ProtocolAdapterError::Serialization(error.to_string()))?,
                ),
                ToolKindV1::Custom => (
                    "ct",
                    "custom_tool_call",
                    "input",
                    arguments
                        .as_str()
                        .ok_or_else(|| {
                            ProtocolAdapterError::ClientUnrepresentable(
                                "custom tool input is not a string".into(),
                            )
                        })?
                        .to_owned(),
                ),
            };
            let mut item = json!({"type":item_type,"id":response_item_id(response_id, item_id, prefix, *index),"call_id":logical_id,"name":name,"status":status.as_str()});
            item[payload_field] = Value::String(payload);
            if let Some(encoding) = encrypted_function_args {
                item["encrypted_function_args"] = json!(encoding);
            }
            if let Some(namespace) = namespace {
                item["namespace"] = Value::String(namespace.clone());
            }
            item
        }
    })
}

fn response_item_id(
    response_id: &str,
    item_id: &Option<String>,
    prefix: &str,
    index: u32,
) -> String {
    item_id
        .clone()
        .unwrap_or_else(|| synthetic_responses_item_id(response_id, prefix, index))
}

fn push_responses_event(
    events: &mut Vec<RenderedSseEvent>,
    sequence: &mut u64,
    event: &str,
    mut data: Value,
) {
    data["sequence_number"] = Value::from(*sequence);
    push_event(events, event, data);
    *sequence += 1;
}
