use std::io::Read;

use super::{AgentTurnHistoryError, CurrentInputPosition, ToolStatus, VisibleContentPart};
use crate::content_ref::ContentValueExt;
use crate::replay::ReplayStore;
use crate::server::core_runtime::model_ir::{
    CanonicalMessage, ContentPart, ImageSource, MessageRole, ModelRequestIRV1, ToolResultStatusV1,
};

pub(crate) struct AnalyzedRequest {
    pub(super) measured: bool,
    pub(super) branch_cursor: (Option<[u8; 32]>, Option<[u8; 32]>),
    pub(super) boundaries: Vec<(usize, super::boundary::BoundaryEvidence)>,
    pub(super) anchors: Option<crate::context_hold::HistoryAnchors>,
    pub(super) retry_setup: Option<[u8; 32]>,
    pub(super) message_count: usize,
    pub(super) user_origin_unverified: bool,
    pub(super) prior_history_missing: bool,
    pub(super) output_after_latest_user: bool,
    /// Start of the actual current input, including every envelope in a window.
    pub(super) current_input_start: Option<usize>,
    pub(super) turns: Vec<AnalyzedTurn>,
    pub(super) tool_results: Vec<(usize, String, ToolStatus)>,
}

impl AnalyzedRequest {
    pub(crate) fn set_branch_cursor(&mut self, leaf: Option<[u8; 32]>, path: Option<[u8; 32]>) {
        self.branch_cursor = (leaf, path);
    }

    pub(crate) fn mark_current_input_unavailable(&mut self) {
        self.current_input_start = None;
        if let Some(turn) = self.turns.last_mut() {
            turn.user = vec![VisibleContentPart::Unavailable {
                source_kind: "native_continuation_input".into(),
            }];
        }
        self.prior_history_missing = true;
    }

    pub(crate) fn select_current_input(
        &mut self,
        parts: &[ContentPart],
        position: Option<CurrentInputPosition>,
        request: &ModelRequestIRV1,
        replay: &ReplayStore,
    ) -> Result<(), AgentTurnHistoryError> {
        match position {
            Some(CurrentInputPosition::Event(index)) => self.select_event(index, request)?,
            Some(CurrentInputPosition::Window { first, last }) => {
                if first > last || last >= self.message_count {
                    return Err(AgentTurnHistoryError::Resource);
                }
                // The validated current activation can contain several input
                // envelopes and be followed by native context. Keep earlier
                // User positions for real gap detection, but do not mistake
                // this batch or its trailing context for missed executions.
                self.prior_history_missing |= self.turns.iter().any(|turn| turn.user_index > last);
                self.turns.retain(|turn| turn.user_index < first);
                self.turns.push(AnalyzedTurn {
                    user_index: last,
                    user: Vec::new(),
                });
                self.current_input_start = Some(first);
                self.output_after_latest_user = output_after(request, last);
            }
            None => {}
        }
        let turn = self
            .turns
            .last_mut()
            .ok_or(AgentTurnHistoryError::NoUserTurn)?;
        turn.user = project_user_parts(parts, replay)?;
        Ok(())
    }

    pub(crate) fn select_unavailable_event(
        &mut self,
        index: usize,
        request: &ModelRequestIRV1,
    ) -> Result<(), AgentTurnHistoryError> {
        self.select_event(index, request)?;
        self.turns
            .last_mut()
            .ok_or(AgentTurnHistoryError::NoUserTurn)?
            .user = vec![VisibleContentPart::Unavailable {
            source_kind: "native_event_input".into(),
        }];
        Ok(())
    }

    fn select_event(
        &mut self,
        index: usize,
        request: &ModelRequestIRV1,
    ) -> Result<(), AgentTurnHistoryError> {
        // A native codec may locate a non-User input event. The projection
        // describes that actual position, never an inserted portable User.
        if index >= self.message_count
            || self
                .turns
                .last()
                .is_some_and(|turn| turn.user_index > index)
        {
            return Err(AgentTurnHistoryError::Resource);
        }
        if self.turns.last().is_none_or(|turn| turn.user_index < index) {
            if let Some(previous) = self.turns.last_mut() {
                previous.user.clear();
            }
            self.turns.push(AnalyzedTurn {
                user_index: index,
                user: Vec::new(),
            });
        }
        self.output_after_latest_user = output_after(request, index);
        self.current_input_start = Some(index);
        Ok(())
    }

    pub(crate) fn mark_prior_history_missing(&mut self) {
        self.prior_history_missing = true;
    }

    pub(crate) fn mark_user_origin_unverified(&mut self) {
        self.user_origin_unverified = true;
        // Reuse the existing visible-content vocabulary: uncertainty is not
        // missing execution and must not disable assessment of accepted output.
        for turn in &mut self.turns {
            turn.user.push(VisibleContentPart::Unavailable {
                source_kind: "unverified_request_user_origin".into(),
            });
        }
    }
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
        // Earlier User positions detect missing intervals; their payloads are
        // never imported as captured executions. Only materialize current input.
        let user = if Some(&user_index) == user_indices.last() {
            project_user(&request.messages[user_index], replay)?
        } else {
            Vec::new()
        };
        turns.push(AnalyzedTurn { user_index, user });
    }
    let output_after_latest_user = user_indices
        .last()
        .is_some_and(|latest| output_after(request, *latest));
    Ok(AnalyzedRequest {
        measured: false,
        branch_cursor: (None, None),
        boundaries: Vec::new(),
        anchors: None,
        retry_setup: None,
        message_count: request.messages.len(),
        user_origin_unverified: false,
        prior_history_missing: false,
        output_after_latest_user,
        current_input_start: user_indices.last().copied(),
        turns,
        tool_results: project_tool_results(request),
    })
}

fn output_after(request: &ModelRequestIRV1, position: usize) -> bool {
    request
        .messages
        .iter()
        .enumerate()
        .skip(position + 1)
        .any(|(index, message)| {
            (message.role == MessageRole::Assistant && !message.content.is_empty())
                || message.content.iter().any(|part| {
                    matches!(
                        part,
                        ContentPart::ToolCall { .. }
                            | ContentPart::ToolResult { .. }
                            | ContentPart::ProviderState { .. }
                    )
                })
                || request.responses_search_history.contains_key(&index)
                || request.responses_reasoning_history.contains_key(&index)
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
    project_user_parts(&message.content, replay)
}

fn project_user_parts(
    parts: &[ContentPart],
    replay: &ReplayStore,
) -> Result<Vec<VisibleContentPart>, AgentTurnHistoryError> {
    parts
        .iter()
        .map(|part| match part {
            ContentPart::Text { text } => {
                resolve_text(text, replay).map(|text| VisibleContentPart::Text { text })
            }
            ContentPart::Image {
                source: ImageSource::Url { .. },
            } => Ok(VisibleContentPart::Unavailable {
                source_kind: "image_url".into(),
            }),
            ContentPart::Image {
                source: ImageSource::Base64 { .. },
            } => Ok(VisibleContentPart::Unavailable {
                source_kind: "image_base64".into(),
            }),
            ContentPart::ToolCall { .. } => Ok(VisibleContentPart::Unavailable {
                source_kind: "user_tool_call".into(),
            }),
            ContentPart::ToolResult { .. } => Ok(VisibleContentPart::Unavailable {
                source_kind: "user_tool_result".into(),
            }),
            ContentPart::ProviderState { .. } => Ok(VisibleContentPart::Unavailable {
                source_kind: "user_provider_state".into(),
            }),
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
