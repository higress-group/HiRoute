//! Candidate-local history projection; original canonical/native history is immutable.
use super::*;
use hiroute_diagnostics::event::RequestProjectionReason;

// Responses envelope identity is separate from a tool's call_id. A completed
// assistant item can be replayed to another protocol without its envelope, but
// unfinished items and native-only fields still require their original protocol.
fn portable_completed_response_item(request: &ModelRequestIRV1, index: usize) -> bool {
    request
        .responses_item_statuses
        .get(&index)
        .is_none_or(|status| status == "completed")
        && request.messages.get(index).is_some_and(|message| {
            message.role == MessageRole::Assistant
                && !message.content.is_empty()
                && message.content.iter().all(|part| {
                    matches!(
                        part,
                        ContentPart::Text { .. }
                            | ContentPart::Image { .. }
                            | ContentPart::ToolCall { .. }
                    )
                })
        })
}

pub(super) fn validate_message_shapes(
    request: &ModelRequestIRV1,
    target: IngressProtocol,
) -> Result<RequestProjectionLoss, ProtocolAdapterError> {
    validate_responses_options(request.responses_options.as_ref(), target)?;
    let mut loss = RequestProjectionLoss::default();
    if target != IngressProtocol::Responses {
        if let Some((index, _)) = request.responses_message_phases.iter().find(|(index, _)| {
            request
                .responses_item_statuses
                .get(*index)
                .is_some_and(|status| status != "completed")
        }) {
            return Err(ProtocolAdapterError::RequestUnrepresentable {
                reason: RequestProjectionReason::ItemLifecycleUnsupported,
                message_index: (*index < request.messages.len()).then_some(*index),
            });
        }
        if request.native_only {
            return Err(ProtocolAdapterError::RequestUnrepresentable {
                reason: RequestProjectionReason::NativeSemanticsUnsupported,
                // Native-only extensions may also occur outside canonical messages.
                // A native replay position alone is not a diagnostic coordinate proof.
                message_index: None,
            });
        }
        for (index, phase) in &request.responses_message_phases {
            let completed_assistant = portable_completed_response_item(request, *index);
            let represented = completed_assistant
                && match phase {
                    ResponsesMessagePhaseV1::FinalAnswer => true,
                    ResponsesMessagePhaseV1::Commentary => {
                        target == IngressProtocol::ChatCompletions
                    }
                };
            if !represented {
                return Err(ProtocolAdapterError::RequestUnrepresentable {
                    reason: RequestProjectionReason::MessagePhaseUnsupported,
                    message_index: (*index < request.messages.len()).then_some(*index),
                });
            }
            if *phase == ResponsesMessagePhaseV1::Commentary {
                loss.first_message_index.get_or_insert(*index);
                loss.message_phases_omitted += 1;
            }
        }
    }
    let mut projected = std::collections::BTreeMap::new();
    if (!request.responses_annotations.is_empty()
        || !request.responses_search_history.is_empty()
        || request.web_search.is_some()
        || request.responses_item_ids.keys().any(|index| {
            !request.responses_reasoning_history.contains_key(index)
                && !portable_completed_response_item(request, *index)
        })
        || request.responses_item_statuses.keys().any(|index| {
            !request.responses_reasoning_history.contains_key(index)
                && !portable_completed_response_item(request, *index)
        })
        || !request.responses_internal_chat_message_metadata.is_empty()
        || (target == IngressProtocol::Messages
            && (!request.tool_namespaces.is_empty() || !request.responses_tool_order.is_empty())))
        && target != IngressProtocol::Responses
    {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "native Responses options require a Responses upstream".into(),
        ));
    }
    if request
        .responses_message_phases
        .keys()
        .chain(request.responses_item_ids.keys())
        .chain(request.responses_item_statuses.keys())
        .chain(request.responses_internal_chat_message_metadata.keys())
        .chain(request.responses_reasoning_history.keys())
        .any(|index| *index >= request.messages.len())
    {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "Responses input-item metadata has no canonical message".into(),
        ));
    }
    for index in request.responses_message_phases.keys() {
        let message = &request.messages[*index];
        if message.content.is_empty()
            || message
                .content
                .iter()
                .any(|part| !matches!(part, ContentPart::Text { .. } | ContentPart::Image { .. }))
        {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "Responses message phase belongs to a non-message item".into(),
            ));
        }
    }
    for index in request.responses_reasoning_history.keys() {
        let message = &request.messages[*index];
        let history = &request.responses_reasoning_history[index];
        let shape_matches = match history.encrypted_content {
            ResponsesReasoningEncryptedContentV1::Opaque => {
                message.content.len() == 1
                    && matches!(
                        message.content.first(),
                        Some(ContentPart::ProviderState { .. })
                    )
            }
            ResponsesReasoningEncryptedContentV1::Absent
            | ResponsesReasoningEncryptedContentV1::Null
            | ResponsesReasoningEncryptedContentV1::Empty => message.content.is_empty(),
        };
        if message.role != MessageRole::Assistant || !shape_matches {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "Responses reasoning history belongs to a non-reasoning item".into(),
            ));
        }
    }
    for message in &request.messages {
        for part in &message.content {
            if request.ingress_protocol != target
                && let ContentPart::ToolCall { logical_id, .. }
                | ContentPart::ToolResult { logical_id, .. } = part
            {
                let target_id = super::super::continuation::project_tool_id(
                    logical_id,
                    request.ingress_protocol,
                    target,
                )?;
                if projected
                    .insert(target_id, logical_id)
                    .is_some_and(|original| original != logical_id)
                {
                    return Err(ModelIrError::ToolContinuationConflict.into());
                }
            }
            match part {
                ContentPart::Text { .. } => {}
                ContentPart::Image { .. } => {
                    if message.role != MessageRole::User {
                        return Err(ProtocolAdapterError::ClientUnrepresentable(
                            "image input must belong to a user message".into(),
                        ));
                    }
                }
                ContentPart::ToolCall { .. } => {
                    if message.role != MessageRole::Assistant {
                        return Err(ProtocolAdapterError::ClientUnrepresentable(
                            "Tool call must belong to an assistant message".into(),
                        ));
                    }
                }
                ContentPart::ToolResult { .. } => {
                    if message.role != MessageRole::User {
                        return Err(ProtocolAdapterError::ClientUnrepresentable(
                            "Tool result must belong to a user message".into(),
                        ));
                    }
                }
                ContentPart::ProviderState { .. } => {}
            }
        }
        // Serializers split representable user content at tool-result boundaries.
        // Message-role and cross-protocol Tool ID validation above remain required.
    }
    Ok(loss)
}
