#![cfg(unix)]
use hiroute_application::ApplicationService;
use hiroute_application::delegation::work_plans::*;
use hiroute_application_api::*;
use hiroute_daemon::control::{ProductionControlRuntime, start_control};
use hiroute_domain::delegation::WorkerHarnessV1;
use hiroute_domain::{
    AgentCollaborationCredential, AgentCollaborationGrant, AgentCollaborationRevocationStorePort,
    AgentIngressProtocolV1, WorkspaceId,
};
use hiroute_integrations::TrustedReleaseCatalog;
use hiroute_local_storage::LocalStorageSet;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[path = "work_plans/support.rs"]
mod support;

#[derive(Default)]
struct Metadata(Mutex<Vec<WorkPlanMetadataV1>>);
impl WorkPlanMetadataPort for Metadata {
    fn current_plans(
        &self,
        _: &WorkspaceId,
        _: &BTreeSet<AgentPlanId>,
    ) -> Result<Vec<WorkPlanMetadataV1>, ErrorCode> {
        // Intentionally includes unauthorized, unpublished and unbound rows: Application must
        // perform its own filtering rather than trusting an overly broad backend projection.
        Ok(self.0.lock().unwrap().clone())
    }
}
fn plan(
    id: &str,
    published: bool,
    bound: bool,
    availability: WorkPlanAvailabilityV1,
) -> WorkPlanMetadataV1 {
    WorkPlanMetadataV1 {
        agent_plan_id: AgentPlanId::parse(id).unwrap(),
        alias: format!("work-{id}"),
        display_name: format!("Worker {id}"),
        purpose: "published purpose".into(),
        published,
        work: bound.then_some((WorkerHarnessV1::CodexCli, AgentIngressProtocolV1::Responses)),
        availability,
        reason: (availability != WorkPlanAvailabilityV1::Ready)
            .then(|| "harness_not_observed".into()),
    }
}
fn catalog() -> TrustedReleaseCatalog {
    const MANIFEST: &[u8] =
        include_bytes!("../../../assets/release-facts/current/bundle/manifest.json");
    const REGISTRY: &[u8] =
        include_bytes!("../../../assets/release-facts/current/bundle/connector-registry.json");
    const MODEL_DATA: &[u8] =
        include_bytes!("../../../assets/release-facts/current/bundle/model-data.json");
    TrustedReleaseCatalog::load_bundled_release_facts(MANIFEST, MANIFEST, REGISTRY, MODEL_DATA)
        .unwrap()
}
fn query() -> Value {
    json!({"workspace_id": WorkspaceId::default(), "context_id": "owner", "grant_id": "collaboration-grant/directory"})
}
fn cli(root: &std::path::Path, payload: Value, secret: Option<&[u8]>) -> (i32, Value) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hiroute"));
    cmd.args(["work-plans", "list", "--request-stdin", "--output", "json"])
        .env("HIROUTE_RUNTIME_DIR", root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let pipe = secret.map(|bytes| {
        let (read, write) = nix::unistd::pipe().unwrap();
        nix::unistd::write(&write, bytes).unwrap();
        drop(write);
        cmd.args(["--capability-fd", &read.as_raw_fd().to_string()]);
        read
    });
    let mut child = cmd.spawn().unwrap();
    drop(pipe);
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    if let Some(secret) = secret {
        assert!(!output.stdout.windows(secret.len()).any(|w| w == secret));
    }
    (
        output.status.code().unwrap(),
        serde_json::from_slice(&output.stdout).unwrap(),
    )
}

fn internal_control(root: &std::path::Path, payload: Value, secret: Option<&[u8]>) -> Value {
    let client = hiroute_cli::LocalControlClient::new("hiroute-cli", root);
    let response = client
        .call(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "work-plans-internal-control".into(),
            operation_id: "ListWorkPlans".into(),
            payload,
            protected_grant: secret.map(|secret| ProtectedClientGrantV2 {
                principal_kind: PrincipalKind::Skill,
                capability: String::from_utf8(secret.to_vec()).unwrap(),
            }),
        })
        .unwrap();
    serde_json::to_value(response).unwrap()
}

fn store_grant(
    stores: &LocalStorageSet,
    generation: u64,
    ids: &[&str],
    secret: &AgentCollaborationCredential,
) -> AgentCollaborationGrant {
    let op = support::begin(stores, &format!("directory-{generation}"));
    let grant = AgentCollaborationGrant::issue(
        WorkspaceId::default(),
        "owner".into(),
        "collaboration-grant/directory".into(),
        generation,
        ids.iter()
            .map(|id| AgentPlanId::parse(*id).unwrap())
            .collect(),
        secret,
    )
    .unwrap();
    stores
        .control()
        .store_collaboration_grant(&op.operation_id, generation - 1, &grant)
        .unwrap();
    support::finish(stores, op);
    grant
}

