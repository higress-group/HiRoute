//! Persisted delegation metadata and its runtime-store port. These records convey no authority.
//! Plan contents remain owned by the exact-version service; body bytes remain in observation.
use super::*;
use crate::{AgentPlanId, CanonicalDigest, OperationId, WorkspaceId};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationWorkspaceV1 {
    pub root_identity: String,
    pub volume_identity: String,
    /// Opaque canonical ancestor identities from the platform resolver, not display paths.
    /// Windows case/reparse handling belongs to that resolver, never string guessing here.
    pub ancestry: Vec<String>,
}
impl DelegationWorkspaceV1 {
    pub fn validate(&self) -> Result<(), DelegationErrorV1> {
        if !valid_delegation_id(&self.root_identity)
            || !valid_delegation_id(&self.volume_identity)
            || self.ancestry.is_empty()
            || self.ancestry.last() != Some(&self.root_identity)
            || self.ancestry.len() > 256
            || self
                .ancestry
                .iter()
                .any(|value| !valid_delegation_id(value))
        {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        Ok(())
    }
}

/// Consumer metadata for 13's full exact reference. A copy of this struct does NOT pin or
/// prove availability; admission must call the injected exact-version port under the gate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationPlanBindingV1 {
    pub authority_id: String,
    pub plan_id: AgentPlanId,
    pub plan_revision: u64,
    pub plan_digest: CanonicalDigest,
    pub publication_revision: u64,
    pub publication_digest: CanonicalDigest,
    pub exact_reference: String,
    pub model_alias: String,
    pub harness: WorkerHarnessV1,
    pub harness_configuration_digest: CanonicalDigest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationSessionBindingV1 {
    pub acp_session_id: String,
    pub native_session_id: Option<String>,
}

/// A durable, opaque pointer to one managed-text object.  The complete scope is derived from
/// the committed task/run record; this metadata is enough to recheck visibility without placing
/// task prose or a filesystem location in runtime.db.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationBodyRefV1 {
    pub opaque_id: String,
    pub scope_run_id: String,
    /// Observation's visibility barrier starts at zero; this is not a grant generation.
    pub visibility_generation: u64,
    pub original_retention_deadline_ms: i64,
}

