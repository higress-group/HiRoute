//! A real downstream disconnect while preparing a continuation has no provider
//! attempt to cancel. Request termination and retained history remain distinct.
use super::*;
use crate::ports::{CredentialLeaseRequest, ExecutionScope};
use crate::server::composition::{
    CredentialLease, CredentialResolver, FileCredentialResolver, PortError,
};
use crate::server::publication::{
    AliasComplexityClassifierV1, AliasGroupIdV1, AliasModelGroupV1, AliasRequestOwnedRouteV1,
    AliasRoutingV1,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

#[test]
fn production_listener_pre_attempt_disconnect_is_cancelled_and_recovers() {
    run_child_case("pre_attempt_disconnect").unwrap();
}

#[test]
fn production_listener_pre_attempt_authority_failure_remains_failed_and_recovers() {
    run_child_case("pre_attempt_authority_failure").unwrap();
}

struct PreparingCredentials {
    inner: FileCredentialResolver,
    calls: AtomicUsize,
    disconnect: bool,
    entered: mpsc::Sender<()>,
    released: mpsc::Sender<()>,
}

struct PreparationGuard(mpsc::Sender<()>);
impl Drop for PreparationGuard {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

#[async_trait::async_trait]
impl CredentialResolver for PreparingCredentials {
    fn acquire(&self, credential_ref: &str) -> Result<CredentialLease, PortError> {
        self.inner.acquire(credential_ref)
    }

    async fn lease_exact(
        &self,
        request: CredentialLeaseRequest<'_>,
        scope: &ExecutionScope,
    ) -> Result<Option<crate::ports::CredentialLease>, PortError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 1 {
            let _guard = PreparationGuard(self.released.clone());
            self.entered.send(()).unwrap();
            if self.disconnect {
                // No timeout, token, shutdown or synthetic transport error ends
                // this future: the real HTTP disconnect must release it.
                std::future::pending::<()>().await;
                unreachable!();
            }
            return Err(PortError::Unavailable("test preparation authority"));
        }
        self.inner.lease_exact(request, scope).await
    }
}

pub(super) fn run_case(case: &str, root: &Path) {
    let disconnect = match case {
        "pre_attempt_disconnect" => true,
        "pre_attempt_authority_failure" => false,
        _ => panic!("unknown pre-attempt case"),
    };
    let facts = Facts::default();
    let listener = TestTlsListener::bind("pre-attempt-provider.invalid").unwrap();
    write_dial_config(root, &[&listener]).unwrap();
    let tool = json!({"type":"function_call","id":"fc-lookup","call_id":"lookup-call",
        "name":"lookup","arguments":"{}","status":"completed"});
    let answer = |text: &str| {
        json!({"type":"message","role":"assistant",
        "content":[{"type":"output_text","text":text}]})
    };
    let outputs = [tool.clone(), answer("B finished"), answer("C finished")];
    let peer = listener.try_clone().unwrap();
    let upstream = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for (index, output) in outputs.into_iter().enumerate() {
            let (mut stream, _) = peer.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            requests.push(request_body(&read_request(&mut stream)));
            reply(
                &mut stream,
                &json!({"id":format!("response-{index}"),
                "model":if index == 0 {"physical-1"} else {"physical-2"},
                "status":"completed","output":[output],
                "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}),
            );
            let _ = stream.finish();
        }
        requests
    });
    let classifier = TcpListener::bind("127.0.0.1:0").unwrap();
    let classifier_endpoint = format!("http://{}/v1/decisions", classifier.local_addr().unwrap());
    let classifier_requests = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..3 {
            let (mut stream, _) = classifier.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            requests.push(request_body(&read_request(&mut stream)));
            let mut decision = json!({"group":if index == 0 {"regular"} else {"primary"}});
            if index > 0 {
                decision["assessment"] = json!({"score":0.75,"partial":false});
            }
            reply(&mut stream, &decision);
        }
        requests
    });
    let publications = Arc::new(GatewayPublicationInstaller::open(root.join("lkg.json")).unwrap());
    let crate::server::publication::GatewayPrepareOutcome::Prepared(prepared) = publications
        .prepare(classified_publication(&listener, classifier_endpoint))
        .unwrap()
    else {
        panic!("new publication must prepare")
    };
    publications.publish(prepared).unwrap();
    fs::write(root.join("lease.json"), serde_json::to_vec(&json!({
        "schema_version":"hiroute.gateway.credential-leases/v1","credential_ref":"pre-attempt-credential",
        "keys":[{"key_id":"test-key","generation":1,"authorization":"Bearer TEST_ONLY"}]
    })).unwrap()).unwrap();
    fs::write(root.join("credentials.json"), serde_json::to_vec(&json!({
        "schema_version":"hiroute.gateway.credentials/v1","credentials":{"pre-attempt-credential":"lease.json"}
    })).unwrap()).unwrap();
    let (entered_tx, entered) = mpsc::channel();
    let (released_tx, released) = mpsc::channel();
    let credentials = Arc::new(PreparingCredentials {
        inner: FileCredentialResolver::open(&root.join("credentials.json")).unwrap(),
        calls: AtomicUsize::new(0),
        disconnect,
        entered: entered_tx,
        released: released_tx,
    });
    let mut ports = ProductionPorts::fail_closed(publications);
    ports.credentials = credentials.clone();
    ports.runtime_state = Arc::new(crate::ports::InMemoryRuntimeStateStore::default());
    let mut sinks = GatewayObservationSinks::discard();
    sinks.execution_fact = Arc::new(facts.clone());
    let runtime = ProductionGatewayRuntime::compose_with_planner_and_observation(
        ports,
        Arc::new(PublicationPlannerInputAuthority),
        Arc::new(GatewayObservation::with_sinks(512 * 1024, sinks)),
    );
    let address = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let mut gateway = crate::server::GatewayLauncher::from_runtime(address, Arc::new(runtime))
        .unwrap()
        .start_managed()
        .unwrap();

    let mut input = vec![json!({"role":"user","content":"actual A: look up the record"})];
    let first = send(address, &input);
    assert_eq!(first["model"], "pre-attempt-alias");
    assert_eq!(first["output"][0], tool);
    wait_requests(&facts, 1);
    input.extend([first["output"][0].clone(),
        json!({"type":"function_call_output","call_id":"lookup-call","output":"actual lookup result"})]);
    let mut continuation = open(address, &input);
    entered
        .recv_timeout(Duration::from_secs(5))
        .expect("continuation reached pre-dispatch preparation");
    if disconnect {
        // A real client close, after the authority gate has been entered. This
        // happens before any gateway shutdown or test cleanup.
        continuation.shutdown(std::net::Shutdown::Both).unwrap();
        drop(continuation);
    } else {
        let mut response = Vec::new();
        continuation.read_to_end(&mut response).unwrap();
        assert!(!response.starts_with(b"HTTP/1.1 200"));
    }
    released
        .recv_timeout(Duration::from_secs(5))
        .expect("preparation future was released");
    let after_stop = wait_requests(&facts, 2);
    assert_stopped_request(&after_stop, disconnect);

    input.push(json!({"role":"user","content":"actual B: design the independent next step"}));
    let second = send(address, &input);
    wait_requests(&facts, 3);
    input.extend(second["output"].as_array().unwrap().iter().cloned());
    input.push(json!({"role":"user","content":"actual C: check the next design"}));
    send(address, &input);
    let final_facts = wait_requests(&facts, 4);
    gateway.shutdown();
    gateway.join(Duration::from_secs(5)).unwrap();
    let forwarded = upstream.join().unwrap();
    let classified = classifier_requests.join().unwrap();
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 4);
    assert_eq!(
        forwarded.len(),
        3,
        "the stopped request never reached a provider"
    );
    assert_eq!(forwarded[0]["model"], "physical-1");
    assert_eq!(forwarded[1]["model"], "physical-2");
    assert_eq!(forwarded[2]["model"], "physical-2");
    assert_eq!(
        classified.len(),
        3,
        "tool continuation inherits; fresh inputs classify"
    );
    assert_eq!(
        classified[1]["visible_conversation"][0]["status"],
        if disconnect { "interrupted" } else { "failed" }
    );
    // A2 produced no accepted output edge. B therefore opens an independent
    // branch and discloses a contextual history gap, while its assessment can
    // still address every actual member of the already frozen A stage.
    assert_eq!(classified[1]["history_partial"], true);
    assert_eq!(classified[1]["preceding_stage"]["from"], 0);
    assert_eq!(
        classified[1]["visible_conversation"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let disclosures = classified[1]["latest_user"].as_array().unwrap();
    for source in [
        "prior_execution_history_missing",
        "assessment_target_association_unverified",
    ] {
        assert!(disclosures.contains(&json!({"kind":"unavailable","source_kind":source})));
    }
    assert!(
        !disclosures
            .iter()
            .any(|part| part["source_kind"] == "assessment_target_capture_gap")
    );
    assert_recovery(&final_facts);
    wait_replay_clean(&root.join("replay"));
}

fn assert_stopped_request(facts: &[Value], disconnect: bool) {
    let requests = named(facts, "request_finished");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["fact"]["outcome"], "accepted");
    assert_eq!(
        requests[1]["fact"]["outcome"],
        if disconnect { "cancelled" } else { "failed" }
    );
    assert_eq!(requests[1]["fact"]["attempts_started"], 0);
    assert_eq!(requests[1]["fact"]["attempts_finished"], 0);
    assert!(requests[1]["fact"]["accepted_attempt_ordinal"].is_null());
    assert_eq!(named(facts, "attempt_started").len(), 1);
    assert_eq!(named(facts, "attempt_finished").len(), 1);
    let stages = named(facts, "agent_turn_finished");
    assert_eq!(stages.len(), 1);
    let stage = &stages[0]["fact"];
    assert_eq!(
        stage["status"],
        if disconnect { "interrupted" } else { "failed" }
    );
    assert_eq!(
        stage["history_membership"]["request_ids"],
        json!([
            requests[0]["correlation"]["request_id"],
            requests[1]["correlation"]["request_id"]
        ])
    );
    assert_eq!(stage["history_membership"]["capture_partial"], false);
    assert_eq!(stage["history_partial"], false);
    assert_eq!(stage["attribution"], "single");
    assert_eq!(
        named(facts, "route_decision")[1]["fact"]["complexity"]["decision_source"],
        "inherited"
    );
}

