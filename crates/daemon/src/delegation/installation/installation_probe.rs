use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hiroute_diagnostics::event::{
    DiagnosticEvent, HarnessKind, InstallCheckEnd, OutcomeKind, PermissionOutcome,
    WorkerStageFailureCode, WorkerStageKind, WorkerStageOutcome,
};
use hiroute_diagnostics::runtime::DiagnosticsPort;
use hiroute_domain::delegation::{
    DelegationErrorV1, DelegationSessionBindingV1, WorkerExecutionIntentV1, WorkerHarnessV1,
    WorkerNetworkV1, WorkerPermissionPolicyV1, WorkspaceAccessV1, WorkspaceExecutionPermitV1,
};
use hiroute_domain::{ProtectedSecret, WorkspaceId};
use serde_json::Value;
use tokio::runtime::Builder;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::probe_server::{PROBE_DEADLINE, PROBE_TEXT, ProbeServer};
use super::{WorkerInstallationConfig, check_installation};
use crate::delegation::acp::{AcpRunInput, AcpRunJournal, AcpRunOutcome, AcpSessionStart};
use crate::delegation::lifecycle::{self, WorkerRunJournal};
use crate::delegation::local_worker::LocalWorkerPlatform;
use crate::delegation::platform::WorkerLaunchRequest;
use crate::delegation::profile::{
    CandidateWorkerProfile, ProfileInput, SessionRootUse, TaskSessionRoot, test_codex_catalog,
};

const PROBE_MODEL: &str = "hiroute/0011223344556677";

/// Test-only progress markers make an opt-in real-installation failure reproducible without
/// printing an artifact path, a temporary directory, or the fresh bearer challenge.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AcceptanceStage {
    Started,
    EntriesChecked,
    ServerReady,
    PrivateRootsReady,
    SessionRootReady,
    FirstProfileReady,
    FirstRunProved,
    LoadProved,
    CancelProfileReady,
    CancelProved,
    ServerFinished,
}

#[cfg(test)]
thread_local! {
    static LAST_ACCEPTANCE_STAGE: std::cell::Cell<AcceptanceStage> =
        const { std::cell::Cell::new(AcceptanceStage::Started) };
}

#[cfg(test)]
pub(crate) fn last_acceptance_stage() -> AcceptanceStage {
    LAST_ACCEPTANCE_STAGE.with(std::cell::Cell::get)
}

macro_rules! checkpoint {
    ($stage:ident) => {
        #[cfg(test)]
        LAST_ACCEPTANCE_STAGE.with(|current| current.set(AcceptanceStage::$stage));
    };
}

/// Explicit opt-in acceptance only; its result is never product installation admission.
pub(crate) fn run_installation_acceptance(
    config: &WorkerInstallationConfig,
    diagnostics: &DiagnosticsPort,
) -> Result<(), DelegationErrorV1> {
    let started = Instant::now();
    let result = run_installation_acceptance_inner(config, diagnostics);
    let outcome = match &result {
        Ok(_) => OutcomeKind::Completed,
        Err(_) => OutcomeKind::Failed,
    };
    diagnostics.emit(DiagnosticEvent::InstallCheckEnd(InstallCheckEnd {
        harness: harness_kind(config.harness),
        outcome,
        // The probe deliberately requests no tool or file permission, so no approval
        // outcome was observed rather than one being inferred from the answer text.
        permission: PermissionOutcome::NotRequired,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }));
    result
}

