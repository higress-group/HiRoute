//! Structural loss accounting. These functions never emit diagnostics during preview.
use crate::server::core_runtime::model_ir::{
    ContentPart, ModelRequestIRV1, ModelResponseIRV1, ResponseBlock,
};
use crate::server::request_plan::IngressProtocol;

pub(crate) fn request_loss(
    request: &ModelRequestIRV1,
    target: IngressProtocol,
    prefix: Option<usize>,
) -> usize {
    request
        .messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            if request.responses_reasoning_history.contains_key(&index) {
                if prefix.is_some_and(|end| index < end) || target == IngressProtocol::Messages {
                    return 1;
                }
                return usize::from(
                    target != IngressProtocol::Responses
                        && message
                            .content
                            .iter()
                            .any(|part| matches!(part, ContentPart::ProviderState { .. })),
                );
            }
            message
                .content
                .iter()
                .filter(|part| {
                    let ContentPart::ProviderState { state } = part else {
                        return false;
                    };
                    if prefix.is_some_and(|end| index < end) {
                        return true;
                    }
                    match target {
                        IngressProtocol::Responses => {
                            state.kind != "encrypted_content" && state.kind != "reasoning_content"
                        }
                        IngressProtocol::Messages => {
                            !matches!(state.kind.as_str(), "thinking" | "redacted_thinking")
                        }
                        IngressProtocol::ChatCompletions => state.kind != "reasoning_content",
                    }
                })
                .count()
        })
        .sum()
}

pub(crate) fn response_loss(response: &ModelResponseIRV1, target: IngressProtocol) -> usize {
    let state = response
        .provider_state
        .iter()
        .filter(|state| match target {
            IngressProtocol::Responses => state.kind != "encrypted_content",
            IngressProtocol::Messages => !matches!(
                state.kind.as_str(),
                "thinking_signature" | "redacted_thinking"
            ),
            IngressProtocol::ChatCompletions => true,
        })
        .count();
    state
        + if target == IngressProtocol::Messages
            && response.source_protocol != IngressProtocol::Messages
        {
            response
                .blocks
                .iter()
                .filter(|block| matches!(block, ResponseBlock::Reasoning { .. }))
                .count()
        } else {
            0
        }
}
