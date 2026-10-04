use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hiroute_application_api::{
    DelegationTaskInputV1, WORKER_CONTINUE_SCHEMA_V1, WorkerContinueRequestV1,
};
use hiroute_domain::WorkspaceId;
use hiroute_domain::delegation::{
    DELEGATION_NATIVE_ROOT_MARKER_FILE_V1, DelegationBodyRefV1, DelegationCheckpointV1,
    DelegationNativeCleanupFailureKindV1, DelegationNativeRootReadyV1, DelegationNativeRootStateV1,
    DelegationProcessBindingV1, DelegationRuntimePort, DelegationSessionBindingV1, RunEventV1,
    RunProcessObservationV1, RunStateV1, RunStopEvidenceV1, RunStopScopeV1,
};
use hiroute_observation::managed_text::{
    ManagedTextInput, ManagedTextNativeCleanup, ManagedTextPurpose, ManagedTextScope, RETENTION_MS,
};

use super::*;
use crate::control::runtime::ProductionControlRuntime;
use crate::delegation::content::read_required_body;
use crate::delegation::profile::{SessionRootUse, TaskSessionRoot, native_history};

const BLOCKED_TEST: &str = "control::runtime::delegation_maintenance::tests::blocked_native_deletion_remains_claimed_and_unacknowledged";
const SUCCESS_TEST: &str = "control::runtime::delegation_maintenance::tests::owned_native_deletion_commits_removed_before_exact_ack";

#[derive(Clone, Copy)]
enum BlockedFixture {
    ReplacedIdentity,
    NonPrivatePermissions,
    MarkerFinalizationInterrupted,
}

struct CleanupFixture {
    runtime: ProductionControlRuntime,
    _directory: tempfile::TempDir,
    workspace_id: WorkspaceId,
    task_id: String,
    native_root: PathBuf,
    job: ManagedTextNativeCleanup,
}

#[test]
fn blocked_native_deletion_remains_claimed_and_unacknowledged() {
    if crate::test_support::isolated_agent_home(BLOCKED_TEST) {
        return;
    }
    assert_blocked_cleanup(BlockedFixture::ReplacedIdentity, "identity");
    assert_blocked_cleanup(BlockedFixture::NonPrivatePermissions, "permissions");
    assert_blocked_cleanup(
        BlockedFixture::MarkerFinalizationInterrupted,
        "marker-interrupted",
    );
}