fn run_installation_acceptance_inner(
    config: &WorkerInstallationConfig,
    diagnostics: &DiagnosticsPort,
) -> Result<(), DelegationErrorV1> {
    checkpoint!(Started);
    if config.node_binary.is_none() {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    check_installation(config)?;
    checkpoint!(EntriesChecked);
    let adapter = &config.adapter;
    let harness = &config.harness_binary;
    let node = config
        .node_binary
        .as_deref()
        .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
    let probe_harness = harness_kind(config.harness);
    let challenge = fresh_challenge()?;
    let server = ProbeServer::bind(challenge.clone(), PROBE_MODEL, config.harness)?;
    checkpoint!(ServerReady);
    let root = ProbeRoot::new()?;
    checkpoint!(PrivateRootsReady);
    let session = TaskSessionRoot::prepare(
        &root.sessions,
        &WorkspaceId::parse("worker-probe").map_err(|_| DelegationErrorV1::InvalidArguments)?,
        "root",
        "task",
        config.harness,
        SessionRootUse::New,
    )?;
    checkpoint!(SessionRootReady);
    let platform = LocalWorkerPlatform::default();
    let first = build_profile(
        config.harness,
        adapter,
        harness,
        node,
        &root.workspace,
        &root.runs.join("first"),
        &session,
        server.address(),
        protected_challenge(&challenge)?,
    )?;
    checkpoint!(FirstProfileReady);
    diagnostics.worker_stage_begin(probe_harness, WorkerStageKind::ProbePrompt);
    let first = run_normal(
        &platform,
        config.harness,
        first,
        AcpSessionStart::New,
        "probe-new",
        Some((diagnostics, config.harness)),
    );
    diagnostics.worker_stage_end(
        probe_harness,
        WorkerStageKind::ProbePrompt,
        stage_outcome(WorkerStageKind::ProbePrompt, &first),
    );
    let first = match first {
        Ok(outcome) => outcome,
        Err(error) => {
            #[cfg(test)]
            eprintln!(
                "worker probe safe event: first-run-failed loopback-requests={}",
                server.request_count()
            );
            return Err(error);
        }
    };
    checkpoint!(FirstRunProved);
    diagnostics.worker_stage_begin(probe_harness, WorkerStageKind::ProbeLoad);
    let loaded = probe_load(
        &platform,
        config.harness,
        adapter,
        harness,
        node,
        &root,
        &session,
        server.address(),
        protected_challenge(&challenge)?,
        &first.session,
    );
    diagnostics.worker_stage_end(
        probe_harness,
        WorkerStageKind::ProbeLoad,
        stage_outcome(WorkerStageKind::ProbeLoad, &loaded),
    );
    loaded?;
    checkpoint!(LoadProved);

    let cancel_profile = build_profile(
        config.harness,
        adapter,
        harness,
        node,
        &root.workspace,
        &root.runs.join("cancel"),
        &session,
        server.address(),
        protected_challenge(&challenge)?,
    )?;
    checkpoint!(CancelProfileReady);
    diagnostics.worker_stage_begin(probe_harness, WorkerStageKind::Loopback);
    let cancel = run_cancel(&platform, cancel_profile, "probe-cancel", &server);
    // The listener thread stops only after it has either served a full valid native request or
    // reported its own failure, so this is the loopback reachability fact.
    let served = server.finish();
    checkpoint!(ServerFinished);
    let loopback_outcome = match (&cancel, &served) {
        (Ok(()), Ok(())) => WorkerStageOutcome::Completed,
        (Err(error), _) => failure_outcome(WorkerStageKind::Loopback, error),
        // The listener's own failure is a loopback contract failure; it carries no domain error.
        (_, Err(_)) => WorkerStageOutcome::Failed {
            code: WorkerStageFailureCode::ContractFailed,
        },
    };
    diagnostics.worker_stage_end(probe_harness, WorkerStageKind::Loopback, loopback_outcome);
    if cancel.is_err() || served.is_err() {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    checkpoint!(CancelProved);

    Ok(())
}

/// A step ends only from the real return of its blocking call.
fn stage_outcome<T>(
    stage: WorkerStageKind,
    result: &Result<T, DelegationErrorV1>,
) -> WorkerStageOutcome {
    match result {
        Ok(_) => WorkerStageOutcome::Completed,
        Err(error) => failure_outcome(stage, error),
    }
}

/// The stable code mirrors the domain error variant; the error text never reaches a record.
/// The artifact and evidence steps have their own fixed code because their failures are not
/// transport errors.
fn failure_outcome(stage: WorkerStageKind, error: &DelegationErrorV1) -> WorkerStageOutcome {
    let code = match stage {
        WorkerStageKind::ArtifactMeasure => WorkerStageFailureCode::ArtifactUnavailable,
        WorkerStageKind::Verify => WorkerStageFailureCode::EvidenceRejected,
        _ => match error {
            DelegationErrorV1::ProtocolFailed | DelegationErrorV1::ResumeUnavailable => {
                WorkerStageFailureCode::ContractFailed
            }
            DelegationErrorV1::DeadlineExceeded | DelegationErrorV1::Cancelled => {
                WorkerStageFailureCode::DeadlineExceeded
            }
            DelegationErrorV1::CapabilityUnavailable
            | DelegationErrorV1::StorageUnavailable
            | DelegationErrorV1::PermissionDenied => WorkerStageFailureCode::CapabilityUnavailable,
            _ => WorkerStageFailureCode::Unknown,
        },
    };
    WorkerStageOutcome::Failed { code }
}

fn harness_kind(harness: WorkerHarnessV1) -> HarnessKind {
    match harness {
        WorkerHarnessV1::CodexCli => HarnessKind::Codex,
        WorkerHarnessV1::ClaudeCode => HarnessKind::Claude,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_profile(
    worker: WorkerHarnessV1,
    adapter: &Path,
    harness: &Path,
    node: &Path,
    workspace: &Path,
    private_root: &Path,
    session: &TaskSessionRoot,
    gateway: std::net::SocketAddr,
    token: ProtectedSecret,
) -> Result<CandidateWorkerProfile, DelegationErrorV1> {
    let now = now_ms()?;
    let permit = WorkspaceExecutionPermitV1 {
        permit_id: "probe".into(),
        generation: 1,
        root_identity: "root".into(),
        access: WorkspaceAccessV1::TrustedNative,
        tools: vec![],
        network: WorkerNetworkV1::Allowed,
        expires_at_ms: now.saturating_add(PROBE_DEADLINE.as_millis() as u64),
        max_run_ms: PROBE_DEADLINE.as_millis() as u64,
        max_concurrent: 1,
        revoked: false,
    };
    let execution = WorkerExecutionIntentV1 {
        root_identity: "root".into(),
        access: WorkspaceAccessV1::TrustedNative,
        tools: vec![],
        network: WorkerNetworkV1::Allowed,
        duration_ms: PROBE_DEADLINE.as_millis() as u64,
        delegation_depth: 1,
    };
    let catalog = (worker == WorkerHarnessV1::CodexCli).then(|| test_codex_catalog(PROBE_MODEL));
    CandidateWorkerProfile::build(ProfileInput {
        harness: worker,
        adapter,
        harness_binary: harness,
        node_binary: Some(node),
        private_root,
        session_root: session,
        workspace,
        alias: PROBE_MODEL,
        codex_catalog: catalog.as_deref(),
        native_effort: None,
        gateway,
        permit: &permit,
        execution: &execution,
        // Launch the same native-autonomous mapping used by the public default. The probe asks
        // for no tools, so it verifies config/start/load compatibility without becoming a
        // tool-permission capability database.
        permission_policy: WorkerPermissionPolicyV1::ApproveAll,
        admitted_at_ms: now,
        token,
    })
}

#[allow(clippy::too_many_arguments)]
fn probe_load(
    platform: &LocalWorkerPlatform,
    worker: WorkerHarnessV1,
    adapter: &Path,
    harness: &Path,
    node: &Path,
    root: &ProbeRoot,
    session: &TaskSessionRoot,
    gateway: std::net::SocketAddr,
    token: ProtectedSecret,
    previous: &DelegationSessionBindingV1,
) -> Result<(), DelegationErrorV1> {
    let profile = build_profile(
        worker,
        adapter,
        harness,
        node,
        &root.workspace,
        &root.runs.join("load"),
        session,
        gateway,
        token,
    )?;
    let loaded = run_normal(
        platform,
        worker,
        profile,
        AcpSessionStart::Load(previous.clone()),
        "probe-load",
        None,
    )?;
    if &loaded.session != previous {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    Ok(())
}

fn run_normal(
    platform: &LocalWorkerPlatform,
    profile_harness: WorkerHarnessV1,
    profile: CandidateWorkerProfile,
    session: AcpSessionStart,
    nonce: &str,
    session_watch: Option<(&DiagnosticsPort, WorkerHarnessV1)>,
) -> Result<AcpRunOutcome, DelegationErrorV1> {
    let journal = Arc::new(ProbeJournal {
        session_watch: session_watch.map(|(diagnostics, harness)| ProbeSessionWatch {
            diagnostics: diagnostics.clone(),
            harness,
            seen: AtomicBool::new(false),
        }),
        ..ProbeJournal::default()
    });
    let journal_for_run = Arc::clone(&journal);
    let cwd = profile.cwd.clone();
    let identity_contract = profile.identity_contract.clone();
    let session_meta = profile.session_meta.clone();
    let native_session_mode = Some(profile.native_session_mode().to_owned());
    let nonce = nonce.to_owned();
    let outcome = block_on(async move {
        let deadline = Instant::now() + PROBE_DEADLINE;
        let result = lifecycle::execute(
            platform,
            WorkerLaunchRequest {
                launch_nonce: nonce,
                profile,
                deadline_unix_ms: now_ms()?.saturating_add(PROBE_DEADLINE.as_millis() as u64),
            },
            AcpRunInput {
                cwd,
                prompt: "Reply with OK only. Do not use tools.".into(),
                session,
                identity_contract,
                session_meta,
                native_session_mode,
                authentication: None,
                deadline,
                cancellation: CancellationToken::new(),
            },
            journal_for_run,
        )
        .await?;
        release_probe(platform, &result)?;
        result.execution
    })?;
    // A Codex Worker receives the private alias catalog before startup. Its absence is a real
    // metadata regression even if the controlled model still answers successfully.
    if profile_harness == WorkerHarnessV1::CodexCli && outcome.text.contains("Model metadata for") {
        return Err(DelegationErrorV1::ProtocolFailed);
    }
    let final_line_matches = outcome.text.lines().last() == Some(PROBE_TEXT);
    let journal_completed = journal.completed();
    if !final_line_matches || outcome.content_incomplete || !journal_completed {
        #[cfg(test)]
        eprintln!(
            "worker probe safe event: unexpected-result stop-reason={} text-bytes={} text-lines={} contains-probe={} final-line-matches={} content-incomplete={} journal-completed={}",
            outcome.stop_reason,
            outcome.text.len(),
            outcome.text.lines().count(),
            outcome.text.contains(PROBE_TEXT),
            final_line_matches,
            outcome.content_incomplete,
            journal_completed,
        );
        return Err(DelegationErrorV1::ProtocolFailed);
    }
    Ok(outcome)
}

fn run_cancel(
    platform: &LocalWorkerPlatform,
    profile: CandidateWorkerProfile,
    nonce: &str,
    server: &ProbeServer,
) -> Result<(), DelegationErrorV1> {
    let journal = Arc::new(ProbeJournal::default());
    let cwd = profile.cwd.clone();
    let identity_contract = profile.identity_contract.clone();
    let session_meta = profile.session_meta.clone();
    let native_session_mode = Some(profile.native_session_mode().to_owned());
    let nonce = nonce.to_owned();
    let expected_request = server.begin_stall();
    block_on(async move {
        let cancellation = CancellationToken::new();
        let deadline = Instant::now() + PROBE_DEADLINE;
        let future = lifecycle::execute(
            platform,
            WorkerLaunchRequest {
                launch_nonce: nonce,
                profile,
                deadline_unix_ms: now_ms()?.saturating_add(PROBE_DEADLINE.as_millis() as u64),
            },
            AcpRunInput {
                cwd,
                prompt: "Wait for the local response. Do not use tools.".into(),
                session: AcpSessionStart::New,
                identity_contract,
                session_meta,
                native_session_mode,
                authentication: None,
                deadline,
                cancellation: cancellation.clone(),
            },
            journal,
        );
        tokio::pin!(future);
        let wait_for_request = server.wait_for_request(expected_request);
        tokio::pin!(wait_for_request);
        let result = tokio::select! {
            result = &mut future => return match result {
                Ok(_) => Err(DelegationErrorV1::ProtocolFailed),
                Err(error) => Err(error),
            },
            result = &mut wait_for_request => {
                result?;
                cancellation.cancel();
                tokio::time::sleep(Duration::from_millis(150)).await;
                server.release_stall();
                future.await?
            }
        };
        release_probe(platform, &result)?;
        match result.execution {
            Ok(outcome) if outcome.stop_reason == "cancelled" => Ok(()),
            Err(DelegationErrorV1::Cancelled) => Ok(()),
            _ => Err(DelegationErrorV1::ProtocolFailed),
        }
    })
}

fn release_probe(
    platform: &LocalWorkerPlatform,
    result: &lifecycle::WorkerRunResult,
) -> Result<(), DelegationErrorV1> {
    if result
        .stop
        .as_ref()
        .is_none_or(|evidence| !evidence.scope_stopped)
    {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    platform.release(&result.identity)
}

fn block_on<T>(
    future: impl std::future::Future<Output = Result<T, DelegationErrorV1>>,
) -> Result<T, DelegationErrorV1> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?
        .block_on(future)
}

fn fresh_challenge() -> Result<Vec<u8>, DelegationErrorV1> {
    let mut entropy = [0_u8; 24];
    getrandom::fill(&mut entropy).map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    let mut value = b"hiroute-probe-".to_vec();
    for byte in entropy {
        value.extend_from_slice(format!("{byte:02x}").as_bytes());
    }
    Ok(value)
}

fn protected_challenge(challenge: &[u8]) -> Result<ProtectedSecret, DelegationErrorV1> {
    ProtectedSecret::new(challenge.to_vec()).map_err(|_| DelegationErrorV1::InvalidArguments)
}

struct ProbeRoot {
    _root: tempfile::TempDir,
    sessions: std::path::PathBuf,
    runs: std::path::PathBuf,
    workspace: std::path::PathBuf,
}

impl ProbeRoot {
    fn new() -> Result<Self, DelegationErrorV1> {
        let mut builder = tempfile::Builder::new();
        builder.prefix("hiroute-worker-probe-");
        // tempfile uses the process umask for directories by default, which commonly leaves a
        // 0755 root on macOS/Linux.  The probe holds a fresh bearer in child-only environment,
        // so its top-level directory must be private before we create any child materials.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(fs::Permissions::from_mode(0o700));
        }
        let root = builder
            .tempdir()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        let path =
            fs::canonicalize(root.path()).map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        private_directory(&path)?;
        let sessions = path.join("sessions");
        let runs = path.join("runs");
        let workspace = path.join("workspace");
        for directory in [&sessions, &runs, &workspace] {
            create_private(directory)?;
        }
        Ok(Self {
            _root: root,
            sessions,
            runs,
            workspace,
        })
    }
}

fn create_private(path: &Path) -> Result<(), DelegationErrorV1> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
    private_directory(path)
}

fn private_directory(path: &Path) -> Result<(), DelegationErrorV1> {
    let metadata = fs::symlink_metadata(path).map_err(|_| DelegationErrorV1::StorageUnavailable)?;
    if !path.is_absolute() || !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(DelegationErrorV1::PermissionDenied);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != nix::unistd::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
            return Err(DelegationErrorV1::PermissionDenied);
        }
    }
    Ok(())
}

#[derive(Default)]
struct ProbeJournal {
    completed: AtomicBool,
    text: Mutex<String>,
    /// Present only for the run that is expected to create the native session.
    session_watch: Option<ProbeSessionWatch>,
}

struct ProbeSessionWatch {
    diagnostics: DiagnosticsPort,
    harness: WorkerHarnessV1,
    seen: AtomicBool,
}

impl ProbeJournal {
    fn completed(&self) -> bool {
        self.completed.load(Ordering::Acquire)
    }
}

impl AcpRunJournal for ProbeJournal {
    fn session_bound(&self, _: &DelegationSessionBindingV1) -> Result<(), DelegationErrorV1> {
        if let Some(watch) = &self.session_watch
            && !watch.seen.swap(true, Ordering::AcqRel)
        {
            // A milestone inside the running prompt step: the native session exists. It is
            // recorded once and never ends the step it was observed in.
            watch
                .diagnostics
                .worker_stage_note(harness_kind(watch.harness), WorkerStageKind::ProbeSession);
        }
        Ok(())
    }
    fn before_prompt(&self) -> Result<(), DelegationErrorV1> {
        Ok(())
    }
    fn text_update(&self, text: &str) -> Result<(), DelegationErrorV1> {
        let mut current = self
            .text
            .lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        if current.len().saturating_add(text.len()) > 1024 {
            return Err(DelegationErrorV1::ContentUnavailable);
        }
        current.push_str(text);
        Ok(())
    }
    fn allow_permission_once(&self, _: &Value) -> bool {
        false
    }
}

impl WorkerRunJournal for ProbeJournal {
    fn before_launch(&self) -> Result<(), DelegationErrorV1> {
        Ok(())
    }
    fn process_spawned(
        &self,
        _: &hiroute_domain::delegation::DelegationProcessBindingV1,
    ) -> Result<(), DelegationErrorV1> {
        Ok(())
    }
    fn stop_intent(&self) -> Result<(), DelegationErrorV1> {
        Ok(())
    }
    fn completed(&self, _: &AcpRunOutcome) -> Result<(), DelegationErrorV1> {
        self.completed.store(true, Ordering::Release);
        Ok(())
    }
    fn execution_failed(&self, _: DelegationErrorV1) -> Result<(), DelegationErrorV1> {
        Ok(())
    }
    fn spawned_unrecorded(&self) -> Result<(), DelegationErrorV1> {
        Ok(())
    }
}

fn now_ms() -> Result<u64, DelegationErrorV1> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DelegationErrorV1::DeadlineExceeded)
        .and_then(|value| {
            u64::try_from(value.as_millis()).map_err(|_| DelegationErrorV1::DeadlineExceeded)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_root_is_private_before_it_holds_child_configuration() {
        let root = ProbeRoot::new().expect("probe root must be private");
        private_directory(root._root.path()).expect("top-level root must be private");
        private_directory(&root.sessions).expect("session root must be private");
        private_directory(&root.runs).expect("run root must be private");
        private_directory(&root.workspace).expect("workspace root must be private");
    }
}
