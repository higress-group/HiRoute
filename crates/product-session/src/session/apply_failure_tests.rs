use super::*;
use hiroute_client_core::ControlDispatch;
use std::sync::{Mutex, atomic::AtomicUsize};
use zeroize::Zeroizing;

struct ReadOnlyAuthority;
impl HostAuthority for ReadOnlyAuthority {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn has_authority(&self) -> bool {
        true
    }
    fn register_operation(
        &self,
        _: &CanonicalDigest,
        _: &RevisionSetV1,
        _: &'static str,
    ) -> Result<Zeroizing<String>, String> {
        panic!("failure readback must not request mutation authority")
    }
    fn register_input(
        &self,
        _: &ComputeCandidateRefV2,
        _: Zeroizing<String>,
    ) -> Result<(), String> {
        panic!("failure readback must not register inputs")
    }
    fn release_model_input(&self, _: &ComputeCandidateRefV2) -> Result<(), String> {
        panic!("no model inputs belong to this test")
    }
}

#[derive(Clone, Copy)]
enum Reply {
    Detail,
    Missing,
    OtherIdentity,
    Stale,
    Unavailable,
}
struct Lookup {
    state: &'static str,
    reply: Mutex<Reply>,
    reads: AtomicUsize,
}
impl ControlDispatch for Lookup {
    fn dispatch(&self, request: LocalControlWireRequestV2) -> MachineEnvelopeV2<Value> {
        assert_eq!(request.operation_id, "FindOperationByIdempotency");
        assert!(request.protected_grant.is_none());
        let lookup: OperationIdempotencyLookupV1 = serde_json::from_value(request.payload).unwrap();
        assert!(lookup.include_failure_detail);
        assert_eq!(lookup.operation_kind, "ApplyAgentConnectionChange");
        assert_eq!(lookup.idempotency_key, "save-once");
        assert_eq!(lookup.accepted_digest, digest());
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let reply = *self.reply.lock().unwrap();
        if matches!(reply, Reply::Unavailable) {
            return MachineEnvelopeV2::failed(
                ErrorV1::new(ErrorCode::DaemonUnavailable),
                Some(request.request_id),
            );
        }
        let operation = (!matches!(reply, Reply::Missing)).then(|| serde_json::json!({
            "operation_id": if matches!(reply, Reply::OtherIdentity) { "operation/other" } else { "operation/failed-artifact" },
            "state": self.state,
            "sequence": if matches!(reply, Reply::Stale) { 8 } else { 9 },
            "cancellable": false,
            "accepted_digest": digest(),
            "safe_error_code": "ADAPTER_FAILURE",
            "failure_detail": {
                "schema": "hiroute.operation-failure/v1", "step": "apply_agent_artifacts",
                "code": "ADAPTER_FAILURE", "cause": "permission_denied"
            }
        }));
        MachineEnvelopeV2::succeeded(
            serde_json::json!({"operation":operation,"digest_matches":true}),
            Some(request.request_id),
        )
    }
}
fn digest() -> CanonicalDigest {
    CanonicalDigest::of_bytes(b"accepted agent edit")
}
fn fixture(state: &'static str, reply: Reply) -> (Session, Arc<Lookup>) {
    let lookup = Arc::new(Lookup {
        state,
        reply: Mutex::new(reply),
        reads: AtomicUsize::new(0),
    });
    let mut session = Session::new(
        Client::in_process("failed-apply-test", lookup.clone()),
        Arc::new(ReadOnlyAuthority),
    );
    session.hint = Some(SubmittedOperation {
        plan_id: String::new(),
        principal_kind: PrincipalKind::InteractiveUser,
        operation_kind: "ApplyAgentConnectionChange".into(),
        idempotency_key: "save-once".into(),
        accepted_digest: digest(),
        operation_id: None,
        after_sequence: 0,
        intent: None,
        latest_edit_not_applied: false,
    });
    (session, lookup)
}
fn accepted(state: &str) -> MachineEnvelopeV2<Value> {
    accepted_for(
        "operation/failed-artifact",
        state,
        &digest(),
        Some("request/save-once".into()),
    )
}

fn accepted_for(
    operation_id: &str,
    state: &str,
    accepted_digest: &CanonicalDigest,
    request_id: Option<String>,
) -> MachineEnvelopeV2<Value> {
    let mut response = MachineEnvelopeV2::accepted(
        serde_json::json!({"operation_id":operation_id, "accepted_digest":accepted_digest, "state":state}),
        request_id,
    );
    response.operation = Some(OperationReferenceV1 {
        operation_id: operation_id.into(),
        state: state.into(),
        sequence: 9,
        cancellable: state == "activating",
    });
    response
}

struct DecisionRemovalDispatch {
    current_digest: CanonicalDigest,
    deferred_readbacks: usize,
    submissions: Mutex<Vec<ApplyRequestV1>>,
    previews: Mutex<Vec<String>>,
    readbacks: AtomicUsize,
}