fn assert_blocked_cleanup(fixture: BlockedFixture, suffix: &str) {
    let cleanup = cleanup_fixture(suffix);
    let preserved_entry = match fixture {
        BlockedFixture::ReplacedIdentity => {
            std::fs::remove_file(
                cleanup
                    .native_root
                    .join(DELEGATION_NATIVE_ROOT_MARKER_FILE_V1),
            )
            .unwrap();
            std::fs::remove_dir(&cleanup.native_root).unwrap();
            private_directory(&cleanup.native_root);
            true
        }
        BlockedFixture::NonPrivatePermissions => {
            std::fs::set_permissions(&cleanup.native_root, std::fs::Permissions::from_mode(0o755))
                .unwrap();
            true
        }
        BlockedFixture::MarkerFinalizationInterrupted => {
            std::fs::remove_file(
                cleanup
                    .native_root
                    .join(DELEGATION_NATIVE_ROOT_MARKER_FILE_V1),
            )
            .unwrap();
            false
        }
    };
    if preserved_entry {
        std::fs::write(cleanup.native_root.join("must-remain"), b"preserved").unwrap();
    }

    let mut remaining = DELETE_BUDGET;
    cleanup
        .runtime
        .adapter
        .process_native_root_for_job(&cleanup.job, 101, &mut remaining)
        .unwrap();
    let blocked = DelegationRuntimePort::native_root(
        cleanup.runtime.adapter.as_ref(),
        &cleanup.workspace_id,
        &cleanup.task_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(blocked.root.state, DelegationNativeRootStateV1::Deleting);
    let failure = blocked.root.last_cleanup_failure.unwrap();
    assert_eq!(
        failure.kind,
        DelegationNativeCleanupFailureKindV1::IdentityMismatch
    );
    assert_eq!(failure.attempted_at_ms, 101);
    assert_eq!(failure.attempts, 1);
    assert_eq!(remaining, DELETE_BUDGET);
    assert_eq!(
        cleanup
            .runtime
            .observation
            .managed_text_pending_native_cleanup_page(None, 10)
            .unwrap(),
        vec![cleanup.job]
    );
    assert!(cleanup.native_root.exists());
    if preserved_entry {
        assert_eq!(
            std::fs::read(cleanup.native_root.join("must-remain")).unwrap(),
            b"preserved"
        );
    }
}

#[test]
fn owned_native_deletion_commits_removed_before_exact_ack() {
    if crate::test_support::isolated_agent_home(SUCCESS_TEST) {
        return;
    }
    let cleanup = cleanup_fixture("success");
    std::fs::write(cleanup.native_root.join("history.jsonl"), b"native history").unwrap();
    let mut remaining = DELETE_BUDGET;
    cleanup
        .runtime
        .adapter
        .process_native_root_for_job(&cleanup.job, 101, &mut remaining)
        .unwrap();
    let removed = DelegationRuntimePort::native_root(
        cleanup.runtime.adapter.as_ref(),
        &cleanup.workspace_id,
        &cleanup.task_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(removed.root.state, DelegationNativeRootStateV1::Removed);
    assert!(removed.root.last_cleanup_failure.is_none());
    assert!(remaining < DELETE_BUDGET);
    assert!(!cleanup.native_root.exists());
    assert!(
        cleanup
            .runtime
            .observation
            .managed_text_pending_native_cleanup_page(None, 10)
            .unwrap()
            .is_empty()
    );
}

fn cleanup_fixture(suffix: &str) -> CleanupFixture {
    let directory = tempfile::tempdir().unwrap();
    let runtime = maintenance_runtime(directory.path());

    let task_id = format!("cleanup-{suffix}-task");
    let run_id = format!("cleanup-{suffix}-run");
    let scope = ManagedTextScope {
        workspace_id: WorkspaceId::default(),
        task_id: task_id.clone(),
        run_id: run_id.clone(),
    };
    let input = ManagedTextInput {
        scope: scope.clone(),
        purpose: ManagedTextPurpose::Result,
        source_event_id: format!("cleanup-{suffix}-body"),
        source_revision: 1,
        original_created_at_ms: 10,
        import_origin: None,
    };
    let reference = runtime.observation.managed_text_put(&input, 100).unwrap();
    runtime
        .observation
        .managed_text_append(&scope, &reference, 0, b"result", 100)
        .unwrap();
    let reference = runtime
        .observation
        .managed_text_finish(&scope, &reference, 1, 100)
        .unwrap();
    let body = DelegationBodyRefV1 {
        opaque_id: reference.opaque_id,
        scope_run_id: run_id.clone(),
        visibility_generation: reference.visibility_generation,
        original_retention_deadline_ms: reference.original_retention_deadline_ms,
    };
    let acceptance = super::super::tests::worker_list_acceptance(&task_id, &run_id, body, 10);
    let accepted = DelegationRuntimePort::accept(runtime.adapter.as_ref(), &acceptance).unwrap();

    let native_base = std::fs::canonicalize(directory.path())
        .unwrap()
        .join("native");
    private_directory(&native_base);
    let creating = DelegationRuntimePort::native_root(
        runtime.adapter.as_ref(),
        &accepted.workspace_id,
        &accepted.task_id,
    )
    .unwrap()
    .unwrap();
    let session = crate::delegation::profile::TaskSessionRoot::prepare(
        &native_base,
        &accepted.workspace_id,
        &creating.root.workspace_root_identity,
        &accepted.task_id,
        creating.root.harness,
        crate::delegation::profile::SessionRootUse::New,
    )
    .unwrap();
    let identity = session
        .create_ownership_marker(&native_base, &creating.root)
        .unwrap();
    let native_root = session.path().to_owned();
    DelegationRuntimePort::commit_native_root_ready(
        runtime.adapter.as_ref(),
        &DelegationNativeRootReadyV1 {
            workspace_id: accepted.workspace_id.clone(),
            task_id: accepted.task_id.clone(),
            root_generation: creating.root.root_generation,
            creation_nonce: creating.root.creation_nonce,
            managed_base_path: native_base.to_str().unwrap().into(),
            filesystem_identity: identity,
        },
    )
    .unwrap();
    DelegationRuntimePort::checkpoint(
        runtime.adapter.as_ref(),
        &accepted.workspace_id,
        &accepted.run_id,
        accepted.progress.revision,
        &format!("cleanup-{suffix}-stopped"),
        &DelegationCheckpointV1::Progress {
            event: RunEventV1::LaunchFailedBeforeSpawn,
        },
    )
    .unwrap();

    let preview = runtime
        .observation
        .managed_text_delete_preview(&scope, 100, 100)
        .unwrap();
    let deleted = runtime
        .observation
        .managed_text_delete_apply(&preview)
        .unwrap();
    assert!(deleted.native_gc_pending);
    let mut jobs = runtime
        .observation
        .managed_text_pending_native_cleanup_page(None, 10)
        .unwrap();
    assert_eq!(jobs.len(), 1);
    CleanupFixture {
        runtime,
        _directory: directory,
        workspace_id: accepted.workspace_id,
        task_id: accepted.task_id,
        native_root,
        job: jobs.pop().unwrap(),
    }
}

#[test]
fn deleted_worker_body_revokes_continue_and_cleans_only_owned_metadata() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::delegation_maintenance::tests::deleted_worker_body_revokes_continue_and_cleans_only_owned_metadata",
    ) {
        return;
    }
    assert_borrowed_retention(RetentionAction::Delete);
}

