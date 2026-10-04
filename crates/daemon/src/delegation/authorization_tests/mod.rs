//! Real SQLite/Operation integration, with a narrow exact-version check fixture.
mod dispatcher_tests;
pub(in crate::delegation) mod fixture;
use fixture::*;
use hiroute_application::delegation::{
    admission::{DelegationAdmission, verify_bootstrap},
    authorization::DelegationAuthorization,
    safety::RunSafetyProjection,
};
use hiroute_application::publication::admission::*;
use hiroute_domain::delegation::*;
use hiroute_domain::*;
use hiroute_local_storage::LocalStorageSet;
use std::collections::BTreeSet;
use std::sync::Arc;

fn scopes(permit_id: &str) -> BTreeSet<AdmissionSubject> {
    BTreeSet::from([
        AdmissionSubject::Grant("collaboration-grant/one".into()),
        AdmissionSubject::Permit(permit_id.into()),
    ])
}
fn setup() -> (
    tempfile::TempDir,
    LocalStorageSet,
    Arc<SharedAdmissionGate>,
    Arc<RunSafetyProjection>,
) {
    let dir = tempfile::tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(dir.path().join("store")).unwrap();
    let gate = Arc::new(SharedAdmissionGate::new());
    let safety = Arc::new(RunSafetyProjection::new(gate.clone(), "epoch".into()).unwrap());
    safety.finish_startup_recovery();
    (dir, stores, gate, safety)
}

#[test]
fn public_local_worker_uses_its_run_profile_not_the_legacy_control_permit() {
    let (_dir, stores, gate, safety) = setup();
    let (grant, _material) = configured(&stores);
    let operation = begin(&stores, "legacy-permit-mutation");
    let mut revoked = permit();
    revoked.generation = 2;
    revoked.revoked = true;
    stores
        .control()
        .commit_permit(&DelegationPermitMutationV1 {
            workspace: grant.workspace_id,
            operation: operation.operation_id.clone(),
            before: Some(permit()),
            after: revoked,
        })
        .unwrap();

    let admission = DelegationAdmission {
        gate: gate.clone(),
        safety: &safety,
        runtime: stores.runtime(),
    };
    let accepted = admission
        .accept(&sample("local-task", "local-run", "root-a"), |guard, _| {
            assert!(guard.belongs_to(&gate));
            Ok(())
        })
        .unwrap();
    assert_eq!(accepted.run_id, "local-run");
}

#[test]
fn collaboration_bootstrap_revocation_does_not_become_worker_authority() {
    let (_dir, stores, gate, safety) = setup();
    let (grant, material) = configured(&stores);
    verify_bootstrap(
        stores.control(),
        &grant.workspace_id,
        &grant.grant_id,
        &grant.context_id,
        1,
        &material,
    )
    .unwrap();
    assert!(
        verify_bootstrap(
            stores.control(),
            &grant.workspace_id,
            &grant.grant_id,
            "forged",
            1,
            &material
        )
        .is_err()
    );
    assert!(
        verify_bootstrap(
            stores.control(),
            &grant.workspace_id,
            &grant.grant_id,
            &grant.context_id,
            1,
            &AgentCollaborationCredential::from_csprng_entropy([8; 32])
        )
        .is_err()
    );
    let admission = DelegationAdmission {
        gate: gate.clone(),
        safety: &safety,
        runtime: stores.runtime(),
    };
    let start = sample("task-a", "run-a", "root-a");
    assert!(
        admission
            .accept(&start, |guard, _| {
                assert!(guard.belongs_to(&gate));
                Ok(())
            })
            .is_ok()
    );
    let operation = begin(&stores, "revoke");
    let change = grant
        .plan_revocation(operation.operation_id.clone(), 1)
        .unwrap();
    let auth = DelegationAuthorization {
        gate: gate.clone(),
        safety: &safety,
        permits: stores.control(),
        cancellations: stores.runtime(),
    };
    {
        let mut guard = gate
            .enter(
                &grant.workspace_id,
                &scopes(&start.run.permit_id),
                AdmissionAction::AuthorizationChange,
                operation.operation_id.as_str(),
            )
            .unwrap();
        auth.revoke_grant(&mut guard, &change, stores.control())
            .unwrap();
    }
    assert!(
        verify_bootstrap(
            stores.control(),
            &grant.workspace_id,
            &grant.grant_id,
            &grant.context_id,
            1,
            &material
        )
        .is_err()
    );
    assert!(
        admission
            .accept(&sample("task-b", "run-b", "root-a"), |_, _| Ok(()))
            .is_ok(),
        "Worker admission is instance-scoped and does not consume a collaboration grant"
    );
    assert!(
        !stores
            .runtime()
            .run(&grant.workspace_id, "run-a")
            .unwrap()
            .unwrap()
            .lease_revoked,
        "instance-scoped Worker runs are not linked to collaboration grants"
    );
}

