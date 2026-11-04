use super::*;

fn permit() -> WorkspaceExecutionPermitV1 {
    WorkspaceExecutionPermitV1 {
        permit_id: "permit-a".into(),
        generation: 3,
        root_identity: "volume:inode:root-a".into(),
        access: WorkspaceAccessV1::WorkspaceWrite,
        tools: vec![WorkerToolV1::Read, WorkerToolV1::Edit],
        network: WorkerNetworkV1::GatewayOnly,
        expires_at_ms: 100_000,
        max_run_ms: MAX_RUN_DURATION_MS,
        max_concurrent: 2,
        revoked: false,
    }
}

#[test]
fn delegation_permit_intersects_scope_duration_and_current_generation() {
    let p = permit();
    let intent = WorkerExecutionIntentV1 {
        root_identity: p.root_identity.clone(),
        access: WorkspaceAccessV1::WorkspaceWrite,
        tools: vec![WorkerToolV1::Edit],
        network: WorkerNetworkV1::GatewayOnly,
        duration_ms: 10_000,
        delegation_depth: 1,
    };
    assert_eq!(p.authorize(&intent, 1_000, 3), Ok(11_000));
    assert_eq!(
        p.authorize(&intent, 1_000, 2),
        Err(DelegationErrorV1::PermissionDenied)
    );
    assert_eq!(
        p.authorize(&intent, 95_000, 3),
        Err(DelegationErrorV1::DeadlineExceeded)
    );
    let mut foreign = intent.clone();
    foreign.root_identity = "another-root".into();
    assert_eq!(
        p.authorize(&foreign, 1_000, 3),
        Err(DelegationErrorV1::PermissionDenied)
    );
    foreign = intent.clone();
    foreign.tools.push(WorkerToolV1::Shell);
    assert_eq!(
        p.authorize(&foreign, 1_000, 3),
        Err(DelegationErrorV1::PermissionDenied)
    );
    foreign = intent;
    foreign.delegation_depth = 2;
    assert_eq!(
        p.authorize(&foreign, 1_000, 3),
        Err(DelegationErrorV1::PermissionDenied)
    );
}

#[test]
fn delegation_read_only_never_authorizes_edit_or_network_widening() {
    let mut p = permit();
    p.access = WorkspaceAccessV1::ReadOnly;
    p.tools = vec![WorkerToolV1::Read];
    let mut intent = WorkerExecutionIntentV1 {
        root_identity: p.root_identity.clone(),
        access: WorkspaceAccessV1::ReadOnly,
        tools: vec![WorkerToolV1::Read],
        network: WorkerNetworkV1::GatewayOnly,
        duration_ms: 1_000,
        delegation_depth: 1,
    };
    assert_eq!(p.authorize(&intent, 1_000, 3), Ok(2_000));
    intent.access = WorkspaceAccessV1::WorkspaceWrite;
    assert!(p.authorize(&intent, 1_000, 3).is_err());
    intent.access = WorkspaceAccessV1::ReadOnly;
    intent.network = WorkerNetworkV1::Allowed;
    assert!(p.authorize(&intent, 1_000, 3).is_err());
    p.revoked = true;
    assert!(p.authorize(&intent, 1_000, 3).is_err());
}

#[test]
fn delegation_permit_rejects_invalid_limits_even_for_deserialized_data() {
    let mut p = permit();
    p.max_run_ms = MAX_RUN_DURATION_MS + 1;
    assert_eq!(p.validate(), Err(DelegationErrorV1::InvalidArguments));
    p = permit();
    p.max_concurrent = 0;
    assert!(p.validate().is_err());
    p = permit();
    p.generation = 0;
    assert!(p.validate().is_err());
}

#[test]
fn delegation_exit_zero_does_not_prove_prompt_completion() {
    let mut run = RunProgressV1::default();
    run.advance(RunEventV1::Preparing).unwrap();
    run.advance(RunEventV1::PromptSendIntent).unwrap();
    run.advance(RunEventV1::ManagedScopeStopped { success: true })
        .unwrap();
    assert_eq!(run.state, RunStateV1::Unknown);
    assert!(run.prompt_may_have_executed);
    assert_eq!(run.cleanup, RunCleanupV1::Complete);
    assert!(run.advance(RunEventV1::PromptSendIntent).is_err());
}

#[test]
fn delegation_cancel_needs_exit_evidence_and_ignores_late_completion() {
    let mut run = RunProgressV1::default();
    run.advance(RunEventV1::Preparing).unwrap();
    run.advance(RunEventV1::PromptSendIntent).unwrap();
    run.advance(RunEventV1::CancelRequested).unwrap();
    assert_eq!(run.state, RunStateV1::Cancelling);
    run.advance(RunEventV1::PromptCompleted).unwrap();
    assert_eq!(run.state, RunStateV1::Cancelling);
    run.advance(RunEventV1::ConnectionLost).unwrap();
    assert_eq!(run.state, RunStateV1::Unknown);
    assert!(run.cancel_requested);
    run.advance(RunEventV1::ManagedScopeStopped { success: true })
        .unwrap();
    assert_eq!(run.state, RunStateV1::Cancelled);
}

