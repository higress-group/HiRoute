use super::ClientContext;
use crate::confirmation::CONFIRMATION_TTL;
pub use crate::web_confirmation::*;
pub(super) async fn request_web_confirmation(
    window: &ClientContext<'_>,
    prompt: WebConfirmationPrompt,
) -> bool {
    let state = window.confirmations;
    let Ok((request, receive)) = state.begin(window.label(), prompt) else {
        return false;
    };
    let id = request.confirmation_id.clone();
    if window
        .notify
        .is_some_and(|notify| notify(&request).is_err())
    {
        state.cancel(window.label(), &id);
    }
    let decision = tokio::time::timeout(CONFIRMATION_TTL, receive).await;
    if decision.is_err() {
        state.cancel(window.label(), &id);
    }
    decision.ok().and_then(Result::ok).unwrap_or(false)
}