struct FailingCancellation;
impl DelegationScopeCancellationPort for FailingCancellation {
    fn cancel_scope(
        &self,
        _: &WorkspaceId,
        _: &OperationId,
        _: &DelegationAuthorizationScopeV1,
    ) -> Result<String, DelegationErrorV1> {
        Err(DelegationErrorV1::StorageUnavailable)
    }
}
#[test]
fn legacy_permit_partial_failure_does_not_replay_into_public_worker_authority() {
    let (dir, stores, gate, safety) = setup();
    let (grant, _material) = configured(&stores);
    let start = sample("task-a", "run-a", "root-a");
    let admission = DelegationAdmission {
        gate: gate.clone(),
        safety: &safety,
        runtime: stores.runtime(),
    };
    admission.accept(&start, |_, _| Ok(())).unwrap();
    let op = begin(&stores, "permit-revoke");
    let auth = DelegationAuthorization {
        gate: gate.clone(),
        safety: &safety,
        permits: stores.control(),
        cancellations: &FailingCancellation,
    };
    let mut after = permit();
    after.generation = 2;
    after.revoked = true;
    let (change, digest) = auth
        .preview_permit(&grant.workspace_id, &op.operation_id, after)
        .unwrap();
    {
        let mut guard = gate
            .enter(
                &grant.workspace_id,
                &scopes("permit"),
                AdmissionAction::AuthorizationChange,
                op.operation_id.as_str(),
            )
            .unwrap();
        assert_eq!(
            auth.commit_permit(&mut guard, &change, &CanonicalDigest::of_bytes(b"wrong")),
            Err(DelegationErrorV1::Conflict)
        );
        assert_eq!(
            auth.commit_permit(&mut guard, &change, &digest),
            Err(DelegationErrorV1::StorageUnavailable)
        );
    }
    assert!(
        stores
            .control()
            .permit(&grant.workspace_id, "permit")
            .unwrap()
            .unwrap()
            .revoked
    );
    assert!(
        !stores
            .runtime()
            .run(&grant.workspace_id, "run-a")
            .unwrap()
            .unwrap()
            .lease_revoked
    );
    let mut next = sample("task-b", "run-b", "root-b");
    assert!(
        admission.accept(&next, |_, _| Ok(())).is_ok(),
        "a legacy permit mutation cannot block an independent run-scoped Worker profile"
    );
    drop(admission);
    drop(auth);
    drop(stores);
    let stores = LocalStorageSet::open_for_daemon_startup(dir.path().join("store")).unwrap();
    let gate = Arc::new(SharedAdmissionGate::new());
    let safety = RunSafetyProjection::new(gate.clone(), "next-epoch".into()).unwrap();
    for _ in 0..2 {
        super::authorization_recovery::recover_authorizations(
            &stores,
            &grant.workspace_id,
            gate.clone(),
            &safety,
        )
        .unwrap();
    }
    safety.finish_startup_recovery();
    let run = stores
        .runtime()
        .run(&grant.workspace_id, "run-a")
        .unwrap()
        .unwrap();
    assert!(!run.lease_revoked);
    assert_eq!(run.progress.state, RunStateV1::Accepted);
    assert!(!run.progress.prompt_may_have_executed);
    let admission = DelegationAdmission {
        gate,
        safety: &safety,
        runtime: stores.runtime(),
    };
    next.run.daemon_epoch = "next-epoch".into();
    assert!(admission.accept(&next, |_, _| Ok(())).is_ok());
}

