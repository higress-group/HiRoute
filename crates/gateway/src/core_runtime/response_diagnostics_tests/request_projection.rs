//! Request loss is reported only after the actual materialized attempt has an owner.
use super::*;

#[test]
fn production_request_projection_warns_for_credential_relay() {
    run_child_case("request_projection_credential_relay").unwrap();
}

#[test]
fn production_request_projection_warns_for_no_credential_relay() {
    run_child_case("request_projection_none_relay").unwrap();
}

#[test]
fn production_request_projection_keeps_credential_then_none_attempts_distinct() {
    run_child_case("request_projection_credential_none_relay").unwrap();
}

#[test]
fn production_request_projection_keeps_none_then_credential_attempts_distinct() {
    run_child_case("request_projection_none_credential_relay").unwrap();
}

#[test]
fn production_request_projection_without_loss_emits_no_warning() {
    for case in [
        "request_projection_zero_none",
        "request_projection_zero_credential",
    ] {
        run_child_case(case).unwrap();
    }
}

#[test]
fn production_request_projection_does_not_require_diagnostics() {
    run_child_case("request_projection_diagnostics_disabled").unwrap();
}

pub(super) fn run_case(case: &str, root: &Path) {
    let no_credentials = match case {
        "request_projection_credential_relay" | "request_projection_zero_credential" => {
            [false, false]
        }
        "request_projection_credential_none_relay" => [false, true],
        "request_projection_none_credential_relay" => [true, false],
        "request_projection_none_relay"
        | "request_projection_zero_none"
        | "request_projection_diagnostics_disabled" => [true, true],
        _ => panic!("unknown request projection case"),
    };
    let relay = case.ends_with("_relay");
    let loss = !case.contains("zero");
    let attempts = if relay { 2 } else { 1 };
    let diagnostics_enabled = case != "request_projection_diagnostics_disabled";
    let facts = Facts::default();
    let listener = TestTlsListener::bind("projection-diagnostic.invalid").unwrap();
    write_dial_config(root, &[&listener]).unwrap();
    let peer = listener.try_clone().unwrap();
    let upstream = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..attempts {
            let (mut stream, _) = peer.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            requests.push(read_request(&mut stream));
            let rejected = relay && index == 0;
            let (content_type, body) = if rejected {
                ("application/json", serde_json::to_vec(&json!({"error":{"type":"overloaded_error","message":"SENTINEL_UPSTREAM_ERROR"}})).unwrap())
            } else {
                let mut value = chat(json!({"content":"SENTINEL_SUCCESS_BODY"}), json!("stop"));
                value["usage"] =
                    json!({"prompt_tokens":11,"completion_tokens":7,"total_tokens":18});
                let mut body = frame(value);
                body.extend_from_slice(b"data: [DONE]\n\n");
                ("text/event-stream", body)
            };
            let status = if rejected {
                "503 Service Unavailable"
            } else {
                "200 OK"
            };
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            stream.write_all(&body).unwrap();
            let _ = stream.finish();
        }
        requests
    });
    let mut publication = publication(&listener, false, false);
    for (index, candidate) in publication.aliases[0].candidates.iter_mut().enumerate() {
        if no_credentials[index] {
            candidate.credential_refs = vec![format!("credential/none/projection-{index}")];
            for profile in &mut candidate.protocol_profiles {
                profile.connector.authentication = hiroute_domain::GatewayCriticalFactV1::Exact(
                    hiroute_domain::GatewayAuthenticationSemanticsV1::None,
                );
            }
            candidate.protocol_profile_digest =
                hiroute_domain::CanonicalDigest::of(&candidate.protocol_profiles).unwrap();
        }
    }
    publication.payload_digest = publication.canonical_digest().unwrap();
    publication.validate().unwrap();
    let publications = Arc::new(GatewayPublicationInstaller::open(root.join("lkg.json")).unwrap());
    let crate::server::publication::GatewayPrepareOutcome::Prepared(prepared) =
        publications.prepare(publication).unwrap()
    else {
        panic!("new publication must prepare");
    };
    publications.publish(prepared).unwrap();
    fs::write(root.join("lease.json"), serde_json::to_vec(&json!({
        "schema_version":"hiroute.gateway.credential-leases/v1","credential_ref":"stream-credential",
        "keys":[{"key_id":"stream-key","generation":1,"authorization":"Bearer SENTINEL_UPSTREAM_CREDENTIAL"}]
    })).unwrap()).unwrap();
    fs::write(root.join("credentials.json"), serde_json::to_vec(&json!({
        "schema_version":"hiroute.gateway.credentials/v1","credentials":{"stream-credential":"lease.json"}
    })).unwrap()).unwrap();
    let mut ports = ProductionPorts::fail_closed(publications);
    ports.credentials = Arc::new(
        crate::server::composition::FileCredentialResolver::open(&root.join("credentials.json"))
            .unwrap(),
    );
    ports.runtime_state = Arc::new(crate::ports::InMemoryRuntimeStateStore::default());
    let diagnostics = diagnostics_enabled.then(|| {
        DiagnosticRuntime::start(RuntimeConfig {
            root: root.join("diagnostics"),
            role: ProcessRole::Daemon,
            component: Component::Gateway,
            parent_session_id: None,
            level_override: Some(DiagnosticLevel::Debug),
        })
    });
    let mut sinks = GatewayObservationSinks::discard();
    sinks.execution_fact = Arc::new(facts.clone());
    let observation = GatewayObservation::with_sinks(512 * 1024, sinks).with_diagnostics(
        diagnostics
            .as_ref()
            .map(|runtime| runtime.port())
            .unwrap_or_default(),
    );
    let runtime = ProductionGatewayRuntime::compose_with_planner_and_observation(
        ports,
        Arc::new(PublicationPlannerInputAuthority),
        Arc::new(observation),
    );
    let address = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let mut gateway = crate::server::GatewayLauncher::from_runtime(address, Arc::new(runtime))
        .unwrap()
        .start_managed()
        .unwrap();
    let mut messages = vec![json!({"role":"user","content":"SENTINEL_REQUEST_BODY"})];
    if loss {
        messages.insert(0, json!({"role":"assistant","content":[
            {"type":"thinking","thinking":"SENTINEL_REASONING_TEXT","signature":"SENTINEL_REASONING_STATE"},
            {"type":"text","text":"SENTINEL_PREVIOUS_ANSWER"}
        ]}));
    }
    let body = serde_json::to_vec(
        &json!({"model":"stream-alias","max_tokens":64,"messages":messages,"stream":true}),
    )
    .unwrap();
    let mut client = TcpStream::connect(address).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    write!(client, "POST /v1/messages HTTP/1.1\r\nHost: {address}\r\nX-HiRoute-Token: {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
    client.write_all(&body).unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).unwrap();
    assert!(
        response.starts_with(b"HTTP/1.1 200"),
        "{}",
        String::from_utf8_lossy(&response)
    );
    assert!(
        response
            .windows(b"SENTINEL_SUCCESS_BODY".len())
            .any(|w| w == b"SENTINEL_SUCCESS_BODY")
    );
    let requests = upstream.join().unwrap();
    assert_eq!(requests.len(), attempts);
    for (index, request) in requests.iter().enumerate() {
        let offset = request.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let headers = String::from_utf8_lossy(&request[..offset]).to_ascii_lowercase();
        assert_eq!(headers.contains("authorization:"), !no_credentials[index]);
        let body: Value = serde_json::from_slice(&request[offset + 4..]).unwrap();
        assert_eq!(
            body["messages"].as_array().unwrap().len(),
            if loss { 2 } else { 1 }
        );
        if loss {
            assert_eq!(
                body["messages"][0]["reasoning_content"],
                "SENTINEL_REASONING_TEXT"
            );
            assert!(body["messages"][0].get("signature").is_none());
        }
    }
    wait_finished(&facts, false);
    gateway.shutdown();
    gateway.join(Duration::from_secs(5)).unwrap();
    let facts = facts.0.lock().unwrap().clone();
    if relay {
        assert_prebody_relay_business(&facts);
    } else {
        assert_business(&facts, false, false);
    }
    let Some(diagnostics) = diagnostics else {
        assert!(!root.join("diagnostics").exists());
        wait_replay_clean(&root.join("replay"));
        return;
    };
    diagnostics.shutdown();
    let log = fs::read_to_string(root.join("diagnostics/daemon/current.jsonl")).unwrap();
    assert!(!log.contains("SENTINEL"), "payload leaked into diagnostics");
    let records: Vec<Value> = log
        .lines()
        .map(|row| serde_json::from_str(row).unwrap())
        .collect();
    let warnings: Vec<_> = records
        .iter()
        .enumerate()
        .filter_map(|(index, row)| {
            payload(row, "reasoning_cleanup").map(|warning| (index, warning))
        })
        .collect();
    assert_eq!(
        warnings.len(),
        if loss { attempts } else { 0 },
        "{case}: {log}"
    );
    let begins: Vec<_> = records
        .iter()
        .enumerate()
        .filter_map(|(index, row)| payload(row, "attempt_begin").map(|begin| (index, begin)))
        .collect();
    let ends: Vec<_> = records
        .iter()
        .filter_map(|row| payload(row, "attempt_end"))
        .collect();
    assert_eq!(begins.len(), attempts);
    assert_eq!(ends.len(), attempts);
    for (index, (_, begin)) in begins.iter().enumerate() {
        let ordinal = (index + 1) as u64;
        assert_eq!(begin["attempt_index"], ordinal);
        assert!(!begin["attempt_token"].is_null());
        assert!(!begin["binding_token"].is_null());
        let end = ends
            .iter()
            .find(|end| end["attempt_token"] == begin["attempt_token"])
            .unwrap();
        assert_eq!(end["reasoning_fields_removed"], u64::from(loss));
        assert_eq!(end["attempt_token"], begin["attempt_token"]);
        if loss {
            let (position, warning) = warnings
                .iter()
                .find(|(_, warning)| warning["attempt_index"] == ordinal)
                .unwrap();
            assert!(
                *position < begins[index].0,
                "projection WARN must precede upstream attempt promotion"
            );
            assert!(warning["attempt_token"].is_null());
            assert_eq!(warning["request_token"], begin["request_token"]);
            assert_eq!(warning["binding_token"], begin["binding_token"]);
            assert_eq!(warning["reason"], "request_protocol_projection");
            assert_eq!(warning["fields_removed_so_far"], 1);
            assert!(warning["cleaned_prefix_len"].is_null());
            if index > 0 {
                assert_ne!(
                    warning["binding_token"],
                    begins[index - 1].1["binding_token"]
                );
            }
        }
    }
    wait_replay_clean(&root.join("replay"));
}
