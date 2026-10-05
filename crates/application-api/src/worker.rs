//! Public local-trust Worker contracts.
//!
//! Requests are scoped to the local daemon instance. A workspace path and permission policy are
//! execution inputs, never caller identity or durable ambient authority.

use hiroute_domain::delegation::{
    MAX_RUN_DURATION_MS, MAX_WORKER_CONCURRENCY, RunStateV1, WorkerExecutionIntentV1,
    WorkerHarnessV1, WorkerNetworkV1, WorkerPermissionPolicyV1, WorkerToolV1, WorkspaceAccessV1,
};
use hiroute_domain::{
    AgentPlanId, CHANGE_SPEC_SCHEMA_V1, CanonicalDigest, ChangeSpecV1, RevisionSetV1,
    WorkerDependencySelectionChangeV1, WorkerDependencySelectionRecordV1,
};
use serde::{Deserialize, Serialize};

use crate::{
    DEFAULT_DELEGATION_LIST_LIMIT, DelegationSubmissionOperationV1, DelegationTaskInputV1,
    MAX_DELEGATION_INPUT_BYTES, MAX_DELEGATION_LIST_LIMIT, MAX_DELEGATION_RESULT_BYTES,
};

pub const WORKER_EXEC_SCHEMA_V1: &str = "hiroute.worker-exec-request/v1";
pub const WORKER_CONTINUE_SCHEMA_V1: &str = "hiroute.worker-continue-request/v1";
pub const WORKER_COMMAND_DATA_SCHEMA_V1: &str = "hiroute.worker-command-data/v1";
pub const WORKER_EXECUTOR_AVAILABILITY_OPERATION_V1: &str = "WorkerExecutorAvailability";
pub const WORKER_EXECUTOR_AVAILABILITY_SCHEMA_V1: &str =
    "hiroute.worker-executor-availability-list/v1";
pub const WORKER_SETTINGS_GET_OPERATION_V1: &str = "WorkerSettingsGet";
pub const WORKER_SETTINGS_SET_OPERATION_V1: &str = "WorkerSettingsSet";
pub const WORKER_SETTINGS_SCHEMA_V1: &str = "hiroute.worker-settings/v1";
pub const WORKER_DEPENDENCIES_DISCOVER_OPERATION_V1: &str = "WorkerDependenciesDiscover";
pub const WORKER_DEPENDENCIES_SELECT_OPERATION_V1: &str = "SelectWorkerDependencies";
pub const WORKER_DEPENDENCIES_VIEW_SCHEMA_V1: &str = "hiroute.worker-dependencies-view/v1";
pub const DEFAULT_WORKER_WAIT_SECS: u32 = 30;
pub const MAX_WORKER_WAIT_SECS: u32 = 30;
pub const DEFAULT_WORKER_RUN_SECS: u32 = 3_600;
pub const MAX_WORKER_RUN_SECS: u32 = (MAX_RUN_DURATION_MS / 1_000) as u32;
pub const DEFAULT_WORKER_READ_BYTES: u32 = 16 * 1024;
pub const MAX_WORKER_READ_BYTES: u32 = 32 * 1024;
pub const MAX_WORKER_CURSOR_BYTES: usize = 4 * 1024;
pub const MAX_WORKER_TITLE_INPUT_BYTES: usize = 4 * 1024;
pub const MAX_WORKER_TITLE_BYTES: usize = 1024;
pub const UNNAMED_WORKER_TASK_TITLE: &str = "未命名任务";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerSettingsV1 {
    pub schema: String,
    pub max_concurrent: u16,
}

impl WorkerSettingsV1 {
    pub fn new(max_concurrent: u16) -> Self {
        Self {
            schema: WORKER_SETTINGS_SCHEMA_V1.to_owned(),
            max_concurrent,
        }
    }

