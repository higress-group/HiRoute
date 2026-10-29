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
    /// The originating client still exists and remains authorized. Hiding a
    /// native window does not invalidate its ordinary reads.
    pub active: &'a (dyn Fn() -> bool + Send + Sync),
    /// Presentation required only when accepting a user confirmation.
    pub visible: &'a (dyn Fn() -> bool + Send + Sync),
    pub notify: Option<&'a ConfirmationNotifier<'a>>,
}
impl ClientContext<'_> {
    pub fn label(&self) -> &str {
        self.id
    }
    pub fn is_visible(&self) -> Result<bool, String> {
        Ok(self.is_active() && (self.visible)())
    }
    pub fn is_active(&self) -> bool {
        (self.active)()
    }
}
pub(super) fn main_window(window: &ClientContext<'_>) -> Result<(), String> {
    if window.id.is_empty() || !window.is_active() {
        Err("CLIENT_CONTEXT_INVALID".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_live_clients_can_read_but_cannot_confirm_and_revoked_clients_can_do_neither() {
        let broker = ConfirmationBroker::default();
        for active in [false, true] {
            for visible in [false, true] {
                let context = ClientContext {
                    id: "main",
                    confirmations: &broker,
                    active: &|| active,
                    visible: &|| visible,
                    notify: None,
                };
                assert_eq!(main_window(&context).is_ok(), active);
                assert_eq!(context.is_visible().unwrap(), active && visible);
            }
        }
    }
}
