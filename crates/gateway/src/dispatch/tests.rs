use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::server::composition::{
    ConversationContentSink, CredentialLease, CredentialResolver, ExecutionFactSink,
    LifecycleTelemetrySink, PortError, ProductionPorts, RuntimeStateStore,
};
use crate::server::core_runtime::ProductionGatewayRuntime;
use crate::server::dispatch::GatewayRequestAuthority;
use crate::server::publication::{
    AliasPlanV1, GatewayPrepareOutcome, GatewayPublicationInstaller, GatewayPublicationSnapshotV3,
    GrantV1, MAX_LOGICAL_REQUEST_DURATION_MS, exact_test_candidate, token_sha256,
};
use crate::server::request_plan::IngressProtocol;

static DIRECTORY_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

#[test]
fn model_selection_does_not_require_model_in_first_sixteen_kib() {
    let (runtime, _, _directory) = runtime("127.0.0.1:45678".parse().unwrap());
    let body = format!(
        "{{\"input\":\"{}\",\"model\":\"fast\"}}",
        "x".repeat(128 * 1024)
    );
    assert!(
        runtime
            .authorize_bytes(
                IngressProtocol::Responses,
                Some("Bearer token"),
                body.as_bytes(),
                Instant::now()
            )
            .is_ok()
    );
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "hiroute-dispatch-test-{}-{}",
            std::process::id(),
            DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct CredentialProbe(AtomicUsize);

impl CredentialResolver for CredentialProbe {
    fn acquire(&self, credential_ref: &str) -> Result<CredentialLease, PortError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(CredentialLease {
            credential_ref: Arc::from(credential_ref),
            generation: 1,
        })
    }
}

struct MarkerPorts;
impl RuntimeStateStore for MarkerPorts {}
impl LifecycleTelemetrySink for MarkerPorts {}
impl ExecutionFactSink for MarkerPorts {}
impl ConversationContentSink for MarkerPorts {}

fn runtime(
    provider_address: SocketAddr,
) -> (
    ProductionGatewayRuntime,
    Arc<CredentialProbe>,
    TestDirectory,
) {
    let directory = TestDirectory::new();
    let publications = Arc::new(
        GatewayPublicationInstaller::open(directory.path().join("publication.json")).unwrap(),
    );
    let candidate =
        |local_id, protocol| exact_test_candidate(local_id, protocol, Some(provider_address));
    let snapshot = GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "authority",
        1,
        1,
        "renderer",
        vec![
            AliasPlanV1 {
                served_model_id: "fast".into(),
                purpose: "fast".into(),
                agent_plan_revision: 11,
                protocols: vec![IngressProtocol::Responses],
                overall_timeout_ms: 900,
                max_attempts: 1,
                routing: None,
                candidates: vec![candidate(1, IngressProtocol::Responses)],
            },
            AliasPlanV1 {
                served_model_id: "private".into(),
                purpose: "private".into(),
                agent_plan_revision: 12,
                protocols: vec![IngressProtocol::Messages],
                overall_timeout_ms: 9_000,
                max_attempts: 3,
                routing: None,
                candidates: vec![candidate(2, IngressProtocol::Messages)],
            },
        ],
        vec![GrantV1 {
            grant_id: "grant".into(),
            generation: 1,
            bearer_token_sha256: token_sha256("token"),
            protocol: IngressProtocol::Responses,
            routes: [crate::server::publication::test_plan_route("fast", 11)].into(),
        }],
    )
    .unwrap();
    let GatewayPrepareOutcome::Prepared(prepared) = publications.prepare(snapshot).unwrap() else {
        panic!("unexpected duplicate")
    };
    publications.publish(prepared).unwrap();
    let credentials = Arc::new(CredentialProbe(AtomicUsize::new(0)));
    let marker = Arc::new(MarkerPorts);
    let ports = ProductionPorts {
        publications,
        credentials: credentials.clone(),
        runtime_state: marker.clone(),
        lifecycle: marker.clone(),
        execution_facts: marker.clone(),
        conversation_content: marker,
    };
    (
        ProductionGatewayRuntime::compose(ports),
        credentials,
        directory,
    )
}

