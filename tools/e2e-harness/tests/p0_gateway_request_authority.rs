use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use hiroute_e2e::gateway_fixture::{
    E2E_DIAL_CONFIG_ENV, E2E_DIAL_CONFIG_FILE, TestTlsListener, read_complete_http_request,
    sealed_native_candidate, serve_one_tls_json_response, write_dial_config,
};
use hiroute_gateway::server::dispatch::GatewayRequestAuthority;
use hiroute_gateway::server::publication::{
    AliasPlanV1, CandidateBindingV1, GatewayPrepareOutcome, GatewayPublicationInstaller,
    GatewayPublicationSnapshotV3, GrantV1, MAX_LOGICAL_REQUEST_DURATION_MS, PublicationFailpoint,
    token_sha256,
};
use hiroute_gateway::server::request_plan::IngressProtocol;
use serde_json::{Value, json};

fn candidate(local_id: u32) -> CandidateBindingV1 {
    sealed_native_candidate(
        local_id,
        &format!("target-{local_id}"),
        &[format!("credential-{local_id}")],
        &format!("provider-{local_id}.invalid"),
        &format!("native-model-{local_id}"),
        &[
            (IngressProtocol::Responses, IngressProtocol::Responses),
            (IngressProtocol::Messages, IngressProtocol::Responses),
        ],
    )
}

fn plan_routes(
    plans: &[(&str, u64)],
) -> std::collections::BTreeMap<String, hiroute_gateway::server::publication::ModelRouteV2> {
    plans
        .iter()
        .map(|(name, revision)| {
            (
                (*name).into(),
                hiroute_gateway::server::publication::ModelRouteV2::Plan {
                    plan_id: format!("legacy/{name}"),
                    alias: (*name).into(),
                    revision: *revision,
                    semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(name.as_bytes()),
                },
            )
        })
        .collect()
}

fn snapshot(publication_revision: u64, fast_plan_revision: u64) -> GatewayPublicationSnapshotV3 {
    GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "application-authority",
        7,
        publication_revision,
        "codex-model-catalog/v3",
        vec![
            AliasPlanV1 {
                served_model_id: "plan-fast".into(),
                purpose: "short low-latency work".into(),
                agent_plan_revision: fast_plan_revision,
                protocols: vec![IngressProtocol::Responses],
                overall_timeout_ms: 1_500,
                max_attempts: 1,
                routing: None,
                candidates: vec![candidate(1)],
            },
            AliasPlanV1 {
                served_model_id: "plan-deep".into(),
                purpose: "long deliberate work".into(),
                agent_plan_revision: 22,
                protocols: vec![IngressProtocol::Responses, IngressProtocol::Messages],
                overall_timeout_ms: 30_000,
                max_attempts: 4,
                routing: None,
                candidates: vec![candidate(2)],
            },
        ],
        vec![GrantV1 {
            grant_id: "agent-a".into(),
            generation: 3,
            bearer_token_sha256: token_sha256("test-agent-token"),
            protocol: IngressProtocol::Responses,
            routes: plan_routes(&[("plan-fast", fast_plan_revision), ("plan-deep", 22)]),
        }],
    )
    .expect("valid publication")
}

fn publish(installer: &GatewayPublicationInstaller, snapshot: GatewayPublicationSnapshotV3) {
    let GatewayPrepareOutcome::Prepared(prepared) = installer.prepare(snapshot).unwrap() else {
        panic!("unexpected duplicate publication")
    };
    installer.publish(prepared).unwrap();
}

#[path = "p0_gateway_request_authority/typed.rs"]
mod typed_authority;

#[test]
fn durable_lkg_restart_and_crash_boundary_never_expose_a_torn_live_revision() {
    let directory = tempfile::tempdir().unwrap();
    let lkg = directory.path().join("gateway-publication-lkg.json");
    let installer = GatewayPublicationInstaller::open(&lkg).unwrap();
    publish(&installer, snapshot(1, 11));
    let GatewayPrepareOutcome::Prepared(prepared) = installer.prepare(snapshot(2, 12)).unwrap()
    else {
        panic!("unexpected duplicate")
    };
    let interrupted = installer
        .publish_with_failpoint(prepared, PublicationFailpoint::AfterDurableLkg)
        .unwrap_err();
    assert!(interrupted.is_crash_boundary());
    assert!(installer.active().is_none());
    drop(installer);

    let restarted = GatewayPublicationInstaller::open(&lkg).unwrap();
    assert_eq!(restarted.active().unwrap().publication_revision(), 2);
    assert_eq!(
        restarted.active().unwrap().agent_plan_revision("plan-fast"),
        Some(12)
    );
}

