//! Bounded native Live verification using the current managed configuration and Gateway receipt.
use super::LocalControlAdapter;
use hiroute_application::control::{AgentConnectionControlPort, ControlReadError};
use hiroute_application_api::{AgentCheckRequestV1, AgentLaunchDescriptorRequestV1};
use hiroute_domain::{
    AGENT_SURFACE_CHECK_SCHEMA, AgentModelSurfaceV2, AgentSurfaceCheckRecordV1,
    AgentSurfaceCheckStateV1, CanonicalDigest, ContentMode, ControlRepositoryPort,
    FactsCompleteness, FrozenExecutionTrustV1, GatewayPublicationSnapshotProjectionV3,
    ObservationQueryError, ObservationQueryPort, PublicationRepositoryPort, RequestOutcome,
    WorkspaceId,
};
use hiroute_gateway::server::core_runtime::observation::{
    NativeAgentObservationIdentityV1, derive_native_agent_observation_session_id,
};
use hiroute_integrations::ManagedClaudeProcessV1;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[path = "agent_live_check_qoder.rs"]
mod qoder;

const LIVE_CHECK_BUDGET: Duration = Duration::from_secs(120);
const RECEIPT_WAIT_BUDGET: Duration = Duration::from_secs(5);
const OUTPUT_LIMIT: usize = 1024 * 1024;
const LIVE_PROMPT_RESPONSE: &str = "HIROUTE_LIVE_CHECK_OK";
const LIVE_PROMPT: &str =
    "Reply with exactly HIROUTE_LIVE_CHECK_OK. Do not use tools or perform any other action.";

const CLIENT_FAILED: &str = "LIVE_CLIENT_FAILED";
const CLIENT_OUTPUT_INVALID: &str = "LIVE_CLIENT_OUTPUT_INVALID";
const CHECK_TIMEOUT: &str = "LIVE_CHECK_TIMEOUT";
const RECEIPT_MISSING: &str = "LIVE_RECEIPT_MISSING";
const RECEIPT_MISMATCH: &str = "LIVE_RECEIPT_MISMATCH";

pub(super) struct AttemptFailure {
    executed: bool,
    reason: &'static str,
}

struct ProcessOutput {
    stdout: Vec<u8>,
}