#[test]
fn unknown_unauthorized_and_unsupported_have_zero_credential_dns_or_connect_side_effects() {
    let provider = TcpListener::bind((IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).unwrap();
    provider.set_nonblocking(true).unwrap();
    let (runtime, credentials, _directory) = runtime(provider.local_addr().unwrap());
    let now = Instant::now();

    let missing_grant = runtime.authorize_bytes(
        IngressProtocol::Responses,
        Some("Bearer wrong"),
        &vec![b'x'; 32 * 1024],
        now,
    );
    assert_eq!(
        missing_grant.unwrap_err().code(),
        "GATEWAY_GRANT_UNAUTHORIZED"
    );
    let non_bearer = runtime.authorize_bytes(
        IngressProtocol::Responses,
        Some("token"),
        br#"{"model":"fast"}"#,
        now,
    );
    assert_eq!(non_bearer.unwrap_err().code(), "GATEWAY_GRANT_UNAUTHORIZED");
    let unknown = runtime.authorize_bytes(
        IngressProtocol::Responses,
        Some("Bearer token"),
        br#"{"model":"unknown","input":"not decoded"}"#,
        now,
    );
    assert_eq!(unknown.unwrap_err().code(), "AGENT_MODEL_NOT_GRANTED");
    let unauthorized = runtime.authorize_bytes(
        IngressProtocol::Messages,
        Some("Bearer token"),
        br#"{"model":"private"}"#,
        now,
    );
    assert_eq!(
        unauthorized.unwrap_err().code(),
        "AGENT_PROTOCOL_UNSUPPORTED"
    );
    let unsupported = runtime.authorize_bytes(
        IngressProtocol::Messages,
        Some("Bearer token"),
        br#"{"model":"fast"}"#,
        now,
    );
    let unsupported = unsupported.unwrap_err();
    assert_eq!(unsupported.code(), "AGENT_PROTOCOL_UNSUPPORTED");
    assert_eq!(unsupported.status(), http::StatusCode::UNPROCESSABLE_ENTITY);

    assert_eq!(credentials.0.load(Ordering::Relaxed), 0);
    assert_eq!(
        provider.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn selector_time_is_charged_to_the_same_alias_deadline() {
    let (runtime, _credentials, _directory) =
        runtime(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9));
    let request_started_at = Instant::now();
    let authenticated = runtime
        .authority()
        .begin(IngressProtocol::Responses, Some("Bearer token"))
        .unwrap();
    std::thread::sleep(Duration::from_millis(30));
    let authorized = authenticated
        .authorize_alias("fast", Instant::now())
        .unwrap();

    assert!(
        authorized.deadline() <= request_started_at + Duration::from_millis(905),
        "model selection must not reset the logical-request timeout"
    );
}

#[test]
fn selector_budget_is_independent_of_extreme_agent_plan_budgets() {
    fn authority(lkg: &Path, include_long: bool) -> GatewayRequestAuthority {
        let installer = Arc::new(GatewayPublicationInstaller::open(lkg).unwrap());
        let mut aliases = vec![AliasPlanV1 {
            served_model_id: "short".into(),
            purpose: "one millisecond plan".into(),
            agent_plan_revision: 31,
            protocols: vec![IngressProtocol::Responses],
            overall_timeout_ms: 1,
            max_attempts: 1,
            routing: None,
            candidates: vec![exact_test_candidate(
                31,
                IngressProtocol::Responses,
                Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9)),
            )],
        }];
        if include_long {
            aliases.push(AliasPlanV1 {
                served_model_id: "long".into(),
                purpose: "maximum duration plan".into(),
                agent_plan_revision: 32,
                protocols: vec![IngressProtocol::Responses],
                overall_timeout_ms: MAX_LOGICAL_REQUEST_DURATION_MS,
                max_attempts: 1,
                routing: None,
                candidates: vec![exact_test_candidate(
                    32,
                    IngressProtocol::Responses,
                    Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9)),
                )],
            });
        }
        let routes = aliases
            .iter()
            .map(|alias| {
                crate::server::publication::test_plan_route(
                    &alias.served_model_id,
                    alias.agent_plan_revision,
                )
            })
            .collect();
        let snapshot = GatewayPublicationSnapshotV3::seal(
            "personal/default",
            "budget-authority",
            1,
            1,
            "renderer",
            aliases,
            vec![GrantV1 {
                grant_id: "budget-grant".into(),
                generation: 1,
                bearer_token_sha256: token_sha256("budget-token"),
                protocol: IngressProtocol::Responses,
                routes,
            }],
        )
        .unwrap();
        let GatewayPrepareOutcome::Prepared(prepared) = installer.prepare(snapshot).unwrap() else {
            panic!("unexpected duplicate")
        };
        installer.publish(prepared).unwrap();
        GatewayRequestAuthority::new(installer)
    }

    let directory = TestDirectory::new();
    let short_only = authority(&directory.path().join("short.json"), false);
    let with_long = authority(&directory.path().join("long.json"), true);
    let request_started_at = Instant::now();
    let short_only_request = short_only
        .begin_at(
            IngressProtocol::Responses,
            Some("Bearer budget-token"),
            request_started_at,
        )
        .unwrap();
    let with_long_request = with_long
        .begin_at(
            IngressProtocol::Responses,
            Some("Bearer budget-token"),
            request_started_at,
        )
        .unwrap();

    assert_eq!(
        with_long_request.selector_deadline(),
        short_only_request.selector_deadline(),
        "adding a maximum-duration plan must not extend incomplete selector retention"
    );
    assert!(
        with_long_request.selector_deadline() <= request_started_at + Duration::from_secs(1),
        "selector retention must use its own bounded ingress horizon"
    );
    assert!(matches!(
        with_long_request.authorize_alias("short", request_started_at + Duration::from_millis(1)),
        Err(super::DispatchError::RequestDeadlineExceeded)
    ));
    assert!(matches!(
        with_long
            .begin_at(
                IngressProtocol::Responses,
                Some("Bearer budget-token"),
                request_started_at,
            )
            .unwrap()
            .authorize_alias("long", request_started_at + Duration::from_secs(1)),
        Err(super::DispatchError::RequestDeadlineExceeded)
    ));
    let selected_short = with_long
        .begin_at(
            IngressProtocol::Responses,
            Some("Bearer budget-token"),
            request_started_at,
        )
        .unwrap()
        .authorize_alias("short", request_started_at)
        .unwrap();
    assert_eq!(
        selected_short.deadline(),
        request_started_at + Duration::from_millis(1),
        "selection must not reset the selected AgentPlan deadline"
    );
    let selected_long = with_long
        .begin_at(
            IngressProtocol::Responses,
            Some("Bearer budget-token"),
            request_started_at,
        )
        .unwrap()
        .authorize_alias("long", request_started_at)
        .unwrap();
    assert_eq!(
        selected_long.deadline(),
        request_started_at + Duration::from_millis(MAX_LOGICAL_REQUEST_DURATION_MS),
        "the short sibling must not shrink the selected long AgentPlan deadline"
    );
}

#[test]
fn logical_and_accepted_plans_are_owned_by_the_authorized_request() {
    let (runtime, _credentials, _directory) =
        runtime(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9));
    let authorized = runtime
        .authorize_bytes(
            IngressProtocol::Responses,
            Some("Bearer token"),
            br#"{"model":"fast"}"#,
            Instant::now(),
        )
        .unwrap();
    let logical = authorized.logical_request_plan();
    let accepted = authorized.accepted_response_plan();
    let mut core = authorized.into_core_binding();
    let logical_binding = core.take_logical_request().unwrap();
    assert!(std::ptr::eq(logical.as_ref(), logical_binding.plan()));
    let accepted_binding = core.take_accepted_response().unwrap();
    assert!(std::ptr::eq(accepted.as_ref(), accepted_binding.plan()));
}
