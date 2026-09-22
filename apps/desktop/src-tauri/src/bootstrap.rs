//! Native-owned child and three private inherited channels. Never exposed over WebView IPC.
use command_fds::{CommandFdExt, FdMapping};
use fs2::FileExt;
use hiroute_application_api::{CanonicalDigest, ComputeCandidateRefV2, RevisionSetV1};
use hiroute_client_core::{Client, LocalEndpoint};
use hiroute_diagnostics::event::{
    ChildExit, DiagnosticEvent, ReadyDirection, ReadyIo, ReadyIoError, StageOutcome,
    StartupFailureCode, StartupStage,
};
use hiroute_diagnostics::runtime::DiagnosticsPort;
use serde::Deserialize;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

use crate::native_diagnostics::NativeDiagnostics;

const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(210);
pub enum ApplyPurpose {
    AgentPlan,
    SourcePrice,
}

pub struct Resident {
    pub client: Client,
    owned: Option<OwnedChild>,
    _lock: File,
}

struct OwnedChild {
    child: Child,
    shutdown: Option<File>,
    capability: File,
    ack: File,
    authority_healthy: bool,
    manual_input_candidates: BTreeSet<String>,
    diagnostics: Option<DiagnosticsPort>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ready {
    schema: String,
    role: String,
    control_endpoint: PathBuf,
    gateway_listen: std::net::SocketAddr,
    process_id: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartupFailure {
    schema: String,
    code: StartupFailureCode,
}

fn decode_startup_failure(value: &serde_json::Value) -> Result<Option<&'static str>, &'static str> {
    if value.get("schema").and_then(serde_json::Value::as_str)
        != Some("hiroute.daemon-startup-failure/v1")
    {
        return Ok(None);
    }
    let failure: StartupFailure =
        serde_json::from_value(value.clone()).map_err(|_| "DAEMON_READY_INVALID")?;
    if failure.schema != "hiroute.daemon-startup-failure/v1" {
        return Err("DAEMON_READY_INVALID");
    }
    Ok(Some(match failure.code {
        StartupFailureCode::StorageUnavailable => "DAEMON_STORAGE_UNREADABLE",
        StartupFailureCode::ReleaseFactsInvalid => "DAEMON_RELEASE_INVALID",
        StartupFailureCode::DependencyUnavailable => "DAEMON_DEPENDENCY_UNAVAILABLE",
        StartupFailureCode::GatewayUnavailable => "DAEMON_GATEWAY_UNAVAILABLE",
        StartupFailureCode::ControlUnavailable => "DAEMON_CONTROL_UNAVAILABLE",
        StartupFailureCode::Cancelled => "DAEMON_START_CANCELLED",
        StartupFailureCode::InvalidConfiguration
        | StartupFailureCode::WorkerRejected
        | StartupFailureCode::ReadyChannelFailed
        | StartupFailureCode::Internal => "DAEMON_START_FAILED",
    }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ack {
    schema: String,
    registration_id: String,
    registered: bool,
}

impl Resident {
    pub fn open(root: &Path, daemon: &Path) -> Result<Self, String> {
        Self::open_managed(
            root,
            daemon,
            &AtomicBool::new(false),
            &NativeDiagnostics::disabled(),
        )
    }

    pub(crate) fn open_managed(
        root: &Path,
        daemon: &Path,
        cancelled: &AtomicBool,
        diagnostics: &NativeDiagnostics,
    ) -> Result<Self, String> {
        Self::open_resident(root, daemon, cancelled, diagnostics, None)
    }

    /// The run loop may pass the desktop.lock this process acquired synchronously in setup,
    /// so a duplicate host is detected before any window exists and the lock never changes
    /// hands between detection and ownership.
    pub(crate) fn open_resident(
        root: &Path,
        daemon: &Path,
        cancelled: &AtomicBool,
        diagnostics: &NativeDiagnostics,
        preacquired_lock: Option<File>,
    ) -> Result<Self, String> {
        let stages = diagnostics.port();
        stages.stage_begin(StartupStage::RootValidate);
        if cancelled.load(Ordering::SeqCst) {
            stages.stage_end(
                StartupStage::RootValidate,
                StageOutcome::Failed {
                    code: StartupFailureCode::Cancelled,
                },
            );
            return Err("DAEMON_START_CANCELLED".into());
        }
        finish_stage(
            &stages,
            StartupStage::RootValidate,
            private_dir(root),
            StartupFailureCode::StorageUnavailable,
        )?;
        #[cfg(target_os = "macos")]
        {
            let standalone = hiroute_host_runtime::StandaloneLayout::from_environment()
                .map_err(|_| "STANDALONE_LAYOUT_INVALID")?;
            let standalone_control =
                hiroute_client_core::LocalEndpoint::from_runtime_root(&standalone.runtime_root);
            if std::fs::symlink_metadata(&standalone.marker_path).is_ok()
                || std::fs::symlink_metadata(standalone_control.path()).is_ok()
                || std::fs::symlink_metadata(standalone.protected_input_socket()).is_ok()
            {
                return Err("STANDALONE_INSTALLATION_CONFLICT".into());
            }
        }
        stages.stage_begin(StartupStage::ResidentLock);
        let mut lock = match preacquired_lock {
            Some(lock) => lock,
            None => acquire_host_lock(root)?,
        };
        stages.stage_end(StartupStage::ResidentLock, StageOutcome::Completed);
        let runtime = root.join("run");
        private_dir(&runtime)?;
        let endpoint = LocalEndpoint::from_runtime_root(&runtime);
        if (std::fs::symlink_metadata(endpoint.path()).is_ok()
            || std::fs::symlink_metadata(runtime.join("hiroute/agent-grant-v1.sock")).is_ok())
            && !crate::resident_ownership::recoverable(&mut lock, &runtime)?
        {
            // An externally owned daemon: no managed child to report.
            return Ok(Self {
                client: Client::new("hiroute-desktop", endpoint)
                    .with_diagnostics(diagnostics.context()),
                owned: None,
                _lock: lock,
            });
        }
        stages.stage_begin(StartupStage::ArtifactValidate);
        let binary = match std::fs::canonicalize(daemon) {
            Ok(binary) if binary.is_file() => binary,
            _ => {
                stages.stage_end(
                    StartupStage::ArtifactValidate,
                    StageOutcome::Failed {
                        code: StartupFailureCode::DependencyUnavailable,
                    },
                );
                return Err("DAEMON_BINARY_UNAVAILABLE".into());
            }
        };
        stages.stage_end(StartupStage::ArtifactValidate, StageOutcome::Completed);
        let (shutdown_read, shutdown_write) = pipe()?;
        let (cap_read, cap_write) = pipe()?;
        let (ack_read, ack_write) = pipe()?;
        // The stable host address: a restart rebinds exactly the recorded address. A publication
        // (gateway.lkg) pins clients to it, so only the previously served address may be reused.
        let last_served = crate::resident_ownership::recorded_gateway_address(&mut lock)?;
        let publication_installed = std::fs::symlink_metadata(root.join("gateway.lkg")).is_ok();
        let reservation =
            crate::gateway_address::reserve(root, last_served, publication_installed)?;
        let listen = reservation.listen;
        let mut listener_activation = crate::gateway_address::ActivationGuard::new(root);
        let mut command = Command::new(&binary);
        #[cfg(feature = "desktop-pilot")]
        let daemon_stderr = Stdio::inherit();
        #[cfg(not(feature = "desktop-pilot"))]
        let daemon_stderr = Stdio::null();
        command
            .args(["--role", "all", "--storage-root"])
            .arg(root.join("storage"))
            .arg("--runtime-root")
            .arg(&runtime)
            .arg("--listen")
            .arg(listen.to_string())
            .arg("--lkg")
            .arg(root.join("gateway.lkg"))
            .args([
                "--shutdown-fd",
                "3",
                "--capability-fd",
                "4",
                "--capability-ack-fd",
                "5",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(daemon_stderr);
        if diagnostics.root.is_absolute() {
            command.arg("--diagnostics-root").arg(&diagnostics.root);
            if let Some(session) = diagnostics.parent_session {
                command
                    .arg("--diagnostics-parent-session")
                    .arg(session.to_hex());
            }
        }
        // Only an explicit desktop-pilot build resolves this override, and the managed daemon
        // receives just the parsed level as an ordinary non-secret argument.
        if let Some(level) = crate::native_diagnostics::level_override() {
            command
                .arg("--diagnostic-level-override")
                .arg(level.as_str());
        }
        if let Some((cpa, expected_sha256)) =
            crate::development_cpa::adjacent_to(&binary).unwrap_or(None)
        {
            // Keep optional CPA failures inside the subscription boundary. The managed
            // locator verifies the final signed bytes before execution.
            command
                .arg("--cpa-binary")
                .arg(cpa)
                .arg("--cpa-sha256")
                .arg(expected_sha256);
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(codex_desktop_engine) = hiroute_desktop_host_effects::codex_desktop_engine()
            {
                command
                    .arg("--codex-desktop-engine")
                    .arg(codex_desktop_engine);
            }
        }
        command
            .fd_mappings(vec![
                FdMapping {
                    parent_fd: shutdown_read.into(),
                    child_fd: 3,
                },
                FdMapping {
                    parent_fd: cap_read.into(),
                    child_fd: 4,
                },
                FdMapping {
                    parent_fd: ack_write.into(),
                    child_fd: 5,
                },
            ])
            .map_err(|_| "PROTECTED_CHANNEL_UNAVAILABLE")?;
        reservation.release(); // A port race fails this exact target; it never selects another.
        stages.stage_begin(StartupStage::Spawn);
        let child = match command.spawn() {
            Ok(child) => child,
            Err(_) => {
                stages.stage_end(
                    StartupStage::Spawn,
                    StageOutcome::Failed {
                        code: StartupFailureCode::DependencyUnavailable,
                    },
                );
                return Err("DAEMON_START_FAILED".into());
            }
        };
        stages.stage_end(StartupStage::Spawn, StageOutcome::Completed);
        drop(command);
        let mut owned = OwnedChild {
            child,
            shutdown: Some(shutdown_write),
            capability: cap_write,
            ack: ack_read,
            authority_healthy: true,
            manual_input_candidates: BTreeSet::new(),
            diagnostics: Some(stages.clone()),
        };
        let mut stdout = owned
            .child
            .stdout
            .take()
            .ok_or("DAEMON_READY_UNAVAILABLE")?;
        nonblocking(&stdout)?;
        nonblocking(&owned.capability)?;
        nonblocking(&owned.ack)?;
        stages.stage_begin_with_budget(
            StartupStage::ReadyWait,
            Some(DAEMON_READY_TIMEOUT.as_millis() as u64),
        );
        let ready_started = Instant::now();
        let ready_read = read_frame(&mut stdout, 4096, DAEMON_READY_TIMEOUT, Some(cancelled));
        stages
            .handle()
            .try_emit(DiagnosticEvent::ReadyRead(ReadyIo {
                direction: ReadyDirection::Read,
                ok: ready_read.is_ok(),
                elapsed_ms: ready_started.elapsed().as_millis() as u64,
                error: ready_read
                    .as_ref()
                    .err()
                    .map(|code| classify_ready_read(code)),
                os_errno: None,
            }));
        match &ready_read {
            Ok(_) => stages.stage_end(StartupStage::ReadyWait, StageOutcome::Completed),
            Err(code) => stages.stage_end(
                StartupStage::ReadyWait,
                StageOutcome::Failed {
                    code: if code.contains("CANCELLED") {
                        StartupFailureCode::Cancelled
                    } else {
                        StartupFailureCode::ReadyChannelFailed
                    },
                },
            ),
        }
        let bytes = ready_read?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "DAEMON_READY_INVALID")?;
        if let Some(code) = decode_startup_failure(&value)? {
            return Err(code.into());
        }
        let ready: Ready = serde_json::from_value(value).map_err(|_| "DAEMON_READY_INVALID")?;
        let endpoint = LocalEndpoint::for_child(&runtime, owned.child.id());
        if ready.schema != "hiroute.daemon-ready/v1"
            || ready.role != "all"
            || ready.process_id != owned.child.id()
            || ready.control_endpoint != endpoint.path()
            || ready.gateway_listen != listen
        {
            return Err("DAEMON_READY_MISMATCH".into());
        }
        crate::resident_ownership::record(&mut lock, &runtime, owned.child.id(), Some(&listen))?;
        listener_activation.succeeded(listen)?;
        Ok(Self {
            client: Client::new("hiroute-desktop", endpoint)
                .with_diagnostics(diagnostics.context()),
            owned: Some(owned),
            _lock: lock,
        })
    }
    pub fn has_authority(&mut self) -> bool {
        self.owned.as_mut().is_some_and(|owned| {
            owned.authority_healthy && matches!(owned.child.try_wait(), Ok(None))
        })
    }
    pub fn register(
        &mut self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "ApplyAgentPlanChange")
    }
    pub fn register_for(
        &mut self,
        purpose: ApplyPurpose,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        let operation = match purpose {
            ApplyPurpose::AgentPlan => "ApplyAgentPlanChange",
            ApplyPurpose::SourcePrice => "ApplyPriceOverrideChange",
        };
        self.register_operation(digest, revisions, operation)
    }
    pub(crate) fn register_observation(
        &mut self,
        request: &hiroute_application_api::ObservationReadRequestV2,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        let digest = CanonicalDigest::of(request).map_err(|_| "OBSERVATION_REQUEST_INVALID")?;
        self.register_operation(&digest, revisions, request.protected_operation())
    }
    pub(crate) fn register_classifier_diagnostic(
        &mut self,
        request: &hiroute_application_api::ClassifierDecisionTestRequestV1,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        let digest = CanonicalDigest::of(request).map_err(|_| "CLASSIFIER_TEST_INVALID")?;
        self.register_operation(&digest, revisions, "TestClassifierDecision")
    }
    #[cfg(feature = "desktop-runtime")]
    pub(crate) fn register_retention(
        &mut self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
        apply: bool,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(
            digest,
            revisions,
            if apply {
                "ApplySessionDeletionV2"
            } else {
                "PreviewSessionDeletionV2"
            },
        )
    }
    pub fn register_agent_settings(
        &mut self,
        restore: bool,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(
            digest,
            revisions,
            if restore {
                "ApplyAgentConnectionRestore"
            } else {
                "ApplyAgentConnectionChange"
            },
        )
    }
    pub fn register_agent_check(
        &mut self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "CheckAgentConnection")
    }
    pub fn register_model_check(
        &mut self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "CheckNativeModelConnection")
    }
    pub fn register_registered_model_check(
        &mut self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "CheckRegisteredModelConnection")
    }
    pub fn register_discovered_model_prepare(
        &mut self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "PrepareDiscoveredModelConnection")
    }
    pub fn register_saved_model_check(
        &mut self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "CheckSavedModelConnection")
    }
    pub fn register_model_save(
        &mut self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "ApplyComputeSave")
    }
    pub fn register_subscription_check(
        &mut self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "ApplySubscriptionCheck")
    }
    pub fn register_subscription_release(
        &mut self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "ReleaseSubscriptionCheck")
    }
    pub fn register_operation_cancel(
        &mut self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
    ) -> Result<Zeroizing<String>, String> {
        self.register_operation(digest, revisions, "CancelOperation")
    }
    pub fn register_model_input(
        &mut self,
        secret: Zeroizing<String>,
    ) -> Result<ComputeCandidateRefV2, String> {
        self.register_protected_input(secret, "candidate/native/")
    }
    pub fn register_agent_token_input(
        &mut self,
        secret: Zeroizing<String>,
    ) -> Result<ComputeCandidateRefV2, String> {
        self.register_protected_input(secret, "candidate/native/agent-token-")
    }
    fn register_protected_input(
        &mut self,
        secret: Zeroizing<String>,
        prefix: &str,
    ) -> Result<ComputeCandidateRefV2, String> {
        if secret.is_empty() || secret.len() > 32 * 1024 || !self.has_authority() {
            return Err("PROTECTED_INPUT_INVALID".into());
        }
        let random = crate::random_id()?;
        let candidate = ComputeCandidateRefV2 {
            candidate_ref: format!("{prefix}{random}"),
            candidate_revision: 1,
        };
        let result = self.register_model_input_once(&candidate, &secret);
        if result.is_err() {
            if let Some(owned) = &mut self.owned {
                owned.authority_healthy = false;
            }
            return result.map(|()| candidate);
        }
        self.owned
            .as_mut()
            .ok_or("TRUSTED_AUTHORITY_UNAVAILABLE")?
            .manual_input_candidates
            .insert(candidate.candidate_ref.clone());
        Ok(candidate)
    }
    fn register_model_input_once(
        &mut self,
        candidate: &ComputeCandidateRefV2,
        secret: &str,
    ) -> Result<(), String> {
        let owned = self.owned.as_mut().ok_or("TRUSTED_AUTHORITY_UNAVAILABLE")?;
        let id = crate::random_id()?;
        #[derive(serde::Serialize)]
        struct Registration<'a> {
            schema: &'static str,
            registration_id: &'a str,
            candidate_ref: &'a str,
            candidate_revision: u64,
            secret: &'a str,
        }
        let mut bytes = Zeroizing::new(
            serde_json::to_vec(&Registration {
                schema: "hiroute.protected-input/v1",
                registration_id: &id,
                candidate_ref: &candidate.candidate_ref,
                candidate_revision: candidate.candidate_revision,
                secret,
            })
            .map_err(|_| "PROTECTED_INPUT_INVALID")?,
        );
        bytes.push(b'\n');
        write_frame(&mut owned.capability, &bytes)?;
        receive_ack(&mut owned.ack, &id, Duration::from_secs(2))
    }
    pub fn release_model_input(&mut self, candidate: &ComputeCandidateRefV2) -> Result<(), String> {
        candidate
            .validate_shape()
            .map_err(|_| "PROTECTED_INPUT_INVALID")?;
        if !self.has_authority() {
            return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
        }
        let owned = self.owned.as_mut().ok_or("TRUSTED_AUTHORITY_UNAVAILABLE")?;
        if !owned
            .manual_input_candidates
            .contains(&candidate.candidate_ref)
        {
            return Ok(());
        }
        let id = crate::random_id()?;
        #[derive(serde::Serialize)]
        struct Release<'a> {
            schema: &'static str,
            registration_id: &'a str,
            candidate_ref: &'a str,
        }
        let mut bytes = Zeroizing::new(
            serde_json::to_vec(&Release {
                schema: "hiroute.protected-input/v1",
                registration_id: &id,
                candidate_ref: &candidate.candidate_ref,
            })
            .map_err(|_| "PROTECTED_INPUT_INVALID")?,
        );
        bytes.push(b'\n');
        let result = write_frame(&mut owned.capability, &bytes)
            .and_then(|()| receive_ack(&mut owned.ack, &id, Duration::from_secs(2)));
        if result.is_ok() {
            owned
                .manual_input_candidates
                .remove(&candidate.candidate_ref);
        } else {
            owned.authority_healthy = false;
        }
        result
    }
    fn register_operation(
        &mut self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
        operation: &'static str,
    ) -> Result<Zeroizing<String>, String> {
        if !self.has_authority() {
            return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
        }
        let result = self.register_once(digest, revisions, operation);
        if result.is_err() {
            // A partial/timed-out channel cannot be reused for a different registration.
            // Keep the owned daemon alive for reads; a new native session re-establishes authority.
            if let Some(owned) = &mut self.owned {
                owned.authority_healthy = false;
            }
        }
        result
    }
    fn register_once(
        &mut self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
        operation: &'static str,
    ) -> Result<Zeroizing<String>, String> {
        let owned = self.owned.as_mut().ok_or("TRUSTED_AUTHORITY_UNAVAILABLE")?;
        let capability = Zeroizing::new(crate::random_id()?);
        let id = crate::random_id()?;
        let expires = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "CLOCK_UNAVAILABLE")?
            .as_secs()
            + 60;
        // Serialize directly into a wiping buffer; never create a serde Value containing the token.
        #[derive(serde::Serialize)]
        struct Registration<'a> {
            schema: &'static str,
            registration_id: &'a str,
            capability: &'a str,
            principal_kind: &'static str,
            workspace_id: &'static str,
            operation_kind: &'static str,
            accepted_digest: &'a CanonicalDigest,
            expected_revisions: &'a RevisionSetV1,
            expires_at_unix: u64,
        }
        let mut bytes = Zeroizing::new(
            serde_json::to_vec(&Registration {
                schema: "hiroute.protected-apply-grant/v2",
                registration_id: &id,
                capability: &capability,
                principal_kind: "desktop",
                workspace_id: "personal/default",
                operation_kind: operation,
                accepted_digest: digest,
                expected_revisions: revisions,
                expires_at_unix: expires,
            })
            .map_err(|_| "REGISTRATION_INVALID")?,
        );
        bytes.push(b'\n');
        write_frame(&mut owned.capability, &bytes)?;
        receive_ack(&mut owned.ack, &id, Duration::from_secs(2))?;
        Ok(capability)
    }
}
fn receive_ack(reader: &mut impl Read, id: &str, timeout: Duration) -> Result<(), String> {
    let ack: Ack = serde_json::from_slice(&read_frame(reader, 1024, timeout, None)?)
        .map_err(|_| "REGISTRATION_ACK_INVALID")?;
    if ack.schema != "hiroute.protected-apply-ack/v2"
        || ack.registration_id != id
        || !ack.registered
    {
        return Err("REGISTRATION_ACK_MISMATCH".into());
    }
    Ok(())
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        drop(self.shutdown.take());
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut status = None;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(exited)) => {
                    status = Some(exited);
                    break;
                }
                Ok(None) => {}
                Err(_) => break,
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if status.is_none() {
            let _ = self.child.kill();
            status = self.child.wait().ok();
        }
        if let (Some(diagnostics), Some(status)) = (&self.diagnostics, status) {
            use std::os::unix::process::ExitStatusExt;
            // The verified exit of the child this parent owned, never a pid guess.
            diagnostics
                .handle()
                .try_emit(DiagnosticEvent::ChildExit(ChildExit {
                    exit_code: status.code(),
                    signal: status.signal(),
                    verified: true,
                }));
        }
    }
}