impl LocalControlAdapter {
    pub(super) fn execute_model_live_check(
        &self,
        request: &AgentCheckRequestV1,
        request_digest: &CanonicalDigest,
    ) -> Result<AgentSurfaceCheckRecordV1, ControlReadError> {
        self.validate_model_check_target(request)?;
        let target = request.target.as_ref().ok_or(ControlReadError::Denied)?;
        let join = self
            .configured_model_settings_join(&target.context_id)?
            .ok_or(ControlReadError::Denied)?;
        let publication = {
            let stores = self.stores_lock().map_err(super::map_port)?;
            let record = stores
                .control()
                .active_publication(&WorkspaceId::default())
                .map_err(super::map_port)?
                .ok_or(ControlReadError::Denied)?;
            if record.publication_revision != target.expected_applied_revision
                || record.digest != join.publication_digest
            {
                return Err(ControlReadError::SnapshotChanged);
            }
            record.verify().map_err(|_| ControlReadError::Corrupt)?
        };
        let snapshot = publication
            .gateway_snapshot()
            .map_err(|_| ControlReadError::Corrupt)?;
        let expected = target
            .client_model_ids
            .iter()
            .map(|model| expected_trust(&snapshot, join.grant.grant_id(), model))
            .collect::<Result<Vec<_>, _>>()?;
        let deadline = Instant::now() + LIVE_CHECK_BUDGET;
        let mut any_executed = false;
        let mut checked_model_ids = Vec::new();
        let mut verified_evidence = Vec::new();
        let mut failure_reason = None;
        let claude = if target.surface == AgentModelSurfaceV2::ClaudeCli {
            let descriptor = self.managed_launch_descriptor(&AgentLaunchDescriptorRequestV1 {
                connection_id: format!("agent-connection/{}", target.context_id),
            })?;
            let trusted_hiroute_executable = self
                .managed_agent_runtime
                .lock()
                .map_err(|_| ControlReadError::Unavailable)?
                .as_ref()
                .map(|runtime| runtime.trusted_hiroute_executable.clone())
                .ok_or(ControlReadError::Unavailable)?;
            Some((descriptor, trusted_hiroute_executable))
        } else {
            None
        };

        for (model, trust) in target.client_model_ids.iter().zip(&expected) {
            let result = match target.surface {
                AgentModelSurfaceV2::PiCli => return Err(ControlReadError::Denied),
                AgentModelSurfaceV2::QoderCli => {
                    self.execute_qoder_live_attempt(&target.context_id, model, trust, deadline)
                }
                AgentModelSurfaceV2::ClaudeCli => {
                    let (descriptor, trusted_hiroute_executable) =
                        claude.as_ref().ok_or(ControlReadError::Corrupt)?;
                    self.execute_claude_live_attempt(
                        descriptor,
                        trusted_hiroute_executable,
                        model,
                        trust,
                        deadline,
                    )
                }
                AgentModelSurfaceV2::CodexCli | AgentModelSurfaceV2::CodexDesktop => self
                    .execute_codex_live_attempt(
                        &target.context_id,
                        target.surface,
                        model,
                        trust,
                        deadline,
                    ),
            };
            match result {
                Ok(evidence) => {
                    any_executed = true;
                    checked_model_ids.push(model.clone());
                    verified_evidence.push(evidence);
                }
                Err(error) => {
                    any_executed |= error.executed;
                    if error.executed {
                        checked_model_ids.push(model.clone());
                    }
                    failure_reason = Some(error.reason);
                    break;
                }
            }
        }
        if !any_executed {
            return Err(ControlReadError::Unavailable);
        }
        // A result that races any settings/publication/grant change is not current evidence and
        // must never reach the surface CAS, including a failure result.
        self.validate_model_check_target(request)?;
        let capability_scope_digest = CanonicalDigest::of(&(
            "hiroute.agent-live-check-evidence/v1",
            target.surface,
            &expected,
            &verified_evidence,
            failure_reason,
        ))
        .map_err(|_| ControlReadError::Corrupt)?;
        let record = AgentSurfaceCheckRecordV1 {
            schema: AGENT_SURFACE_CHECK_SCHEMA.into(),
            context_id: target.context_id.clone(),
            surface: target.surface,
            applied_revision: target.expected_applied_revision,
            state: if failure_reason.is_some() {
                AgentSurfaceCheckStateV1::Failed
            } else {
                AgentSurfaceCheckStateV1::Passed
            },
            checked_model_ids,
            capability_scope_digest,
            check_request_digest: request_digest.clone(),
            reason_code: failure_reason.map(str::to_owned),
        };
        record.validate().map_err(|_| ControlReadError::Corrupt)?;
        Ok(record)
    }

    pub(super) fn save_model_live_check_result(
        &self,
        record: &AgentSurfaceCheckRecordV1,
    ) -> Result<bool, ControlReadError> {
        self.stores_lock()
            .map_err(super::map_port)?
            .control()
            .save_agent_surface_check(&WorkspaceId::default(), record)
            .map_err(super::map_port)
    }

    fn execute_claude_live_attempt(
        &self,
        descriptor: &hiroute_domain::ManagedClaudeLaunchDescriptorV2,
        trusted_hiroute_executable: &str,
        model: &str,
        trust: &FrozenExecutionTrustV1,
        deadline: Instant,
    ) -> Result<CanonicalDigest, AttemptFailure> {
        let arguments = [
            "--print",
            "--output-format",
            "json",
            "--model",
            model,
            "--max-turns",
            "1",
            LIVE_PROMPT,
        ]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
        let mut process = ManagedClaudeProcessV1::prepare(
            descriptor,
            &serde_json::json!({}),
            &arguments,
            trusted_hiroute_executable,
        )
        .map_err(|_| AttemptFailure {
            executed: false,
            reason: CLIENT_FAILED,
        })?;
        let working = tempfile::tempdir().map_err(|_| AttemptFailure {
            executed: false,
            reason: CLIENT_FAILED,
        })?;
        process.command_mut().current_dir(working.path());
        let output = run_bounded(process.command_mut(), deadline)?;
        let identity = parse_claude_output(&output.stdout).ok_or(AttemptFailure {
            executed: true,
            reason: CLIENT_OUTPUT_INVALID,
        })?;
        let response_digest = CanonicalDigest::of(&output.stdout).map_err(|_| AttemptFailure {
            executed: true,
            reason: CLIENT_OUTPUT_INVALID,
        })?;
        self.verify_live_receipt(trust, &[identity], &response_digest, deadline)
    }