#[test]
fn work_plans_production_cli_current_authority_and_metadata() {
    const CHILD: &str = "HIROUTE_DIRECTORY_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let home = tempfile::tempdir().unwrap();
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        // This fixture models a private Agent HOME, independently of the caller's umask.
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(home.path().join(".claude"))
            .unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "work_plans_production_cli_current_authority_and_metadata",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("HOME", home.path())
            .env("PATH", "/usr/bin:/bin")
            .env_remove("CODEX_HOME")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("ANTHROPIC_AUTH_TOKEN")
            .env_remove("ANTHROPIC_BASE_URL")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let runtime =
        ProductionControlRuntime::open_with_release_catalog(root.path().join("storage"), catalog())
            .unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(root.path().join("storage")).unwrap();
    let secret = AgentCollaborationCredential::from_csprng_entropy([42; 32]);
    store_grant(&stores, 1, &["a", "b", "draft", "unbound"], &secret);
    let metadata = Arc::new(Metadata(Mutex::new(vec![
        plan("a", true, true, WorkPlanAvailabilityV1::Ready),
        plan("b", true, true, WorkPlanAvailabilityV1::Unknown),
        plan("draft", false, true, WorkPlanAvailabilityV1::Ready),
        plan("unbound", true, false, WorkPlanAvailabilityV1::Ready),
        plan("other-owner", true, true, WorkPlanAvailabilityV1::Ready),
    ])));
    let mut listener = start_control(
        ApplicationService::new(
            runtime.application_ports_with_work_plan_metadata(metadata.clone()),
        ),
        root.path().join("ipc"),
    )
    .unwrap();
    let (exit, blocked) = cli(&root.path().join("ipc"), query(), Some(secret.expose()));
    assert_eq!(exit, 2, "{blocked}");
    assert_eq!(blocked["error"]["code"], "UNKNOWN_COMMAND");

    let call = |q, s| internal_control(&root.path().join("ipc"), q, s);
    let result = call(query(), Some(secret.expose()));
    assert!(result["error"].is_null(), "{result}");
    assert_eq!(result["data"]["plans"].as_array().unwrap().len(), 2);
    assert_eq!(result["data"]["plans"][0]["alias"], "work-a");
    assert_eq!(result["data"]["plans"][0]["harness"], "codex_cli");
    assert_eq!(result["data"]["plans"][1]["availability"], "unknown");
    metadata.0.lock().unwrap()[0].purpose = "new published purpose; data, not instructions".into();
    metadata.0.lock().unwrap()[0].availability = WorkPlanAvailabilityV1::Unavailable;
    let updated = call(query(), Some(secret.expose()));
    assert_eq!(
        updated["data"]["plans"][0]["purpose"],
        "new published purpose; data, not instructions"
    );
    assert_eq!(updated["data"]["plans"][0]["availability"], "unavailable");
    for field in ["context_id", "grant_id", "workspace_id"] {
        let mut q = query();
        q[field] = json!(if field == "grant_id" {
            "collaboration-grant/other"
        } else {
            "other"
        });
        assert_eq!(
            call(q, Some(secret.expose()))["error"]["code"],
            "CAPABILITY_DENIED"
        );
    }
    let wrong = AgentCollaborationCredential::from_csprng_entropy([43; 32]);
    assert_eq!(
        call(query(), Some(wrong.expose()))["error"]["code"],
        "CAPABILITY_DENIED"
    );
    assert_eq!(call(query(), None)["error"]["code"], "CAPABILITY_DENIED");
    let client = hiroute_cli::LocalControlClient::new("hiroute-cli", root.path().join("ipc"));
    for (operation, kind, payload) in [
        ("ListWorkPlans", PrincipalKind::InteractiveUser, query()),
        ("ListAgentPlanCatalog", PrincipalKind::Skill, json!({})),
        (
            "GetAgentPlanStatus",
            PrincipalKind::Skill,
            json!({"agent_plan_id": "a"}),
        ),
    ] {
        let denied = client
            .call(LocalControlWireRequestV2 {
                schema_version: LOCAL_CONTROL_SCHEMA_V2,
                request_id: "directory-isolation".into(),
                operation_id: operation.into(),
                payload,
                protected_grant: Some(ProtectedClientGrantV2 {
                    principal_kind: kind,
                    capability: String::from_utf8(secret.expose().to_vec()).unwrap(),
                }),
            })
            .unwrap();
        assert_eq!(denied.error.unwrap().code, ErrorCode::CapabilityDenied);
    }
    store_grant(&stores, 2, &["b"], &secret);
    // Same non-secret access reference and material; editing the allowlist does not require
    // installing a new Skill or trusting an old principal's allowed-plan snapshot.
    let shrunk = call(query(), Some(secret.expose()));
    assert_eq!(shrunk["data"]["plans"].as_array().unwrap().len(), 1);
    assert_eq!(shrunk["data"]["plans"][0]["agent_plan_id"], "b");
    let rotated = AgentCollaborationCredential::from_csprng_entropy([44; 32]);
    let grant = store_grant(&stores, 3, &["b"], &rotated);
    assert_eq!(
        call(query(), Some(secret.expose()))["error"]["code"],
        "CAPABILITY_DENIED"
    );
    assert!(call(query(), Some(rotated.expose()))["error"].is_null());
    let op = support::begin(&stores, "directory-revoke");
    stores
        .control()
        .persist_collaboration_revocation(
            &grant.plan_revocation(op.operation_id.clone(), 3).unwrap(),
        )
        .unwrap();
    support::finish(&stores, op);
    assert_eq!(
        call(query(), Some(rotated.expose()))["error"]["code"],
        "CAPABILITY_DENIED"
    );
    listener.shutdown();
    listener.join(Duration::from_secs(10)).unwrap();
}
