#![cfg(unix)]
use hiroute_application_api::*;
use hiroute_desktop::bootstrap::Resident;

#[tokio::test]
async fn killed_owned_daemon_recovers_its_stale_endpoint() {
    if isolated_agent_home("killed_owned_daemon_recovers_its_stale_endpoint") {
        return;
    }
    let root = tempfile::Builder::new()
        .prefix("hr02-")
        .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
        .unwrap();
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("hirouted");
    let mut resident = Resident::open(root.path(), &binary).unwrap();
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output()
        .unwrap();
    let pattern = format!("--storage-root {}", root.path().join("storage").display());
    let rows = String::from_utf8(output.stdout).unwrap();
    let pids: Vec<_> = rows
        .lines()
        .filter(|line| line.contains(&pattern))
        .map(|line| line.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(pids.len(), 1);
    assert!(
        std::process::Command::new("/bin/kill")
            .args(["-KILL", pids[0]])
            .status()
            .unwrap()
            .success()
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while resident.has_authority() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(!resident.has_authority());
    assert!(
        resident.client.endpoint().path().exists(),
        "SIGKILL leaves the socket"
    );
    drop(resident);
    let mut restarted = Resident::open(root.path(), &binary).unwrap();
    assert!(
        restarted.has_authority(),
        "must recover the dead owned endpoint"
    );
    let result: MachineEnvelopeV2<ClientServiceStatusV1> = restarted
        .client
        .query(
            "GetClientServiceStatus",
            "recovered",
            &ClientEmptyRequestV1 {},
        )
        .await
        .unwrap();
    assert!(result.data.unwrap().recovery_ready);
}

#[tokio::test]
async fn corrupt_observation_store_reports_data_failure_without_rewriting_it() {
    if isolated_agent_home("corrupt_observation_store_reports_data_failure_without_rewriting_it") {
        return;
    }
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::Builder::new()
        .prefix("hr02-corrupt-")
        .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
        .unwrap();
    let observation = root.path().join("storage/observation");
    std::fs::create_dir_all(&observation).unwrap();
    std::fs::set_permissions(
        root.path().join("storage"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    std::fs::set_permissions(&observation, std::fs::Permissions::from_mode(0o700)).unwrap();
    let database = observation.join("activity.db");
    let original = b"not a sqlite observation store";
    std::fs::write(&database, original).unwrap();
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600)).unwrap();

    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("hirouted");
    let error = Resident::open(root.path(), &binary)
        .err()
        .expect("corrupt observation data must stop startup");
    assert_eq!(error, "DAEMON_STORAGE_UNREADABLE");
    assert_eq!(std::fs::read(&database).unwrap(), original);
}

#[tokio::test]
async fn a_restart_rebinds_the_recorded_gateway_address() {
    if isolated_agent_home("a_restart_rebinds_the_recorded_gateway_address") {
        return;
    }
    let root = tempfile::Builder::new()
        .prefix("hr02-")
        .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
        .unwrap();
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("hirouted");
    assert!(
        binary.exists(),
        "build the production hirouted binary first"
    );
    let record = root.path().join("gateway-listener.json");
    let recorded_port = || {
        let record: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&record).unwrap()).unwrap();
        assert_eq!(record["schema_version"], "hiroute.gateway-listener/v1");
        assert_eq!(record["desired"]["address"], "127.0.0.1");
        record["desired"]["port"].as_u64().unwrap()
    };
    let mut resident = Resident::open(root.path(), &binary).unwrap();
    assert!(resident.has_authority());
    let port = recorded_port();
    assert_ne!(port, 0, "the record must exist before any publication");
    drop(resident);
    let mut restarted = Resident::open(root.path(), &binary).unwrap();
    assert!(
        restarted.has_authority(),
        "the ready handshake proves the daemon rebound the recorded address"
    );
    assert_eq!(recorded_port(), port, "restarts never select a new port");
    drop(restarted);
    let receipt = std::fs::read_to_string(root.path().join("desktop.lock")).unwrap();
    assert!(
        receipt.contains(&format!("\"127.0.0.1:{port}\"")),
        "the receipt must carry the served address for recovery: {receipt}"
    );
}