    fn execute_codex_live_attempt(
        &self,
        context_id: &str,
        surface: AgentModelSurfaceV2,
        model: &str,
        trust: &FrozenExecutionTrustV1,
        deadline: Instant,
    ) -> Result<CanonicalDigest, AttemptFailure> {
        let executable = self
            .scanner
            .codex_engine_target(surface)
            .ok_or(AttemptFailure {
                executed: false,
                reason: CLIENT_FAILED,
            })?;
        let working = tempfile::tempdir().map_err(|_| AttemptFailure {
            executed: false,
            reason: CLIENT_FAILED,
        })?;
        let mut command = Command::new(executable);
        let root = self.scanner.codex_user_config_target();
        let home = root.parent().ok_or(AttemptFailure {
            executed: false,
            reason: CLIENT_FAILED,
        })?;
        command.env("CODEX_HOME", home);
        if self.settings_agent_for_context(context_id)
            == Some(super::settings_facts::SettingsAgentClass::CodexProfile)
        {
            if surface != AgentModelSurfaceV2::CodexCli {
                return Err(AttemptFailure {
                    executed: false,
                    reason: CLIENT_FAILED,
                });
            }
            command.args([
                "--profile",
                hiroute_integrations::CODEX_MANAGED_PROFILE_NAME,
            ]);
        }
        command.current_dir(working.path()).args([
            "exec",
            "--json",
            "--ephemeral",
            "--sandbox",
            "read-only",
            "--skip-git-repo-check",
            "--model",
            model,
            LIVE_PROMPT,
        ]);
        let output = run_bounded(&mut command, deadline)?;
        let identities = parse_codex_output(&output.stdout).ok_or(AttemptFailure {
            executed: true,
            reason: CLIENT_OUTPUT_INVALID,
        })?;
        let response_digest = CanonicalDigest::of(&output.stdout).map_err(|_| AttemptFailure {
            executed: true,
            reason: CLIENT_OUTPUT_INVALID,
        })?;
        self.verify_live_receipt(trust, &identities, &response_digest, deadline)
    }

