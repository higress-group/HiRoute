use std::io::Read;

use super::{AgentTurnHistoryError, ToolStatus, VisibleContentPart};
use crate::content_ref::ContentValueExt;
use crate::replay::ReplayStore;
use crate::server::core_runtime::model_ir::{
    CanonicalMessage, ContentPart, ImageSource, MessageRole, ModelRequestIRV1, ToolResultStatusV1,
};

pub(super) struct AnalyzedRequest {
    pub(super) message_count: usize,
    pub(super) turns: Vec<AnalyzedTurn>,
    pub(super) tool_results: Vec<(usize, String, ToolStatus)>,
}

pub(super) struct AnalyzedTurn {
    pub(super) user_index: usize,
    pub(super) user: Vec<VisibleContentPart>,
}

pub(super) fn analyze_request(
    request: &ModelRequestIRV1,
    replay: &ReplayStore,
) -> Result<AnalyzedRequest, AgentTurnHistoryError> {
    let user_indices = request
        .messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| is_real_user(message).then_some(index))
        .collect::<Vec<_>>();
    let mut turns = Vec::with_capacity(user_indices.len());
    for user_index in user_indices.iter().copied() {
        let user = project_user(&request.messages[user_index], replay)?;
        turns.push(AnalyzedTurn { user_index, user });
    }
    Ok(AnalyzedRequest {
        message_count: request.messages.len(),
        turns,
        tool_results: project_tool_results(request),
    })
}

fn is_real_user(message: &CanonicalMessage) -> bool {
    message.role == MessageRole::User
        && message
            .content
            .iter()
            .any(|part| matches!(part, ContentPart::Text { .. } | ContentPart::Image { .. }))
}

pub(super) fn project_user(
    message: &CanonicalMessage,
    replay: &ReplayStore,
) -> Result<Vec<VisibleContentPart>, AgentTurnHistoryError> {
    message
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => {
                Some(resolve_text(text, replay).map(|text| VisibleContentPart::Text { text }))
            }
            ContentPart::Image {
                source: ImageSource::Url { .. },
            } => Some(Ok(VisibleContentPart::Unavailable {
                source_kind: "image_url".into(),
            })),
            ContentPart::Image {
                source: ImageSource::Base64 { .. },
            } => Some(Ok(VisibleContentPart::Unavailable {
                source_kind: "image_base64".into(),
            })),
            ContentPart::ToolCall { .. }
            | ContentPart::ToolResult { .. }
            | ContentPart::ProviderState { .. } => None,
        })
        .collect()
}

fn project_tool_results(request: &ModelRequestIRV1) -> Vec<(usize, String, ToolStatus)> {
    let mut results = Vec::new();
    for (message_index, message) in request.messages.iter().enumerate() {
        for part in &message.content {
            if let ContentPart::ToolResult {
                logical_id, status, ..
            } = part
            {
                results.push((
                    message_index,
                    logical_id.clone(),
                    match status {
                        ToolResultStatusV1::Completed => ToolStatus::Completed,
                        ToolResultStatusV1::Failed => ToolStatus::Failed,
                        ToolResultStatusV1::Unknown => ToolStatus::Unknown,
                    },
                ));
            }
        }
    }
    results
}

fn resolve_text(value: &str, replay: &ReplayStore) -> Result<String, AgentTurnHistoryError> {
    let Some(reference) = value.content_ref() else {
        return Ok(value.to_owned());
    };
    let capacity =
        usize::try_from(reference.byte_len()).map_err(|_| AgentTurnHistoryError::Resource)?;
    let mut resolved = String::with_capacity(capacity);
    let mut reader = replay
        .reader(&reference)
        .map_err(|_| AgentTurnHistoryError::Integrity)?;
    reader
        .read_to_string(&mut resolved)
        .map_err(|_| AgentTurnHistoryError::Integrity)?;
    reader
        .verify_terminal()
        .map_err(|_| AgentTurnHistoryError::Integrity)?;
    Ok(resolved)
}