#[cfg(unix)]
#[test]
fn prepared_credential_crosses_only_protected_channel_and_uses_current_grant() {
    use super::bootstrap;
    use std::os::{fd::OwnedFd, unix::net::UnixStream};
    let (_dir, stores, _gate, _safety) = setup();
    let (grant, material) = configured(&stores);
    let (write, read) = UnixStream::pair().unwrap();
    let write = std::fs::File::from(OwnedFd::from(write));
    let read = std::fs::File::from(OwnedFd::from(read));
    bootstrap::supply(&write, &material).unwrap();
    let principal = bootstrap::receive(
        &read,
        stores.control(),
        &grant.workspace_id,
        &grant.grant_id,
        &grant.context_id,
        1,
    )
    .unwrap();
    assert_eq!(principal.grant_id(), grant.grant_id);
    let file = tempfile::tempfile().unwrap();
    assert!(bootstrap::supply(&file, &material).is_err());
    let operation = begin(&stores, "channel-revoke");
    stores
        .control()
        .record_collaboration_revocation(&grant.plan_revocation(operation.operation_id, 1).unwrap())
        .unwrap();
    bootstrap::supply(&write, &material).unwrap();
    assert!(
        bootstrap::receive(
            &read,
            stores.control(),
            &grant.workspace_id,
            &grant.grant_id,
            &grant.context_id,
            1
        )
        .is_err()
    );
}