#[test]
fn production_hirouted_wire_authority_catalog_deadline_and_actual_restart() {
    #[rustfmt::skip] hiroute_e2e::p0_execution_receipt!("authority.wire_listener", ["gateway.isolated_native_listener", "authority.shared_entry_selects_alias_plan", "authority.unknown_or_unauthorized_alias", "authority.catalog_and_etag", "authority.alias_budget", "gateway.loopback_h1"]);
    let directory = tempfile::tempdir().unwrap();
    let provider = TestTlsListener::bind("wire-provider.invalid").unwrap();
    provider.set_nonblocking(true).unwrap();
    let publication_path = directory.path().join("publication.json");
    let credentials_path = directory.path().join("credentials.json");
    let lkg_path = directory.path().join("publication-lkg.json");
    let publication = wire_snapshot(provider.authority());
    std::fs::write(
        &publication_path,
        serde_json::to_vec_pretty(&publication).unwrap(),
    )
    .unwrap();
    write_dial_config(directory.path(), &[&provider]).unwrap();
    for local_id in 1..=3 {
        std::fs::write(
            directory
                .path()
                .join(format!("wire-credential-{local_id}.json")),
            serde_json::to_vec_pretty(&json!({
                "schema_version": "hiroute.gateway.credential-leases/v1",
                "credential_ref": format!("wire-credential-{local_id}"),
                "keys": [{
                    "key_id": format!("wire-key-{local_id}"),
                    "generation": 1,
                    "authorization": format!("Bearer wire-provider-secret-{local_id}"),
                }],
            }))
            .unwrap(),
        )
        .unwrap();
    }
    std::fs::write(
        &credentials_path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "hiroute.gateway.credentials/v1",
            "credentials": {
                "wire-credential-1": "wire-credential-1.json",
                "wire-credential-2": "wire-credential-2.json",
                "wire-credential-3": "wire-credential-3.json",
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let binary = exact_hirouted_binary();
    let address = reserve_address();
    let mut first = HiroutedProcess::spawn(
        &binary,
        address,
        &lkg_path,
        Some(&publication_path),
        Some(&credentials_path),
        directory.path(),
        "first",
    );
    first.wait_ready();
    let disabled_control =
        publication_control_request(address, "disabled", "default-disabled", &publication);
    assert_ne!(
        serde_json::from_slice::<Value>(&disabled_control.body).unwrap()["schema_version"],
        "hiroute.gateway.e2e-control-response/v1"
    );
    let ready = request(address, "GET", "/_hiroute/ready?probe=authority", &[], b"");
    assert_json_response(&ready, 200);
    let ready_body: Value = serde_json::from_slice(&ready.body).unwrap();
    assert_eq!(ready_body["schema_version"], "hiroute.gateway.ready/v2");
    assert_eq!(ready_body["publication_revision"], 31);
    assert_eq!(ready_body["publication_digest"], publication.payload_digest);
    assert!(ready_body["executable_sha256"].is_null());

    for headers in [
        "Authorization: Bearer wire-token\r\n",
        "X-HiRoute-Token: wrong-token\r\nAuthorization: Bearer wire-token\r\n",
        "X-HiRoute-Token: wire-token\r\nX-HiRoute-Token: wire-token\r\n",
    ] {
        let unauthorized =
            header_only_request(address, "/v1/responses", headers, Duration::from_secs(1));
        assert_json_response(&unauthorized, 401);
        assert_eq!(
            serde_json::from_slice::<Value>(&unauthorized.body).unwrap()["code"],
            "GATEWAY_GRANT_UNAUTHORIZED"
        );
    }

    let models = request(
        address,
        "GET",
        "/v1/models?client_version=e2e",
        &[("X-HiRoute-Token", "wire-token")],
        b"",
    );
    assert_json_response(&models, 200);
    assert_eq!(
        models.body,
        br#"{"object":"list","data":[{"id":"wire-deep","object":"model","created":0,"owned_by":"hiroute","purpose":"wire deep","agent_plan_revision":42,"protocols":["responses"]},{"id":"wire-fast","object":"model","created":0,"owned_by":"hiroute","purpose":"wire fast","agent_plan_revision":41,"protocols":["responses"]}]}"#
    );
    let etag = models.headers["etag"].clone();
    assert!(etag.starts_with("\"hiroute-") && etag.ends_with('"'));
    let unchanged = request(
        address,
        "GET",
        "/v1/models",
        &[("X-HiRoute-Token", "wire-token"), ("If-None-Match", &etag)],
        b"",
    );
    assert_eq!(unchanged.status, 304);
    assert_eq!(unchanged.headers["etag"], etag);
    assert!(unchanged.body.is_empty());

    for (path, model, header, status, code) in [
        (
            "/v1/responses",
            "unknown",
            ("X-HiRoute-Token", "wire-token"),
            404,
            "AGENT_MODEL_NOT_GRANTED",
        ),
        (
            "/v1/responses",
            "wire-private",
            ("X-HiRoute-Token", "wire-token"),
            404,
            "AGENT_MODEL_NOT_GRANTED",
        ),
        (
            "/v1/messages",
            "wire-fast",
            ("Authorization", "Bearer wire-messages-token"),
            404,
            "AGENT_MODEL_NOT_GRANTED",
        ),
        (
            "/v1/messages",
            "wire-deep",
            ("Authorization", "Bearer wire-token"),
            422,
            "AGENT_PROTOCOL_UNSUPPORTED",
        ),
    ] {
        let body = serde_json::to_vec(&json!({"model": model})).unwrap();
        let response = request(address, "POST", path, &[header], &body);
        assert_json_response(&response, status);
        assert_eq!(
            serde_json::from_slice::<Value>(&response.body).unwrap(),
            json!({
                "schema_version": "hiroute.gateway.error/v1",
                "code": code,
                "phase": "request_authority",
            })
        );
    }

    let provider_thread = serve_one_tls_json_response(
        provider.try_clone().unwrap(),
        "Bearer wire-provider-secret-1",
        provider_response("wire-provider", "ok"),
    );
    let accepted = request(
        address,
        "POST",
        "/v1/responses",
        &[("X-HiRoute-Token", "wire-token")],
        br#"{"model":"wire-fast","input":"opaque"}"#,
    );
    provider_thread.join().unwrap();
    assert_json_response(&accepted, 200);
    assert_response_text(&accepted, "wire-fast", "wire-provider", "ok");

    let selector_started = Instant::now();
    let timed_out = trickle_request(address, Duration::from_secs(3));
    assert!(
        selector_started.elapsed() < Duration::from_secs(2),
        "the maximum-duration sibling must not extend incomplete selector retention"
    );
    assert_json_response(&timed_out, 408);
    assert_eq!(
        serde_json::from_slice::<Value>(&timed_out.body).unwrap()["code"],
        "REQUEST_DEADLINE_EXCEEDED"
    );
    assert_eq!(
        provider_accept_error_kind(&provider),
        std::io::ErrorKind::WouldBlock,
        "rejected and authority-only requests must not connect to Provider"
    );
    assert!(lkg_path.exists());

    first.crash();
    let mut restarted = HiroutedProcess::spawn(
        &binary,
        address,
        &lkg_path,
        None,
        Some(&credentials_path),
        directory.path(),
        "restarted",
    );
    restarted.wait_ready();
    let restored = request_models(address, "wire-token");
    assert_json_response(&restored, 200);
    assert_eq!(restored.headers["etag"], etag);
    assert_eq!(restored.body, models.body);
    restarted.crash();
}

#[test]
fn production_hirouted_live_publication_control_pins_cutover_and_preserves_lkg_on_nack() {
    #[rustfmt::skip] hiroute_e2e::p0_execution_receipt!("authority.live_publication", ["publication.incompatible_or_gap", "publication.inflight_old_revision", "publication.lkg_restart"]);
    let directory = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let old_provider = TestTlsListener::bind("live-old-provider.invalid").unwrap();
    let new_provider = TestTlsListener::bind("live-new-provider.invalid").unwrap();
    let initial = live_snapshot(71, 81, old_provider.authority(), "live-credential");
    let updated = live_snapshot(72, 82, new_provider.authority(), "live-credential");
    let publication_path = directory.path().join("live-publication.json");
    let credentials_path = write_live_credentials(directory.path());
    let lkg_path = directory.path().join("live-publication-lkg.json");
    write_dial_config(directory.path(), &[&old_provider, &new_provider]).unwrap();
    std::fs::write(
        &publication_path,
        serde_json::to_vec_pretty(&initial).unwrap(),
    )
    .unwrap();
    let binary = exact_hirouted_binary();
    let address = reserve_address();
    let control_address = reserve_address();
    let (old_address, old_control_address) = (reserve_address(), reserve_address());
    let (old_nonce_path, old_nonce) = write_control_nonce(directory.path(), "old", 0o600);
    let mut old_process = HiroutedProcess::spawn_with_control(
        &binary,
        old_address,
        &lkg_path,
        Some(&publication_path),
        Some(&credentials_path),
        directory.path(),
        "live-control-old",
        Some((old_control_address, &old_nonce_path)),
    );
    old_process.wait_ready();
    assert!(!old_nonce_path.exists());
    old_process.crash();
    let (nonce_path, control_nonce) = write_control_nonce(directory.path(), "current", 0o600);
    assert_ne!(old_nonce, control_nonce);
    let mut process = HiroutedProcess::spawn_with_control(
        &binary,
        address,
        &lkg_path,
        Some(&publication_path),
        Some(&credentials_path),
        directory.path(),
        "live-control",
        Some((control_address, &nonce_path)),
    );
    process.wait_ready();
    assert!(!nonce_path.exists());
    let initial_models = request_models(address, "live-wire-token");
    assert_json_response(&initial_models, 200);
    let initial_etag = initial_models.headers["etag"].clone();
    let unauthorized =
        publication_control_request(control_address, &old_nonce, "unauthorized-update", &updated);
    assert_json_response(&unauthorized, 403);
    assert_eq!(
        serde_json::from_slice::<Value>(&unauthorized.body).unwrap()["code"],
        "E2E_CONTROL_UNAUTHORIZED"
    );
    assert_eq!(ready_revision(address), 71);
    let (old_started_tx, old_started_rx) = mpsc::channel();
    let (release_old_tx, release_old_rx) = mpsc::channel();
    let old_provider_thread = serve_gated_provider_response(
        old_provider,
        "Bearer live-provider",
        "old-provider",
        "old",
        Some(old_started_tx),
        Some(release_old_rx),
    );
    let old_request = std::thread::spawn(move || {
        request(
            address,
            "POST",
            "/v1/responses",
            &[("X-HiRoute-Token", "live-wire-token")],
            br#"{"model":"wire-live","input":"opaque"}"#,
        )
    });
    old_started_rx.recv_timeout(Duration::from_secs(9)).unwrap();
    let ack = publication_control_request(
        control_address,
        &control_nonce,
        "apply-revision-72",
        &updated,
    );
    assert_json_response(&ack, 200);
    let expected_ack = format!(
        r#"{{"schema_version":"hiroute.gateway.e2e-control-response/v1","request_id":"apply-revision-72","command":"publication_update","outcome":"ack","code":"PUBLICATION_APPLIED","proposed_publication_revision":72,"active":{{"authority_id":"live-authority","authority_epoch":17,"publication_revision":72,"publication_digest":"{}"}}}}"#,
        updated.payload_digest
    );
    assert_eq!(ack.body, expected_ack.as_bytes());
    let new_provider_thread = serve_gated_provider_response(
        new_provider.clone(),
        "Bearer live-provider",
        "new-provider",
        "new",
        None,
        None,
    );
    let new_request = request(
        address,
        "POST",
        "/v1/responses",
        &[("X-HiRoute-Token", "live-wire-token")],
        br#"{"model":"wire-live","input":"opaque"}"#,
    );
    new_provider_thread.join().unwrap();
    assert_json_response(&new_request, 200);
    assert_response_text(&new_request, "wire-live", "new-provider", "new");
    release_old_tx.send(()).unwrap();
    let old_request = old_request.join().unwrap();
    old_provider_thread.join().unwrap();
    assert_json_response(&old_request, 200);
    assert_response_text(&old_request, "wire-live", "old-provider", "old");
    let current_models = request_models(address, "live-wire-token");
    assert_json_response(&current_models, 200);
    assert_ne!(current_models.headers["etag"], initial_etag);
    assert_eq!(
        serde_json::from_slice::<Value>(&current_models.body).unwrap()["data"][0]["agent_plan_revision"],
        82
    );
    let durable_last_good = std::fs::read(&lkg_path).unwrap();
    let mut incompatible = live_snapshot(73, 83, new_provider.authority(), "live-credential");
    incompatible.schema_version = "hiroute.gateway.publication-snapshot/v999".into();
    incompatible.payload_digest = incompatible.canonical_digest().unwrap();
    let incompatible_nack = publication_control_request(
        control_address,
        &control_nonce,
        "reject-incompatible",
        &incompatible,
    );
    assert_control_nack(
        &incompatible_nack,
        422,
        "reject-incompatible",
        "PUBLICATION_SCHEMA_INCOMPATIBLE",
        73,
        72,
    );
    assert_eq!(std::fs::read(&lkg_path).unwrap(), durable_last_good);
    let gap = live_snapshot(74, 84, new_provider.authority(), "live-credential");
    let gap_nack = publication_control_request(control_address, &control_nonce, "reject-gap", &gap);
    assert_control_nack(
        &gap_nack,
        409,
        "reject-gap",
        "PUBLICATION_REVISION_GAP",
        74,
        72,
    );
    assert_eq!(std::fs::read(&lkg_path).unwrap(), durable_last_good);
    assert_eq!(ready_revision(address), 72);
    process.crash();
    let mut restarted = HiroutedProcess::spawn(
        &binary,
        address,
        &lkg_path,
        None,
        Some(&credentials_path),
        directory.path(),
        "live-restarted",
    );
    restarted.wait_ready();
    assert_eq!(ready_revision(address), 72);
    let restored_models = request_models(address, "live-wire-token");
    assert_json_response(&restored_models, 200);
    assert_eq!(restored_models.body, current_models.body);
    restarted.crash();
}

fn wire_snapshot(provider_authority: &str) -> GatewayPublicationSnapshotV3 {
    let candidate = |local_id| {
        sealed_native_candidate(
            local_id,
            &format!("wire-target-{local_id}"),
            &[format!("wire-credential-{local_id}")],
            provider_authority,
            &format!("wire-native-model-{local_id}"),
            &[
                (IngressProtocol::Responses, IngressProtocol::Responses),
                (IngressProtocol::Messages, IngressProtocol::Responses),
            ],
        )
    };
    GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "wire-authority",
        9,
        31,
        "wire-renderer/v1",
        vec![
            AliasPlanV1 {
                served_model_id: "wire-fast".into(),
                purpose: "wire fast".into(),
                agent_plan_revision: 41,
                protocols: vec![IngressProtocol::Responses],
                overall_timeout_ms: 1_500,
                max_attempts: 1,
                routing: None,
                candidates: vec![candidate(1)],
            },
            AliasPlanV1 {
                served_model_id: "wire-deep".into(),
                purpose: "wire deep".into(),
                agent_plan_revision: 42,
                protocols: vec![IngressProtocol::Responses, IngressProtocol::Messages],
                overall_timeout_ms: MAX_LOGICAL_REQUEST_DURATION_MS,
                max_attempts: 4,
                routing: None,
                candidates: vec![candidate(2)],
            },
            AliasPlanV1 {
                served_model_id: "wire-private".into(),
                purpose: "wire private".into(),
                agent_plan_revision: 43,
                protocols: vec![IngressProtocol::Responses],
                overall_timeout_ms: 200,
                max_attempts: 1,
                routing: None,
                candidates: vec![candidate(3)],
            },
        ],
        vec![
            GrantV1 {
                grant_id: "wire-grant".into(),
                generation: 5,
                bearer_token_sha256: token_sha256("wire-token"),
                protocol: IngressProtocol::Responses,
                routes: plan_routes(&[("wire-fast", 41), ("wire-deep", 42)]),
            },
            GrantV1 {
                grant_id: "wire-messages-grant".into(),
                generation: 5,
                bearer_token_sha256: token_sha256("wire-messages-token"),
                protocol: IngressProtocol::Messages,
                routes: plan_routes(&[("wire-deep", 42)]),
            },
        ],
    )
    .unwrap()
}

fn live_snapshot(
    publication_revision: u64,
    plan_revision: u64,
    provider_authority: &str,
    credential_ref: &str,
) -> GatewayPublicationSnapshotV3 {
    GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "live-authority",
        17,
        publication_revision,
        format!("live-renderer/v{publication_revision}"),
        vec![AliasPlanV1 {
            served_model_id: "wire-live".into(),
            purpose: "live publication cutover".into(),
            agent_plan_revision: plan_revision,
            protocols: vec![IngressProtocol::Responses],
            overall_timeout_ms: 60_000,
            max_attempts: 1,
            routing: None,
            candidates: vec![sealed_native_candidate(
                1,
                "live-target",
                &[credential_ref.into()],
                provider_authority,
                "live-native-model",
                &[(IngressProtocol::Responses, IngressProtocol::Responses)],
            )],
        }],
        vec![GrantV1 {
            grant_id: "live-grant".into(),
            generation: 1,
            bearer_token_sha256: token_sha256("live-wire-token"),
            protocol: IngressProtocol::Responses,
            routes: plan_routes(&[("wire-live", plan_revision)]),
        }],
    )
    .unwrap()
}