/// End a startup stage with the outcome of one fallible step.
fn finish_stage<T>(
    stages: &DiagnosticsPort,
    stage: StartupStage,
    result: Result<T, String>,
    failure: StartupFailureCode,
) -> Result<T, String> {
    match &result {
        Ok(_) => stages.stage_end(stage, StageOutcome::Completed),
        Err(_) => stages.stage_end(stage, StageOutcome::Failed { code: failure }),
    }
    result
}

fn classify_ready_read(code: &str) -> ReadyIoError {
    if code.contains("TIMEOUT") {
        ReadyIoError::Timeout
    } else if code.contains("CLOSED") {
        ReadyIoError::Closed
    } else {
        ReadyIoError::Io
    }
}
#[cfg(target_os = "linux")]
fn pipe() -> Result<(File, File), String> {
    // Linux cannot reopen socket descriptors via /dev/fd; the daemon accepts pipes too.
    let (read, write) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)
        .map_err(|_| "PROTECTED_CHANNEL_UNAVAILABLE")?;
    Ok((read.into(), write.into()))
}
#[cfg(not(target_os = "linux"))]
fn pipe() -> Result<(File, File), String> {
    let (read, write) =
        std::os::unix::net::UnixStream::pair().map_err(|_| "PROTECTED_CHANNEL_UNAVAILABLE")?;
    Ok((
        std::os::fd::OwnedFd::from(read).into(),
        std::os::fd::OwnedFd::from(write).into(),
    ))
}
fn nonblocking(file: &impl std::os::fd::AsFd) -> Result<(), String> {
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    let flags = fcntl(file, FcntlArg::F_GETFL).map_err(|_| "PROTECTED_CHANNEL_UNAVAILABLE")?;
    fcntl(
        file,
        FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
    )
    .map_err(|_| "PROTECTED_CHANNEL_UNAVAILABLE")?;
    Ok(())
}
fn read_frame(
    reader: &mut impl Read,
    limit: usize,
    timeout: Duration,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + timeout;
    let mut bytes = Vec::new();
    loop {
        if cancelled.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            return Err("DAEMON_START_CANCELLED".into());
        }
        if Instant::now() >= deadline {
            return Err("PROTECTED_CHANNEL_TIMEOUT".into());
        }
        let mut byte = [0u8];
        match reader.read(&mut byte) {
            Ok(0) => return Err("PROTECTED_CHANNEL_CLOSED".into()),
            Ok(_) if byte[0] == b'\n' => return Ok(bytes),
            Ok(_) => {
                if bytes.len() >= limit {
                    return Err("PROTECTED_CHANNEL_OVERSIZED".into());
                }
                bytes.push(byte[0]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(_) => return Err("PROTECTED_CHANNEL_UNAVAILABLE".into()),
        }
    }
}
fn write_frame(writer: &mut impl Write, mut bytes: &[u8]) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !bytes.is_empty() {
        if Instant::now() >= deadline {
            return Err("PROTECTED_CHANNEL_TIMEOUT".into());
        }
        match writer.write(bytes) {
            Ok(0) => return Err("PROTECTED_CHANNEL_CLOSED".into()),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(_) => return Err("PROTECTED_CHANNEL_UNAVAILABLE".into()),
        }
    }
    Ok(())
}
pub(crate) fn private_dir(path: &Path) -> Result<(), String> {
    crate::resident_ownership::validate_ancestors(path)?;
    if path.exists()
        && !std::fs::symlink_metadata(path)
            .map_err(|_| "PRIVATE_PATH_UNAVAILABLE")?
            .is_dir()
    {
        return Err("PRIVATE_PATH_INVALID".into());
    }
    std::fs::create_dir_all(path).map_err(|_| "PRIVATE_PATH_UNAVAILABLE")?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| "PRIVATE_PATH_UNAVAILABLE".into())
}