    pub(super) fn verify_live_receipt(
        &self,
        expected: &FrozenExecutionTrustV1,
        identities: &[NativeAgentObservationIdentityV1],
        response_digest: &CanonicalDigest,
        deadline: Instant,
    ) -> Result<CanonicalDigest, AttemptFailure> {
        let sessions = identities
            .iter()
            .filter_map(|identity| {
                derive_native_agent_observation_session_id(
                    &WorkspaceId::default(),
                    &self.observation_workspace_key,
                    expected,
                    identity,
                )
                .ok()
            })
            .collect::<BTreeSet<_>>();
        if sessions.is_empty() {
            return Err(AttemptFailure {
                executed: true,
                reason: RECEIPT_MISMATCH,
            });
        }
        let wait_until = deadline.min(Instant::now() + RECEIPT_WAIT_BUDGET);
        loop {
            let mut matching = Vec::new();
            for session in &sessions {
                let detail = match self.delegation_observation.get_session(
                    &WorkspaceId::default(),
                    session,
                    ContentMode::None,
                ) {
                    Ok(detail) => detail,
                    Err(ObservationQueryError::NotFound) => continue,
                    Err(_) => {
                        return Err(AttemptFailure {
                            executed: true,
                            reason: RECEIPT_MISSING,
                        });
                    }
                };
                for receipt_id in detail.turns.iter().flat_map(|turn| turn.receipt_ids.iter()) {
                    let receipt = self
                        .delegation_observation
                        .get_receipt(&WorkspaceId::default(), receipt_id)
                        .map_err(|_| AttemptFailure {
                            executed: true,
                            reason: RECEIPT_MISSING,
                        })?;
                    if &receipt.trust == expected {
                        matching.push(receipt);
                    }
                }
            }
            if matching.len() == 1 {
                let receipt = &matching[0];
                // Identity is established by the unique matching receipt and frozen
                // trust; a failed request is still our connectivity probe. Mark it
                // before separately judging whether the live check succeeded.
                self.delegation_observation
                    .mark_observed_connectivity_probe(&receipt.workspace_id, &receipt.request_id)
                    .map_err(|_| AttemptFailure {
                        executed: true,
                        reason: RECEIPT_MISMATCH,
                    })?;
                if receipt.outcome == RequestOutcome::Accepted
                    && receipt.facts_completeness == FactsCompleteness::Complete
                {
                    return CanonicalDigest::of(&(
                        "hiroute.agent-live-check-receipt/v1",
                        response_digest,
                        identities,
                        receipt,
                    ))
                    .map_err(|_| AttemptFailure {
                        executed: true,
                        reason: RECEIPT_MISMATCH,
                    });
                }
                return Err(AttemptFailure {
                    executed: true,
                    reason: RECEIPT_MISMATCH,
                });
            }
            if matching.len() > 1 {
                return Err(AttemptFailure {
                    executed: true,
                    reason: RECEIPT_MISMATCH,
                });
            }
            if Instant::now() >= wait_until {
                return Err(AttemptFailure {
                    executed: true,
                    reason: RECEIPT_MISSING,
                });
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

fn expected_trust(
    snapshot: &GatewayPublicationSnapshotProjectionV3,
    grant_id: &str,
    served_model_id: &str,
) -> Result<FrozenExecutionTrustV1, ControlReadError> {
    let snapshot: hiroute_gateway::server::publication::GatewayPublicationSnapshotV3 =
        serde_json::from_value(
            serde_json::to_value(snapshot).map_err(|_| ControlReadError::Corrupt)?,
        )
        .map_err(|_| ControlReadError::Corrupt)?;
    snapshot
        .native_agent_execution_trust(grant_id, served_model_id)
        .map_err(|_| ControlReadError::Corrupt)
}

fn run_bounded(command: &mut Command, deadline: Instant) -> Result<ProcessOutput, AttemptFailure> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|_| AttemptFailure {
        executed: false,
        reason: CLIENT_FAILED,
    })?;
    let stdout = child.stdout.take().ok_or(AttemptFailure {
        executed: true,
        reason: CLIENT_FAILED,
    })?;
    let stderr = child.stderr.take().ok_or(AttemptFailure {
        executed: true,
        reason: CLIENT_FAILED,
    })?;
    let stdout_reader = thread::spawn(move || drain_bounded(stdout));
    let stderr_reader = thread::spawn(move || drain_bounded(stderr));
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                terminate_remaining_process_group(child.id());
                break Some(status);
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                terminate_owned_process(&mut child);
                break None;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                terminate_owned_process(&mut child);
                break None;
            }
        }
    };
    let (stdout, stdout_overflow) = stdout_reader.join().unwrap_or_default();
    let (_, stderr_overflow) = stderr_reader.join().unwrap_or_default();
    let Some(status) = status else {
        return Err(AttemptFailure {
            executed: true,
            reason: CHECK_TIMEOUT,
        });
    };
    if !status.success() {
        return Err(AttemptFailure {
            executed: true,
            reason: CLIENT_FAILED,
        });
    }
    if stdout_overflow || stderr_overflow {
        return Err(AttemptFailure {
            executed: true,
            reason: CLIENT_OUTPUT_INVALID,
        });
    }
    Ok(ProcessOutput { stdout })
}

fn drain_bounded(mut reader: impl Read) -> (Vec<u8>, bool) {
    let mut output = Vec::new();
    let mut overflow = false;
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(length) => {
                let remaining = OUTPUT_LIMIT.saturating_sub(output.len());
                output.extend_from_slice(&buffer[..length.min(remaining)]);
                overflow |= length > remaining;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                overflow = true;
                break;
            }
        }
    }
    (output, overflow)
}

fn terminate_owned_process(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let group = nix::unistd::Pid::from_raw(child.id() as i32);
        let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGTERM);
        let until = Instant::now() + Duration::from_millis(500);
        while Instant::now() < until {
            if child.try_wait().ok().flatten().is_some() {
                let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL);
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn terminate_remaining_process_group(pid: u32) {
    #[cfg(unix)]
    {
        let group = nix::unistd::Pid::from_raw(pid as i32);
        let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGTERM);
        thread::sleep(Duration::from_millis(25));
        let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL);
    }
    #[cfg(not(unix))]
    let _ = pid;
}