#[test]
fn continued_run_keeps_original_session_and_requires_exact_run_configuration() {
    let (_dir, stores, gate, safety) = setup();
    let admission = DelegationAdmission {
        gate: gate.clone(),
        safety: &safety,
        runtime: stores.runtime(),
    };
    let run = admission
        .accept(&sample("task-a", "run-a", "root-a"), |_, _| Ok(()))
        .unwrap();
    let native = stores
        .runtime()
        .native_root(&run.workspace_id, &run.task_id)
        .unwrap()
        .unwrap();
    stores
        .runtime()
        .commit_native_root_ready(&DelegationNativeRootReadyV1 {
            workspace_id: run.workspace_id.clone(),
            task_id: run.task_id.clone(),
            root_generation: native.root.root_generation,
            creation_nonce: native.root.creation_nonce,
            managed_base_path: "/managed/sessions".into(),
            filesystem_identity: DelegationNativeFilesystemIdentityV1 {
                scheme: "unix-dev-inode-v1".into(),
                base_device: 1,
                base_inode: 2,
                root_device: 1,
                root_inode: 3,
                marker_device: 1,
                marker_inode: 4,
            },
        })
        .unwrap();
    dispatcher_tests::bind(&stores, &run);
    let run = stores
        .runtime()
        .run(&run.workspace_id, &run.run_id)
        .unwrap()
        .unwrap();
    let run = stores
        .runtime()
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "session",
            &DelegationCheckpointV1::SessionBound {
                binding: DelegationSessionBindingV1 {
                    acp_session_id: "session-a".into(),
                    native_session_id: Some("native-a".into()),
                },
            },
        )
        .unwrap();
    stores
        .runtime()
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "stopped",
            &DelegationCheckpointV1::ProcessStopped {
                evidence: RunStopEvidenceV1 {
                    scope: RunStopScopeV1::ProcessGroup,
                    observation: RunProcessObservationV1::Exited { code: Some(0) },
                    scope_stopped: true,
                    residual_unknown: false,
                },
            },
        )
        .unwrap();
    stores
        .runtime()
        .set_resume_materials(
            &run.workspace_id,
            &run.task_id,
            &run.run_id,
            10000,
            &[],
            &["history".into()],
        )
        .unwrap();
    let mut next = sample("task-a", "run-b", "root-a");
    next.task = stores
        .runtime()
        .task(&run.workspace_id, &run.task_id)
        .unwrap()
        .unwrap();
    next.task.latest_run_id = "run-b".into();
    next.task.required_body_ids.push("next-input".into());
    next.task.body_refs.push(DelegationBodyRefV1 {
        opaque_id: "next-input".into(),
        scope_run_id: "run-b".into(),
        visibility_generation: 1,
        original_retention_deadline_ms: 10000,
    });
    next.run.ordinal = 2;
    next.run.continued_from = Some(run.run_id.clone());
    next.expected_latest_run_id = Some(run.run_id.clone());
    next.title_lookup_key = None;
    let mut bad = next.clone();
    bad.run.configuration.scope_id = "run-config/another-run".into();
    assert_eq!(
        admission.accept(&bad, |_, _| panic!(
            "invalid run configuration before exact load"
        )),
        Err(DelegationErrorV1::InvalidArguments)
    );
    let accepted = admission
        .accept(&next, |guard, input| {
            assert!(guard.belongs_to(&gate));
            assert_eq!(
                input
                    .task
                    .session
                    .as_ref()
                    .unwrap()
                    .native_session_id
                    .as_deref(),
                Some("native-a")
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(accepted.continued_from, Some(run.run_id));
    assert_ne!(accepted.lease_id, run.lease_id);
}

#[test]
fn prepared_legacy_permit_survives_restart_without_blocking_public_worker() {
    let (dir, stores, _gate, _safety) = setup();
    let (grant, _material) = configured(&stores);
    let op = begin(&stores, "prepared-permit");
    let mut after = permit();
    after.generation = 2;
    after.revoked = true;
    let change = DelegationPermitMutationV1 {
        workspace: grant.workspace_id.clone(),
        operation: op.operation_id.clone(),
        before: Some(permit()),
        after,
    };
    stores.control().prepare_permit(&change).unwrap();
    drop(stores);
    let stores = LocalStorageSet::open_for_daemon_startup(dir.path().join("store")).unwrap();
    let gate = Arc::new(SharedAdmissionGate::new());
    let safety = RunSafetyProjection::new(gate.clone(), "new-epoch".into()).unwrap();
    super::authorization_recovery::recover_authorizations(
        &stores,
        &grant.workspace_id,
        gate.clone(),
        &safety,
    )
    .unwrap();
    safety.finish_startup_recovery();
    assert_eq!(
        stores
            .control()
            .permit(&grant.workspace_id, "permit")
            .unwrap(),
        Some(permit())
    );
    assert_eq!(
        stores
            .control()
            .pending_permit_mutations(&grant.workspace_id)
            .unwrap(),
        vec![change.clone()]
    );
    let admission = DelegationAdmission {
        gate: gate.clone(),
        safety: &safety,
        runtime: stores.runtime(),
    };
    let mut input = sample("task-new", "run-new", "root-a");
    input.run.daemon_epoch = "new-epoch".into();
    assert!(admission.accept(&input, |_, _| Ok(())).is_ok());
    assert!(
        gate.enter(
            &grant.workspace_id,
            &BTreeSet::from([AdmissionSubject::Permit("unrelated".into())]),
            AdmissionAction::Start,
            "unrelated-run"
        )
        .is_ok()
    );
    let auth = DelegationAuthorization {
        gate: gate.clone(),
        safety: &safety,
        permits: stores.control(),
        cancellations: stores.runtime(),
    };
    let mut guard = gate
        .enter(
            &grant.workspace_id,
            &BTreeSet::from([AdmissionSubject::Permit("permit".into())]),
            AdmissionAction::Recovery,
            op.operation_id.as_str(),
        )
        .unwrap();
    auth.commit_permit(&mut guard, &change, &change.digest().unwrap())
        .unwrap();
    // Only the original settings owner, once its other steps are done, can clear this guard.
    guard.complete_recovery().unwrap();
    drop(guard);
    assert!(
        stores
            .control()
            .pending_permit_mutations(&grant.workspace_id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        stores
            .control()
            .permit(&grant.workspace_id, "permit")
            .unwrap(),
        Some(change.after)
    );
    assert!(
        !stores
            .runtime()
            .run(&grant.workspace_id, "run-new")
            .unwrap()
            .unwrap()
            .lease_revoked
    );
}