impl ControlDispatch for DecisionRemovalDispatch {
    fn dispatch(&self, request: LocalControlWireRequestV2) -> MachineEnvelopeV2<Value> {
        use std::sync::atomic::Ordering::SeqCst;
        let request_id = Some(request.request_id);
        match request.operation_id.as_str() {
            "ApplyDecisionServiceV3" if request.payload.get("accept_digest").is_none() => {
                let preview: PreviewRequestV1 = serde_json::from_value(request.payload).unwrap();
                self.previews
                    .lock()
                    .unwrap()
                    .push(preview.spec.resource_id.clone().unwrap());
                let expected_revisions = RevisionSetV1 {
                    target: 21,
                    dependencies: Default::default(),
                };
                MachineEnvelopeV2::succeeded(
                    serde_json::json!({
                        "change_digest": preview.spec.canonical_digest(&expected_revisions).unwrap(),
                        "normalized_spec": preview.spec,
                        "expected_revisions": expected_revisions,
                    }),
                    request_id,
                )
            }
            RESOLVE_DECISION_TEST_REVISION_V1 => {
                let previews = self.previews.lock().unwrap();
                assert_eq!(
                    previews.len(),
                    self.submissions.lock().unwrap().len() + 1,
                    "the CAS must be pinned before checking the decision identity"
                );
                let target: DecisionTestRevisionQueryV1 =
                    serde_json::from_value(request.payload).unwrap();
                assert_eq!(previews.last(), Some(&target.id));
                assert_eq!(target.revision, 1);
                MachineEnvelopeV2::succeeded(
                    serde_json::json!({
                        "id":target.id, "revision":target.revision,
                        "expected_configuration_digest":self.current_digest,
                    }),
                    request_id,
                )
            }
            "ApplyDecisionServiceV3" => {
                let apply: ApplyRequestV1 = serde_json::from_value(request.payload).unwrap();
                assert_eq!(apply.expected_revisions.target, 21);
                assert_eq!(
                    apply.accept_digest,
                    apply
                        .spec
                        .canonical_digest(&apply.expected_revisions)
                        .unwrap()
                );
                assert_eq!(apply.spec.desired_state["change"]["service"], Value::Null);
                let (operation_id, state) = match apply.spec.resource_id.as_deref() {
                    Some("decision/extension") => ("operation/failed-artifact", "rolled_back"),
                    Some("decision/other") => ("operation/removed-other", "succeeded"),
                    target => panic!("unexpected removal target {target:?}"),
                };
                let response = accepted_for(operation_id, state, &apply.accept_digest, request_id);
                self.submissions.lock().unwrap().push(apply);
                response
            }
            "FindOperationByIdempotency" => {
                let lookup: OperationIdempotencyLookupV1 =
                    serde_json::from_value(request.payload).unwrap();
                assert_eq!(lookup.principal_kind, PrincipalKind::InteractiveUser);
                assert_eq!(lookup.operation_kind, "ApplyDecisionServiceV3");
                assert!(lookup.include_failure_detail);
                let submissions = self.submissions.lock().unwrap();
                let [original] = submissions.as_slice() else {
                    panic!("readback must precede any second submission");
                };
                assert_eq!(lookup.idempotency_key, original.idempotency_key);
                assert_eq!(lookup.accepted_digest, original.accept_digest);
                if self.readbacks.fetch_add(1, SeqCst) < self.deferred_readbacks {
                    return MachineEnvelopeV2::failed(
                        ErrorV1::new(ErrorCode::DaemonUnavailable),
                        request_id,
                    );
                }
                MachineEnvelopeV2::succeeded(
                    serde_json::json!({
                        "digest_matches": true,
                        "operation": {
                            "operation_id":"operation/failed-artifact", "state":"rolled_back",
                            "accepted_digest":original.accept_digest, "sequence":9, "cancellable":false,
                        }
                    }),
                    request_id,
                )
            }
            _ => panic!("unexpected operation {}", request.operation_id),
        }
    }
}

fn removal_change() -> hiroute_domain::DecisionServiceChangeV3 {
    hiroute_domain::DecisionServiceChangeV3 {
        id: "decision/extension".into(),
        expected_revision: 1,
        service: None,
        expected_source_revisions: Vec::new(),
    }
}