    pub fn valid(&self) -> bool {
        self.schema == WORKER_SETTINGS_SCHEMA_V1
            && (1..=MAX_WORKER_CONCURRENCY).contains(&self.max_concurrent)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerDependenciesDiscoverRequestV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<WorkerHarnessV1>,
}

impl WorkerDependenciesDiscoverRequestV1 {
    pub fn valid(&self) -> bool {
        true
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerDependenciesSelectRequestV1 {
    pub harness: WorkerHarnessV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_path: Option<String>,
    pub cli_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_path: Option<String>,
    pub expected_selection_revision: u64,
}

impl WorkerDependenciesSelectRequestV1 {
    pub fn valid(&self) -> bool {
        hiroute_domain::delegation::WorkerLaunchFormV1::validate(
            self.harness,
            self.adapter_path.as_deref(),
            &self.cli_path,
            self.node_path.as_deref(),
        )
        .is_ok()
    }
}

/// Pure, shared preparation for the protected dependency-selection Operation. Callers must pass
/// paths already normalized by the native metadata validator; discovery candidates are
/// intentionally not part of the digest or CAS authority.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkerDependencySelectionPlanV1 {
    pub spec: ChangeSpecV1,
    pub expected_revisions: RevisionSetV1,
    pub accept_digest: CanonicalDigest,
    pub idempotency_key: String,
    pub change: WorkerDependencySelectionChangeV1,
}

pub fn plan_worker_dependency_selection(
    request: &WorkerDependenciesSelectRequestV1,
) -> Option<WorkerDependencySelectionPlanV1> {
    if !request.valid() {
        return None;
    }
    let selection = WorkerDependencySelectionRecordV1::new(
        request.harness,
        request.adapter_path.clone(),
        request.cli_path.clone(),
        request.node_path.clone(),
    )
    .ok()?;
    let change =
        WorkerDependencySelectionChangeV1::new(request.expected_selection_revision, selection)
            .ok()?;
    let harness = match request.harness {
        WorkerHarnessV1::CodexCli => "codex_cli",
        WorkerHarnessV1::ClaudeCode => "claude_code",
        WorkerHarnessV1::QoderCli => "qoder_cli",
        WorkerHarnessV1::Pi => "pi",
    };
    let spec = ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: "worker.dependencies.select".to_owned(),
        resource_id: Some(format!("worker-dependency-selection/{harness}")),
        desired_state: serde_json::to_value(&change.after_selection).ok()?,
    };
    let expected_revisions = RevisionSetV1 {
        target: request.expected_selection_revision,
        dependencies: std::collections::BTreeMap::new(),
    };
    let accept_digest = spec.canonical_digest(&expected_revisions).ok()?;
    let request_digest = CanonicalDigest::of(&serde_json::json!({
        "schema": "worker-dependency-selection-request/v1",
        "request": request,
    }))
    .ok()?;
    let idempotency_key = format!(
        "worker-dependency-selection:{}",
        &request_digest.as_str()["sha256:".len()..]
    );
    Some(WorkerDependencySelectionPlanV1 {
        spec,
        expected_revisions,
        accept_digest,
        idempotency_key,
        change,
    })
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerDependencyComponentV1 {
    Cli,
    Adapter,
    Node,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerDependencyCandidateSourceV1 {
    Selected,
    Path,
    Common,
    NpmGlobal,
    NpxCache,
    Manual,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerDependencyCandidateStateV1 {
    Found,
    Missing,
    Invalid,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerDependencyCandidateV1 {
    pub harness: WorkerHarnessV1,
    pub component: WorkerDependencyComponentV1,
    pub path: String,
    pub source: WorkerDependencyCandidateSourceV1,
    pub state: WorkerDependencyCandidateStateV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerDependencySelectionRevisionV1 {
    pub harness: WorkerHarnessV1,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerDependencySelectionV1 {
    pub harness: WorkerHarnessV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_path: Option<String>,
    pub cli_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_path: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerDependencyInstallHintV1 {
    pub harness: WorkerHarnessV1,
    pub component: WorkerDependencyComponentV1,
    pub platform: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub reason_code: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerDependenciesViewV1 {
    pub schema: String,
    pub selection_revisions: Vec<WorkerDependencySelectionRevisionV1>,
    pub candidates: Vec<WorkerDependencyCandidateV1>,
    pub selected: Vec<WorkerDependencySelectionV1>,
    pub install_hints: Vec<WorkerDependencyInstallHintV1>,
}

impl WorkerDependenciesViewV1 {
    pub fn valid(&self) -> bool {
        use std::collections::BTreeSet;
        let revisions = self
            .selection_revisions
            .iter()
            .map(|entry| entry.harness)
            .collect::<BTreeSet<_>>();
        let selections = self
            .selected
            .iter()
            .map(|entry| entry.harness)
            .collect::<BTreeSet<_>>();
        self.schema == WORKER_DEPENDENCIES_VIEW_SCHEMA_V1
            && !self.selection_revisions.is_empty()
            && self.selection_revisions.len() <= 3
            && revisions.len() == self.selection_revisions.len()
            && selections.len() == self.selected.len()
            && self.selected.iter().all(|selection| {
                hiroute_domain::delegation::WorkerLaunchFormV1::validate(
                    selection.harness,
                    selection.adapter_path.as_deref(),
                    &selection.cli_path,
                    selection.node_path.as_deref(),
                )
                .is_ok()
            })
            && self.candidates.len() <= 256
            && self.candidates.iter().all(|candidate| {
                absolute_path(&candidate.path)
                    && candidate
                        .reason_code
                        .as_ref()
                        .is_none_or(|reason| reference(reason))
            })
            && self.install_hints.len() <= 32
            && self.install_hints.iter().all(|hint| {
                reference(&hint.platform)
                    && reference(&hint.reason_code)
                    && hint.command.as_ref().is_none_or(|command| {
                        !command.is_empty()
                            && command.len() <= 4096
                            && !command.contains(char::is_control)
                    })
            })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerSubmissionStateV1 {
    Accepted,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerExecutorAvailabilityStateV1 {
    Ready,
    Unavailable,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerExecutorAvailabilityReasonV1 {
    RuntimeUnavailable,
    InstallationNotConfigured,
    CapabilityUnverified,
    ArtifactUnavailable,
    ResumeUnavailable,
    RestrictedPolicyUnverified,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerExecutorCapabilityAvailabilityV1 {
    pub state: WorkerExecutorAvailabilityStateV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<WorkerExecutorAvailabilityReasonV1>,
}

impl WorkerExecutorCapabilityAvailabilityV1 {
    pub fn valid(&self) -> bool {
        matches!(
            (self.state, self.reason),
            (WorkerExecutorAvailabilityStateV1::Ready, None)
                | (
                    WorkerExecutorAvailabilityStateV1::Unavailable
                        | WorkerExecutorAvailabilityStateV1::Unknown,
                    Some(_)
                )
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerExecutorAvailabilityV1 {
    pub harness: WorkerHarnessV1,
    pub state: WorkerExecutorAvailabilityStateV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<WorkerExecutorAvailabilityReasonV1>,
    pub start_approve_all: WorkerExecutorCapabilityAvailabilityV1,
    pub cancel: WorkerExecutorCapabilityAvailabilityV1,
    pub continue_session: WorkerExecutorCapabilityAvailabilityV1,
    pub restricted_policy: WorkerExecutorCapabilityAvailabilityV1,
}

impl WorkerExecutorAvailabilityV1 {
    pub fn valid(&self) -> bool {
        let overall = WorkerExecutorCapabilityAvailabilityV1 {
            state: self.state,
            reason: self.reason,
        };
        overall.valid()
            && self.start_approve_all.valid()
            && self.cancel.valid()
            && self.continue_session.valid()
            && self.restricted_policy.valid()
            && (self.state != WorkerExecutorAvailabilityStateV1::Ready
                || (self.start_approve_all.state == WorkerExecutorAvailabilityStateV1::Ready
                    && self.cancel.state == WorkerExecutorAvailabilityStateV1::Ready))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerExecutorAvailabilityListV1 {
    pub schema: String,
    pub executors: Vec<WorkerExecutorAvailabilityV1>,
}

impl WorkerExecutorAvailabilityListV1 {
    pub fn valid(&self) -> bool {
        self.schema == WORKER_EXECUTOR_AVAILABILITY_SCHEMA_V1
            && self.executors.len() == 4
            && self.executors[0].harness == WorkerHarnessV1::CodexCli
            && self.executors[1].harness == WorkerHarnessV1::ClaudeCode
            && self.executors[2].harness == WorkerHarnessV1::QoderCli
            && self.executors[3].harness == WorkerHarnessV1::Pi
            && self
                .executors
                .iter()
                .all(WorkerExecutorAvailabilityV1::valid)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerPlansRequestV1 {}

impl WorkerPlansRequestV1 {
    pub fn valid(&self) -> bool {
        true
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerListRequestV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u16>,
}

impl WorkerListRequestV1 {
    pub fn valid(&self) -> bool {
        self.title
            .as_ref()
            .is_none_or(|title| normalize_worker_title(title).is_ok())
            && self
                .cursor
                .as_ref()
                .is_none_or(|cursor| valid_opaque_cursor(cursor))
            && self
                .limit
                .is_none_or(|limit| limit != 0 && limit <= MAX_DELEGATION_LIST_LIMIT)
    }

    pub fn effective_limit(&self) -> u16 {
        self.limit.unwrap_or(DEFAULT_DELEGATION_LIST_LIMIT)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerExecRequestV1 {
    pub schema: String,
    pub plan_id: AgentPlanId,
    pub cwd: String,
    #[serde(default)]
    pub permission_policy: WorkerPermissionPolicyV1,
    pub run_timeout_secs: u32,
    pub input: DelegationTaskInputV1,
    pub submission_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_task_ref: Option<String>,
}

impl WorkerExecRequestV1 {
    pub fn valid(&self) -> bool {
        self.schema == WORKER_EXEC_SCHEMA_V1
            && AgentPlanId::parse(self.plan_id.as_str()).is_ok()
            && absolute_path(&self.cwd)
            && valid_run_timeout(self.run_timeout_secs)
            && self.input.valid()
            && self.input.prompt().len() <= MAX_DELEGATION_INPUT_BYTES
            && reference(&self.submission_key)
            && self
                .title
                .as_ref()
                .is_none_or(|title| normalize_worker_title(title).is_ok())
            && self
                .parent_task_ref
                .as_ref()
                .is_none_or(|value| reference(value))
    }

    pub fn execution(&self, root_identity: String) -> WorkerExecutionIntentV1 {
        WorkerExecutionIntentV1 {
            root_identity,
            access: WorkspaceAccessV1::TrustedNative,
            tools: vec![WorkerToolV1::Read, WorkerToolV1::Edit, WorkerToolV1::Shell],
            network: WorkerNetworkV1::Allowed,
            duration_ms: u64::from(self.run_timeout_secs) * 1_000,
            delegation_depth: 1,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerStatusRequestV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission_operation: Option<DelegationSubmissionOperationV1>,
}

impl WorkerStatusRequestV1 {
    pub fn valid(&self) -> bool {
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerWaitRequestV1 {
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_revision: Option<u64>,
    pub wait_timeout_secs: u32,
}

impl WorkerWaitRequestV1 {
    pub fn valid(&self) -> bool {
        reference(&self.run_id) && (1..=MAX_WORKER_WAIT_SECS).contains(&self.wait_timeout_secs)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerResultRequestV1 {
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u32>,
}

impl WorkerResultRequestV1 {
    pub fn valid(&self) -> bool {
        reference(&self.run_id)
            && self
                .max_bytes
                .is_none_or(|size| size != 0 && size <= MAX_DELEGATION_RESULT_BYTES)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerCancelRequestV1 {
    pub run_id: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub reason: String,
}

impl WorkerCancelRequestV1 {
    pub fn valid(&self) -> bool {
        reference(&self.run_id)
            && reference(&self.idempotency_key)
            && (self.reason.is_empty() || reference(&self.reason))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerContinueRequestV1 {
    pub schema: String,
    pub task_id: String,
    pub expected_latest_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default)]
    pub permission_policy: WorkerPermissionPolicyV1,
    pub run_timeout_secs: u32,
    pub input: DelegationTaskInputV1,
    pub submission_key: String,
}

impl WorkerContinueRequestV1 {
    pub fn valid(&self) -> bool {
        self.schema == WORKER_CONTINUE_SCHEMA_V1
            && reference(&self.task_id)
            && reference(&self.expected_latest_run_id)
            && self.cwd.as_ref().is_none_or(|path| absolute_path(path))
            && valid_run_timeout(self.run_timeout_secs)
            && self.input.valid()
            && self.input.prompt().len() <= MAX_DELEGATION_INPUT_BYTES
            && reference(&self.submission_key)
    }

    pub fn execution(&self, root_identity: String) -> WorkerExecutionIntentV1 {
        WorkerExecutionIntentV1 {
            root_identity,
            access: WorkspaceAccessV1::TrustedNative,
            tools: vec![WorkerToolV1::Read, WorkerToolV1::Edit, WorkerToolV1::Shell],
            network: WorkerNetworkV1::Allowed,
            duration_ms: u64::from(self.run_timeout_secs) * 1_000,
            delegation_depth: 1,
        }
    }
}

/// CLI presentation data kept inside the normal machine-envelope/v2 transport. Pending is a
/// successful query/acceptance state, not a fabricated completed result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerCommandDataV1 {
    pub schema: String,
    pub operation: String,
    pub submission_state: WorkerSubmissionStateV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_state: Option<RunStateV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub replayed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default)]
    pub timed_out: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerReadRequestV1 {
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u32>,
}

impl WorkerReadRequestV1 {
    pub fn valid(&self) -> bool {
        reference(&self.run_id)
            && self
                .cursor
                .as_ref()
                .is_none_or(|cursor| valid_opaque_cursor(cursor))
            && self
                .max_bytes
                .is_none_or(|size| (1..=MAX_WORKER_READ_BYTES).contains(&size))
    }

    pub fn effective_max_bytes(&self) -> u32 {
        self.max_bytes.unwrap_or(DEFAULT_WORKER_READ_BYTES)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerReadContentStateV1 {
    Pending,
    Available,
    Deleted,
    Expired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerReadDataV1 {
    pub task_id: String,
    pub run_id: String,
    pub run_state: RunStateV1,
    pub state_revision: u64,
    pub content_state: WorkerReadContentStateV1,
    pub segment: Option<String>,
    pub window_start: Option<String>,
    pub window_end: Option<String>,
    pub text: Option<String>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub truncated: Option<bool>,
}

impl WorkerReadDataV1 {
    pub fn valid(&self) -> bool {
        if !reference(&self.task_id) || !reference(&self.run_id) || self.state_revision == 0 {
            return false;
        }
        match self.content_state {
            WorkerReadContentStateV1::Pending => {
                self.segment.as_deref() == Some("0")
                    && self.window_start.as_deref() == Some("0")
                    && self.window_end.as_deref() == Some("0")
                    && self.text.as_deref() == Some("")
                    && self
                        .next_cursor
                        .as_ref()
                        .is_some_and(|cursor| valid_opaque_cursor(cursor))
                    && !self.has_more
                    && self.truncated == Some(false)
            }
            WorkerReadContentStateV1::Available => {
                let segment = self.segment.as_deref().and_then(canonical_position);
                let start = self.window_start.as_deref().and_then(canonical_position);
                let end = self.window_end.as_deref().and_then(canonical_position);
                segment.is_some()
                    && start.zip(end).is_some_and(|(start, end)| start <= end)
                    && self
                        .text
                        .as_ref()
                        .is_some_and(|text| text.len() <= MAX_WORKER_READ_BYTES as usize)
                    && self
                        .next_cursor
                        .as_ref()
                        .is_some_and(|cursor| valid_opaque_cursor(cursor))
                    && self.truncated.is_some()
            }
            WorkerReadContentStateV1::Deleted | WorkerReadContentStateV1::Expired => {
                self.segment.is_none()
                    && self.window_start.is_none()
                    && self.window_end.is_none()
                    && self.text.is_none()
                    && self.next_cursor.is_none()
                    && !self.has_more
                    && self.truncated.is_none()
            }
        }
    }
}

fn valid_run_timeout(run_timeout_secs: u32) -> bool {
    (1..=MAX_WORKER_RUN_SECS).contains(&run_timeout_secs)
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

fn valid_opaque_cursor(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_WORKER_CURSOR_BYTES && !value.contains(char::is_control)
}

fn canonical_position(value: &str) -> Option<u64> {
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return None;
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|position| *position <= i64::MAX as u64)
}

/// Normalize the one public task-title contract without locale-dependent trimming.
pub fn normalize_worker_title(value: &str) -> Result<String, WorkerTitleError> {
    if value.len() > MAX_WORKER_TITLE_INPUT_BYTES {
        return Err(WorkerTitleError::InputTooLarge);
    }
    normalize_title_chars(value.chars(), false).and_then(|normalized| {
        if normalized.is_empty() {
            Err(WorkerTitleError::Empty)
        } else if normalized.len() > MAX_WORKER_TITLE_BYTES {
            Err(WorkerTitleError::NormalizedTooLarge)
        } else {
            Ok(normalized)
        }
    })
}

/// Derive a deterministic title from the first usable Goal line. Forbidden formatting controls
/// are omitted from automatic presentation rather than making otherwise valid task input fail.
pub fn derive_worker_title(goal: &str) -> String {
    for line in goal.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Ok(normalized) = normalize_title_chars(line.chars(), true)
            && !normalized.is_empty()
        {
            return truncate_utf8(&normalized, MAX_WORKER_TITLE_BYTES);
        }
    }
    UNNAMED_WORKER_TASK_TITLE.to_owned()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum WorkerTitleError {
    #[error("task title input exceeds its bound")]
    InputTooLarge,
    #[error("task title contains a forbidden control character")]
    ForbiddenCharacter,
    #[error("task title is empty after normalization")]
    Empty,
    #[error("normalized task title exceeds its bound")]
    NormalizedTooLarge,
}

fn normalize_title_chars(
    characters: impl IntoIterator<Item = char>,
    omit_forbidden: bool,
) -> Result<String, WorkerTitleError> {
    let mut normalized = String::new();
    let mut pending_space = false;
    for character in characters {
        if forbidden_title_character(character) {
            if omit_forbidden {
                continue;
            }
            return Err(WorkerTitleError::ForbiddenCharacter);
        }
        if unicode_white_space(character) {
            pending_space = !normalized.is_empty();
            continue;
        }
        if pending_space {
            normalized.push(' ');
            pending_space = false;
        }
        normalized.push(character);
    }
    Ok(normalized)
}

fn forbidden_title_character(character: char) -> bool {
    (matches!(character as u32, 0x00..=0x1f) && !matches!(character, '\t' | '\n' | '\r'))
        || matches!(character as u32, 0x7f..=0x9f)
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

fn unicode_white_space(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000d}'
            | '\u{0020}'
            | '\u{0085}'
            | '\u{00a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].trim_end().to_owned()
}

fn absolute_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.contains(char::is_control)
        && std::path::Path::new(value).is_absolute()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exec() -> WorkerExecRequestV1 {
        WorkerExecRequestV1 {
            schema: WORKER_EXEC_SCHEMA_V1.into(),
            plan_id: AgentPlanId::parse("worker").unwrap(),
            cwd: std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            permission_policy: WorkerPermissionPolicyV1::ApproveAll,
            run_timeout_secs: DEFAULT_WORKER_RUN_SECS,
            input: DelegationTaskInputV1 {
                goal: "implement the change".into(),
                context: String::new(),
                constraints: String::new(),
                acceptance_criteria: String::new(),
            },
            submission_key: "worker/start-one".into(),
            title: None,
            parent_task_ref: None,
        }
    }

    #[test]
    fn worker_settings_are_strict_and_bounded() {
        assert!(WorkerSettingsV1::new(10).valid());
        assert!(WorkerSettingsV1::new(1_000).valid());
        assert!(!WorkerSettingsV1::new(0).valid());
        let mut value = serde_json::to_value(WorkerSettingsV1::new(10)).unwrap();
        value["revision"] = serde_json::json!(1);
        assert!(serde_json::from_value::<WorkerSettingsV1>(value).is_err());
    }

    #[test]
    fn dependency_selection_plan_binds_only_the_normalized_tuple_and_target_revision() {
        let request = WorkerDependenciesSelectRequestV1 {
            harness: WorkerHarnessV1::CodexCli,
            adapter_path: Some("/opt/acp/codex-acp.js".into()),
            cli_path: "/opt/bin/codex".into(),
            node_path: Some("/opt/bin/node".into()),
            expected_selection_revision: 7,
        };
        let first = plan_worker_dependency_selection(&request).unwrap();
        let second = plan_worker_dependency_selection(&request).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.expected_revisions.target, 7);
        assert!(first.expected_revisions.dependencies.is_empty());
        assert_eq!(
            first.spec.resource_id.as_deref(),
            Some("worker-dependency-selection/codex_cli")
        );
        assert_eq!(first.accept_digest, second.accept_digest);
        assert_eq!(first.idempotency_key, second.idempotency_key);

        let mut another_harness = request.clone();
        another_harness.harness = WorkerHarnessV1::ClaudeCode;
        assert_ne!(
            plan_worker_dependency_selection(&another_harness)
                .unwrap()
                .accept_digest,
            first.accept_digest
        );
        let mut another_revision = request;
        another_revision.expected_selection_revision = 8;
        let changed = plan_worker_dependency_selection(&another_revision).unwrap();
        assert_ne!(changed.accept_digest, first.accept_digest);
        assert_ne!(changed.idempotency_key, first.idempotency_key);
    }

    #[test]
    fn dependency_selection_plan_rejects_relative_or_control_character_paths() {
        let request = WorkerDependenciesSelectRequestV1 {
            harness: WorkerHarnessV1::CodexCli,
            adapter_path: Some("relative/adapter".into()),
            cli_path: "/opt/bin/codex".into(),
            node_path: Some("/opt/bin/node".into()),
            expected_selection_revision: 0,
        };
        assert!(plan_worker_dependency_selection(&request).is_none());
        let invalid = WorkerDependenciesSelectRequestV1 {
            adapter_path: Some("/opt/acp/codex\nacp".into()),
            ..request
        };
        assert!(plan_worker_dependency_selection(&invalid).is_none());
    }

    #[test]
    fn worker_exec_is_instance_scoped_and_rejects_authority_injection_fields() {
        let request = exec();
        assert!(request.valid());
        let value = serde_json::to_value(request).unwrap();
        for forbidden in [
            "caller",
            "agent_id",
            "permit_id",
            "permit_generation",
            "root_identity",
            "grant_id",
            "grant_generation",
            "access",
            "tools",
            "network",
        ] {
            assert!(value.get(forbidden).is_none(), "{forbidden}");
        }
        let mut injected = value;
        injected["caller"] = serde_json::json!({"context_id":"forged"});
        assert!(serde_json::from_value::<WorkerExecRequestV1>(injected).is_err());
    }

    #[test]
    fn permission_policy_defaults_to_approve_all_and_old_fields_are_rejected() {
        let mut value = serde_json::to_value(exec()).unwrap();
        value.as_object_mut().unwrap().remove("permission_policy");
        let decoded: WorkerExecRequestV1 = serde_json::from_value(value).unwrap();
        assert_eq!(
            decoded.permission_policy,
            WorkerPermissionPolicyV1::ApproveAll
        );
        let mut old = serde_json::to_value(exec()).unwrap();
        old["agent_id"] = serde_json::json!("codex");
        assert!(serde_json::from_value::<WorkerExecRequestV1>(old).is_err());
    }

    #[test]
    fn worker_status_accepts_each_exact_selector_form() {
        assert!(
            WorkerStatusRequestV1 {
                task_id: None,
                run_id: Some("run/one".into()),
                submission_key: None,
                submission_operation: None,
            }
            .valid()
        );
        assert!(
            WorkerStatusRequestV1 {
                task_id: Some("task/one".into()),
                run_id: Some("run/one".into()),
                submission_key: None,
                submission_operation: None,
            }
            .valid()
        );
        assert!(
            WorkerStatusRequestV1 {
                task_id: None,
                run_id: None,
                submission_key: Some("worker/start-one".into()),
                submission_operation: Some(DelegationSubmissionOperationV1::Start),
            }
            .valid()
        );
    }

    #[test]
    fn worker_list_is_a_strict_instance_page_request() {
        let request = WorkerListRequestV1 {
            title: Some("  同一\u{00a0}标题  ".into()),
            cursor: Some("sequence/42".into()),
            limit: Some(25),
        };
        assert!(request.valid());
        assert_eq!(request.effective_limit(), 25);

        let mut invalid = request.clone();
        invalid.cursor = Some(String::new());
        assert!(!invalid.valid());
        invalid.cursor = None;
        invalid.limit = Some(0);
        assert!(!invalid.valid());

        let mut injected = serde_json::to_value(request).unwrap();
        injected["caller"] = serde_json::json!({"context_id":"forged"});
        assert!(serde_json::from_value::<WorkerListRequestV1>(injected).is_err());
    }

    #[test]
    fn task_title_normalization_is_bounded_deterministic_and_not_locale_dependent() {
        assert_eq!(
            normalize_worker_title("  设计\u{00a0}\u{2009}任务  ").unwrap(),
            "设计 任务"
        );
        assert_eq!(
            normalize_worker_title("  设计\t\r\n任务  ").unwrap(),
            "设计 任务"
        );
        assert_eq!(
            derive_worker_title("\n\r\n  第一\u{3000}行  \n第二行"),
            "第一 行"
        );
        assert_eq!(derive_worker_title("\n\r\n"), UNNAMED_WORKER_TASK_TITLE);
        assert_eq!(
            normalize_worker_title("unsafe\u{202e}title"),
            Err(WorkerTitleError::ForbiddenCharacter)
        );
        assert_eq!(
            normalize_worker_title("unsafe\u{0085}title"),
            Err(WorkerTitleError::ForbiddenCharacter)
        );
        let long = "界".repeat(MAX_WORKER_TITLE_BYTES);
        let derived = derive_worker_title(&long);
        assert!(derived.len() <= MAX_WORKER_TITLE_BYTES);
        assert!(derived.is_char_boundary(derived.len()));
    }

    #[test]
    fn worker_read_request_and_terminal_visibility_shapes_are_strict() {
        let request = WorkerReadRequestV1 {
            run_id: "run/one".into(),
            cursor: None,
            max_bytes: None,
        };
        assert!(request.valid());
        assert_eq!(request.effective_max_bytes(), DEFAULT_WORKER_READ_BYTES);
        let pending = WorkerReadDataV1 {
            task_id: "task/one".into(),
            run_id: "run/one".into(),
            run_state: RunStateV1::Accepted,
            state_revision: 1,
            content_state: WorkerReadContentStateV1::Pending,
            segment: Some("0".into()),
            window_start: Some("0".into()),
            window_end: Some("0".into()),
            text: Some(String::new()),
            next_cursor: Some("signed/pending".into()),
            has_more: false,
            truncated: Some(false),
        };
        assert!(pending.valid());
        let mut malformed_pending = pending;
        malformed_pending.has_more = true;
        assert!(!malformed_pending.valid());

        let available = WorkerReadDataV1 {
            task_id: "task/one".into(),
            run_id: "run/one".into(),
            run_state: RunStateV1::Running,
            state_revision: 2,
            content_state: WorkerReadContentStateV1::Available,
            segment: Some("1".into()),
            window_start: Some("0".into()),
            window_end: Some("2".into()),
            text: Some("ok".into()),
            next_cursor: Some("signed/available".into()),
            has_more: false,
            truncated: Some(false),
        };
        assert!(available.valid());
        let mut leading_zero = available.clone();
        leading_zero.segment = Some("01".into());
        assert!(!leading_zero.valid());
        let mut inverted = available.clone();
        inverted.window_start = Some("3".into());
        assert!(!inverted.valid());
        let mut outside_storage_range = available;
        outside_storage_range.window_end = Some((i64::MAX as u64 + 1).to_string());
        assert!(!outside_storage_range.valid());

        let deleted = WorkerReadDataV1 {
            task_id: "task/one".into(),
            run_id: "run/one".into(),
            run_state: RunStateV1::Succeeded,
            state_revision: 2,
            content_state: WorkerReadContentStateV1::Deleted,
            segment: None,
            window_start: None,
            window_end: None,
            text: None,
            next_cursor: None,
            has_more: false,
            truncated: None,
        };
        assert!(deleted.valid());
        let mut invalid = deleted;
        invalid.text = Some(String::new());
        assert!(!invalid.valid());
    }

    #[test]
    fn executor_availability_lists_all_supported_capability_scoped_harnesses() {
        let ready = WorkerExecutorCapabilityAvailabilityV1 {
            state: WorkerExecutorAvailabilityStateV1::Ready,
            reason: None,
        };
        let unknown_restricted = WorkerExecutorCapabilityAvailabilityV1 {
            state: WorkerExecutorAvailabilityStateV1::Unknown,
            reason: Some(WorkerExecutorAvailabilityReasonV1::RestrictedPolicyUnverified),
        };
        let unavailable_resume = WorkerExecutorCapabilityAvailabilityV1 {
            state: WorkerExecutorAvailabilityStateV1::Unavailable,
            reason: Some(WorkerExecutorAvailabilityReasonV1::ResumeUnavailable),
        };
        let executor = |harness| WorkerExecutorAvailabilityV1 {
            harness,
            state: WorkerExecutorAvailabilityStateV1::Ready,
            reason: None,
            start_approve_all: ready.clone(),
            cancel: ready.clone(),
            continue_session: unavailable_resume.clone(),
            restricted_policy: unknown_restricted.clone(),
        };
        let response = WorkerExecutorAvailabilityListV1 {
            schema: WORKER_EXECUTOR_AVAILABILITY_SCHEMA_V1.into(),
            executors: vec![
                executor(WorkerHarnessV1::CodexCli),
                executor(WorkerHarnessV1::ClaudeCode),
                executor(WorkerHarnessV1::QoderCli),
                executor(WorkerHarnessV1::Pi),
            ],
        };
        assert!(response.valid());

        let mut incomplete = response.clone();
        incomplete.executors.pop();
        assert!(!incomplete.valid());
        let mut duplicate = response.clone();
        duplicate.executors[2].harness = WorkerHarnessV1::CodexCli;
        assert!(!duplicate.valid());
        let mut reordered = response.clone();
        reordered.executors.swap(0, 1);
        assert!(!reordered.valid());

        let mut overstated = response;
        overstated.executors[0].start_approve_all = unavailable_resume;
        assert!(!overstated.valid());
    }
}

#[cfg(test)]
#[path = "worker_selection_tests.rs"]
mod selection_tests;