#[tokio::test]
async fn real_role_all_child_ack_and_read_only_lookup_use_shared_core() {
    if isolated_agent_home("real_role_all_child_ack_and_read_only_lookup_use_shared_core") {
        return;
    }
    let root = tempfile::Builder::new()
        .prefix("hr02-")
        .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
        .unwrap();
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("hirouted");
    assert!(
        binary.exists(),
        "build the production hirouted binary first"
    );
    let mut resident = Resident::open(root.path(), &binary).unwrap();
    assert!(resident.has_authority());
    let envelope: MachineEnvelopeV2<ClientServiceStatusV1> = resident
        .client
        .query("GetClientServiceStatus", "status", &ClientEmptyRequestV1 {})
        .await
        .unwrap();
    assert!(envelope.error.is_none(), "{:?}", envelope.error);
    let status = envelope.data.unwrap();
    assert_eq!(status.daemon_role, "all");
    assert!(status.recovery_ready);
    assert!(status.mutation_available);
    assert_eq!(status.gateway, ClientGatewayStateV1::Empty);
    let digest = CanonicalDigest::of_bytes(b"not an admitted change");
    let grant = resident.register(&digest, &status.revisions).unwrap();
    assert_eq!(grant.len(), 64);
    let lookup = OperationIdempotencyLookupV1 {
        principal_kind: PrincipalKind::Desktop,
        operation_kind: "ApplyAgentPlanChange".into(),
        idempotency_key: "never-submitted".into(),
        accepted_digest: digest,
    };
    let result: MachineEnvelopeV2<OperationIdempotencyResultV1> = resident
        .client
        .query("FindOperationByIdempotency", "lookup", &lookup)
        .await
        .unwrap();
    assert!(result.error.is_none());
    assert!(result.data.unwrap().operation.is_none());
    let plans: MachineEnvelopeV2<AgentPlanCatalogViewV2> = resident
        .client
        .query("ListAgentPlanCatalog", "plans", &ClientEmptyRequestV1 {})
        .await
        .unwrap();
    assert!(plans.error.is_none());
    assert!(plans.data.unwrap().plans.is_empty());
    drop(grant);
    for restore in [false, true] {
        let grant = resident
            .register_agent_settings(restore, &lookup.accepted_digest, &status.revisions)
            .unwrap();
        assert_eq!(grant.len(), 64);
    }
    let mut session = hiroute_desktop::session::Session::new(resident);
    let agents = session.agent_snapshot().await.unwrap();
    assert!(agents.trusted_authority);
    assert!(agents.plans.plans.is_empty());
    assert!(
        agents
            .agents
            .iter()
            .any(|agent| agent.agent_id == "agent_codex_default")
    );
    assert!(agents.agents.iter().all(|agent| {
        agent
            .settings
            .as_ref()
            .and_then(|status| status.model())
            .is_none_or(|status| !status.model_verified)
    }));
    drop(session);
}

#[tokio::test]
async fn obsolete_client_receipt_is_ignored_and_native_restart_requeries_backend() {
    if isolated_agent_home(
        "obsolete_client_receipt_is_ignored_and_native_restart_requeries_backend",
    ) {
        return;
    }
    let root = tempfile::Builder::new()
        .prefix("hr02-")
        .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
        .unwrap();
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("hirouted");
    std::fs::write(
        root.path().join("pending-intent.json"),
        b"invalid interrupted write",
    )
    .unwrap();
    let resident = Resident::open(root.path(), &binary).unwrap();
    let mut session = hiroute_desktop::session::Session::new(resident);
    let snapshot = session.snapshot().await.unwrap();
    assert!(snapshot.service.recovery_ready);
    assert!(snapshot.service.mutation_available);
    assert!(snapshot.pending.is_none());
    assert!(
        session
            .preview(hiroute_desktop::session::RenameInput {
                plan_id: "unavailable".into(),
                display_name: "new".into(),
                language: "en".into()
            })
            .await
            .is_err()
    );
    drop(session);
    let mut restarted = Resident::open(root.path(), &binary).unwrap();
    assert!(restarted.has_authority());
    let result: MachineEnvelopeV2<ClientServiceStatusV1> = restarted
        .client
        .query(
            "GetClientServiceStatus",
            "restarted",
            &ClientEmptyRequestV1 {},
        )
        .await
        .unwrap();
    assert!(result.data.unwrap().recovery_ready);
}
#[tokio::test]
async fn dropping_a_read_only_desktop_connection_does_not_stop_an_external_daemon() {
    if isolated_agent_home(
        "dropping_a_read_only_desktop_connection_does_not_stop_an_external_daemon",
    ) {
        return;
    }
    use std::process::{Command, Stdio};
    let root = tempfile::Builder::new()
        .prefix("hr02-")
        .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
        .unwrap();
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("hirouted");
    // Prepare only formal release inputs using the normal bootstrap, then quit its owned child.
    drop(Resident::open(root.path(), &binary).unwrap());
    let mut external = Command::new(&binary)
        .args(["--role", "control", "--storage-root"])
        .arg(root.path().join("storage"))
        .arg("--runtime-root")
        .arg(root.path().join("run"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let endpoint = hiroute_client_core::LocalEndpoint::from_runtime_root(root.path().join("run"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !endpoint.path().exists() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let mut resident = Resident::open(root.path(), &binary).unwrap();
    assert!(!resident.has_authority());
    let client = resident.client.clone();
    drop(resident);
    let result: MachineEnvelopeV2<ClientServiceStatusV1> = client
        .query(
            "GetClientServiceStatus",
            "external",
            &ClientEmptyRequestV1 {},
        )
        .await
        .unwrap();
    let status = result.data.unwrap();
    assert_eq!(status.daemon_role, "control_only");
    assert_eq!(status.gateway, ClientGatewayStateV1::NotComposed);
    assert!(!status.mutation_available);
    assert!(external.try_wait().unwrap().is_none());
    external.kill().unwrap();
    external.wait().unwrap();
}

// Bootstrap scans native targets during startup. Never inherit the developer's real HOME or
// CODEX_HOME; a protected native path outside this test is neither a fixture nor test-owned.
fn isolated_agent_home(test: &str) -> bool {
    if std::env::var("HIROUTE_DESKTOP_BOOTSTRAP_CASE").as_deref() == Ok(test) {
        return false;
    }
    let home = tempfile::Builder::new()
        .prefix("hr02-home-")
        .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
        .unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--nocapture", "--test-threads=1"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home.path())
        .env("CODEX_HOME", home.path().join(".codex"))
        .env("TMPDIR", home.path())
        .env("HIROUTE_DESKTOP_BOOTSTRAP_CASE", test)
        .status()
        .unwrap();
    assert!(status.success(), "isolated Desktop bootstrap case failed");
    true
}
