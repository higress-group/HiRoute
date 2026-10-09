use super::DelegationErrorV1;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStateV1 {
    Accepted,
    Preparing,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunCleanupV1 {
    Pending,
    Complete,
    Unknown,
    ResidualAcknowledged,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunProgressV1 {
    pub state: RunStateV1,
    pub revision: u64,
    pub prompt_may_have_executed: bool,
    pub cancel_requested: bool,
    pub cleanup: RunCleanupV1,
    #[serde(default)]
    pub process_running: bool,
}

impl Default for RunProgressV1 {
    fn default() -> Self {
        Self {
            state: RunStateV1::Accepted,
            revision: 1,
            prompt_may_have_executed: false,
            cancel_requested: false,
            cleanup: RunCleanupV1::Pending,
            process_running: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunEventV1 {
    Preparing,
    /// Persist BEFORE sending; a crash can no longer justify replaying the prompt.
    PromptSendIntent,
    PromptCompleted,
    /// The ACP prompt response authoritatively reported a terminal error.
    PromptFailed,
    CancelRequested,
    ConnectionLost,
    RootProcessExited {
        success: bool,
    },
    LaunchFailedBeforeSpawn,
    ProcessRunning,
    ProcessUnknown,
    ResidualAcknowledged,
    ManagedScopeStopped {
        success: bool,
    },
}

impl RunProgressV1 {
    pub fn workspace_releasable(&self) -> bool {
        matches!(
            self.cleanup,
            RunCleanupV1::Complete | RunCleanupV1::ResidualAcknowledged
        ) && !self.process_running
            && !matches!(
                self.state,
                RunStateV1::Accepted
                    | RunStateV1::Preparing
                    | RunStateV1::Running
                    | RunStateV1::Cancelling
            )
    }

    /// Persistence uses revision CAS around this pure transition. A late event cannot
    /// recreate an execution or clear the evidence that a prompt may have run.
    pub fn advance(&mut self, event: RunEventV1) -> Result<(), DelegationErrorV1> {
        use RunEventV1 as E;
        use RunStateV1 as S;
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(DelegationErrorV1::Conflict)?;
        match (self.state, event) {
            (S::Accepted, E::Preparing) => self.state = S::Preparing,
            (S::Preparing, E::PromptSendIntent) if !self.prompt_may_have_executed => {
                self.prompt_may_have_executed = true;
                self.state = S::Running;
            }
            (S::Running, E::PromptCompleted) => self.state = S::Succeeded,
            (S::Running, E::PromptFailed) => self.state = S::Failed,
            (S::Accepted, E::CancelRequested) => {
                // Preparing is persisted before spawn. This revision change also fences an
                // executor that read Accepted but has not yet acquired launch authority.
                self.cancel_requested = true;
                self.state = S::Cancelled;
                self.cleanup = RunCleanupV1::Complete;
            }
            (S::Preparing | S::Running | S::Unknown, E::CancelRequested) => {
                self.cancel_requested = true;
                self.state = S::Cancelling;
            }
            (S::Cancelling, E::CancelRequested | E::PromptCompleted) => return Ok(()),
            (S::Succeeded | S::Failed | S::Cancelled, E::CancelRequested) => return Ok(()),
            (S::Accepted | S::Preparing | S::Running | S::Cancelling, E::ConnectionLost) => {
                self.state = if self.prompt_may_have_executed {
                    S::Unknown
                } else {
                    S::Failed
                };
                self.cleanup = RunCleanupV1::Unknown;
            }
            (S::Accepted | S::Preparing | S::Cancelling, E::LaunchFailedBeforeSpawn)
                if !self.prompt_may_have_executed =>
            {
                self.process_running = false;
                self.cleanup = RunCleanupV1::Complete;
                self.state = if self.cancel_requested {
                    S::Cancelled
                } else {
                    S::Failed
                };
            }
            (state, E::ProcessRunning) => {
                self.process_running = true;
                self.cleanup = RunCleanupV1::Pending;
                self.state = state;
            }
            (S::Unknown | S::Failed | S::Succeeded | S::Cancelled, E::ResidualAcknowledged)
                if self.cleanup == RunCleanupV1::Unknown && !self.process_running =>
            {
                self.cleanup = RunCleanupV1::ResidualAcknowledged;
            }
            (state, E::ProcessUnknown) => {
                self.process_running = false;
                self.cleanup = RunCleanupV1::Unknown;
                self.state = match state {
                    S::Succeeded | S::Failed | S::Cancelled => state,
                    _ if self.prompt_may_have_executed => S::Unknown,
                    _ => S::Failed,
                };
            }
            (state, E::RootProcessExited { .. }) => {
                self.process_running = false;
                self.process_running = false;
                self.cleanup = RunCleanupV1::Unknown;
                self.state = match state {
                    S::Succeeded | S::Failed | S::Cancelled => state,
                    _ if self.prompt_may_have_executed => S::Unknown,
                    _ => S::Failed,
                };
            }
            (state, E::ManagedScopeStopped { success }) => {
                self.process_running = false;
                self.cleanup = RunCleanupV1::Complete;
                self.state = match state {
                    _ if self.cancel_requested => S::Cancelled,
                    S::Cancelling => S::Cancelled,
                    S::Succeeded | S::Failed | S::Cancelled | S::Unknown => state,
                    _ if self.prompt_may_have_executed && success => S::Unknown,
                    _ => S::Failed,
                };
            }
            _ => return Err(DelegationErrorV1::Conflict),
        }
        self.revision = revision;
        Ok(())
    }
}
