use super::*;
use hiroute_domain::CanonicalDigest;
use std::cell::RefCell;

struct Fixture {
    run: DelegationRunV1,
    allowed: bool,
    calls: RefCell<Vec<&'static str>>,
}
impl DelegationControlAccess for Fixture {
    fn authorize(
        &self,
        _: &DelegationRunV1,
        _: DelegationControlAction,
    ) -> Result<String, DelegationErrorV1> {
        self.calls.borrow_mut().push("authorize");
        if self.allowed {
            Ok("verified-user".into())
        } else {
            Err(DelegationErrorV1::PermissionDenied)
        }
    }
}
impl DelegationRunDenial for Fixture {
    fn deny(&self, _: &DelegationRunV1) -> Result<(), DelegationErrorV1> {
        self.calls.borrow_mut().push("deny");
        Ok(())
    }
}
impl DelegationRuntimePort for Fixture {
    fn run(&self, _: &WorkspaceId, _: &str) -> Result<Option<DelegationRunV1>, DelegationErrorV1> {
        Ok(Some(self.run.clone()))
    }
    fn request_cancel(
        &self,
        _: &WorkspaceId,
        _: &str,
        _: &OperationId,
        _: &str,
    ) -> Result<DelegationCancelReceiptV1, DelegationErrorV1> {
        self.calls.borrow_mut().push("persist");
        Err(DelegationErrorV1::StorageUnavailable)
    }
    fn checkpoint(
        &self,
        _: &WorkspaceId,
        _: &str,
        _: u64,
        _: &str,
        event: &DelegationCheckpointV1,
    ) -> Result<DelegationRunV1, DelegationErrorV1> {
        let DelegationCheckpointV1::ResidualConfirmed { actor_id, .. } = event else {
            panic!("wrong event")
        };
        assert_eq!(actor_id, "verified-user");
        self.calls.borrow_mut().push("confirm");
        Err(DelegationErrorV1::Conflict)
    }
    fn task(
        &self,
        _: &WorkspaceId,
        _: &str,
    ) -> Result<Option<DelegationTaskV1>, DelegationErrorV1> {
        unreachable!()
    }
    fn find_submission(
        &self,
        _: &WorkspaceId,
        _: bool,
        _: &str,
    ) -> Result<Option<DelegationRunV1>, DelegationErrorV1> {
        unreachable!()
    }
    fn accept(&self, _: &DelegationAcceptanceV1) -> Result<DelegationRunV1, DelegationErrorV1> {
        unreachable!()
    }
    fn unreconciled(&self, _: &WorkspaceId) -> Result<Vec<DelegationRunV1>, DelegationErrorV1> {
        unreachable!()
    }
    fn set_resume_materials(
        &self,
        _: &WorkspaceId,
        _: &str,
        _: &str,
        _: u64,
        _: &[DelegationBodyRefV1],
        _: &[String],
    ) -> Result<(), DelegationErrorV1> {
        unreachable!()
    }
}
fn fixture(allowed: bool) -> Fixture {
    Fixture {
        allowed,
        calls: RefCell::new(vec![]),
        run: DelegationRunV1 {
            execution_failure: None,
            workspace_id: WorkspaceId::default(),
            task_id: "task".into(),
            run_id: "run".into(),
            ordinal: 1,
            continued_from: None,
            idempotency_key: "key".into(),
            request_digest: CanonicalDigest::of_bytes(b"request"),
            admission_sequence: 1,
            accepted_at_ms: None,
            execution_owner_ref: "owner-ref".into(),
            lease_id: "lease".into(),
            daemon_epoch: "epoch".into(),
            permit_id: "run-config/run".into(),
            permit_generation: 1,
            configuration: DelegationRunConfigurationV1 {
                format_version: DELEGATION_RUN_CONFIGURATION_VERSION_V1,
                scope_id: "run-config/run".into(),
                generation: 1,
                canonical_workspace_path: "/workspace".into(),
                permission_policy: WorkerPermissionPolicyV1::ApproveAll,
            },
            execution: WorkerExecutionIntentV1 {
                root_identity: "root".into(),
                access: WorkspaceAccessV1::TrustedNative,
                tools: vec![WorkerToolV1::Read],
                network: WorkerNetworkV1::Allowed,
                duration_ms: 1000,
                delegation_depth: 1,
            },
            deadline_ms: 1001,
            lease_revoked: false,
            launch_nonce: "nonce".into(),
            process: None,
            session: None,
            progress: RunProgressV1::default(),
            stop_evidence: None,
            result_body: None,
            result_incomplete: false,
        },
    }
}
fn operation() -> OperationId {
    OperationId::parse("op_00000000000000000000000000000003").unwrap()
}

#[test]
fn unauthorized_control_never_revokes_or_mutates_other_run() {
    let fixture = fixture(false);
    let service = DelegationControl {
        runtime: &fixture,
        access: &fixture,
        denial: &fixture,
    };
    assert!(
        service
            .cancel(&WorkspaceId::default(), "run", &operation(), "user")
            .is_err()
    );
    assert_eq!(*fixture.calls.borrow(), ["authorize"]);
    fixture.calls.borrow_mut().clear();
    assert!(
        service
            .confirm_residual(&WorkspaceId::default(), "run", 1, &operation())
            .is_err()
    );
    assert_eq!(*fixture.calls.borrow(), ["authorize"]);
}

#[test]
fn cancellation_storage_failure_occurs_after_irreversible_denial() {
    let fixture = fixture(true);
    let service = DelegationControl {
        runtime: &fixture,
        access: &fixture,
        denial: &fixture,
    };
    assert_eq!(
        service.cancel(&WorkspaceId::default(), "run", &operation(), "user"),
        Err(DelegationErrorV1::StorageUnavailable)
    );
    assert_eq!(*fixture.calls.borrow(), ["authorize", "deny", "persist"]);
    fixture.calls.borrow_mut().clear();
    assert!(
        service
            .confirm_residual(&WorkspaceId::default(), "run", 1, &operation())
            .is_err()
    );
    assert_eq!(*fixture.calls.borrow(), ["authorize", "deny", "confirm"]);
}