fn parse_claude_output(bytes: &[u8]) -> Option<NativeAgentObservationIdentityV1> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if value["type"] != "result"
        || value["subtype"] != "success"
        || value["is_error"] != false
        || value["result"].as_str()?.trim() != LIVE_PROMPT_RESPONSE
    {
        return None;
    }
    let session_id = value["session_id"].as_str()?;
    valid_native_identity(session_id).then(|| {
        NativeAgentObservationIdentityV1::ClaudeMetadataSession {
            session_id: session_id.to_owned(),
        }
    })
}

fn parse_codex_output(bytes: &[u8]) -> Option<Vec<NativeAgentObservationIdentityV1>> {
    let mut thread_id = None;
    let mut session_id = None;
    let mut completed = false;
    let mut response = None;
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let value: serde_json::Value = serde_json::from_slice(line).ok()?;
        match value["type"].as_str()? {
            "thread.started" => {
                let observed = value["thread_id"].as_str()?;
                if !valid_native_identity(observed)
                    || thread_id.replace(observed.to_owned()).is_some()
                {
                    return None;
                }
                if let Some(observed) = value["session_id"].as_str() {
                    if !valid_native_identity(observed) {
                        return None;
                    }
                    session_id = Some(observed.to_owned());
                }
            }
            "turn.completed" => completed = true,
            "turn.failed" | "error" => return None,
            "item.completed" if value["item"]["type"] == "agent_message" => {
                let observed = value["item"]["text"].as_str()?;
                if response.replace(observed.to_owned()).is_some() {
                    return None;
                }
            }
            _ => {}
        }
    }
    let thread_id = thread_id?;
    if !completed || response.as_deref()?.trim() != LIVE_PROMPT_RESPONSE {
        return None;
    }
    let mut identities = vec![NativeAgentObservationIdentityV1::CodexThread {
        thread_id: thread_id.clone(),
    }];
    if let Some(session_id) = session_id {
        identities.push(NativeAgentObservationIdentityV1::CodexSession {
            session_id: session_id.clone(),
        });
        identities.push(NativeAgentObservationIdentityV1::CodexThreadSession {
            thread_id,
            session_id,
        });
    } else {
        // Current Codex JSONL exposes the thread ID, while some protocol revisions also send the
        // same value as session metadata. Try only producer-defined identities for that value.
        identities.push(NativeAgentObservationIdentityV1::CodexSession {
            session_id: thread_id.clone(),
        });
        identities.push(NativeAgentObservationIdentityV1::CodexThreadSession {
            session_id: thread_id.clone(),
            thread_id,
        });
    }
    Some(identities)
}

fn valid_native_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_success_parsers_require_terminal_success_and_return_producer_identities() {
        assert!(matches!(
            parse_claude_output(
                br#"{"type":"result","subtype":"success","is_error":false,"result":"HIROUTE_LIVE_CHECK_OK","session_id":"claude-session"}"#
            ),
            Some(NativeAgentObservationIdentityV1::ClaudeMetadataSession { session_id })
                if session_id == "claude-session"
        ));
        assert!(parse_claude_output(
            br#"{"type":"result","subtype":"error","is_error":true,"result":"no","session_id":"claude-session"}"#
        )
        .is_none());
        assert!(parse_claude_output(
            br#"{"type":"result","subtype":"success","is_error":false,"result":"almost","session_id":"claude-session"}"#
        )
        .is_none());

        let identities = parse_codex_output(
            b"{\"type\":\"thread.started\",\"thread_id\":\"thread-one\"}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"HIROUTE_LIVE_CHECK_OK\"}}\n{\"type\":\"turn.completed\"}\n",
        )
        .unwrap();
        assert_eq!(identities.len(), 3);
        assert!(parse_codex_output(
            b"{\"type\":\"thread.started\",\"thread_id\":\"thread-one\"}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"almost\"}}\n{\"type\":\"turn.completed\"}\n"
        )
        .is_none());
        assert!(parse_codex_output(
            b"{\"type\":\"thread.started\",\"thread_id\":\"thread-one\"}\n{\"type\":\"turn.failed\"}\n"
        )
        .is_none());
    }
}
