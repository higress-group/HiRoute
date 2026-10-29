//! Combined production listener path: accepted tool, disconnect relay, HTTP 500,
//! exact native retry, next-input composite evaluation and single-model recovery.
use super::native_retry::send;
use super::pre_attempt_cancel::{
    classified_publication, named, reply, request_body, response_body, wait_requests,
};
use super::*;
use crate::server::composition::FileCredentialResolver;
use crate::server::publication::AliasRequestOwnedRouteV1;
use hiroute_domain::*;

#[test]
fn production_listener_composite_stage_recovers_quality_after_relay_and_retry() {
    run_child_case("composite_stage").unwrap();
}

pub(super) fn run_case(root: &Path) {
    let facts = Facts::default();
    let listener = TestTlsListener::bind("composite-provider.invalid").unwrap();
    write_dial_config(root, &[&listener]).unwrap();
    let peer = listener.try_clone().unwrap();
    let upstream = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..7 {
            let (mut stream, _) = peer.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let request = request_body(&read_request(&mut stream));
            let model = request["model"].clone();
            requests.push(request);
            if index == 1 {
                continue;
            } // No HTTP response, before downstream commit.
            if index == 3 {
                let body = br#"{"error":{"message":"controlled temporary failure"}}"#;
                write!(stream,"HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(body).unwrap();
            } else {
                let output = if matches!(index, 0 | 2) {
                    let call = if index == 0 { "one" } else { "two" };
                    json!({"type":"function_call","id":format!("fc-{call}"),"call_id":call,
                        "name":"lookup","arguments":"{}","status":"completed"})
                } else {
                    json!({"type":"message","role":"assistant","content":[{"type":"output_text",
                        "text":match index {4=>"A complete",5=>"B complete",_=>"C complete"}}]})
                };
                reply(
                    &mut stream,
                    &json!({"id":format!("joint-{index}"),"model":model,"status":"completed",
                    "output":[output],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}),
                );
            }
            let _ = stream.finish();
        }
        requests
    });
    let classifier = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/c", classifier.local_addr().unwrap());
    let decisions = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..3 {
            let (mut stream, _) = classifier.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let request = request_body(&read_request(&mut stream));
            let questions = request["questions"].as_object().unwrap();
            let mut answers = serde_json::Map::new();
            for id in questions.keys() {
                answers.insert(id.clone(),if id=="q0" {
                    json!({"type":"score","probabilities":{"0":if index==2 {0.1} else {1.0},"1":if index==2 {0.9} else {0.0}}})
                } else if id == "q2" {
                    json!({"type":"choice","probabilities":{"applicable":1,"not_established":0}})
                } else { json!({"type":"score","score":1.8,"probabilities":{"0":0,"1":0,"2":1}}) });
            }
            requests.push(request);
            reply(&mut stream, &json!({"answers":answers}));
        }
        requests
    });
    let mut snapshot = classified_publication(&listener, endpoint.clone());
    // The pre-attempt helper deliberately limits each request to one attempt.
    // This scenario exercises the normal two-attempt precommit relay budget.
    snapshot.aliases[0].max_attempts = 2;
    let mut judgment = JudgmentSettingsV1::default();
    let protocol = NativeProtocolV1::default();
    protocol.stock_assessment(&mut judgment);
    if let AliasRequestOwnedRouteV1::Classified {
        native_protocol,
        judgment: saved,
        classifier,
        simple_groups,
        ..
    } = &mut snapshot.aliases[0].routing.as_mut().unwrap().request_owned
    {
        *native_protocol = Some(
            FrozenNativeProtocolV1::freeze(
                &protocol,
                judgment.degree.definition().into(),
                [(SMART_SAVING_SCOPE_ID.into(), judgment.clone())],
            )
            .unwrap(),
        );
        *saved = Some(judgment);
        // The source helper tests isolated groups. Declare the actual Regular
        // then Primary exhaustion path rather than assuming group affinity
        // implicitly authorizes a relay.
        *simple_groups = vec![
            crate::server::publication::AliasGroupIdV1::Economy,
            crate::server::publication::AliasGroupIdV1::Primary,
        ];
        let ComplexityClassifierModeV1::DecisionService { service } = &mut classifier.mode else {
            unreachable!()
        };
        service.connection = DecisionConnectionV1::SystemOne {
            provider: "controlled".into(),
            model: "fixture-c".into(),
            endpoint,
            timeout_ms: 5000,
            auth_header: ClassifierAuthHeaderV1 {
                name: "Authorization".into(),
                value_secret_ref: "pre-attempt-credential".into(),
            },
        };
    } else {
        panic!("classified publication")
    }
    snapshot.payload_digest = snapshot.canonical_digest().unwrap();
    snapshot.validate().unwrap();
    let publications = Arc::new(GatewayPublicationInstaller::open(root.join("lkg.json")).unwrap());
    let crate::server::publication::GatewayPrepareOutcome::Prepared(prepared) =
        publications.prepare(snapshot).unwrap()
    else {
        panic!("prepare")
    };
    publications.publish(prepared).unwrap();
    fs::write(root.join("lease.json"),serde_json::to_vec(&json!({"schema_version":"hiroute.gateway.credential-leases/v1",
        "credential_ref":"pre-attempt-credential","keys":[{"key_id":"test-key","generation":1,"authorization":"Bearer TEST_ONLY"}]})).unwrap()).unwrap();
    fs::write(
        root.join("credentials.json"),
        serde_json::to_vec(&json!({"schema_version":"hiroute.gateway.credentials/v1",
        "credentials":{"pre-attempt-credential":"lease.json"}}))
        .unwrap(),
    )
    .unwrap();
    let mut ports = ProductionPorts::fail_closed(publications);
    ports.credentials =
        Arc::new(FileCredentialResolver::open(&root.join("credentials.json")).unwrap());
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
    let mut input = vec![json!({"role":"user","content":"A"})];
    for (index, call) in ["one", "two"].into_iter().enumerate() {
        let response = send(address, &input, "1");
        if !response.starts_with(b"HTTP/1.1 200") {
            let captured = wait_requests(&facts, index + 1);
            let trace: Vec<_> = captured
                .iter()
                .filter(|event| {
                    matches!(
                        event["fact"]["kind"].as_str(),
                        Some(
                            "route_decision"
                                | "candidate_decision"
                                | "attempt_started"
                                | "attempt_finished"
                                | "request_finished"
                        )
                    )
                })
                .collect();
            panic!(
                "tool {call}: {}; execution trace: {}",
                String::from_utf8_lossy(&response),
                json!(trace)
            );
        }
        assert!(
            response.starts_with(b"HTTP/1.1 200"),
            "tool {call}: {}",
            String::from_utf8_lossy(&response)
        );
        input.extend(
            response_body(&response)["output"]
                .as_array()
                .unwrap()
                .iter()
                .cloned(),
        );
        input.push(
            json!({"type":"function_call_output","call_id":call,"output":"actual lookup result"}),
        );
    }
    let failed = send(address, &input, "1");
    assert!(!failed.starts_with(b"HTTP/1.1 200"));
    let frozen = wait_requests(&facts, 3);
    let before = named(&frozen, "agent_turn_finished")[0]["fact"].clone();
    assert_eq!(before["status"], "failed");
    let retried = send(address, &input, "1");
    assert!(retried.starts_with(b"HTTP/1.1 200"));
    input.extend(
        response_body(&retried)["output"]
            .as_array()
            .unwrap()
            .iter()
            .cloned(),
    );
    for (turn, text) in [("2", "B"), ("3", "C")] {
        input.push(json!({"role":"user","content":text}));
        let response = send(address, &input, turn);
        assert!(response.starts_with(b"HTTP/1.1 200"));
        input.extend(
            response_body(&response)["output"]
                .as_array()
                .unwrap()
                .iter()
                .cloned(),
        );
    }
    let captured = wait_requests(&facts, 6);
    gateway.shutdown();
    gateway.join(Duration::from_secs(5)).unwrap();
    let forwarded = upstream.join().unwrap();
    assert_eq!(forwarded.len(), 7);
    assert_eq!(forwarded[0]["model"], "physical-1");
    assert_eq!(forwarded[1]["model"], "physical-1");
    assert_eq!(forwarded[2]["model"], "physical-2");
    let classified = decisions.join().unwrap();
    assert_eq!(classified.len(), 3);
    assert_eq!(
        classified[1]["state"]["assessment_stage"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        classified[1]["state"]["assessment_stage"]
            .to_string()
            .contains("A complete")
    );
    let stages = named(&captured, "agent_turn_finished");
    assert_eq!(
        stages[0]["fact"], before,
        "failed revision must remain immutable"
    );
    assert_eq!(stages[1]["fact"]["agent_turn_id"], before["agent_turn_id"]);
    assert_eq!(stages[1]["fact"]["attribution"], "mixed");
    assert_eq!(stages[1]["fact"]["history_partial"], false);
    let joint = named(&captured, "stage_assessment_recorded");
    assert_eq!(joint.len(), 1);
    assert_eq!(
        joint[0]["fact"]["history_membership"]["request_ids"],
        stages[1]["fact"]["history_membership"]["request_ids"]
    );
    assert_eq!(joint[0]["fact"]["partial"], false);
    assert_eq!(joint[0]["fact"]["score"], 0.9);
    assert_eq!(
        joint[0]["fact"]["history_membership"]["source_unverified"],
        true
    );
    assert!(joint[0]["fact"].get("model_configuration_id").is_none());
    assert_eq!(
        named(&captured, "branch_assessment_recorded").len(),
        1,
        "next single executor remains independently graded"
    );
    for decision in named(&captured, "route_decision") {
        assert_eq!(decision["fact"]["complexity"]["fallback_used"], false);
    }
    wait_replay_clean(&root.join("replay"));
}
