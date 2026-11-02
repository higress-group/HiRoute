//! Typed Local Control contracts for one bounded Worker delegation.
//!
//! These payloads deliberately carry no credential, upstream secret, native session material,
//! or caller-claimed authorization.  The protected collaboration material remains outside JSON
//! and Local Control verifies the current grant and permit again at acceptance.
use hiroute_domain::delegation::{
    MAX_RUN_DURATION_MS, RunCleanupV1, RunStateV1, WorkerExecutionIntentV1, WorkerHarnessV1,
    WorkerPermissionPolicyV1,
};
use hiroute_domain::{AgentPlanId, PlanExecutionRef, WorkspaceId};
use serde::{Deserialize, Serialize};

pub const DELEGATION_START_SCHEMA_V1: &str = "hiroute.delegation-start-request/v1";
pub const DELEGATION_ACCEPTED_SCHEMA_V1: &str = "hiroute.delegation-accepted/v1";
pub const DELEGATION_LIST_SCHEMA_V1: &str = "hiroute.delegation-list/v1";
pub const DELEGATION_GET_SCHEMA_V1: &str = "hiroute.delegation-get/v1";
pub const DELEGATION_WAIT_SCHEMA_V1: &str = "hiroute.delegation-wait/v1";
pub const DELEGATION_CANCEL_SCHEMA_V1: &str = "hiroute.delegation-cancel/v1";
pub const MAX_DELEGATION_INPUT_BYTES: usize = 256 * 1024;
pub const DEFAULT_DELEGATION_LIST_LIMIT: u16 = 50;
pub const MAX_DELEGATION_LIST_LIMIT: u16 = 200;
pub const DEFAULT_DELEGATION_WAIT_MS: u32 = 20_000;
pub const MAX_DELEGATION_WAIT_MS: u32 = 30_000;
pub const MAX_DELEGATION_RESULT_BYTES: u32 = 1024 * 1024;

/// Non-secret selectors that must agree with the authenticated collaboration grant.  They are
/// not an authority by themselves and are kept in request JSON only so a legacy protected-FD
/// client can select its already-issued material without exposing it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationCallerV1 {
    pub workspace_id: WorkspaceId,
    pub context_id: String,
    pub grant_id: String,
    pub grant_generation: u64,
}

impl DelegationCallerV1 {
    pub fn valid(&self) -> bool {
        WorkspaceId::parse(self.workspace_id.as_str()).is_ok()
            && self.grant_generation != 0
            && reference(&self.context_id)
            && reference(&self.grant_id)
            && self.grant_id.starts_with("collaboration-grant/")
    }
}

/// The bounded, explicit body a main Agent asks a Worker to perform.  It is persisted in the
/// managed body store by the daemon; `request_digest` is keyed there and never becomes a raw
/// prompt hash index.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationTaskInputV1 {
    pub goal: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub constraints: String,
    #[serde(default)]
    pub acceptance_criteria: String,
}

impl DelegationTaskInputV1 {
    pub fn valid(&self) -> bool {
        !self.goal.trim().is_empty()
            && [
                &self.goal,
                &self.context,
                &self.constraints,
                &self.acceptance_criteria,
            ]
            .into_iter()
            .all(|value| !value.contains('\0'))
            && self.byte_len() <= MAX_DELEGATION_INPUT_BYTES
    }

    pub fn byte_len(&self) -> usize {
        self.goal
            .len()
            .saturating_add(self.context.len())
            .saturating_add(self.constraints.len())
            .saturating_add(self.acceptance_criteria.len())
    }

