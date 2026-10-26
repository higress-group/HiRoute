//! Real listener, native DSH metadata, controlled HTTP failure, retry and next input.
use super::pre_attempt_cancel::{
    classified_publication, named, reply, request_body, response_body, wait_requests,
};
use super::*;
use crate::server::composition::FileCredentialResolver;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn production_listener_native_retry_preserves_stage_and_next_input_assessment() {
    run_child_case("native_failed_retry").unwrap();
}

pub(super) fn run_case(root: &Path) {
    let facts = Facts::default();
    let listener = TestTlsListener::bind("native-retry-provider.invalid").unwrap();
    write_dial_config(root, &[&listener]).unwrap();
    let peer = listener.try_clone().unwrap();
    let upstream = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..3 {
            let (mut stream, _) = peer.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            requests.push(request_body(&read_request(&mut stream)));
            if index == 0 {
                let body = br#"{"error":{"message":"temporary upstream failure"}}"#;
                write!(stream, "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(body).unwrap();
            } else {
                reply(
                    &mut stream,
                    &json!({"id":format!("retry-response-{index}"),
                    "model":if index == 1 {"physical-2"} else {"physical-1"},
                    "status":"completed","output":[{"type":"message","role":"assistant",
                    "content":[{"type":"output_text","text":if index == 1 {"actual answer A"} else {"actual answer B"}}]}],
                    "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}),
                );
            }
            let _ = stream.finish();
        }
        requests
    });
    let classifier = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1/decisions", classifier.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let classifications = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..2 {
            let (mut stream, _) = classifier.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            requests.push(request_body(&read_request(&mut stream)));
            count.fetch_add(1, Ordering::SeqCst);
            let decision = if index == 0 {
                json!({"group":"primary"})
            } else {
                json!({"group":"regular","assessment":{"score":0.8,"partial":true}})
            };
            reply(&mut stream, &decision);
        }
        requests
    });
    let publications = Arc::new(GatewayPublicationInstaller::open(root.join("lkg.json")).unwrap());
    let crate::server::publication::GatewayPrepareOutcome::Prepared(prepared) = publications
        .prepare(classified_publication(&listener, endpoint))
        .unwrap()
    else {
        panic!("prepare")
    };
    publications.publish(prepared).unwrap();
    fs::write(root.join("lease.json"), serde_json::to_vec(&json!({
        "schema_version":"hiroute.gateway.credential-leases/v1","credential_ref":"pre-attempt-credential",
        "keys":[{"key_id":"test-key","generation":1,"authorization":"Bearer TEST_ONLY"}]
    })).unwrap()).unwrap();
    fs::write(root.join("credentials.json"), serde_json::to_vec(&json!({
        "schema_version":"hiroute.gateway.credentials/v1","credentials":{"pre-attempt-credential":"lease.json"}
    })).unwrap()).unwrap();
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
    let mut input = vec![json!({"role":"user","content":"actual A"})];
    let failed = send(address, &input, "1");
    assert!(!failed.starts_with(b"HTTP/1.1 200"));
    let initial = wait_requests(&facts, 1);
    let requests = named(&initial, "request_finished");
    assert_eq!(requests[0]["fact"]["outcome"], "failed");
    assert!(requests[0]["fact"]["accepted_attempt_ordinal"].is_null());
    let failed_turns = named(&initial, "agent_turn_finished");
    assert_eq!(failed_turns.len(), 1);
    assert_eq!(failed_turns[0]["fact"]["status"], "failed");
    assert_eq!(failed_turns[0]["fact"]["attribution"], "unknown");
    let retry = send(address, &input, "1");
    assert!(retry.starts_with(b"HTTP/1.1 200"));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the same native retry must not classify again; initial facts: {initial:?}"
    );
    let a = response_body(&retry);
    assert_eq!(a["output"][0]["content"][0]["text"], "actual answer A");
    wait_requests(&facts, 2);
    input.extend([
        a["output"][0].clone(),
        json!({"role":"user","content":"actual B"}),
    ]);
    let b = send(address, &input, "2");
    assert!(b.starts_with(b"HTTP/1.1 200"));
    assert_eq!(
        response_body(&b)["output"][0]["content"][0]["text"],
        "actual answer B"
    );
    let captured = wait_requests(&facts, 3);
    gateway.shutdown();
    gateway.join(Duration::from_secs(5)).unwrap();
    let forwarded = upstream.join().unwrap();
    assert_eq!(
        forwarded
            .iter()
            .map(|r| r["model"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["physical-2", "physical-2", "physical-1"]
    );
    let classified = classifications.join().unwrap();
    assert_eq!(classified.len(), 2);
    assert_eq!(
        classified[1]["visible_conversation"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        classified[1]["visible_conversation"]
            .to_string()
            .contains("actual answer A")
    );
    let finished = named(&captured, "request_finished");
    assert_eq!(finished[0]["fact"]["outcome"], "failed");
    assert_eq!(finished[1]["fact"]["outcome"], "accepted");
    let turns = named(&captured, "agent_turn_finished");
    assert_eq!(turns.len(), 3);
    assert_eq!(
        turns[0]["fact"]["agent_turn_id"],
        turns[1]["fact"]["agent_turn_id"]
    );
    assert_ne!(
        turns[1]["fact"]["agent_turn_id"],
        turns[2]["fact"]["agent_turn_id"]
    );
    let members = json!([
        finished[0]["correlation"]["request_id"],
        finished[1]["correlation"]["request_id"]
    ]);
    assert_eq!(
        turns[1]["fact"]["history_membership"]["request_ids"],
        members
    );
    let assessments = named(&captured, "branch_assessment_recorded");
    assert_eq!(assessments.len(), 1);
    assert_eq!(
        assessments[0]["fact"]["history_membership"]["request_ids"],
        members
    );
    let decisions = named(&captured, "route_decision");
    assert_eq!(
        decisions[1]["fact"]["complexity"]["decision_source"],
        "inherited"
    );
    assert_eq!(
        decisions[2]["fact"]["complexity"]["decision_source"],
        "external_classifier"
    );
    assert_eq!(decisions[2]["fact"]["complexity"]["fallback_used"], false);
    wait_replay_clean(&root.join("replay"));
}

pub(super) fn send(address: std::net::SocketAddr, input: &[Value], turn: &str) -> Vec<u8> {
    let body =
        serde_json::to_vec(&json!({"model":"pre-attempt-alias","input":input,"stream":false}))
            .unwrap();
    let context = json!({"version":2,"agent":"dsh","purpose":"main","session_id":"native-retry-session","turn_id":turn});
    let mut client = TcpStream::connect(address).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(client, "POST /v1/responses HTTP/1.1\r\nHost: {address}\r\nX-HiRoute-Token: {TOKEN}\r\nX-HiRoute-Agent: dsh\r\nX-Deepseek-Harness-Session-ID: native-retry-session\r\nX-HiRoute-Routing-Context: {context}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
    client.write_all(&body).unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).unwrap();
    response
}
