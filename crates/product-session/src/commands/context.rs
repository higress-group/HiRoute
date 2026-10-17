use crate::{
    session::Session,
    web_confirmation::{ConfirmationBroker, WebConfirmationRequest},
};
use hiroute_diagnostics::context::DiagnosticHandle;
use std::sync::{Arc, OnceLock, atomic::AtomicU64};
use tokio::sync::Mutex;
pub type ConfirmationNotifier<'a> =
    dyn Fn(&WebConfirmationRequest) -> Result<(), String> + Send + Sync + 'a;
pub struct CommandState<'a>(
    pub &'a Arc<Mutex<Option<Session>>>,
    pub &'a OnceLock<String>,
    pub &'a AtomicU64,
    pub &'a DiagnosticHandle,
);
pub struct ClientContext<'a> {
    pub id: &'a str,
    pub confirmations: &'a ConfirmationBroker,
    pub active: &'a (dyn Fn() -> bool + Send + Sync),
    pub notify: Option<&'a ConfirmationNotifier<'a>>,
}
impl ClientContext<'_> {
    pub fn label(&self) -> &str {
        self.id
    }
    pub fn is_visible(&self) -> Result<bool, String> {
        Ok((self.active)())
    }
}
pub(super) fn main_window(window: &ClientContext<'_>) -> Result<(), String> {
    if window.id.is_empty() {
        Err("CLIENT_CONTEXT_INVALID".into())
    } else {
        Ok(())
    }
}