fn assert_recovery(facts: &[Value]) {
    let requests = named(facts, "request_finished");
    let stages = named(facts, "agent_turn_finished");
    assert_eq!(stages.len(), 3);
    for (stage, request) in stages[1..].iter().zip(&requests[2..]) {
        assert_eq!(stage["fact"]["status"], "completed");
        assert_eq!(stage["fact"]["opening_cause"], "user_message");
        assert_eq!(
            stage["fact"]["history_membership"]["request_ids"],
            json!([request["correlation"]["request_id"]])
        );
        assert_eq!(
            stage["fact"]["history_membership"]["capture_partial"],
            false
        );
    }
    let decisions = named(facts, "route_decision");
    for decision in &decisions[2..] {
        assert_eq!(
            decision["fact"]["complexity"]["decision_source"],
            "external_classifier"
        );
        assert_eq!(decision["fact"]["complexity"]["fallback_used"], false);
    }
    let assessments = named(facts, "branch_assessment_recorded");
    assert_eq!(assessments.len(), 2);
    for (index, (assessment, stage)) in assessments.iter().zip(&stages).enumerate() {
        let mut expected_membership = stage["fact"]["history_membership"].clone();
        if index == 0 {
            // Fork projection discloses the uncertain A -> B association. It
            // must not change A's frozen branch, revision, members or coverage.
            expected_membership["relation"] = json!("contextual");
            expected_membership["association_unverified"] = json!(true);
        }
        assert_eq!(
            assessment["fact"]["history_membership"],
            expected_membership
        );
        assert_eq!(
            assessment["fact"]["target_from_turn_id"],
            stage["fact"]["agent_turn_id"]
        );
        assert_eq!(
            assessment["fact"]["target_through_turn_id"],
            stage["fact"]["agent_turn_id"]
        );
        assert_eq!(
            assessment["fact"]["model_configuration_id"],
            stage["fact"]["model_configuration_id"]
        );
        assert_eq!(assessment["fact"]["partial"], false);
    }
}