    /// A stable process-local rendering sent exactly once after the journal records prompt
    /// intent.  Labels preserve the independently supplied contract fields without accepting
    /// a caller-supplied shell/template language.
    pub fn prompt(&self) -> String {
        format!(
            "Goal:\n{}\n\nContext:\n{}\n\nConstraints:\n{}\n\nAcceptance criteria:\n{}",
            self.goal, self.context, self.constraints, self.acceptance_criteria
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationStartRequestV1 {
    pub schema: String,
    pub caller: DelegationCallerV1,
    pub agent_plan_id: AgentPlanId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_plan: Option<PlanExecutionRef>,
    pub permit_id: String,
    pub permit_generation: u64,
    /// An absolute local directory; the daemon canonicalizes it and derives opaque identity.
    /// Callers cannot supply a root/volume/ancestry identity directly.
    pub workspace_path: String,
    pub execution: WorkerExecutionIntentV1,
    pub input: DelegationTaskInputV1,
    pub idempotency_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_task_ref: Option<String>,
}

impl DelegationStartRequestV1 {
    pub fn valid(&self) -> bool {
        self.schema == DELEGATION_START_SCHEMA_V1
            && self.caller.valid()
            && AgentPlanId::parse(self.agent_plan_id.as_str()).is_ok()
            && self.expected_plan.as_ref().is_none_or(|expected| {
                expected.validate().is_ok()
                    && expected.workspace_id == self.caller.workspace_id
                    && expected.plan_id == self.agent_plan_id
            })
            && reference(&self.permit_id)
            && self.permit_generation != 0
            && absolute_path(&self.workspace_path)
            && valid_execution(&self.execution)
            && self.input.valid()
            && reference(&self.idempotency_key)
            && self
                .parent_task_ref
                .as_ref()
                .is_none_or(|value| reference(value))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationAcceptedV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<hiroute_domain::delegation::WorkerExecutionFailureV1>,
    pub schema: String,
    pub task_id: String,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub state: RunStateV1,
    pub state_revision: u64,
    /// True only when the exact caller/workspace/operation/key is replayed with the same keyed
    /// request digest.  It never means a second prompt was sent.
    pub replayed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationListRequestV1 {
    pub caller: DelegationCallerV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u16>,
}

impl DelegationListRequestV1 {
    pub fn valid(&self) -> bool {
        self.caller.valid()
            && self
                .limit
                .is_none_or(|limit| limit != 0 && limit <= MAX_DELEGATION_LIST_LIMIT)
            && self
                .cursor
                .as_ref()
                .is_none_or(|cursor| valid_sequence_cursor(cursor))
    }

    pub fn effective_limit(&self) -> u16 {
        self.limit.unwrap_or(DEFAULT_DELEGATION_LIST_LIMIT)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationGetRequestV1 {
    pub caller: DelegationCallerV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission_operation: Option<DelegationSubmissionOperationV1>,
}

impl DelegationGetRequestV1 {
    pub fn valid(&self) -> bool {
        if !self.caller.valid() {
            return false;
        }
        match (
            &self.task_id,
            &self.run_id,
            &self.submission_key,
            self.submission_operation,
        ) {
            (Some(task), run, None, None) => {
                reference(task) && run.as_ref().is_none_or(|run| reference(run))
            }
            (None, Some(run), None, None) => reference(run),
            (None, None, Some(key), Some(_)) => reference(key),
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationSubmissionOperationV1 {
    Start,
    Continue,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationWaitRequestV1 {
    pub caller: DelegationCallerV1,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_ms: Option<u32>,
}

impl DelegationWaitRequestV1 {
    pub fn valid(&self) -> bool {
        self.caller.valid()
            && reference(&self.run_id)
            && self
                .wait_ms
                .is_none_or(|wait| wait != 0 && wait <= MAX_DELEGATION_WAIT_MS)
    }

    pub fn effective_wait_ms(&self) -> u32 {
        self.wait_ms.unwrap_or(DEFAULT_DELEGATION_WAIT_MS)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationResultRequestV1 {
    pub caller: DelegationCallerV1,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u32>,
}

impl DelegationResultRequestV1 {
    pub fn valid(&self) -> bool {
        self.caller.valid()
            && reference(&self.run_id)
            && self
                .max_bytes
                .is_none_or(|size| size != 0 && size <= MAX_DELEGATION_RESULT_BYTES)
    }

    pub fn effective_max_bytes(&self) -> u32 {
        self.max_bytes.unwrap_or(MAX_DELEGATION_RESULT_BYTES)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationCancelRequestV1 {
    pub caller: DelegationCallerV1,
    pub run_id: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub reason: String,
}

impl DelegationCancelRequestV1 {
    pub fn valid(&self) -> bool {
        self.caller.valid()
            && reference(&self.run_id)
            && reference(&self.idempotency_key)
            && (self.reason.is_empty() || reference(&self.reason))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationContinueRequestV1 {
    pub caller: DelegationCallerV1,
    pub task_id: String,
    pub expected_latest_run_id: String,
    pub permit_id: String,
    pub permit_generation: u64,
    pub execution: WorkerExecutionIntentV1,
    pub input: DelegationTaskInputV1,
    pub idempotency_key: String,
}

impl DelegationContinueRequestV1 {
    pub fn valid(&self) -> bool {
        self.caller.valid()
            && reference(&self.task_id)
            && reference(&self.expected_latest_run_id)
            && reference(&self.permit_id)
            && self.permit_generation != 0
            && valid_execution(&self.execution)
            && self.input.valid()
            && reference(&self.idempotency_key)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskFeedbackOutcomeV1 {
    Accepted,
    NeedsRework,
    Failed,
    Abstain,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationFeedbackRequestV1 {
    pub caller: DelegationCallerV1,
    pub run_id: String,
    pub expected_revision: u64,
    pub outcome: TaskFeedbackOutcomeV1,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub evidence: String,
    pub idempotency_key: String,
}

impl DelegationFeedbackRequestV1 {
    pub fn valid(&self) -> bool {
        self.caller.valid()
            && reference(&self.run_id)
            && self.expected_revision != 0
            && self.reason.len().saturating_add(self.evidence.len()) <= MAX_DELEGATION_INPUT_BYTES
            && !self.reason.contains('\0')
            && !self.evidence.contains('\0')
            && reference(&self.idempotency_key)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationRunViewV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<hiroute_domain::delegation::WorkerExecutionFailureV1>,
    pub task_id: String,
    pub run_id: String,
    pub ordinal: u64,
    pub continued_from: Option<String>,
    pub admission_sequence: u64,
    pub accepted_at_ms: Option<u64>,
    pub accepted_time_state: DelegationAcceptedTimeStateV1,
    pub executor: WorkerExecutorPresentationV1,
    pub state: RunStateV1,
    pub state_revision: u64,
    pub cleanup: RunCleanupV1,
    pub result_available: bool,
    pub scope: DelegationRunScopeV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationAcceptedTimeStateV1 {
    Recorded,
    LegacyUnavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerExecutorPresentationBasisV1 {
    FrozenPlan,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerExecutorPresentationV1 {
    pub harness: Option<WorkerHarnessV1>,
    pub display_name: Option<String>,
    pub basis: WorkerExecutorPresentationBasisV1,
}

/// Persisted runtime facts for the selected run. These values describe execution context and
/// never authorize a caller, directory, tool, model, or task.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationRunScopeV1 {
    pub canonical_cwd: String,
    pub permission_policy: WorkerPermissionPolicyV1,
    pub deadline_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationContentAvailabilityV1 {
    Available,
    Unavailable,
    Indeterminate,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationTaskViewV1 {
    pub task_id: String,
    pub created_at_ms: u64,
    pub latest_admission_sequence: u64,
    /// A bounded mechanical projection of the first visible Goal line, never a model summary.
    pub title: Option<String>,
    /// A bounded mechanical projection of the visible initial Goal, never separately persisted.
    pub brief: Option<String>,
    pub content_availability: DelegationContentAvailabilityV1,
    pub plan_id: AgentPlanId,
    pub plan_revision: u64,
    pub latest_run_id: String,
    pub run: DelegationRunViewV1,
    pub resumable_until_ms: Option<u64>,
    /// Confirmed Gateway observation sessions related to this exact run. Empty with
    /// `session_links_complete=true` means that no association has been observed yet.
    #[serde(default)]
    pub session_ids: Vec<String>,
    #[serde(default)]
    pub session_links_complete: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationListV1 {
    pub schema: String,
    pub tasks: Vec<DelegationTaskViewV1>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationGetV1 {
    pub schema: String,
    pub task: DelegationTaskViewV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationWaitV1 {
    pub schema: String,
    pub run: DelegationRunViewV1,
    pub changed: bool,
    pub timed_out: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationCancelV1 {
    pub schema: String,
    pub operation_id: String,
    pub run: DelegationRunViewV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationResultV1 {
    pub schema: String,
    pub run: DelegationRunViewV1,
    pub text: Option<String>,
    pub next_offset: Option<u32>,
    pub incomplete: bool,
}

fn reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.starts_with('/')
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_./:-".contains(&byte))
}

fn absolute_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.contains('\0')
        && std::path::Path::new(value).is_absolute()
}

pub(crate) fn valid_sequence_cursor(value: &str) -> bool {
    value
        .strip_prefix("sequence/")
        .filter(|sequence| !sequence.starts_with('0'))
        .and_then(|sequence| sequence.parse::<u64>().ok())
        .is_some_and(|sequence| sequence != 0)
}

fn valid_execution(value: &WorkerExecutionIntentV1) -> bool {
    reference(&value.root_identity)
        && value.duration_ms != 0
        && value.duration_ms <= MAX_RUN_DURATION_MS
        && value.delegation_depth == 1
        && value.tools.len() <= 3
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_domain::delegation::{WorkerNetworkV1, WorkerToolV1, WorkspaceAccessV1};

    const WORKSPACE: &str = if cfg!(windows) {
        "C:/workspace"
    } else {
        "/workspace"
    };

    fn request() -> DelegationStartRequestV1 {
        DelegationStartRequestV1 {
            schema: DELEGATION_START_SCHEMA_V1.to_owned(),
            caller: DelegationCallerV1 {
                workspace_id: WorkspaceId::default(),
                context_id: "owner".to_owned(),
                grant_id: "collaboration-grant/one".to_owned(),
                grant_generation: 1,
            },
            agent_plan_id: AgentPlanId::parse("plan").unwrap(),
            expected_plan: None,
            permit_id: "permit".to_owned(),
            permit_generation: 1,
            workspace_path: WORKSPACE.to_owned(),
            execution: WorkerExecutionIntentV1 {
                root_identity: "root".to_owned(),
                access: WorkspaceAccessV1::ReadOnly,
                tools: vec![WorkerToolV1::Read],
                network: WorkerNetworkV1::GatewayOnly,
                duration_ms: 1,
                delegation_depth: 1,
            },
            input: DelegationTaskInputV1 {
                goal: "summarize".to_owned(),
                context: String::new(),
                constraints: String::new(),
                acceptance_criteria: String::new(),
            },
            idempotency_key: "start/one".to_owned(),
            parent_task_ref: None,
        }
    }

    #[test]
    fn start_contract_rejects_relative_paths_nested_delegation_and_oversized_bodies() {
        let mut value = request();
        assert!(value.valid());
        value.workspace_path = "relative".to_owned();
        assert!(!value.valid());
        value.workspace_path = WORKSPACE.to_owned();
        assert!(value.valid());
        if cfg!(windows) {
            for rooted_or_drive_relative in ["/workspace", r"\workspace", "C:workspace"] {
                value.workspace_path = rooted_or_drive_relative.into();
                assert!(!value.valid(), "{rooted_or_drive_relative}");
            }
            value.workspace_path = WORKSPACE.to_owned();
        }
        value.execution.delegation_depth = 2;
        assert!(!value.valid());
        value.execution.delegation_depth = 1;
        value.input.goal = "x".repeat(MAX_DELEGATION_INPUT_BYTES + 1);
        assert!(!value.valid());
    }

    #[test]
    fn task_prompt_has_fixed_sections_and_never_treats_input_as_a_template() {
        let mut value = request();
        value.input.context = "${not-expanded}".to_owned();
        let prompt = value.input.prompt();
        assert!(prompt.contains("Goal:\n"));
        assert!(prompt.contains("Context:\n${not-expanded}"));
    }

    #[test]
    fn get_contract_accepts_exactly_one_task_run_or_submission_selector() {
        let caller = request().caller;
        let mut value = DelegationGetRequestV1 {
            caller,
            task_id: Some("task/one".into()),
            run_id: Some("run/one".into()),
            submission_key: None,
            submission_operation: None,
        };
        assert!(value.valid());
        value.submission_key = Some("start/one".into());
        value.submission_operation = Some(DelegationSubmissionOperationV1::Start);
        assert!(!value.valid());
        value.task_id = None;
        value.run_id = None;
        assert!(value.valid());
        value.submission_operation = None;
        assert!(!value.valid());
        value.submission_key = None;
        value.run_id = Some("run/one".into());
        assert!(value.valid());
    }

    #[test]
    fn list_cursor_is_the_exact_exclusive_sequence_form() {
        let mut value = DelegationListRequestV1 {
            caller: request().caller,
            cursor: Some("sequence/42".into()),
            limit: Some(1),
        };
        assert!(value.valid());
        value.cursor = Some("sequence/0".into());
        assert!(!value.valid());
        value.cursor = Some("other/42".into());
        assert!(!value.valid());
    }

    #[test]
    fn cancel_contract_rejects_control_text_and_oversized_reasons() {
        let mut value = DelegationCancelRequestV1 {
            caller: request().caller,
            run_id: "run/one".into(),
            idempotency_key: "cancel/one".into(),
            reason: "user-requested".into(),
        };
        assert!(value.valid());
        value.reason = "bad\nreason".into();
        assert!(!value.valid());
        value.reason = "x".repeat(257);
        assert!(!value.valid());
    }
}