/// Creates the private root and takes the single-host desktop.lock. Another host process
/// holding the lock is reported as DESKTOP_ALREADY_RUNNING; any other failure keeps its
/// existing startup failure code.
pub(crate) fn acquire_host_lock(root: &Path) -> Result<File, String> {
    private_dir(root)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(root.join("desktop.lock"))
        .map_err(|_| "RESIDENT_LOCK_UNAVAILABLE")?;
    crate::resident_ownership::validate_lock(&lock)?;
    lock.try_lock_exclusive()
        .map_err(|_| "DESKTOP_ALREADY_RUNNING")?;
    Ok(lock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_interrupts_a_pending_daemon_ready_read() {
        let (mut reader, _writer) = std::os::unix::net::UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let cancelled = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(20));
                cancelled.store(true, Ordering::SeqCst);
            });
            let started = Instant::now();
            assert_eq!(
                read_frame(&mut reader, 4096, DAEMON_READY_TIMEOUT, Some(&cancelled)).unwrap_err(),
                "DAEMON_START_CANCELLED"
            );
            assert!(started.elapsed() < Duration::from_secs(2));
        });
    }

    #[test]
    fn startup_failure_frame_maps_storage_without_accepting_extra_payload() {
        assert_eq!(
            decode_startup_failure(&serde_json::json!({
                "schema": "hiroute.daemon-startup-failure/v1",
                "code": "storage_unavailable"
            })),
            Ok(Some("DAEMON_STORAGE_UNREADABLE"))
        );
        assert_eq!(
            decode_startup_failure(&serde_json::json!({
                "schema": "hiroute.daemon-startup-failure/v1",
                "code": "storage_unavailable",
                "message": "/private/data/path"
            })),
            Err("DAEMON_READY_INVALID")
        );
        assert_eq!(
            decode_startup_failure(&serde_json::json!({
                "schema": "hiroute.daemon-ready/v1"
            })),
            Ok(None)
        );
    }

    #[test]
    fn cancelled_startup_does_not_create_state_or_launch_a_child() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("not-created");
        let result = Resident::open_managed(
            &root,
            &root.join("hirouted"),
            &AtomicBool::new(true),
            &NativeDiagnostics::disabled(),
        );
        assert_eq!(result.err().as_deref(), Some("DAEMON_START_CANCELLED"));
        assert!(!root.exists());
    }

    #[test]
    fn a_second_descriptor_of_a_held_host_lock_reports_the_duplicate() {
        // Parallel process tests may fork while this test holds a file lock. A child
        // retains the same open-file description until exec, even with CLOEXEC. Exercise
        // exact close/reclaim ordering in one isolated test process instead.
        const CASE: &str =
            "bootstrap::tests::a_second_descriptor_of_a_held_host_lock_reports_the_duplicate";
        const CHILD: &str = "HIROUTE_ISOLATED_LOCK_TEST";
        if std::env::var(CHILD).as_deref() != Ok(CASE) {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", CASE])
                .env(CHILD, CASE)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success()
                    && stdout
                        .lines()
                        .any(|line| line == format!("test {CASE} ... ok")),
                "isolated lock test must execute its exact case: {stdout} {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("host");
        let held = acquire_host_lock(&root).unwrap();
        assert_eq!(
            acquire_host_lock(&root).unwrap_err(),
            "DESKTOP_ALREADY_RUNNING",
            "flock on a second descriptor of the same lock file conflicts within one process"
        );
        // A duplicated descriptor models the shared open-file description inherited
        // across fork. Closing one descriptor alone must not release the lease.
        let inherited = held.try_clone().unwrap();
        drop(held);
        assert_eq!(
            acquire_host_lock(&root).unwrap_err(),
            "DESKTOP_ALREADY_RUNNING"
        );
        drop(inherited);
        assert!(acquire_host_lock(&root).is_ok());
    }

    #[test]
    fn acknowledgement_requires_the_exact_successful_registration() {
        for (id, registered, expected) in [
            ("current", true, true),
            ("older", true, false),
            ("current", false, false),
        ] {
            let bytes = format!(
                "{{\"schema\":\"hiroute.protected-apply-ack/v2\",\"registration_id\":\"{id}\",\"registered\":{registered}}}\n"
            );
            assert_eq!(
                receive_ack(&mut bytes.as_bytes(), "current", Duration::from_secs(1)).is_ok(),
                expected
            );
        }
        for bytes in [b"".as_slice(), b"{}", b"{}\n", &[b'a'; 1026]] {
            let mut reader: &[u8] = bytes;
            assert!(receive_ack(&mut reader, "current", Duration::from_secs(1)).is_err());
        }
    }
    #[test]
    fn stalled_acknowledgement_and_registration_writes_have_absolute_deadlines() {
        struct Stalled;
        impl Read for Stalled {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::WouldBlock.into())
            }
        }
        let started = Instant::now();
        assert_eq!(
            receive_ack(&mut Stalled, "current", Duration::from_millis(20)).unwrap_err(),
            "PROTECTED_CHANNEL_TIMEOUT"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Ok(0)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert_eq!(
            write_frame(&mut Closed, b"registration").unwrap_err(),
            "PROTECTED_CHANNEL_CLOSED"
        );
    }
}