pub(super) fn named<'a>(facts: &'a [Value], kind: &str) -> Vec<&'a Value> {
    facts
        .iter()
        .filter(|event| event["fact"]["kind"] == kind)
        .collect()
}

pub(super) fn wait_requests(facts: &Facts, count: usize) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let captured = facts.0.lock().unwrap().clone();
        if named(&captured, "request_finished").len() == count {
            return captured;
        }
        assert!(
            Instant::now() < deadline,
            "request did not finish before cleanup"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn open(address: std::net::SocketAddr, input: &[Value]) -> TcpStream {
    let body =
        serde_json::to_vec(&json!({"model":"pre-attempt-alias","input":input,"stream":false}))
            .unwrap();
    let mut client = TcpStream::connect(address).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(client, "POST /v1/responses HTTP/1.1\r\nHost: {address}\r\nX-HiRoute-Token: {TOKEN}\r\nsession-id: pre-attempt-session\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
    client.write_all(&body).unwrap();
    client
}

fn send(address: std::net::SocketAddr, input: &[Value]) -> Value {
    let mut client = open(address, input);
    let mut response = Vec::new();
    client.read_to_end(&mut response).unwrap();
    assert!(
        response.starts_with(b"HTTP/1.1 200"),
        "{}",
        String::from_utf8_lossy(&response)
    );
    response_body(&response)
}

pub(super) fn request_body(raw: &[u8]) -> Value {
    let start = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    serde_json::from_slice(&raw[start..]).unwrap()
}

pub(super) fn response_body(raw: &[u8]) -> Value {
    let start = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let head = std::str::from_utf8(&raw[..start]).unwrap();
    let mut remaining = &raw[start..];
    if head
        .lines()
        .any(|line| line.eq_ignore_ascii_case("transfer-encoding: chunked"))
    {
        let mut decoded = Vec::new();
        loop {
            let end = remaining.windows(2).position(|w| w == b"\r\n").unwrap();
            let size =
                usize::from_str_radix(std::str::from_utf8(&remaining[..end]).unwrap(), 16).unwrap();
            remaining = &remaining[end + 2..];
            if size == 0 {
                assert_eq!(
                    remaining, b"\r\n",
                    "response entity must finish exactly once"
                );
                return serde_json::from_slice(&decoded).unwrap();
            }
            assert_eq!(&remaining[size..size + 2], b"\r\n");
            decoded.extend_from_slice(&remaining[..size]);
            remaining = &remaining[size + 2..];
        }
    }
    let length: usize = head
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .unwrap()
        .1
        .trim()
        .parse()
        .unwrap();
    assert_eq!(
        remaining.len(),
        length,
        "response entity must satisfy its length"
    );
    serde_json::from_slice(remaining).unwrap()
}

#[test]
fn response_reader_requires_complete_entity_before_accepting_valid_json() {
    let complete = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n";
    assert_eq!(response_body(complete), json!({}));
    for wire in [
        &complete[..complete.len() - 2],
        &complete[..complete.len() - 5],
        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n{}",
    ] {
        assert!(std::panic::catch_unwind(|| response_body(wire)).is_err());
    }
}

pub(super) fn reply(stream: &mut impl Write, body: &Value) {
    let body = serde_json::to_vec(body).unwrap();
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
    stream.write_all(&body).unwrap();
    stream.flush().unwrap();
}

pub(super) fn classified_publication(
    provider: &TestTlsListener,
    endpoint: String,
) -> GatewayPublicationSnapshotV3 {
    let protocol = IngressProtocol::Responses;
    let mut snapshot = publication(provider, true, false);
    let alias = &mut snapshot.aliases[0];
    alias.served_model_id = "pre-attempt-alias".into();
    alias.protocols = vec![protocol];
    alias.candidates = (1..=2)
        .map(|i| {
            sealed_native_candidate(
                i,
                &format!("pre-attempt-target-{i}"),
                &["pre-attempt-credential".into()],
                provider.authority(),
                &format!("physical-{i}"),
                &[(protocol, protocol)],
            )
        })
        .collect();
    alias.routing = Some(AliasRoutingV1 {
        agent_plan_id: "pre-attempt-plan".into(),
        plan_display_name: None,
        request_owned: AliasRequestOwnedRouteV1::Classified {
            native_protocol: None,
            judgment: Default::default(),
            reselect_on_user_message: true,
            classifier: AliasComplexityClassifierV1 {
                revision: "pre-attempt-decision/v1".into(),
                user_keywords: Vec::new(),
                mode: hiroute_domain::ComplexityClassifierModeV1::DecisionService {
                    service: Box::new(hiroute_domain::DecisionServiceV1 {
                        id: "pre-attempt-decision".into(),
                        revision: 1,
                        name: "Test decisions".into(),
                        connection: hiroute_domain::DecisionConnectionV1::Custom {
                            endpoint,
                            timeout_ms: 5_000,
                            auth_header: None,
                        },
                    }),
                },
            },
            simple_groups: vec![AliasGroupIdV1::Economy],
            complex_groups: vec![AliasGroupIdV1::Primary],
        },
        groups: vec![
            AliasModelGroupV1 {
                group_id: AliasGroupIdV1::Economy,
                candidate_local_ids: vec![1],
            },
            AliasModelGroupV1 {
                group_id: AliasGroupIdV1::Primary,
                candidate_local_ids: vec![2],
            },
        ],
    });
    let grant = &mut snapshot.grants[0];
    grant.protocol = protocol;
    grant.routes = [(
        "pre-attempt-alias".into(),
        ModelRouteV2::Plan {
            plan_id: "pre-attempt-plan".into(),
            alias: "pre-attempt-alias".into(),
            revision: 1,
            semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(b"pre-attempt-plan"),
        },
    )]
    .into_iter()
    .collect();
    snapshot.payload_digest = snapshot.canonical_digest().unwrap();
    snapshot.validate().unwrap();
    snapshot
}
