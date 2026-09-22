use hiroute_daemon::delegation::{
    local_worker::LocalWorkerPlatform,
    platform::{ReadyWorker, WorkerLaunchRequest, WorkerPlatformPort},
    profile::{CandidateWorkerProfile, ProfileInput, SessionRootUse, TaskSessionRoot},
};
use hiroute_domain::{ProtectedSecret, delegation::*};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncBufReadExt, BufReader};
use zeroize::Zeroizing;

pub fn request(root: &Path, cwd: &Path, nonce: &str, mode: &str) -> WorkerLaunchRequest {
    let binary = std::env::current_exe().unwrap();
    let execution = WorkerExecutionIntentV1 {
        root_identity: "workspace".into(),
        access: WorkspaceAccessV1::TrustedNative,
        tools: vec![WorkerToolV1::Shell],
        network: WorkerNetworkV1::Allowed,
        duration_ms: 60_000,
        delegation_depth: 1,
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let permit = WorkspaceExecutionPermitV1 {
        permit_id: "permit".into(),
        generation: 1,
        root_identity: "workspace".into(),
        access: execution.access,
        tools: execution.tools.clone(),
        network: execution.network,
        expires_at_ms: now + 120_000,
        max_run_ms: 60_000,
        max_concurrent: 2,
        revoked: false,
    };
    let session = TaskSessionRoot::prepare(
        cwd,
        &hiroute_domain::WorkspaceId::default(),
        "workspace",
        nonce,
        WorkerHarnessV1::CodexCli,
        SessionRootUse::New,
    )
    .unwrap();
    let builder_root = cwd.join(format!("build-{nonce}"));
    let mut profile = CandidateWorkerProfile::build(ProfileInput {
        claude_context_window: None,
        harness: WorkerHarnessV1::CodexCli,
        adapter: &binary,
        harness_binary: &binary,
        node_binary: None,
        private_root: &builder_root,
        session_root: &session,
        workspace: cwd,
        alias: "probe",
        codex_catalog: None,
        native_effort: None,
        gateway: "127.0.0.1:1".parse().unwrap(),
        permit: &permit,
        execution: &execution,
        permission_policy: WorkerPermissionPolicyV1::ApproveAll,
        admitted_at_ms: now,
        token: ProtectedSecret::new(b"probe-secret".to_vec()).unwrap(),
    })
    .unwrap();
    profile.private_root = root.to_owned();
    profile.args = ["--exact", "local_worker_probe", "--nocapture"]
        .into_iter()
        .map(PathBuf::from)
        .collect();
    profile.env.clear();
    profile
        .env
        .insert("HIROUTE_PROBE_MODE".into(), Zeroizing::new(mode.into()));
    profile.env.insert(
        "HIROUTE_PROBE_CWD".into(),
        Zeroizing::new(cwd.to_str().unwrap().into()),
    );
    WorkerLaunchRequest {
        launch_nonce: nonce.into(),
        profile,
        deadline_unix_ms: now + 60_000,
    }
}

pub async fn start(
    platform: &LocalWorkerPlatform,
    root: &Path,
    cwd: &Path,
    nonce: &str,
    mode: &str,
) -> (
    ReadyWorker,
    BufReader<hiroute_daemon::delegation::platform::WorkerStdout>,
) {
    let mut ready = platform
        .launch(request(root, cwd, nonce, mode))
        .await
        .unwrap();
    let stdout = std::mem::replace(&mut ready.stdout, Box::pin(tokio::io::empty()));
    let mut reader = BufReader::new(stdout);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let mut line = String::new();
            assert_ne!(reader.read_line(&mut line).await.unwrap(), 0);
            if line.contains("PROBE_READY") {
                break;
            }
        }
    })
    .await
    .unwrap();
    (ready, reader)
}

pub fn private_tempdir() -> std::io::Result<tempfile::TempDir> {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir()?;
    // The production consumer requires a private session store. Set permissions only on
    // this test-owned directory; don't rely on the remote runner's restrictive umask.
    std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700))?;
    Ok(temp)
}