#[test]
fn delegation_normal_completion_keeps_cleanup_separate() {
    let mut run = RunProgressV1::default();
    run.advance(RunEventV1::Preparing).unwrap();
    run.advance(RunEventV1::PromptSendIntent).unwrap();
    run.advance(RunEventV1::PromptCompleted).unwrap();
    assert_eq!(run.state, RunStateV1::Succeeded);
    assert_eq!(run.cleanup, RunCleanupV1::Pending);
    assert!(!run.workspace_releasable());
    run.advance(RunEventV1::ManagedScopeStopped { success: true })
        .unwrap();
    assert!(run.workspace_releasable());
}

#[test]
fn delegation_authoritative_prompt_failure_is_not_unknown_or_success() {
    let mut run = RunProgressV1::default();
    run.advance(RunEventV1::Preparing).unwrap();
    run.advance(RunEventV1::ProcessRunning).unwrap();
    run.advance(RunEventV1::PromptSendIntent).unwrap();
    run.advance(RunEventV1::PromptFailed).unwrap();
    assert_eq!(run.state, RunStateV1::Failed);
    assert!(run.prompt_may_have_executed);
    assert_eq!(run.cleanup, RunCleanupV1::Pending);
    run.advance(RunEventV1::ManagedScopeStopped { success: true })
        .unwrap();
    assert!(run.workspace_releasable());
}

#[test]
fn delegation_disconnect_before_and_after_send_are_distinct() {
    let mut before = RunProgressV1::default();
    before.advance(RunEventV1::Preparing).unwrap();
    before.advance(RunEventV1::ConnectionLost).unwrap();
    assert_eq!(before.state, RunStateV1::Failed);
    assert!(!before.workspace_releasable());
    let mut after = RunProgressV1::default();
    after.advance(RunEventV1::Preparing).unwrap();
    after.advance(RunEventV1::PromptSendIntent).unwrap();
    after.advance(RunEventV1::ConnectionLost).unwrap();
    assert_eq!(after.state, RunStateV1::Unknown);
    assert!(!after.workspace_releasable());
}

#[test]
fn delegation_cancel_keeps_released_unknown_terminal_and_preserves_uncertainty() {
    for acknowledged in [false, true] {
        let mut run = RunProgressV1::default();
        run.advance(RunEventV1::Preparing).unwrap();
        run.advance(RunEventV1::PromptSendIntent).unwrap();
        run.advance(RunEventV1::ConnectionLost).unwrap();
        if acknowledged {
            run.advance(RunEventV1::ResidualAcknowledged).unwrap();
        } else {
            run.advance(RunEventV1::ManagedScopeStopped { success: true })
                .unwrap();
        }
        assert_eq!(run.state, RunStateV1::Unknown);
        assert!(run.workspace_releasable());
        let before = run.clone();
        for _ in 0..2 {
            run.advance(RunEventV1::CancelRequested).unwrap();
            assert_eq!(run, before);
        }
    }

    let mut run = RunProgressV1::default();
    run.advance(RunEventV1::Preparing).unwrap();
    run.advance(RunEventV1::PromptSendIntent).unwrap();
    run.advance(RunEventV1::ConnectionLost).unwrap();
    run.advance(RunEventV1::CancelRequested).unwrap();
    assert_eq!(run.state, RunStateV1::Cancelling);
    assert!(!run.workspace_releasable());
    run.advance(RunEventV1::ManagedScopeStopped { success: false })
        .unwrap();
    assert_eq!(run.state, RunStateV1::Cancelled);
    assert!(run.workspace_releasable());
}

#[test]
fn stored_strict_permit_requires_explicit_native_mode_approval() {
    let mut p = permit();
    let intent = WorkerExecutionIntentV1 {
        root_identity: p.root_identity.clone(),
        access: WorkspaceAccessV1::TrustedNative,
        tools: vec![WorkerToolV1::Read],
        network: WorkerNetworkV1::Allowed,
        duration_ms: 1000,
        delegation_depth: 1,
    };
    assert_eq!(
        p.authorize(&intent, 1, 3),
        Err(DelegationErrorV1::PermissionDenied)
    );
    p.access = WorkspaceAccessV1::TrustedNative;
    assert_eq!(
        p.authorize(&intent, 1, 3),
        Err(DelegationErrorV1::PermissionDenied)
    );
    p.network = WorkerNetworkV1::Allowed;
    assert_eq!(p.authorize(&intent, 1, 3), Ok(1001));
}