fn write_live_credentials(directory: &Path) -> PathBuf {
    std::fs::write(
        directory.join("live-credential.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": "hiroute.gateway.credential-leases/v1",
            "credential_ref": "live-credential",
            "keys": [{
                "key_id": "live-key",
                "generation": 1,
                "authorization": "Bearer live-provider",
            }],
        }))
        .unwrap(),
    )
    .unwrap();
    let path = directory.join("live-credentials.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "hiroute.gateway.credentials/v1",
            "credentials": {
                "live-credential": "live-credential.json",
            }
        }))
        .unwrap(),
    )
    .unwrap();
    path
}
fn write_control_nonce(directory: &Path, label: &str, mode: u32) -> (PathBuf, String) {
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).unwrap();
    let nonce = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let path = directory.join(format!("e2e-control-{label}.nonce"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(mode);
    let mut file = options.open(&path).unwrap();
    file.write_all(nonce.as_bytes()).unwrap();
    file.sync_all().unwrap();
    (path, nonce)
}
fn request_models(address: SocketAddr, token: &str) -> WireResponse {
    request(
        address,
        "GET",
        "/v1/models",
        &[("X-HiRoute-Token", token)],
        b"",
    )
}
fn publication_control_request(
    address: SocketAddr,
    nonce: &str,
    request_id: &str,
    snapshot: &GatewayPublicationSnapshotV3,
) -> WireResponse {
    let body = serde_json::to_vec(&json!({
        "schema_version": "hiroute.gateway.e2e-control-request/v1",
        "request_id": request_id,
        "command": {
            "kind": "publication_update",
            "snapshot": snapshot,
        }
    }))
    .unwrap();
    request(
        address,
        "POST",
        "/_hiroute/e2e-control/v1",
        &[
            ("Content-Type", "application/json"),
            ("X-HiRoute-E2E-Control-Nonce", nonce),
        ],
        &body,
    )
}

fn assert_control_nack(
    response: &WireResponse,
    status: u16,
    request_id: &str,
    code: &str,
    proposed_revision: u64,
    active_revision: u64,
) {
    assert_json_response(response, status);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(
        body["schema_version"],
        "hiroute.gateway.e2e-control-response/v1"
    );
    assert_eq!(body["request_id"], request_id);
    assert_eq!(body["command"], "publication_update");
    assert_eq!(body["outcome"], "nack");
    assert_eq!(body["code"], code);
    assert_eq!(body["proposed_publication_revision"], proposed_revision);
    assert_eq!(body["active"]["publication_revision"], active_revision);
}

fn ready_revision(address: SocketAddr) -> u64 {
    let ready = request(address, "GET", "/_hiroute/ready", &[], b"");
    assert_json_response(&ready, 200);
    serde_json::from_slice::<Value>(&ready.body).unwrap()["publication_revision"]
        .as_u64()
        .unwrap()
}

fn serve_gated_provider_response(
    listener: TestTlsListener,
    expected_authorization: &'static str,
    response_id: &'static str,
    text: &'static str,
    started: Option<mpsc::Sender<()>>,
    release: Option<mpsc::Receiver<()>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_complete_http_request(&mut stream, Duration::from_secs(3))
            .expect("complete gated Provider request");
        assert!(
            String::from_utf8_lossy(&request)
                .to_ascii_lowercase()
                .contains(&format!(
                    "authorization: {}",
                    expected_authorization.to_ascii_lowercase()
                ))
        );
        if let Some(started) = started {
            started.send(()).unwrap();
        }
        if let Some(release) = release {
            release.recv_timeout(Duration::from_secs(9)).unwrap();
        }
        let body = provider_response(response_id, text);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(&body).unwrap();
        stream.finish().unwrap();
    })
}

