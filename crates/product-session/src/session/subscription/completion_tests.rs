//! Component regressions over the production completion entry and real Client Core transport.
//! The scoped socket server models an A record arriving after the close-time sweep.
use super::*;
use hiroute_client_core::{Client, LocalEndpoint};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use zeroize::Zeroizing;

const RELEASE_GRANT: &str = "completion-release-grant";

struct CompletionAuthority(Arc<AtomicUsize>);
impl HostAuthority for CompletionAuthority {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn has_authority(&self) -> bool {
        true
    }
    fn register_operation(
        &self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
        operation: &'static str,
    ) -> Result<Zeroizing<String>, String> {
        let validation: ComputeValidationRefV2 =
            serde_json::from_value(result(false)["validation"].clone()).unwrap();
        assert_eq!(operation, RELEASE_SUBSCRIPTION_CHECK_OPERATION_V2);
        assert_eq!(digest, &CanonicalDigest::of(&validation).unwrap());
        assert_eq!(revisions.target, 1);
        assert!(revisions.dependencies.is_empty());
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Zeroizing::new(RELEASE_GRANT.into()))
    }
    fn register_input(
        &self,
        _: &ComputeCandidateRefV2,
        _: Zeroizing<String>,
    ) -> Result<(), String> {
        panic!("completion must not register a new protected input")
    }
    fn release_model_input(&self, _: &ComputeCandidateRefV2) -> Result<(), String> {
        panic!("completion must not release an unrelated protected input")
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    client: Client,
    calls: Arc<AtomicUsize>,
    releases: Arc<AtomicUsize>,
    registrations: Arc<AtomicUsize>,
    created: Arc<AtomicBool>,
    server: tokio::task::JoinHandle<()>,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("hrsc-")
            .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
            .unwrap();
        let runtime = root.path().join("run");
        let directory = runtime.join("hiroute");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let endpoint = LocalEndpoint::for_child(&runtime, std::process::id());
        let listener = UnixListener::bind(endpoint.path()).unwrap();
        std::fs::set_permissions(endpoint.path(), std::fs::Permissions::from_mode(0o600)).unwrap();
        let client = Client::new("desktop-test", endpoint);
        let calls = Arc::new(AtomicUsize::new(0));
        let releases = Arc::new(AtomicUsize::new(0));
        let created = Arc::new(AtomicBool::new(false));
        let (server_calls, server_releases, server_created) =
            (calls.clone(), releases.clone(), created.clone());
        let server = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let mut stream = BufReader::new(stream);
                let mut line = String::new();
                stream.read_line(&mut line).await.unwrap();
                let hello: ClientHelloV1 = serde_json::from_str(&line).unwrap();
                write(
                    &mut stream,
                    serde_json::to_value(negotiate_hello(&hello).unwrap()).unwrap(),
                )
                .await;
                line.clear();
                stream.read_line(&mut line).await.unwrap();
                let request: LocalControlWireRequestV2 = serde_json::from_str(&line).unwrap();
                server_calls.fetch_add(1, Ordering::SeqCst);
                let data = match request.operation_id.as_str() {
                    "FindOperationByIdempotency" => json!({
                        "digest_matches": true,
                        "operation": if server_created.load(Ordering::SeqCst) { Some(json!({
                            "operation_id": "operation-a", "state": "succeeded", "sequence": 1,
                            "cancellable": false, "accepted_digest": CanonicalDigest::of_bytes(b"accepted"),
                            "safe_error_code": null
                        })) } else { None }
                    }),
                    LIST_COMPUTE_SUBSCRIPTIONS_OPERATION_V2 => json!({
                        "candidates": [], "discovery_state": "complete"
                    }),
                    GET_SUBSCRIPTION_CHECK_RESULT_OPERATION_V2 => result(false),
                    "GetClientServiceStatus" => json!({
                        "schema": "hiroute.client-service-status/v1", "daemon_role": "all",
                        "recovery_ready": true, "mutation_available": true, "gateway": "ready",
                        "active_publication": null, "revisions": { "target": 1, "dependencies": {} }
                    }),
                    RELEASE_SUBSCRIPTION_CHECK_OPERATION_V2 => {
                        assert_eq!(request.payload["validation_ref"], "validation-a");
                        let grant = request.protected_grant.as_ref().unwrap();
                        assert_eq!(grant.principal_kind, PrincipalKind::Desktop);
                        assert_eq!(grant.capability, RELEASE_GRANT);
                        server_releases.fetch_add(1, Ordering::SeqCst);
                        result(true)
                    }
                    operation => panic!("unexpected completion operation: {operation}"),
                };
                write(
                    &mut stream,
                    json!({
                        "schema_version": LOCAL_CONTROL_SCHEMA_V2, "request_id": request.request_id,
                        "status": "succeeded", "data": data, "operation": null,
                        "warnings": [], "next_actions": [], "error": null
                    }),
                )
                .await;
            }
        });
        Self {
            _root: root,
            client,
            calls,
            releases,
            registrations: Arc::new(AtomicUsize::new(0)),
            created,
            server,
        }
    }
    fn session(&self) -> Session {
        Session::new(
            self.client.clone(),
            Arc::new(CompletionAuthority(self.registrations.clone())),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn write(stream: &mut BufReader<tokio::net::UnixStream>, value: Value) {
    let mut bytes = serde_json::to_vec(&value).unwrap();
    bytes.push(b'\n');
    stream.get_mut().write_all(&bytes).await.unwrap();
}
fn operation() -> OperationReferenceV1 {
    OperationReferenceV1 {
        operation_id: "operation-a".into(),
        state: "succeeded".into(),
        sequence: 1,
        cancellable: false,
    }
}
fn result(released: bool) -> Value {
    json!({
        "candidate": {"candidate_ref": "candidate/one", "candidate_revision": 1},
        "approval_operation": operation(), "status": if released { "released" } else { "source_changed" },
        "validation": if released { None } else { Some(json!({
            "approval_operation": operation(), "validation_ref": "validation-a", "validation_revision": 1
        })) }
    })
}
fn submitted(session: &mut Session) -> SubscriptionCheckCompletion {
    let digest = CanonicalDigest::of_bytes(b"accepted");
    session.subscription_hint = Some(SubmittedOperation {
        plan_id: "candidate/one".into(),
        principal_kind: PrincipalKind::Desktop,
        operation_kind: APPLY_SUBSCRIPTION_CHECK_OPERATION_V2.into(),
        idempotency_key: "a".repeat(64),
        accepted_digest: digest.clone(),
        operation_id: None,
        after_sequence: 0,
        intent: None,
        latest_edit_not_applied: false,
    });
    SubscriptionCheckCompletion {
        owner: session.confirmation.owner(),
        idempotency_key: "a".repeat(64),
        accepted_digest: digest.clone(),
        close_requested: session.subscription_close_requested.clone(),
        outcome: SubscriptionCheckSubmission::Submitted(Box::new(Ok(MachineEnvelopeV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: Some("request-a".into()),
            status: MachineStatus::Succeeded,
            data: Some(ApplyResultV1 {
                operation_id: "operation-a".into(),
                accepted_digest: digest,
                state: "succeeded".into(),
            }),
            operation: Some(operation()),
            warnings: vec![],
            next_actions: vec![],
            error: None,
        }))),
    }
}

#[tokio::test]
async fn close_before_operation_record_releases_late_completed_validation() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let completion = submitted(&mut session);
    // The submitted request is in flight, but the close-time query cannot find A yet.
    session.invalidate_confirmation();
    session.close_subscription_check().await.unwrap();
    assert_eq!(fixture.releases.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.registrations.load(Ordering::SeqCst), 0);
    assert!(session.subscription_hint.is_some());
    fixture.created.store(true, Ordering::SeqCst);
    let sessions = tokio::sync::Mutex::new(Some(session));
    complete_subscription_check(&sessions, completion)
        .await
        .unwrap();
    assert_eq!(fixture.releases.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.registrations.load(Ordering::SeqCst), 1);
    assert!(
        sessions
            .lock()
            .await
            .as_ref()
            .unwrap()
            .subscription_hint
            .is_none()
    );
}

#[tokio::test]
async fn replacement_session_rejects_completion_before_queries_or_hint_changes() {
    let fixture = Fixture::new();
    let mut old = fixture.session();
    let completion = submitted(&mut old);
    drop(old); // The host lock also belongs to exactly one scoped fixture Session.
    let mut replacement = fixture.session();
    let _ = submitted(&mut replacement);
    replacement
        .subscription_hint
        .as_mut()
        .unwrap()
        .idempotency_key = "new-hint".into();
    let sessions = tokio::sync::Mutex::new(Some(replacement));
    let failure = complete_subscription_check(&sessions, completion)
        .await
        .unwrap_err();
    assert!(matches!(failure, DesktopFailure::Native { code } if code == "CONFIRMATION_STALE"));
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.releases.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.registrations.load(Ordering::SeqCst), 0);
    assert_eq!(
        sessions
            .lock()
            .await
            .as_ref()
            .unwrap()
            .subscription_hint
            .as_ref()
            .unwrap()
            .idempotency_key,
        "new-hint"
    );
}
