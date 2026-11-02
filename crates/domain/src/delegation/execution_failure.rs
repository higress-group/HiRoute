//! Closed execution facts, independent of native prose and retained across run cleanup.
use super::DelegationErrorV1;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerExecutionStageV1 {
    Preparation,
    Spawn,
    Initialize,
    Authentication,
    SessionNew,
    SessionLoad,
    SessionResume,
    ModeSelection,
    ModelSelection,
    Prompt,
}

impl WorkerExecutionStageV1 {
    pub const fn key(self) -> &'static str {
        match self {
            Self::Preparation => "preparation",
            Self::Spawn => "spawn",
            Self::Initialize => "initialize",
            Self::Authentication => "authentication",
            Self::SessionNew => "session_new",
            Self::SessionLoad => "session_load",
            Self::SessionResume => "session_resume",
            Self::ModeSelection => "mode_selection",
            Self::ModelSelection => "model_selection",
            Self::Prompt => "prompt",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerExecutionFailureV1 {
    pub stage: WorkerExecutionStageV1,
    pub error: DelegationErrorV1,
}

impl WorkerExecutionFailureV1 {
    pub const fn next_action_hint(self) -> &'static str {
        match self.error {
            DelegationErrorV1::DependencyCheckFailed(failure) if failure.reason.retryable() => {
                "Retry the same dependency selection after the temporary check failure. Inspect this run before submitting another task."
            }
            DelegationErrorV1::DependencyCheckFailed(failure) => match failure.check {
                super::NativeDependencyCheckV1::ClaudeHostManagedProvider => {
                    "The selected Claude CLI must retain the host route and credential when settings conflict. Check its host-managed provider capability; a version label does not establish support."
                }
                super::NativeDependencyCheckV1::OpenCodeDatabase => {
                    "Check the selected OpenCode CLI's JSON database command and the existing history's migration format. HiRoute does not migrate borrowed history; preserve it when changing clients."
                }
                super::NativeDependencyCheckV1::OpenCodeConfiguration => {
                    "Inspect the selected OpenCode configuration and project resources for unsupported routing, permission or executable overrides."
                }
                _ => {
                    "Inspect the selected Worker dependencies and the reported capability check before retrying."
                }
            },
            DelegationErrorV1::ResumeUnavailable => {
                "Check the original native context and exact session history. No replacement session was created."
            }
            DelegationErrorV1::Cancelled => {
                "HiRoute requested cancellation. Inspect the saved run state and progress."
            }
            _ => match self.stage {
                WorkerExecutionStageV1::Preparation => {
                    "Inspect the frozen Plan, selected dependencies and native configuration before submitting another task."
                }
                WorkerExecutionStageV1::Spawn => {
                    "Check that the selected native CLI, adapter and runtime can be launched. The run did not reach ACP initialization."
                }
                WorkerExecutionStageV1::Initialize | WorkerExecutionStageV1::Authentication => {
                    "Check the selected adapter's ACP initialization and authentication capabilities. No task prompt was sent."
                }
                WorkerExecutionStageV1::SessionNew
                | WorkerExecutionStageV1::SessionLoad
                | WorkerExecutionStageV1::SessionResume => {
                    "Check the native session capability and, for Continue, the original history. No task prompt was sent."
                }
                WorkerExecutionStageV1::ModeSelection | WorkerExecutionStageV1::ModelSelection => {
                    "The native client must confirm the exact task mode and frozen model through ACP config options. No task prompt was sent."
                }
                WorkerExecutionStageV1::Prompt => {
                    "Inspect the saved progress and model route before retrying. The prompt may have executed; do not automatically resubmit it."
                }
            },
        }
    }

    pub fn message_key(self) -> String {
        let reason = match self.error {
            DelegationErrorV1::DependencyCheckFailed(failure) => {
                return format!(
                    "worker.execution.{}.{}.{}",
                    self.stage.key(),
                    failure.check.key(),
                    failure.reason.key()
                );
            }
            DelegationErrorV1::InvalidArguments => "configuration_invalid",
            DelegationErrorV1::PermissionDenied => "permission_denied",
            DelegationErrorV1::DeadlineExceeded => "deadline_exceeded",
            DelegationErrorV1::Cancelled => "cancelled",
            DelegationErrorV1::CapabilityUnavailable => "capability_unavailable",
            DelegationErrorV1::DependenciesMissing => "dependencies_missing",
            DelegationErrorV1::DependenciesInvalid => "dependencies_invalid",
            DelegationErrorV1::DependenciesUnavailable => "dependencies_unavailable",
            DelegationErrorV1::ResumeUnavailable => "resume_unavailable",
            DelegationErrorV1::ContentUnavailable => "content_unavailable",
            DelegationErrorV1::StorageUnavailable => "storage_unavailable",
            DelegationErrorV1::ProtocolFailed => "protocol_failed",
            DelegationErrorV1::PromptFailed => "native_prompt_failed",
            DelegationErrorV1::Busy
            | DelegationErrorV1::CapacityExceeded
            | DelegationErrorV1::Conflict
            | DelegationErrorV1::NotFound => "state_unavailable",
        };
        format!("worker.execution.{}.{reason}", self.stage.key())
    }
}