fn provider_accept_error_kind(listener: &TestTlsListener) -> std::io::ErrorKind {
    match listener.accept() {
        Err(error) => error.kind(),
        Ok(_) => panic!("unexpected Provider connection"),
    }
}

fn provider_response(id: &str, text: &str) -> Vec<u8> {
    format!(
        r#"{{"id":"{id}","model":"physical-model","status":"completed","output":[{{"type":"message","id":"provider-message","status":"completed","role":"assistant","content":[{{"type":"output_text","text":"{text}","annotations":[]}}]}}],"usage":{{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}}"#
    )
    .into_bytes()
}

fn assert_response_text(response: &WireResponse, model: &str, id: &str, text: &str) {
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["model"], model);
    assert_eq!(body["id"], id);
    assert_eq!(body["output"][0]["content"][0]["text"], text);
}
struct WireResponse {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

fn request(
    address: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> WireResponse {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
    read_wire_response(stream)
}

fn trickle_request(address: SocketAddr, bound: Duration) -> WireResponse {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream.set_read_timeout(Some(bound)).unwrap();
    write!(
        stream,
        "POST /v1/responses HTTP/1.1\r\nHost: {address}\r\nX-HiRoute-Token: wire-token\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{{"
    )
    .unwrap();
    stream.flush().unwrap();
    read_wire_response(stream)
}

fn header_only_request(
    address: SocketAddr,
    path: &str,
    headers: &str,
    bound: Duration,
) -> WireResponse {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream.set_read_timeout(Some(bound)).unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {address}\r\n{headers}Content-Type: application/json\r\nContent-Length: 1048576\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    stream.flush().unwrap();
    read_wire_response(stream)
}

fn read_wire_response(mut stream: TcpStream) -> WireResponse {
    let mut wire = Vec::new();
    let read_error = stream.read_to_end(&mut wire).err();
    let split = wire
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap_or_else(|| {
            panic!(
                "response has no HTTP head: {}",
                String::from_utf8_lossy(&wire)
            )
        });
    let head = std::str::from_utf8(&wire[..split]).unwrap();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .map(|line| line.split_once(':').unwrap())
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect::<BTreeMap<_, _>>();
    if let Some(error) = read_error {
        let complete = headers
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .is_some_and(|length| length == wire.len() - split - 4)
            || (status == 304 && wire.len() == split + 4);
        assert!(
            error.kind() == std::io::ErrorKind::ConnectionReset && complete,
            "incomplete HTTP response ended with {error}: {}",
            String::from_utf8_lossy(&wire)
        );
    }
    WireResponse {
        status,
        headers,
        body: wire[split + 4..].to_vec(),
    }
}

fn assert_json_response(response: &WireResponse, expected_status: u16) {
    assert_eq!(
        response.status,
        expected_status,
        "unexpected body: {}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!(response.headers["content-type"], "application/json");
    if let Some(length) = response.headers.get("content-length") {
        assert_eq!(length.parse::<usize>().unwrap(), response.body.len());
    } else {
        assert_eq!(
            response.headers.get("connection").map(String::as_str),
            Some("close")
        );
    }
}

struct HiroutedProcess {
    child: Option<Child>,
    address: SocketAddr,
    stderr_path: PathBuf,
}

impl HiroutedProcess {
    fn spawn(
        binary: &Path,
        address: SocketAddr,
        lkg: &Path,
        publication: Option<&Path>,
        credentials: Option<&Path>,
        directory: &Path,
        label: &str,
    ) -> Self {
        Self::spawn_with_control(
            binary,
            address,
            lkg,
            publication,
            credentials,
            directory,
            label,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_with_control(
        binary: &Path,
        address: SocketAddr,
        lkg: &Path,
        publication: Option<&Path>,
        credentials: Option<&Path>,
        directory: &Path,
        label: &str,
        control: Option<(SocketAddr, &Path)>,
    ) -> Self {
        let stdout = File::create(directory.join(format!("hirouted-{label}.stdout"))).unwrap();
        let stderr_path = directory.join(format!("hirouted-{label}.stderr"));
        let stderr = File::create(&stderr_path).unwrap();
        let mut command = Command::new(binary);
        command
            .env_clear()
            .env("TMPDIR", directory)
            .env("NO_PROXY", "127.0.0.1,localhost,::1")
            .env(E2E_DIAL_CONFIG_ENV, directory.join(E2E_DIAL_CONFIG_FILE))
            .args(["--listen", &address.to_string(), "--lkg"])
            .arg(lkg)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr);
        if let Some(publication) = publication {
            command.arg("--publication").arg(publication);
        }
        if let Some(credentials) = credentials {
            command.arg("--credentials").arg(credentials);
        }
        if let Some((control_address, nonce_file)) = control {
            command
                .args(["--e2e-control-listen", &control_address.to_string()])
                .arg("--e2e-control-nonce-file")
                .arg(nonce_file);
        }
        let child = command.spawn().unwrap();
        Self {
            child: Some(child),
            address,
            stderr_path,
        }
    }

    fn wait_ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(stream) = TcpStream::connect_timeout(&self.address, Duration::from_millis(50))
            {
                drop(stream);
                if request(self.address, "GET", "/_hiroute/ready", &[], b"").status == 200 {
                    return;
                }
            }
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                let stderr = std::fs::read_to_string(&self.stderr_path)
                    .unwrap_or_else(|error| format!("<failed to read stderr: {error}>"));
                panic!("hirouted exited before readiness ({status}): {stderr}");
            }
            assert!(Instant::now() < deadline, "hirouted readiness timed out");
            std::thread::yield_now();
        }
    }

    fn crash(&mut self) {
        let mut child = self.child.take().unwrap();
        child.kill().unwrap();
        let status = child.wait().unwrap();
        assert!(!status.success());
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;

            assert_eq!(status.signal(), Some(libc::SIGKILL));
        }
    }
}

impl Drop for HiroutedProcess {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn exact_hirouted_binary() -> PathBuf {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            if std::env::var_os("HIROUTE_VALIDATION_EXECUTION").is_some() {
                let path = PathBuf::from(
                    std::env::var_os("HIROUTE_VALIDATION_GATEWAY_BIN")
                        .expect("prepared Gateway binary missing"),
                );
                assert!(path.is_file());
                return path;
            }
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            let output = Command::new(cargo)
                .current_dir(&root)
                .args([
                    "build",
                    "--locked",
                    "--offline",
                    "-p",
                    "hiroute-gateway",
                    "--bin",
                    "hirouted",
                    "--all-features",
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "exact hirouted build failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let target = std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .map(|path| {
                    if path.is_absolute() {
                        path
                    } else {
                        root.join(path)
                    }
                })
                .unwrap_or_else(|| root.join("target"));
            target.join("debug").join(if cfg!(windows) {
                "hirouted.exe"
            } else {
                "hirouted"
            })
        })
        .clone()
}

fn reserve_address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}
