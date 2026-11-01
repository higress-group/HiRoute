//! Claude terminal publication and bounded history observation on the live ACP connection.
use super::{ClaudeHistoryCheckpoint, DelegationFinalizationLease, WorkerRunJournal};
use crate::delegation::acp::{AcpRunOutcome, AcpTerminalCheckpoint};
use hiroute_domain::delegation::DelegationErrorV1;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

pub(super) struct TerminalReceipt {
    pub execution: Result<AcpRunOutcome, DelegationErrorV1>,
    pub finalization: Option<DelegationFinalizationLease>,
    pub history_settled: bool,
}

#[derive(Default)]
struct State {
    receipt: Option<TerminalReceipt>,
    revoked: bool,
    stop_intent_error: Option<DelegationErrorV1>,
}

pub(super) struct LiveTerminalCheckpoint {
    journal: Arc<dyn WorkerRunJournal>,
    history: ClaudeHistoryCheckpoint,
    state: Mutex<State>,
}

impl LiveTerminalCheckpoint {
    pub fn new(journal: Arc<dyn WorkerRunJournal>, history: ClaudeHistoryCheckpoint) -> Self {
        Self {
            journal,
            history,
            state: Mutex::default(),
        }
    }

    /// Outer cancellation, the terminal callback and final cleanup share one revocation.
    /// No guard crosses an await. Retain a failed revocation as evidence without replaying it.
    pub fn revoke(&self) -> Option<DelegationErrorV1> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.revoked {
            state.revoked = true;
            state.stop_intent_error = self.journal.stop_intent().err();
        }
        state.stop_intent_error
    }

    pub fn take_receipt(&self) -> Option<TerminalReceipt> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .receipt
            .take()
    }

    pub fn has_receipt(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .receipt
            .is_some()
    }
}

impl AcpTerminalCheckpoint for LiveTerminalCheckpoint {
    fn before_transport_close<'a>(
        &'a self,
        outcome: &'a AcpRunOutcome,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let finalization = self.journal.begin_finalization();
            let execution = self.journal.completed(outcome).map(|()| outcome.clone());
            let observe_history = execution.is_ok();
            // Publish this receipt before waiting: an outer deadline/disconnect cannot
            // overwrite a task result already recorded as completed. An interrupted
            // checkpoint remains false, so it cannot make Continue eligible.
            self.state.lock().unwrap_or_else(|e| e.into_inner()).receipt = Some(TerminalReceipt {
                execution,
                finalization,
                history_settled: false,
            });
            self.revoke();
            if observe_history {
                let settled = self
                    .history
                    .wait_until_settled(outcome.session.native_session_id.as_deref())
                    .await;
                if let Some(receipt) = self
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .receipt
                    .as_mut()
                {
                    receipt.history_settled = settled;
                }
            }
        })
    }
}