#[tokio::test]
async fn decision_removal_refuses_a_replacement_at_the_same_revision() {
    let dispatch = Arc::new(DecisionRemovalDispatch {
        current_digest: CanonicalDigest::of_bytes(b"replacement service"),
        deferred_readbacks: 0,
        submissions: Mutex::default(),
        previews: Mutex::default(),
        readbacks: AtomicUsize::new(0),
    });
    let mut session = Session::new(
        Client::in_process("replaced-decision", dispatch.clone()),
        Arc::new(ReadOnlyAuthority),
    );
    let failure = session
        .save_decision_service_v3(removal_change(), None, Some(digest()))
        .await
        .unwrap_err();
    assert!(
        matches!(failure, DesktopFailure::Native { code } if code == "DECISION_REMOVAL_TARGET_CHANGED")
    );
    assert!(dispatch.submissions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn decision_removal_reads_back_the_original_rollback_without_resubmitting() {
    let dispatch = Arc::new(DecisionRemovalDispatch {
        current_digest: digest(),
        deferred_readbacks: 1,
        submissions: Mutex::default(),
        previews: Mutex::default(),
        readbacks: AtomicUsize::new(0),
    });
    let mut session = Session::new(
        Client::in_process("interrupted-decision", dispatch.clone()),
        Arc::new(ReadOnlyAuthority),
    );
    let change = removal_change();
    assert!(
        session
            .save_decision_service_v3(change.clone(), None, Some(digest()))
            .await
            .is_err()
    );
    let rolled_back = session
        .save_decision_service_v3(change, None, Some(digest()))
        .await
        .unwrap();
    assert_eq!(rolled_back.state, "rolled_back");
    assert_eq!(rolled_back.operation_id, "operation/failed-artifact");
    assert_eq!(dispatch.submissions.lock().unwrap().len(), 1);
    assert_eq!(dispatch.previews.lock().unwrap().len(), 1);
    assert_eq!(
        dispatch.readbacks.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
}

#[tokio::test]
async fn decision_removal_preserves_unconfirmed_operation_when_switching_targets() {
    let dispatch = Arc::new(DecisionRemovalDispatch {
        current_digest: digest(),
        deferred_readbacks: 2,
        submissions: Mutex::default(),
        previews: Mutex::default(),
        readbacks: AtomicUsize::new(0),
    });
    let mut session = Session::new(
        Client::in_process("switch-decision-removal-target", dispatch.clone()),
        Arc::new(ReadOnlyAuthority),
    );
    session
        .save_decision_service_v3(removal_change(), None, Some(digest()))
        .await
        .unwrap_err();
    let original = session.hint.as_ref().unwrap().clone();

    let mut other = removal_change();
    other.id = "decision/other".into();
    let failure = session
        .save_decision_service_v3(other.clone(), None, Some(digest()))
        .await
        .unwrap_err();
    assert!(matches!(
        failure,
        DesktopFailure::Backend { envelope }
            if envelope.error.as_ref().is_some_and(|error| error.code == ErrorCode::DaemonUnavailable)
    ));
    let pending = session.hint.as_ref().unwrap();
    assert_eq!(pending.idempotency_key, original.idempotency_key);
    assert_eq!(pending.accepted_digest, original.accepted_digest);
    assert_eq!(dispatch.submissions.lock().unwrap().len(), 1);
    assert_eq!(dispatch.previews.lock().unwrap().len(), 1);

    let recovered = session.observe().await.unwrap().unwrap();
    assert_eq!(recovered.operation_id, "operation/failed-artifact");
    assert_eq!(recovered.state, "rolled_back");
    assert_eq!(recovered.accepted_digest, original.accepted_digest);

    let removed = session
        .save_decision_service_v3(other, None, Some(digest()))
        .await
        .unwrap();
    assert_eq!(removed.operation_id, "operation/removed-other");
    assert_eq!(removed.state, "succeeded");
    assert_eq!(dispatch.submissions.lock().unwrap().len(), 2);
    assert!(session.hint.is_none());
}

#[tokio::test]
async fn failed_apply_response_reads_failure_detail_before_returning_a_terminal_view() {
    for state in ["rolled_back", "needs_attention"] {
        let (mut session, lookup) = fixture(state, Reply::Detail);
        let result = session
            .reconcile_apply_response(Ok(accepted(state)))
            .await
            .unwrap();
        let operation = result.operation.unwrap();
        assert_eq!(operation.state, state);
        assert_eq!(
            operation.safe_error_code.as_deref(),
            Some("ADAPTER_FAILURE")
        );
        let detail = operation.failure_detail.unwrap();
        assert_eq!(
            detail.step,
            hiroute_domain::OperationStepKind::ApplyAgentArtifacts
        );
        assert_eq!(
            detail.cause,
            hiroute_domain::OperationFailureCauseV1::PermissionDenied
        );
        assert_eq!(lookup.reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn successful_and_pending_apply_responses_add_no_readback_dependency() {
    for state in ["succeeded", "activating"] {
        let (mut session, lookup) = fixture(state, Reply::Unavailable);
        let result = session
            .reconcile_apply_response(Ok(accepted(state)))
            .await
            .unwrap();
        assert_eq!(result.operation.unwrap().state, state);
        assert_eq!(lookup.reads.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn failed_apply_detail_read_rejects_missing_mismatched_or_stale_results() {
    for reply in [
        Reply::Missing,
        Reply::OtherIdentity,
        Reply::Stale,
        Reply::Unavailable,
    ] {
        let (mut session, lookup) = fixture("rolled_back", reply);
        assert!(
            session
                .reconcile_apply_response(Ok(accepted("rolled_back")))
                .await
                .is_err()
        );
        let hint = session.hint.as_ref().unwrap();
        assert_eq!(
            hint.operation_id.as_deref(),
            Some("operation/failed-artifact")
        );
        assert_eq!(hint.after_sequence, 9);
        // A later read recovers the same operation without another Apply or fresh authority.
        *lookup.reply.lock().unwrap() = Reply::Detail;
        assert!(
            session
                .observe()
                .await
                .unwrap()
                .unwrap()
                .failure_detail
                .is_some()
        );
    }
}