#[test]
fn expired_worker_body_revokes_continue_and_cleans_only_owned_metadata() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::delegation_maintenance::tests::expired_worker_body_revokes_continue_and_cleans_only_owned_metadata",
    ) {
        return;
    }
    assert_borrowed_retention(RetentionAction::Expire);
}

enum RetentionAction {
    Delete,
    Expire,
}

struct RetainedTask {
    reference: ManagedTextRef,
    session: TaskSessionRoot,
    native_session_id: String,
}

// These are retention-consumer fixtures: lifecycle checkpoints stand in for a completed Worker.
// They exercise the real body barrier and cleanup consumer, not a public sessions.delete API.
fn assert_borrowed_retention(action: RetentionAction) {
    let directory = tempfile::tempdir().unwrap();
    let runtime = maintenance_runtime(directory.path());
    let base = std::fs::canonicalize(directory.path()).unwrap();
    for name in ["native", "home", "workspace"] {
        private_directory(&base.join(name));
    }
    let home = base.join("home");
    for name in [".codex", ".codex/skills", ".codex/sessions"] {
        private_directory(&home.join(name));
    }
    let shared_files: Vec<_> = [
        ("user-note.txt", b"user-owned home bytes".as_slice()),
        (".codex/config.toml", b"user-owned configuration".as_slice()),
        (".codex/skills/skill.md", b"user-owned skill".as_slice()),
        (
            ".codex/sessions/target.jsonl",
            b"opaque target native history".as_slice(),
        ),
        (
            ".codex/sessions/neighbor.jsonl",
            b"opaque neighbor native history".as_slice(),
        ),
    ]
    .into_iter()
    .map(|(relative, bytes)| {
        let path = home.join(relative);
        std::fs::write(&path, bytes).unwrap();
        (path, bytes.to_vec())
    })
    .collect();
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let target = retained_borrowed_task(&runtime, &base, "target", now);
    // A later body deadline keeps the neighbor visible during target expiry.
    let neighbor = retained_borrowed_task(&runtime, &base, "neighbor", now + 1_000);
    let workspace = &target.reference.scope.workspace_id;
    let target_id = &target.reference.scope.task_id;
    let before = DelegationRuntimePort::task(runtime.adapter.as_ref(), workspace, target_id)
        .unwrap()
        .unwrap();
    assert!(before.resume_until_ms > now as u64);
    assert!(
        runtime
            .adapter
            .required_bodies_visible(&before, now)
            .unwrap()
    );
    assert_eq!(
        read_retained_body(&runtime, &target, now).unwrap(),
        b"retained Worker body"
    );
    let neighbor_before = DelegationRuntimePort::task(
        runtime.adapter.as_ref(),
        workspace,
        &neighbor.reference.scope.task_id,
    )
    .unwrap()
    .unwrap();
    let neighbor_metadata = owned_metadata_bytes(neighbor.session.path());
    let effect_time = match action {
        RetentionAction::Expire => now + RETENTION_MS,
        RetentionAction::Delete => now + 1_001,
    };
    if matches!(action, RetentionAction::Expire) {
        assert_eq!(
            runtime
                .observation
                .managed_text_expire_pending(effect_time, 10)
                .unwrap(),
            1
        );
    } else {
        let preview = runtime
            .observation
            .managed_text_delete_preview(&target.reference.scope, effect_time, effect_time)
            .unwrap();
        assert!(
            runtime
                .observation
                .managed_text_delete_apply(&preview)
                .unwrap()
                .native_gc_pending
        );
    }

    // A retained native binding cannot restore text hidden by HiRoute's body authority, even
    // before cleanup removes the owned metadata. This is the same reader Continue invokes.
    target
        .session
        .verify_native_session(&target.native_session_id)
        .unwrap();
    assert_eq!(
        read_retained_body(&runtime, &target, effect_time),
        Err(DelegationErrorV1::ResumeUnavailable)
    );
    assert_eq!(
        runtime
            .observation
            .managed_text_pending_native_cleanup_page(None, 10)
            .unwrap()
            .len(),
        1
    );
    runtime.adapter.scan_task_maintenance(effect_time).unwrap();
    runtime.adapter.scan_native_cleanup(effect_time).unwrap();

    let after = DelegationRuntimePort::task(runtime.adapter.as_ref(), workspace, target_id)
        .unwrap()
        .unwrap();
    assert_eq!(after.resume_until_ms, 0);
    assert!(after.title.is_none());
    // Check the closed admission path after revocation. This fixture does not publish a Plan
    // or prove a successful Continue; the body reader above covers the pre-cleanup barrier.
    let request = WorkerContinueRequestV1 {
        schema: WORKER_CONTINUE_SCHEMA_V1.into(),
        task_id: target_id.clone(),
        expected_latest_run_id: target.reference.scope.run_id.clone(),
        cwd: None,
        permission_policy: Default::default(),
        run_timeout_secs: 60,
        input: DelegationTaskInputV1 {
            goal: "Continue the retained task".into(),
            context: String::new(),
            constraints: String::new(),
            acceptance_criteria: String::new(),
        },
        submission_key: "retention-continue".into(),
    };
    assert!(matches!(
        runtime.adapter.continue_worker(&request),
        Err(DelegationErrorV1::ResumeUnavailable)
    ));
    let removed =
        DelegationRuntimePort::native_root(runtime.adapter.as_ref(), workspace, target_id)
            .unwrap()
            .unwrap();
    assert_eq!(removed.root.state, DelegationNativeRootStateV1::Removed);
    assert!(!target.session.path().exists());
    assert!(
        runtime
            .observation
            .managed_text_pending_native_cleanup_page(None, 10)
            .unwrap()
            .is_empty()
    );
    assert!(
        DelegationRuntimePort::pending_continuation_releases(runtime.adapter.as_ref(), 10)
            .unwrap()
            .is_empty()
    );

    assert_eq!(
        DelegationRuntimePort::task(
            runtime.adapter.as_ref(),
            workspace,
            &neighbor.reference.scope.task_id
        )
        .unwrap()
        .unwrap(),
        neighbor_before
    );
    assert_eq!(
        owned_metadata_bytes(neighbor.session.path()),
        neighbor_metadata
    );
    neighbor
        .session
        .verify_native_session(&neighbor.native_session_id)
        .unwrap();
    assert_eq!(
        read_retained_body(&runtime, &neighbor, effect_time).unwrap(),
        b"retained Worker body"
    );
    for (path, expected) in shared_files {
        assert_eq!(
            std::fs::read(&path).unwrap(),
            expected,
            "borrowed file changed: {}",
            path.display()
        );
    }
}