impl DelegationBodyRefV1 {
    pub fn validate(&self) -> Result<(), DelegationErrorV1> {
        if !valid_delegation_id(&self.opaque_id)
            || !valid_delegation_id(&self.scope_run_id)
            || self.original_retention_deadline_ms <= 0
        {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationProcessBindingV1 {
    pub launch_nonce: String,
    pub handle_id: String,
    pub creation_identity: String,
}

pub const DELEGATION_RUN_CONFIGURATION_VERSION_V1: u16 = 1;

/// Per-run Worker execution configuration accepted in the same runtime transaction as the run.
///
/// The canonical path supports launch, continuation identity and audit. The permission policy is
/// caller-selected Harness behavior. Neither field authenticates the caller or creates a
/// filesystem/tool authorization, and cwd is not promoted into a directory allowlist or lock.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationRunConfigurationV1 {
    pub format_version: u16,
    pub scope_id: String,
    pub generation: u64,
    pub canonical_workspace_path: String,
    #[serde(default)]
    pub permission_policy: WorkerPermissionPolicyV1,
}

impl DelegationRunConfigurationV1 {
    pub fn validate_for(&self, run: &DelegationRunV1) -> Result<(), DelegationErrorV1> {
        if self.format_version != DELEGATION_RUN_CONFIGURATION_VERSION_V1
            || self.scope_id != format!("run-config/{}", run.run_id)
            || self.scope_id != run.permit_id
            || self.generation != 1
            || self.generation != run.permit_generation
            || self.canonical_workspace_path.is_empty()
            || self.canonical_workspace_path.len() > 4096
            || self.canonical_workspace_path.contains('\0')
            || !std::path::Path::new(&self.canonical_workspace_path).is_absolute()
        {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        Ok(())
    }

    /// Full native compatibility view for the existing process-profile boundary. Public Worker
    /// execution never queries or writes the Control permit table: every field comes from the
    /// accepted run, and caller restrictions are represented only by `permission_policy`.
    pub fn profile_permit(
        &self,
        run: &DelegationRunV1,
    ) -> Result<WorkspaceExecutionPermitV1, DelegationErrorV1> {
        self.validate_for(run)?;
        let permit = WorkspaceExecutionPermitV1 {
            permit_id: self.scope_id.clone(),
            generation: self.generation,
            root_identity: run.execution.root_identity.clone(),
            access: run.execution.access,
            tools: run.execution.tools.clone(),
            network: run.execution.network,
            expires_at_ms: run.deadline_ms,
            max_run_ms: run.execution.duration_ms,
            max_concurrent: DEFAULT_WORKER_CONCURRENCY,
            revoked: run.lease_revoked,
        };
        permit.validate()?;
        Ok(permit)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationTaskV1 {
    pub workspace_id: WorkspaceId,
    pub task_id: String,
    /// Caller-provided provenance is preserved as unverified provenance only.  It never
    /// authorizes a parent session or another task's result.
    #[serde(default)]
    pub parent_task_ref: Option<String>,
    pub plan: DelegationPlanBindingV1,
    pub workspace: DelegationWorkspaceV1,
    pub created_at_ms: u64,
    pub latest_run_id: String,
    pub latest_admission_sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<DelegationTaskTitleV1>,
    pub session: Option<DelegationSessionBindingV1>,
    pub resume_until_ms: u64,
    /// Opaque references only. Continue must resolve/read them through the current body port.
    pub required_body_ids: Vec<String>,
    /// Visibility metadata for `required_body_ids`, maintained only by the daemon's managed
    /// text adapter.  Older task records decode as an empty vector and cannot silently gain a
    /// readable body.
    #[serde(default)]
    pub body_refs: Vec<DelegationBodyRefV1>,
    /// Exact task-root-relative native files observed after the last clean run. Continue checks
    /// every entry before asking ACP to load the persisted native session; it never searches for
    /// a newest session or creates replacement history.
    #[serde(default)]
    pub native_history_paths: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationTaskTitleSourceV1 {
    Explicit,
    Goal,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationTaskTitleV1 {
    pub value: String,
    pub source: DelegationTaskTitleSourceV1,
    pub initial_body_ref: DelegationBodyRefV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationRunV1 {
    pub workspace_id: WorkspaceId,
    pub task_id: String,
    pub run_id: String,
    pub ordinal: u64,
    pub continued_from: Option<String>,
    pub idempotency_key: String,
    pub request_digest: CanonicalDigest,
    pub admission_sequence: u64,
    /// Persisted by the runtime acceptance transaction. Older records decode without it and
    /// are projected conservatively by the query adapter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_at_ms: Option<u64>,
    pub execution_owner_ref: String,
    pub lease_id: String,
    pub daemon_epoch: String,
    pub permit_id: String,
    pub permit_generation: u64,
    pub configuration: DelegationRunConfigurationV1,
    pub execution: WorkerExecutionIntentV1,
    pub deadline_ms: u64,
    pub lease_revoked: bool,
    pub launch_nonce: String,
    pub process: Option<DelegationProcessBindingV1>,
    pub session: Option<DelegationSessionBindingV1>,
    pub progress: RunProgressV1,
    #[serde(default)]
    pub stop_evidence: Option<RunStopEvidenceV1>,
    /// Completion text is independent from transport/process completion.  Absence or an
    /// incomplete stream is surfaced honestly instead of becoming an empty successful result.
    #[serde(default)]
    pub result_body: Option<DelegationBodyRefV1>,
    #[serde(default)]
    pub result_incomplete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_failure: Option<super::WorkerExecutionFailureV1>,
}

impl DelegationRunV1 {
    pub fn profile_permit(&self) -> Result<WorkspaceExecutionPermitV1, DelegationErrorV1> {
        self.configuration.profile_permit(self)
    }
}

#[derive(Clone, Debug)]
pub struct DelegationAcceptanceV1 {
    pub task: DelegationTaskV1,
    pub run: DelegationRunV1,
    pub title_lookup_key: Option<String>,
    pub expected_latest_run_id: Option<String>,
    pub admitted_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DelegationCheckpointV1 {
    ExecutionFailed {
        event: RunEventV1,
        failure: super::WorkerExecutionFailureV1,
    },
    Progress {
        event: RunEventV1,
    },
    ProcessSpawned {
        binding: DelegationProcessBindingV1,
    },
    SessionBound {
        binding: DelegationSessionBindingV1,
    },
    /// Records the managed-text result relation independently from ACP completion and process
    /// cleanup.  A missing body is valid only when the stream has an explicit gap; callers must
    /// never manufacture an empty successful result after a content write failure.
    ResultRecorded {
        body: Option<DelegationBodyRefV1>,
        incomplete: bool,
    },
    ProcessObserved {
        observation: RunProcessObservationV1,
    },
    ProcessStopped {
        evidence: RunStopEvidenceV1,
    },
    ResidualConfirmed {
        operation_id: OperationId,
        actor_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationCancelReceiptV1 {
    pub operation_id: OperationId,
    pub run_id: String,
    pub reason: String,
    pub state_revision: u64,
}

pub trait DelegationRuntimePort {
    fn worker_concurrency_settings(
        &self,
    ) -> Result<WorkerConcurrencySettingsV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn set_worker_concurrency_settings(
        &self,
        _settings: WorkerConcurrencySettingsV1,
    ) -> Result<WorkerConcurrencySettingsV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn task(
        &self,
        workspace: &WorkspaceId,
        task_id: &str,
    ) -> Result<Option<DelegationTaskV1>, DelegationErrorV1>;
    fn run(
        &self,
        workspace: &WorkspaceId,
        run_id: &str,
    ) -> Result<Option<DelegationRunV1>, DelegationErrorV1>;
    fn find_submission(
        &self,
        workspace: &WorkspaceId,
        continuation: bool,
        key: &str,
    ) -> Result<Option<DelegationRunV1>, DelegationErrorV1>;
    /// Latest run for each instance task, optionally restricted by a keyed exact title lookup,
    /// ordered newest first by the durable admission sequence. `before_sequence` is exclusive.
    fn list_latest_runs(
        &self,
        _workspace: &WorkspaceId,
        _title_lookup_key: Option<&str>,
        _before_sequence: Option<u64>,
        _limit: u16,
    ) -> Result<Vec<DelegationRunV1>, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    /// Complete bounded owner set used only during startup version-hold reconciliation.
    fn resumable_tasks(
        &self,
        _workspace: &WorkspaceId,
        _now_ms: u64,
    ) -> Result<Vec<DelegationTaskV1>, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    /// Atomic accepted record + task slot + workspace occupancy + start intent. This commit
    /// is the admission point; the caller still holds the single shared admission gate.
    fn accept(
        &self,
        acceptance: &DelegationAcceptanceV1,
    ) -> Result<DelegationRunV1, DelegationErrorV1>;
    fn checkpoint(
        &self,
        workspace: &WorkspaceId,
        run: &str,
        expected_revision: u64,
        event_id: &str,
        event: &DelegationCheckpointV1,
    ) -> Result<DelegationRunV1, DelegationErrorV1>;
    fn request_cancel(
        &self,
        workspace: &WorkspaceId,
        run: &str,
        operation: &OperationId,
        reason: &str,
    ) -> Result<DelegationCancelReceiptV1, DelegationErrorV1>;
    fn unreconciled(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<Vec<DelegationRunV1>, DelegationErrorV1>;
    fn set_resume_materials(
        &self,
        workspace: &WorkspaceId,
        task: &str,
        latest_run: &str,
        until_ms: u64,
        body_refs: &[DelegationBodyRefV1],
        native_history_paths: &[String],
    ) -> Result<(), DelegationErrorV1>;

    /// Exact durable ownership for one task-scoped native session root.  Missing means an older
    /// unsupported task, never permission to derive a filesystem target from its ids.
    fn native_root(
        &self,
        _workspace: &WorkspaceId,
        _task_id: &str,
    ) -> Result<Option<DelegationNativeRootSnapshotV1>, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    /// Publish the identity created from a prior `Creating` intent before any spawn can occur.
    fn commit_native_root_ready(
        &self,
        _ready: &DelegationNativeRootReadyV1,
    ) -> Result<DelegationNativeRootSnapshotV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    /// A failed create whose filesystem result cannot be proved is quarantined permanently.
    fn mark_native_root_unknown(
        &self,
        _workspace: &WorkspaceId,
        _task_id: &str,
        _root_generation: u64,
        _creation_nonce: &str,
    ) -> Result<(), DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    /// CAS the ready/no-native generation into `Deleting`.  Continue insertion and resume
    /// retention reject a claimed generation in their own runtime transactions.
    fn claim_native_cleanup(
        &self,
        _workspace: &WorkspaceId,
        _task_id: &str,
        _claim: &DelegationNativeCleanupClaimV1,
    ) -> Result<DelegationNativeRootSnapshotV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    /// Record bounded deletion progress without treating a partial batch as completion.
    fn record_native_cleanup_batch(
        &self,
        _workspace: &WorkspaceId,
        _task_id: &str,
        _root_generation: u64,
        _claim_id: &str,
    ) -> Result<(), DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    fn record_native_cleanup_failure(
        &self,
        _workspace: &WorkspaceId,
        _task_id: &str,
        _root_generation: u64,
        _claim_id: &str,
        _kind: DelegationNativeCleanupFailureKindV1,
        _attempted_at_ms: i64,
    ) -> Result<(), DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    /// Persist `Removed` (or the claimed durable NoNative equivalent) before observation jobs
    /// may be acknowledged.
    fn complete_native_cleanup(
        &self,
        _workspace: &WorkspaceId,
        _task_id: &str,
        _root_generation: u64,
        _claim_id: &str,
    ) -> Result<DelegationNativeRootSnapshotV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    /// Bounded keyset page for retention-driven task metadata work.  The page is not an owner
    /// set and must never be passed to Plan-version reconciliation.
    fn maintenance_tasks(
        &self,
        _after: Option<&DelegationTaskMaintenanceCursorV1>,
        _limit: u16,
    ) -> Result<Vec<DelegationTaskV1>, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    /// Clear only the title and its lookup index if the exact task snapshot is still current.
    fn clear_task_title(&self, _expected: &DelegationTaskV1) -> Result<bool, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    /// Atomically disables one exact continuation and records the independent Plan-hold release
    /// handoff.  The observation decision is made by the trusted daemon before this CAS.
    fn claim_continuation_release(
        &self,
        _expected: &DelegationTaskV1,
    ) -> Result<DelegationContinuationReleaseV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    fn pending_continuation_releases(
        &self,
        _limit: u16,
    ) -> Result<Vec<DelegationContinuationReleaseV1>, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    fn complete_continuation_release(
        &self,
        _release: &DelegationContinuationReleaseV1,
    ) -> Result<(), DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
}

pub fn valid_native_history_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.starts_with('/')
        && !value
            .chars()
            .any(|character| matches!(character, '\\' | '\0' | '\r' | '\n'))
        && value
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

pub fn valid_delegation_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