fn retained_borrowed_task(
    runtime: &ProductionControlRuntime,
    base: &Path,
    suffix: &str,
    created_at_ms: i64,
) -> RetainedTask {
    let task_id = format!("retention-{suffix}-task");
    let run_id = format!("retention-{suffix}-run");
    let scope = ManagedTextScope {
        workspace_id: WorkspaceId::default(),
        task_id,
        run_id,
    };
    let reference = runtime
        .observation
        .managed_text_put(
            &ManagedTextInput {
                scope: scope.clone(),
                purpose: ManagedTextPurpose::Result,
                source_event_id: format!("retention-{suffix}-body"),
                source_revision: 1,
                original_created_at_ms: created_at_ms,
                import_origin: None,
            },
            created_at_ms,
        )
        .unwrap();
    runtime
        .observation
        .managed_text_append(
            &scope,
            &reference,
            0,
            b"retained Worker body",
            created_at_ms,
        )
        .unwrap();
    let reference = runtime
        .observation
        .managed_text_finish(&scope, &reference, 1, created_at_ms)
        .unwrap();
    let body = DelegationBodyRefV1 {
        opaque_id: reference.opaque_id.clone(),
        scope_run_id: scope.run_id.clone(),
        visibility_generation: reference.visibility_generation,
        original_retention_deadline_ms: reference.original_retention_deadline_ms,
    };
    let mut acceptance = super::super::tests::worker_list_acceptance(
        &scope.task_id,
        &scope.run_id,
        body.clone(),
        created_at_ms as u64,
    );
    acceptance.run.idempotency_key = format!("retention-{suffix}-start");
    acceptance.run.request_digest = hiroute_domain::CanonicalDigest::of_bytes(suffix.as_bytes());
    acceptance.run.execution_owner_ref = format!("retention-{suffix}-owner");
    acceptance.run.lease_id = format!("retention-{suffix}-lease");
    acceptance.run.launch_nonce = format!("retention-{suffix}-launch");
    acceptance.title_lookup_key = Some(format!("retention-{suffix}-title"));
    let mut run = DelegationRuntimePort::accept(runtime.adapter.as_ref(), &acceptance).unwrap();
    let creating = DelegationRuntimePort::native_root(
        runtime.adapter.as_ref(),
        &scope.workspace_id,
        &scope.task_id,
    )
    .unwrap()
    .unwrap();
    let native_base = base.join("native");
    let session = TaskSessionRoot::prepare(
        &native_base,
        &scope.workspace_id,
        &creating.root.workspace_root_identity,
        &scope.task_id,
        creating.root.harness,
        SessionRootUse::New,
    )
    .unwrap();
    session
        .bind_borrowed_context(
            &base.join("home"),
            &base.join("home/.codex"),
            &base.join("workspace"),
        )
        .unwrap();
    let identity = session
        .create_ownership_marker(&native_base, &creating.root)
        .unwrap();
    DelegationRuntimePort::commit_native_root_ready(
        runtime.adapter.as_ref(),
        &DelegationNativeRootReadyV1 {
            workspace_id: scope.workspace_id.clone(),
            task_id: scope.task_id.clone(),
            root_generation: creating.root.root_generation,
            creation_nonce: creating.root.creation_nonce,
            managed_base_path: native_base.to_str().unwrap().into(),
            filesystem_identity: identity,
        },
    )
    .unwrap();
    let native_session_id = format!("retention-{suffix}-native");
    let checkpoints = [
        DelegationCheckpointV1::Progress {
            event: RunEventV1::Preparing,
        },
        DelegationCheckpointV1::ProcessSpawned {
            binding: DelegationProcessBindingV1 {
                launch_nonce: run.launch_nonce.clone(),
                handle_id: format!("retention-{suffix}-process"),
                creation_identity: format!("retention-{suffix}-created"),
            },
        },
        DelegationCheckpointV1::SessionBound {
            binding: DelegationSessionBindingV1 {
                acp_session_id: native_session_id.clone(),
                native_session_id: Some(native_session_id.clone()),
            },
        },
        DelegationCheckpointV1::Progress {
            event: RunEventV1::PromptSendIntent,
        },
        DelegationCheckpointV1::ResultRecorded {
            body: Some(body.clone()),
            incomplete: false,
        },
        DelegationCheckpointV1::Progress {
            event: RunEventV1::PromptCompleted,
        },
        DelegationCheckpointV1::ProcessStopped {
            evidence: RunStopEvidenceV1 {
                scope: RunStopScopeV1::ProcessGroup,
                observation: RunProcessObservationV1::Exited { code: Some(0) },
                scope_stopped: true,
                residual_unknown: false,
            },
        },
    ];
    for (index, checkpoint) in checkpoints.iter().enumerate() {
        run = DelegationRuntimePort::checkpoint(
            runtime.adapter.as_ref(),
            &scope.workspace_id,
            &scope.run_id,
            run.progress.revision,
            &format!("retention-{suffix}-{index}"),
            checkpoint,
        )
        .unwrap();
    }
    assert_eq!(run.progress.state, RunStateV1::Succeeded);
    assert!(run.progress.workspace_releasable());
    let history =
        native_history(session.path(), creating.root.harness, &native_session_id).unwrap();
    DelegationRuntimePort::set_resume_materials(
        runtime.adapter.as_ref(),
        &scope.workspace_id,
        &scope.task_id,
        &scope.run_id,
        reference.original_retention_deadline_ms as u64,
        &[body],
        &history,
    )
    .unwrap();
    RetainedTask {
        reference,
        session,
        native_session_id,
    }
}

fn read_retained_body(
    runtime: &ProductionControlRuntime,
    task: &RetainedTask,
    now_ms: i64,
) -> Result<Vec<u8>, DelegationErrorV1> {
    read_required_body(
        &runtime.observation,
        &task.reference.scope,
        &task.reference,
        now_ms,
        1_024,
        |_| Ok(()),
    )
}

fn owned_metadata_bytes(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    files.sort();
    files
}

fn maintenance_runtime(directory: &Path) -> ProductionControlRuntime {
    let storage = directory.join("storage");
    private_directory(&storage);
    let mut runtime = ProductionControlRuntime::open_with_release_catalog(
        &storage,
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    runtime._observation_maintenance.take();
    for _ in 0..100 {
        if !runtime.observation.maintenance_status().0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!runtime.observation.maintenance_status().0);
    runtime
}

fn private_directory(path: &Path) {
    std::fs::create_dir(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}
